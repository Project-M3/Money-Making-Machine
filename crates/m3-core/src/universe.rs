use crate::{
    calculator, edgar,
    lake::{
        LakeAnnualRow, LakeCompanyRow, LakeDb, LakePriceDailyRow, LakeTtmRow, QuoteSnapshotUpdate,
        SEC_LAKE_EXTRACTOR_VERSION, SEC_LAKE_EXTRACTOR_VERSION_NUM, SecSnapshotMeta, SyncWatermark,
        UniverseRow, UniverseSummary, WM_SEC_EXTRACTOR_VERSION, WM_SEC_INPUT_GEN,
        is_issuer_coverage_regression, valuation_assumptions_digest,
    },
    market,
    model::{
        Bucket, CompanyEntry, DataSource, MungerReport, PriceSnapshot, RawCompanyFactsSummary,
        ValuationAssumptions, WarningSeverity, WatchlistFile,
    },
    store::{self, WorkspacePaths},
};
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
    thread,
    time::Duration,
};
use zip::ZipArchive;

const COMPANY_TICKERS_EXCHANGE_URL: &str =
    "https://www.sec.gov/files/company_tickers_exchange.json";
const SUBMISSIONS_ZIP_URL: &str =
    "https://www.sec.gov/Archives/edgar/daily-index/bulkdata/submissions.zip";
const COMPANYFACTS_ZIP_URL: &str =
    "https://www.sec.gov/Archives/edgar/daily-index/xbrl/companyfacts.zip";

const TICKERS_EXCHANGE_FILE: &str = "company_tickers_exchange.json";
const SUBMISSIONS_ZIP_FILE: &str = "submissions.zip";
const SIC_MAP_FILE: &str = "sic_map.json";
const COMPANYFACTS_ZIP_FILE: &str = "companyfacts.zip";

const WM_TICKERS_EXCHANGE: &str = "sec_company_tickers_exchange";
const WM_SUBMISSIONS_ZIP: &str = "sec_submissions_zip";
const WM_COMPANYFACTS_ZIP: &str = "sec_companyfacts_zip";
const WM_YAHOO_PRICES: &str = "yahoo_prices";

const CHUNK_SIZE: usize = 32;
const MAX_ZIP_BATCH_BYTES: usize = 32 * 1024 * 1024;
const MAX_SUBMISSION_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FACTS_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const PRICE_CACHE_FRESH_MS: u64 = 6 * 3600 * 1000;
const MAX_FILING_STALENESS_DAYS: i64 = 550;
const MAX_SKIP_SAMPLES: usize = 25;
const MAX_PRICE_ERR_SAMPLES: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseSyncOptions {
    pub full_sec: bool,
    pub prices_top: usize,
    pub skip_prices: bool,
}

impl Default for UniverseSyncOptions {
    fn default() -> Self {
        Self {
            full_sec: false,
            prices_top: 400,
            skip_prices: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkipDiagnostic {
    pub ticker: String,
    pub cik: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniverseSyncStage {
    Index,
    DownloadSec,
    ParseAndCommit,
    Quotes,
    Exporting,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UniverseSyncProgress {
    pub stage: UniverseSyncStage,
    pub fraction: f32,
    pub message: String,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub parsed_companies: Option<usize>,
    pub quotes_done: Option<usize>,
    pub quotes_total: Option<usize>,
    pub lake_ready: bool,
}

impl UniverseSyncProgress {
    fn new(stage: UniverseSyncStage, fraction: f32, message: impl Into<String>) -> Self {
        Self {
            stage,
            fraction: fraction.clamp(0.0, 1.0),
            message: message.into(),
            downloaded_bytes: None,
            total_bytes: None,
            parsed_companies: None,
            quotes_done: None,
            quotes_total: None,
            lake_ready: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UniverseSyncReport {
    pub schema_version: u32,
    pub synced_at_ms: u64,
    pub sec_downloaded: bool,
    pub sec_not_modified: bool,
    pub submissions_downloaded: bool,
    pub total_listed_companies: usize,
    pub matched_companyfacts_entries: usize,
    pub parsed_companies: usize,
    pub skipped_companies: usize,
    pub financial_sector_companies: usize,
    pub non_financial_companies: usize,
    pub quality_pool_count: usize,
    pub prices_attempted: usize,
    pub prices_succeeded: usize,
    pub prices_cached_fallback: usize,
    pub prices_failed: usize,
    pub skip_samples: Vec<SkipDiagnostic>,
    pub price_error_samples: Vec<String>,
    #[serde(default)]
    pub sic_warning: Option<String>,
    pub summary: UniverseSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedCompany {
    pub cik: String,
    pub ticker: String,
    pub name: String,
    pub exchange: String,
    pub is_non_common_listing: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SicInfo {
    pub sic: Option<u32>,
    pub sic_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecInputManifest {
    schema_version: u32,
    generation_id: String,
    generation_dir: String,
    committed_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ResolvedSecInputPaths {
    #[cfg_attr(not(test), allow(dead_code))]
    generation_id: Option<String>,
    tickers_path: PathBuf,
    submissions_zip_path: PathBuf,
    sic_map_path: PathBuf,
    facts_zip_path: PathBuf,
}

#[derive(Default)]
struct PreSyncLakeState {
    tickers_wm: Option<SyncWatermark>,
    submissions_wm: Option<SyncWatermark>,
    facts_wm: Option<SyncWatermark>,
    prev_sec_gen: Option<(u64, u64)>,
    prev_input_gen: Option<String>,
    parsed_financial_companies: usize,
    needs_schema_backfill: bool,
    all_ciks: HashSet<String>,
    parsed_ciks: HashSet<String>,
    prev_ttm_ciks: HashSet<String>,
    classified_sic_ciks: HashSet<String>,
    prev_sic_map: HashMap<String, SicInfo>,
    prev_parsed_rows: HashMap<String, UniverseRow>,
    prev_extractor_versions: HashMap<String, u32>,
    prev_annual_period_ends: HashMap<String, Vec<String>>,
    prev_ttm_period_ends: HashMap<String, String>,
    prev_assumptions_rev: u64,
}

fn read_pre_sync_state(paths: &WorkspacePaths) -> Result<PreSyncLakeState> {
    let db_path = paths.lake_db_path();
    match fs::symlink_metadata(&db_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() && !db_path.exists() {
                anyhow::bail!(
                    "refusing to inspect or create DuckDB through dangling symlink at {}",
                    db_path.display()
                );
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(PreSyncLakeState::default());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "inspect DuckDB lakehouse {} before sync (refusing to treat unreadable lake as empty)",
                    db_path.display()
                )
            });
        }
    }
    let db = LakeDb::open_existing_for_pre_sync(paths)
        .context("open existing DuckDB lakehouse for pre-sync state (refusing to treat unreadable lake as empty)")?
        .with_context(|| {
            format!(
                "existing DuckDB lakehouse {} could not be opened for pre-sync inspection",
                db_path.display()
            )
        })?;
    let tickers_wm = db
        .read_watermark(WM_TICKERS_EXCHANGE)
        .context("read tickers exchange watermark")?;
    let submissions_wm = db
        .read_watermark(WM_SUBMISSIONS_ZIP)
        .context("read submissions zip watermark")?;
    let facts_wm = db
        .read_watermark(WM_COMPANYFACTS_ZIP)
        .context("read companyfacts zip watermark")?;
    let prev_sec_gen = db
        .read_sec_generation()
        .context("read pre-sync SEC lakehouse generation")?;
    let prev_input_gen = db
        .read_sec_input_generation()
        .context("read pre-sync SEC input generation")?;
    let (prev_assumptions_rev, _) = db
        .load_assumptions_state()
        .context("read pre-sync assumptions revision")?;
    let parsed_financial_companies = db
        .universe_summary()
        .context("read pre-sync universe summary")?
        .parsed_financial_companies;
    let needs_schema_backfill = db
        .has_missing_exact_decimals()
        .context("check pre-sync DuckDB schema/extractor version")?;
    let prev = db
        .load_previous_sec_snapshot()
        .context("load previous SEC lakehouse snapshot")?;
    let prev_sic_map = prev
        .sic_map
        .into_iter()
        .map(|(cik, (sic, sic_description))| {
            (
                cik,
                SicInfo {
                    sic,
                    sic_description,
                },
            )
        })
        .collect();
    Ok(PreSyncLakeState {
        tickers_wm,
        submissions_wm,
        facts_wm,
        prev_sec_gen,
        prev_input_gen,
        parsed_financial_companies,
        needs_schema_backfill,
        all_ciks: prev.all_ciks,
        parsed_ciks: prev.parsed_ciks,
        prev_ttm_ciks: prev.ttm_ciks,
        classified_sic_ciks: prev.classified_sic_ciks,
        prev_sic_map,
        prev_parsed_rows: prev.parsed_rows,
        prev_extractor_versions: prev.extractor_versions,
        prev_annual_period_ends: prev.annual_period_ends,
        prev_ttm_period_ends: prev.ttm_period_ends,
        prev_assumptions_rev,
    })
}

fn load_watchlist_strict(paths: &WorkspacePaths) -> Result<Option<WatchlistFile>> {
    if !paths.watchlist_path().exists() {
        return Ok(None);
    }
    let wl = store::load_watchlist(paths).with_context(|| {
        format!(
            "load watchlist at {} (refusing to treat unreadable watchlist as empty)",
            paths.watchlist_path().display()
        )
    })?;
    Ok(Some(wl))
}

struct SecFeedUrls<'a> {
    tickers_exchange_url: &'a str,
    submissions_zip_url: &'a str,
    companyfacts_zip_url: &'a str,
}

impl<'a> SecFeedUrls<'a> {
    const LIVE: Self = Self {
        tickers_exchange_url: COMPANY_TICKERS_EXCHANGE_URL,
        submissions_zip_url: SUBMISSIONS_ZIP_URL,
        companyfacts_zip_url: COMPANYFACTS_ZIP_URL,
    };
}

#[allow(dead_code)]
pub fn sync_universe(
    paths: &WorkspacePaths,
    opts: &UniverseSyncOptions,
) -> Result<UniverseSyncReport> {
    sync_universe_with_progress(paths, opts, |_| {})
}

pub fn sync_universe_with_progress<F: FnMut(&str)>(
    paths: &WorkspacePaths,
    opts: &UniverseSyncOptions,
    mut on_progress: F,
) -> Result<UniverseSyncReport> {
    sync_universe_with_structured_progress(paths, opts, |prog| on_progress(&prog.message))
}

/// Syncs the US L0 lakehouse while streaming structured stage, byte, and readiness updates.
pub fn sync_universe_with_structured_progress<F: FnMut(UniverseSyncProgress)>(
    paths: &WorkspacePaths,
    opts: &UniverseSyncOptions,
    on_progress: F,
) -> Result<UniverseSyncReport> {
    sync_universe_with_sec_urls_structured(paths, opts, &SecFeedUrls::LIVE, on_progress)
}

#[cfg(test)]
fn sync_universe_with_sec_urls<F: FnMut(&str)>(
    paths: &WorkspacePaths,
    opts: &UniverseSyncOptions,
    sec_urls: &SecFeedUrls<'_>,
    mut on_progress: F,
) -> Result<UniverseSyncReport> {
    sync_universe_with_sec_urls_structured(paths, opts, sec_urls, |prog| on_progress(&prog.message))
}

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn parse_progress_ratio(msg: &str) -> Option<(usize, usize)> {
    let start = msg.rfind('(')?;
    let end = msg[start + 1..].find(')')?;
    let inside = &msg[start + 1..start + 1 + end];
    let (done_s, tot_s) = inside.split_once('/')?;
    let done = done_s.trim().parse::<usize>().ok()?;
    let total = tot_s.trim().parse::<usize>().ok()?;
    (total > 0).then_some((done, total))
}

fn sync_universe_with_sec_urls_structured<F: FnMut(UniverseSyncProgress)>(
    paths: &WorkspacePaths,
    opts: &UniverseSyncOptions,
    sec_urls: &SecFeedUrls<'_>,
    mut on_progress: F,
) -> Result<UniverseSyncReport> {
    let _sec_sync_lock = acquire_sec_sync_lock(paths)?;
    let now = store::now_ms();
    let pre_state = read_pre_sync_state(paths)?;
    // Never prune older gen-* directories before asset validation & repair commit.
    let active_paths = reconcile_sec_input_generation_on_disk_for_sync_locked(
        paths,
        pre_state.prev_input_gen.as_deref(),
        now,
    )?;
    let watchlist = load_watchlist_strict(paths)?;
    let watchlist_ctx = WatchlistLookup::from_opt(watchlist.as_ref());

    let mut max_fraction = 0.0_f32;
    let mut on_progress = move |mut prog: UniverseSyncProgress| {
        prog.fraction = prog.fraction.clamp(0.0, 1.0).max(max_fraction);
        max_fraction = prog.fraction;
        on_progress(prog);
    };

    on_progress(UniverseSyncProgress::new(
        UniverseSyncStage::Index,
        0.02,
        "下载/校验 SEC 上市公司索引...",
    ));
    let meta_client = sec_http_client_with_timeout(60)?;
    let bulk_client = sec_http_client_with_timeout(600)?;

    let mut pending_sec_watermarks: Vec<SyncWatermark> = Vec::new();

    let tickers_valid = is_valid_tickers_exchange_file(
        &active_paths.tickers_path,
        &pre_state.all_ciks,
        pre_state
            .tickers_wm
            .as_ref()
            .and_then(|w| w.cursor.as_deref()),
    );
    let staged_tickers = fetch_sec_file_staged(
        &meta_client,
        &ConditionalFetchSpec {
            url: sec_urls.tickers_exchange_url,
            dest_path: &active_paths.tickers_path,
            watermark_key: WM_TICKERS_EXCHANGE,
            cached_wm: pre_state.tickers_wm.as_ref(),
            now_ms: now,
            force_download: !tickers_valid,
            record_integrity_cursor: true,
        },
        |tmp_path| validate_tickers_exchange_file(tmp_path, &pre_state.all_ciks),
    )?;
    let tickers_downloaded = matches!(staged_tickers, StagedSecFetch::Downloaded { .. });
    if let StagedSecFetch::Downloaded { ref watermark, .. } = staged_tickers {
        pending_sec_watermarks.push(watermark.clone());
    }

    let active_tickers_path = staged_tickers.active_path(&active_paths.tickers_path);
    validate_tickers_exchange_file(active_tickers_path, &pre_state.all_ciks)?;
    let tickers_bytes = fs::read(active_tickers_path)
        .with_context(|| format!("read {}", active_tickers_path.display()))?;
    let listed = parse_canonical_tickers_exchange(&tickers_bytes, &watchlist_ctx.cik_to_ticker)?;
    ensure!(
        !listed.is_empty(),
        "SEC company_tickers_exchange.json produced zero Nasdaq/NYSE/CBOE companies"
    );

    on_progress(UniverseSyncProgress::new(
        UniverseSyncStage::Index,
        0.04,
        "获取/读取 SEC SIC 行业分类...",
    ));
    let listed_ciks: HashSet<String> = listed.iter().map(|c| c.cik.clone()).collect();
    let expected_sic_active: HashSet<String> = pre_state
        .classified_sic_ciks
        .intersection(&listed_ciks)
        .cloned()
        .collect();
    let expected_parsed_active: HashSet<String> = pre_state
        .parsed_ciks
        .intersection(&listed_ciks)
        .cloned()
        .collect();
    let SicLoadOutcome {
        map: sic_map,
        downloaded: submissions_downloaded,
        watermark: sub_wm_opt,
        warning: sic_warning,
        staged_submissions,
    } = load_or_fetch_sic_map_with_url_and_progress(
        sec_urls.submissions_zip_url,
        &active_paths.submissions_zip_path,
        &active_paths.sic_map_path,
        &bulk_client,
        pre_state.submissions_wm.as_ref(),
        &listed_ciks,
        &expected_sic_active,
        &pre_state.prev_sic_map,
        tickers_downloaded,
        now,
        &mut |dl, total| {
            let ratio = total
                .filter(|&t| t > 0)
                .map(|t| (dl as f32 / t as f32).clamp(0.0, 1.0))
                .unwrap_or(0.5);
            let msg = match total.filter(|&t| t > 0) {
                Some(t) => format!(
                    "下载 SEC SIC 索引（{} / {}）...",
                    format_mb(dl),
                    format_mb(t)
                ),
                None => format!("下载 SEC SIC 索引（{}）...", format_mb(dl)),
            };
            let mut prog =
                UniverseSyncProgress::new(UniverseSyncStage::Index, 0.04 + 0.08 * ratio, msg);
            prog.downloaded_bytes = Some(dl);
            prog.total_bytes = total;
            on_progress(prog);
        },
    )?;
    if let Some(wm) = sub_wm_opt {
        pending_sec_watermarks.push(wm);
    }

    on_progress(UniverseSyncProgress::new(
        UniverseSyncStage::DownloadSec,
        0.12,
        "检查 SEC companyfacts.zip 批量包...",
    ));
    let facts_cursor_ok = pre_state
        .facts_wm
        .as_ref()
        .and_then(|w| w.cursor.as_deref())
        .is_some_and(|c| {
            c.starts_with("len=") && verify_file_integrity_cursor(&active_paths.facts_zip_path, c)
        });
    let facts_zip_valid = active_paths.facts_zip_path.is_file()
        && facts_cursor_ok
        && is_valid_zip_file(&active_paths.facts_zip_path);
    let staged_facts = fetch_sec_file_staged_with_progress(
        &bulk_client,
        &ConditionalFetchSpec {
            url: sec_urls.companyfacts_zip_url,
            dest_path: &active_paths.facts_zip_path,
            watermark_key: WM_COMPANYFACTS_ZIP,
            cached_wm: pre_state.facts_wm.as_ref(),
            now_ms: now,
            force_download: !facts_zip_valid,
            record_integrity_cursor: true,
        },
        |tmp_path| validate_companyfacts_zip_tmp(tmp_path, &listed_ciks, &expected_parsed_active),
        &mut |dl, total| {
            let ratio = total
                .filter(|&t| t > 0)
                .map(|t| (dl as f32 / t as f32).clamp(0.0, 1.0))
                .unwrap_or(0.5);
            let msg = match total.filter(|&t| t > 0) {
                Some(t) => format!(
                    "下载 SEC companyfacts.zip（{} / {}）...",
                    format_mb(dl),
                    format_mb(t)
                ),
                None => format!("下载 SEC companyfacts.zip（{}）...", format_mb(dl)),
            };
            let mut prog =
                UniverseSyncProgress::new(UniverseSyncStage::DownloadSec, 0.12 + 0.36 * ratio, msg);
            prog.downloaded_bytes = Some(dl);
            prog.total_bytes = total;
            on_progress(prog);
        },
    )?;
    let sec_downloaded = matches!(staged_facts, StagedSecFetch::Downloaded { .. });
    let sec_not_modified = matches!(staged_facts, StagedSecFetch::NotModified);
    if let StagedSecFetch::Downloaded { ref watermark, .. } = staged_facts {
        pending_sec_watermarks.push(watermark.clone());
    }

    let ticker_changed_since_prev = listed.iter().any(|co| {
        pre_state
            .prev_parsed_rows
            .get(&co.cik)
            .is_some_and(|prev| !prev.ticker.eq_ignore_ascii_case(&co.ticker))
    });
    let need_sec_parse = sec_downloaded
        || tickers_downloaded
        || submissions_downloaded
        || ticker_changed_since_prev
        || opts.full_sec
        || pre_state.parsed_financial_companies == 0
        || pre_state.needs_schema_backfill;

    let mut report = UniverseSyncReport {
        schema_version: 1,
        synced_at_ms: now,
        sec_downloaded,
        sec_not_modified,
        submissions_downloaded,
        total_listed_companies: listed.len(),
        sic_warning,
        ..Default::default()
    };

    let cached_prices: HashMap<String, PriceSnapshot> = if need_sec_parse {
        listed
            .iter()
            .filter_map(|co| {
                store::load_cached_price(paths, &co.ticker)
                    .filter(|p| {
                        p.symbol.eq_ignore_ascii_case(&co.ticker)
                            && market::validate_quote_session(p, now).is_ok()
                    })
                    .map(|p| (co.ticker.clone(), p))
            })
            .collect()
    } else {
        HashMap::new()
    };

    let parsed_bundle_opt: Option<ParsedBundle> = if need_sec_parse {
        on_progress(UniverseSyncProgress::new(
            UniverseSyncStage::ParseAndCommit,
            0.48,
            "流式解压并并行计算全美股 XBRL 财务指标...",
        ));
        let active_facts_path = staged_facts.active_path(&active_paths.facts_zip_path);
        let mut parse_progress_adapter = |msg: &str| {
            let mut prog = if let Some((done, total)) = parse_progress_ratio(msg) {
                let frac = 0.48 + 0.20 * (done as f32 / total.max(1) as f32);
                let mut p = UniverseSyncProgress::new(UniverseSyncStage::ParseAndCommit, frac, msg);
                p.parsed_companies = Some(done);
                p
            } else {
                UniverseSyncProgress::new(UniverseSyncStage::ParseAndCommit, 0.48, msg)
            };
            prog.lake_ready = false;
            on_progress(prog);
        };
        let parsed_bundle = parse_companyfacts_zip_streaming(
            active_facts_path,
            &listed,
            &sic_map,
            &watchlist_ctx,
            &cached_prices,
            now,
            &mut parse_progress_adapter,
        )?;
        validate_parsed_bundle_coverage(
            &parsed_bundle.snapshot,
            &parsed_bundle.annuals,
            &parsed_bundle.ttms,
            &pre_state,
            &expected_parsed_active,
        )?;
        Some(parsed_bundle)
    } else {
        None
    };

    // Identify issuers needing preserved facts/provenance (failed/skipped OR period-key regression)
    // before ranking Scheme 1 pool flags; exact `(CIK, SHA-256)` bytes are also verified and staged
    // inside DuckDB pre_commit before active archive replacement or DB commit.
    let mut new_annual_ends_by_cik: HashMap<&str, Vec<String>> = HashMap::new();
    if let Some(b) = parsed_bundle_opt.as_ref() {
        for a in &b.annuals {
            new_annual_ends_by_cik
                .entry(a.cik.as_str())
                .or_default()
                .push(a.period_end.clone());
        }
    }
    let mut new_ttm_end_by_cik: HashMap<&str, &str> = HashMap::new();
    if let Some(b) = parsed_bundle_opt.as_ref() {
        for t in &b.ttms {
            new_ttm_end_by_cik.insert(t.cik.as_str(), t.period_end.as_str());
        }
    }
    let parsed_fail_ciks: HashSet<String> = parsed_bundle_opt
        .as_ref()
        .map(|b| {
            b.snapshot
                .iter()
                .filter_map(|u| {
                    let prev_row = pre_state.prev_parsed_rows.get(&u.cik)?;
                    let is_unparsed = u.years_of_data == 0 && u.raw_sha256.is_none();
                    let regressed =
                        if let Some(prev_ends) = pre_state.prev_annual_period_ends.get(&u.cik) {
                            let new_ends = new_annual_ends_by_cik
                                .get(u.cik.as_str())
                                .map(Vec::as_slice)
                                .unwrap_or(&[]);
                            edgar::is_annual_period_coverage_regression(
                                prev_ends,
                                pre_state
                                    .prev_ttm_period_ends
                                    .get(&u.cik)
                                    .map(String::as_str),
                                new_ends,
                                new_ttm_end_by_cik.get(u.cik.as_str()).copied(),
                                u.latest_filing_end.as_deref(),
                            )
                        } else {
                            is_issuer_coverage_regression(
                                prev_row.years_of_data,
                                pre_state.prev_ttm_ciks.contains(&u.cik),
                                prev_row.latest_filing_end.as_deref(),
                                u.years_of_data,
                                new_ttm_end_by_cik.contains_key(u.cik.as_str()),
                                u.latest_filing_end.as_deref(),
                            )
                        };
                    (is_unparsed || regressed).then(|| u.cik.clone())
                })
                .collect()
        })
        .unwrap_or_default();
    let preserved_fail_ciks = &parsed_fail_ciks;

    let price_sync_inputs = {
        let mut db = LakeDb::open(paths)?;
        if let Some(parsed_bundle) = parsed_bundle_opt {
            report.matched_companyfacts_entries = parsed_bundle.matched_entries;
            report.parsed_companies = parsed_bundle.parsed_companies;
            report.skipped_companies = parsed_bundle.skipped_companies;
            report.skip_samples = parsed_bundle.skip_samples;

            let default_dig = valuation_assumptions_digest(&default_assumptions());
            let mut snapshot = parsed_bundle.snapshot;
            let mut preserved_quality_overrides: HashMap<String, Decimal> = HashMap::new();
            // Rank effective merged rows once (preserved previous metrics + newly parsed) before
            // applying Scheme 1 pool flags and clearing out-of-pool quotes atomically in DuckDB.
            let mut effective_snapshot: Vec<UniverseRow> = snapshot
                .iter()
                .map(|row| {
                    if preserved_fail_ciks.contains(&row.cik)
                        && let Some(prev_row) = pre_state.prev_parsed_rows.get(&row.cik)
                    {
                        let mut merged = prev_row.clone();
                        let ticker_changed = !prev_row.ticker.eq_ignore_ascii_case(&row.ticker);
                        let prev_had_mismatch = row_has_share_basis_mismatch(prev_row);
                        let (warn_cnt, warn_json, has_share_mismatch) =
                            crate::lake::reconcile_preserved_warnings_json(
                                &prev_row.warnings_json,
                                &row.warnings_json,
                                &row.ticker,
                                !ticker_changed,
                                row.sic.is_none(),
                            );
                        merged.ticker = row.ticker.clone();
                        merged.name = row.name.clone();
                        merged.exchange = row.exchange.clone();
                        merged.sic = row.sic;
                        merged.sic_description = row.sic_description.clone();
                        merged.is_financial = row.is_financial;
                        merged.in_watchlist = row.in_watchlist;
                        merged.bucket_opt = row.bucket_opt;
                        merged.warnings_count = warn_cnt;
                        merged.warnings_json = warn_json;
                        if row.is_financial {
                            merged.quality_score = Decimal::ZERO;
                        } else if !prev_had_mismatch && has_share_mismatch {
                            merged.quality_score = (prev_row.quality_score - Decimal::from(3))
                                .max(Decimal::ZERO)
                                .round_dp(2);
                        } else if prev_had_mismatch && !has_share_mismatch && !ticker_changed {
                            merged.quality_score = prev_row.quality_score;
                        }
                        if ticker_changed || has_share_mismatch {
                            merged.strike_price = None;
                            merged.distance_to_strike_pct = None;
                            merged.strict_fcf_yield_pct = None;
                        }
                        if ticker_changed {
                            merged.latest_price = None;
                        }
                        preserved_quality_overrides
                            .insert(merged.cik.clone(), merged.quality_score);
                        merged
                    } else {
                        row.clone()
                    }
                })
                .collect();
            let mut pool_count =
                assign_scheme1_quality_pool(&mut effective_snapshot, opts.prices_top, now);
            for (staged_row, eff_row) in snapshot.iter_mut().zip(effective_snapshot.iter()) {
                staged_row.in_price_pool = eff_row.in_price_pool;
                if preserved_fail_ciks.contains(&staged_row.cik) {
                    staged_row.warnings_count = eff_row.warnings_count;
                    staged_row.warnings_json = eff_row.warnings_json.clone();
                    staged_row.quality_score = eff_row.quality_score;
                }
            }
            clear_out_of_pool_quote_fields(&mut snapshot);

            let mut snapshot_meta_by_cik: HashMap<String, SecSnapshotMeta> =
                HashMap::with_capacity(snapshot.len());
            for r in &snapshot {
                let assump_opt = watchlist_ctx
                    .find(&r.ticker, &r.cik)
                    .map(|w| w.assumptions.clone());
                let assumptions_dig = assump_opt
                    .as_ref()
                    .map(valuation_assumptions_digest)
                    .unwrap_or_else(|| default_dig.clone());
                let cached_p = if r.in_price_pool && r.latest_price.is_some() {
                    cached_prices.get(&r.ticker)
                } else {
                    None
                };
                let ext_ver = if preserved_fail_ciks.contains(&r.cik) {
                    pre_state
                        .prev_extractor_versions
                        .get(&r.cik)
                        .copied()
                        .unwrap_or(0)
                } else if r.years_of_data > 0 && r.raw_sha256.is_some() {
                    SEC_LAKE_EXTRACTOR_VERSION_NUM
                } else {
                    0
                };
                snapshot_meta_by_cik.insert(
                    r.cik.clone(),
                    SecSnapshotMeta {
                        quote_market_time_ms: cached_p.and_then(|p| p.market_time_ms),
                        quote_fetched_at_ms: cached_p.map(|p| p.fetched_at_ms),
                        assumptions_digest: Some(assumptions_dig),
                        assumptions: assump_opt,
                        extractor_version: ext_ver,
                        quality_score_override: preserved_quality_overrides.get(&r.cik).copied(),
                    },
                );
            }

            let all_extractors_current = !preserved_fail_ciks.iter().any(|cik| {
                pre_state
                    .prev_extractor_versions
                    .get(cik)
                    .copied()
                    .unwrap_or(0)
                    < SEC_LAKE_EXTRACTOR_VERSION_NUM
            });
            if all_extractors_current {
                pending_sec_watermarks.push(SyncWatermark {
                    source_key: WM_SEC_EXTRACTOR_VERSION.to_owned(),
                    cursor: Some(SEC_LAKE_EXTRACTOR_VERSION.to_owned()),
                    etag: None,
                    last_modified: None,
                    synced_at_ms: now,
                });
            }

            report.financial_sector_companies =
                effective_snapshot.iter().filter(|r| r.is_financial).count();
            report.non_financial_companies = effective_snapshot
                .iter()
                .filter(|r| !r.is_financial)
                .count();

            on_progress(UniverseSyncProgress::new(
                UniverseSyncStage::ParseAndCommit,
                0.70,
                "写入 DuckDB 湖仓...",
            ));
            let commit_now = store::now_ms().max(now);
            let latest_watchlist = load_watchlist_strict(paths)?;
            let post_parse_watchlist =
                (latest_watchlist != watchlist).then_some(latest_watchlist.as_ref());
            if post_parse_watchlist.is_some() {
                let latest_ctx = WatchlistLookup::from_opt(latest_watchlist.as_ref());
                for r in &mut effective_snapshot {
                    let wl_entry = latest_ctx.find(&r.ticker, &r.cik);
                    r.in_watchlist = wl_entry.is_some();
                    r.bucket_opt = wl_entry.map(|w| w.bucket);
                }
                pool_count = assign_scheme1_quality_pool(
                    &mut effective_snapshot,
                    opts.prices_top,
                    commit_now,
                );
                for (staged_row, eff_row) in snapshot.iter_mut().zip(effective_snapshot.iter()) {
                    staged_row.in_price_pool = eff_row.in_price_pool;
                    staged_row.in_watchlist = eff_row.in_watchlist;
                    staged_row.bucket_opt = eff_row.bucket_opt;
                    if !staged_row.in_price_pool
                        && let Some(meta) = snapshot_meta_by_cik.get_mut(&staged_row.cik)
                    {
                        meta.quote_market_time_ms = None;
                        meta.quote_fetched_at_ms = None;
                    }
                }
                clear_out_of_pool_quote_fields(&mut snapshot);
            }
            report.quality_pool_count = pool_count;
            let mut refreshed_disk_quotes: HashMap<String, PriceSnapshot> = HashMap::new();
            for r in &effective_snapshot {
                if !r.in_price_pool || row_has_share_basis_mismatch(r) {
                    continue;
                }
                if let Some(p) = store::load_cached_price(paths, &r.ticker).filter(|p| {
                    p.symbol.eq_ignore_ascii_case(&r.ticker)
                        && market::validate_quote_session(p, commit_now).is_ok()
                }) {
                    refreshed_disk_quotes.insert(r.cik.clone(), p);
                }
            }
            let next_input_gen_id = format!(
                "gen-{:020}-{commit_now}-{}",
                pre_state.prev_sec_gen.map_or(1, |(g, _)| g + 1),
                std::process::id()
            );
            pending_sec_watermarks.push(SyncWatermark {
                source_key: WM_SEC_INPUT_GEN.to_owned(),
                cursor: Some(next_input_gen_id.clone()),
                etag: None,
                last_modified: None,
                synced_at_ms: commit_now,
            });
            let mut gen_guard = StagedSecGenerationGuard::new(
                paths,
                next_input_gen_id,
                staged_tickers,
                staged_submissions,
                staged_facts,
                sic_map.clone(),
                active_paths.clone(),
            );
            let mut committed_keep_preserved: HashMap<String, Vec<Option<String>>> = HashMap::new();
            let expectations = crate::lake::SecGenerationExpectations {
                expected_prev_sec_gen: Some(pre_state.prev_sec_gen),
                expected_prev_assumptions_rev: Some(pre_state.prev_assumptions_rev),
                batch_synced_at_ms: commit_now,
                post_parse_watchlist,
                recompute_prices_top: Some(opts.prices_top),
                refreshed_disk_quotes: &refreshed_disk_quotes,
            };
            db.replace_sec_lake_generation_with_meta_and_commit_hook(
                &parsed_bundle.companies,
                &parsed_bundle.annuals,
                &parsed_bundle.ttms,
                &snapshot,
                &snapshot_meta_by_cik,
                &pending_sec_watermarks,
                &expectations,
                |actual_preserved_with_sha| {
                    stage_preserved_raw_issuer_bytes(
                        paths,
                        &active_paths.facts_zip_path,
                        actual_preserved_with_sha,
                    )?;
                    committed_keep_preserved = actual_preserved_with_sha.clone();
                    gen_guard.promote_before_commit()
                },
            )?;
            gen_guard.finalize(paths, commit_now)?;
            prune_preserved_raw_issuer_bytes(paths, &committed_keep_preserved)?;
            if post_parse_watchlist.is_some() {
                let updated = db.load_all_universe_rows()?;
                report.quality_pool_count = updated.iter().filter(|r| r.in_price_pool).count();
            }
            if opts.skip_prices {
                let mut prog = UniverseSyncProgress::new(
                    UniverseSyncStage::Exporting,
                    0.96,
                    "原子导出 ZSTD Parquet 视图...",
                );
                prog.lake_ready = true;
                prog.parsed_companies = Some(report.parsed_companies);
                on_progress(prog);
                db.export_all_parquets(paths)?;
                report.summary = db.universe_summary()?;
                None
            } else {
                let live_wl = load_watchlist_strict(paths)?;
                if live_wl != latest_watchlist {
                    let empty_wl = WatchlistFile {
                        schema_version: 1,
                        benchmark_symbol: "^GSPC".to_owned(),
                        default_target_return_pct: Decimal::from(10),
                        companies: Vec::new(),
                    };
                    db.reconcile_watchlist_and_pool(
                        live_wl.as_ref().unwrap_or(&empty_wl),
                        opts.prices_top,
                        store::now_ms().max(commit_now),
                    )?;
                }
                let live_wl_ctx = WatchlistLookup::from_opt(live_wl.as_ref());
                let rows = db.load_all_universe_rows()?;
                report.quality_pool_count = rows.iter().filter(|r| r.in_price_pool).count();
                let basis_map = db.load_latest_valuation_basis()?;
                let (_, assump_state) = db.load_assumptions_state()?;
                let assump_revs: HashMap<String, u64> = assump_state
                    .into_iter()
                    .map(|(cik, (_, rev))| (cik, rev))
                    .collect();
                Some((live_wl, live_wl_ctx, rows, basis_map, assump_revs))
            }
        } else {
            on_progress(UniverseSyncProgress::new(
                UniverseSyncStage::ParseAndCommit,
                0.68,
                "SEC 数据未变更，刷新观察池标记与方案 1 候选池...",
            ));
            let refresh_wl = load_watchlist_strict(paths)?;
            db.refresh_watchlist_flags(paths)?;
            let mut snapshot = db.load_all_universe_rows()?;
            let pool_count = assign_scheme1_quality_pool(&mut snapshot, opts.prices_top, now);
            let pool_ciks: HashSet<String> = snapshot
                .iter()
                .filter(|r| r.in_price_pool)
                .map(|r| r.cik.clone())
                .collect();
            db.sync_price_pool_flags_checked(&pool_ciks, Some(pre_state.prev_sec_gen))?;
            if let Some(gen_id) = active_paths.generation_id.as_deref() {
                let gens_dir = paths.lake_raw_sec_generations_dir();
                let gen_dir =
                    ensure_valid_sec_gen_dir(&gens_dir, gen_id, "active SEC input generation")?;
                if read_manifest_gen_id(paths)?.as_deref() != Some(gen_id) {
                    publish_sec_input_manifest_and_links(paths, gen_id, &gen_dir, now)?;
                }
                cleanup_sec_input_generations(paths, &gens_dir, &[gen_id]);
            }
            report.quality_pool_count = pool_count;
            report.parsed_companies = snapshot.iter().filter(|r| r.years_of_data > 0).count();
            report.financial_sector_companies = snapshot.iter().filter(|r| r.is_financial).count();
            report.non_financial_companies = snapshot.iter().filter(|r| !r.is_financial).count();
            if opts.skip_prices {
                let mut prog = UniverseSyncProgress::new(
                    UniverseSyncStage::Exporting,
                    0.96,
                    "原子导出 ZSTD Parquet 视图...",
                );
                prog.lake_ready = true;
                prog.parsed_companies = Some(report.parsed_companies);
                on_progress(prog);
                db.export_all_parquets(paths)?;
                report.summary = db.universe_summary()?;
                None
            } else {
                let live_wl = load_watchlist_strict(paths)?;
                if live_wl != refresh_wl {
                    let empty_wl = WatchlistFile {
                        schema_version: 1,
                        benchmark_symbol: "^GSPC".to_owned(),
                        default_target_return_pct: Decimal::from(10),
                        companies: Vec::new(),
                    };
                    db.reconcile_watchlist_and_pool(
                        live_wl.as_ref().unwrap_or(&empty_wl),
                        opts.prices_top,
                        store::now_ms().max(now),
                    )?;
                }
                let live_wl_ctx = WatchlistLookup::from_opt(live_wl.as_ref());
                let rows = db.load_all_universe_rows()?;
                report.quality_pool_count = rows.iter().filter(|r| r.in_price_pool).count();
                let basis_map = db.load_latest_valuation_basis()?;
                let (_, assump_state) = db.load_assumptions_state()?;
                let assump_revs: HashMap<String, u64> = assump_state
                    .into_iter()
                    .map(|(cik, (_, rev))| (cik, rev))
                    .collect();
                Some((live_wl, live_wl_ctx, rows, basis_map, assump_revs))
            }
        }
    };

    if let Some((live_wl, live_wl_ctx, rows, basis_map, assump_revs)) = price_sync_inputs {
        let total_pool = report.quality_pool_count;
        let parsed_companies = report.parsed_companies;
        let mut ready_prog = UniverseSyncProgress::new(
            UniverseSyncStage::Quotes,
            0.72,
            format!(
                "全美股 L0 基础库已就绪（{parsed_companies} 家），正在补充 Top {total_pool} 候选池报价…"
            ),
        );
        ready_prog.lake_ready = true;
        ready_prog.parsed_companies = Some(parsed_companies);
        ready_prog.quotes_done = Some(0);
        ready_prog.quotes_total = Some(total_pool);
        on_progress(ready_prog);

        let pool_batch = fetch_pool_prices_lock_free_with_progress(
            paths,
            &rows,
            live_wl.as_ref(),
            &live_wl_ctx,
            &basis_map,
            &assump_revs,
            opts.full_sec,
            now,
            &mut report,
            &mut |done, total| {
                let ratio = (done as f32 / total.max(1) as f32).clamp(0.0, 1.0);
                let mut q_prog = UniverseSyncProgress::new(
                    UniverseSyncStage::Quotes,
                    0.72 + 0.23 * ratio,
                    format!("补充候选池与观察池报价 ({done}/{total})…"),
                );
                q_prog.lake_ready = true;
                q_prog.parsed_companies = Some(parsed_companies);
                q_prog.quotes_done = Some(done);
                q_prog.quotes_total = Some(total);
                on_progress(q_prog);
            },
        )?;

        let mut db = LakeDb::open(paths)?;
        let commit = crate::lake::PriceSyncCommit {
            prices: &pool_batch.price_rows,
            updated_rows: &pool_batch.updated_snapshot_rows,
            expected_assumptions_digests: &pool_batch.expected_assumptions_digests,
            expected_assumptions_revs: &pool_batch.expected_assumptions_revs,
            watermarks: &pool_batch.price_watermarks,
        };
        db.sync_watchlist_and_upsert_prices(
            paths,
            pool_batch.live_watchlist.as_ref(),
            opts.prices_top,
            &commit,
        )?;
        report.quality_pool_count = db
            .load_all_universe_rows()?
            .iter()
            .filter(|r| r.in_price_pool)
            .count();
        let mut exp_prog = UniverseSyncProgress::new(
            UniverseSyncStage::Exporting,
            0.96,
            "原子导出 ZSTD Parquet 视图...",
        );
        exp_prog.lake_ready = true;
        exp_prog.parsed_companies = Some(parsed_companies);
        on_progress(exp_prog);
        db.export_all_parquets(paths)?;
        report.summary = db.universe_summary()?;
    }

    Ok(report)
}

fn sec_http_client_with_timeout(timeout_secs: u64) -> Result<Client> {
    Client::builder()
        .user_agent(edgar::sec_user_agent())
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("failed to build SEC HTTP client")
}

struct ConditionalFetchSpec<'a> {
    url: &'a str,
    dest_path: &'a Path,
    watermark_key: &'a str,
    cached_wm: Option<&'a SyncWatermark>,
    now_ms: u64,
    force_download: bool,
    record_integrity_cursor: bool,
}

struct StagedTempFile(Option<PathBuf>);

impl StagedTempFile {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn as_path(&self) -> Option<&Path> {
        self.0.as_deref()
    }
}

impl Drop for StagedTempFile {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = fs::remove_file(p);
        }
    }
}

enum StagedSecFetch {
    NotModified,
    Unavailable,
    Downloaded {
        tmp_file: StagedTempFile,
        watermark: SyncWatermark,
    },
}

impl StagedSecFetch {
    fn active_path<'a>(&'a self, dest_path: &'a Path) -> &'a Path {
        match self {
            Self::Downloaded { tmp_file, .. } => tmp_file.as_path().unwrap_or(dest_path),
            Self::NotModified | Self::Unavailable => dest_path,
        }
    }
}

fn link_or_copy_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::hard_link(src, dst).is_err() {
        fs::copy(src, dst)
            .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
    }
    Ok(())
}

fn same_inode_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) {
            return ma.is_file() && mb.is_file() && ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    let _ = (a, b);
    false
}

fn atomic_link_or_copy_replace(src: &Path, dest: &Path) -> Result<()> {
    if same_inode_file(src, dest) {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = store::atomic_temp_path(dest);
    let _ = fs::remove_file(&tmp);
    if let Err(err) = link_or_copy_file(src, &tmp).and_then(|_| {
        fs::rename(&tmp, dest).with_context(|| {
            format!(
                "atomic link replace {} -> {}",
                src.display(),
                dest.display()
            )
        })
    }) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

fn sec_sync_process_mutex_for(dir: &Path) -> &'static Mutex<()> {
    static MAP: OnceLock<Mutex<HashMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();
    let map = MAP.get_or_init(|| Mutex::new(HashMap::new()));
    let key = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(key)
        .or_insert_with(|| Box::leak(Box::new(Mutex::new(()))))
}

#[allow(dead_code)]
struct SecSyncLockGuard {
    _file_lock: File,
    _thread_guard: MutexGuard<'static, ()>,
}

fn acquire_sec_sync_lock(paths: &WorkspacePaths) -> Result<SecSyncLockGuard> {
    let raw_sec_dir = paths.lake_raw_sec_dir();
    fs::create_dir_all(&raw_sec_dir)
        .with_context(|| format!("create SEC raw dir {} for sync lock", raw_sec_dir.display()))?;
    let proc_mtx = sec_sync_process_mutex_for(&raw_sec_dir);
    let thread_guard = proc_mtx.lock().unwrap_or_else(|e| e.into_inner());
    let lock_path = raw_sec_dir.join(".sync.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("open SEC sync lock {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("acquire exclusive SEC sync lock {}", lock_path.display()))?;
    Ok(SecSyncLockGuard {
        _file_lock: file,
        _thread_guard: thread_guard,
    })
}

pub(crate) fn is_valid_gen_dir_name(name: &str) -> bool {
    name.starts_with("gen-")
        && name.len() > "gen-".len()
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn read_manifest_gen_id(paths: &WorkspacePaths) -> Result<Option<String>> {
    let manifest_path = paths.lake_raw_sec_manifest_path();
    match fs::symlink_metadata(&manifest_path) {
        Ok(_) => {
            let manifest =
                store::read_json::<SecInputManifest>(&manifest_path).with_context(|| {
                    format!(
                        "failed to read/parse SEC input manifest {}",
                        manifest_path.display()
                    )
                })?;
            let id_trimmed = manifest.generation_id.trim();
            let trimmed = manifest.generation_dir.trim();
            ensure!(
                is_valid_gen_dir_name(trimmed) && id_trimmed == trimmed,
                "invalid or mismatched SEC input manifest generation (id={id_trimmed:?}, dir={trimmed:?}) in {}",
                manifest_path.display()
            );
            Ok(Some(trimmed.to_owned()))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| {
            format!(
                "failed to inspect SEC input manifest {}",
                manifest_path.display()
            )
        }),
    }
}

fn read_live_db_sec_input_gen(paths: &WorkspacePaths) -> Result<Option<String>> {
    let db_path = paths.lake_db_path();
    match fs::symlink_metadata(&db_path) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed to inspect DuckDB lakehouse {}", db_path.display())
            });
        }
    }
    let Some(db) = LakeDb::open_read_only_if_exists(paths)
        .context("failed to open DuckDB lakehouse while reading active SEC input generation")?
    else {
        return Ok(None);
    };
    let Some(raw) = db
        .read_sec_input_generation()
        .context("failed to read DuckDB SEC input generation watermark")?
    else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    ensure!(
        is_valid_gen_dir_name(trimmed),
        "invalid DuckDB SEC input generation watermark {trimmed:?}"
    );
    Ok(Some(trimmed.to_owned()))
}

fn try_acquire_exclusive_lock(lock_path: &Path) -> Option<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .or_else(|_| File::open(lock_path))
        .ok()?;
    match file.try_lock() {
        Ok(()) => Some(file),
        Err(_) => None,
    }
}

fn remove_dir_under_exclusive_lock(gens_dir: &Path, dir: &Path, name: &str, lock_name: &str) {
    let Some(_excl_lock) = try_acquire_exclusive_lock(&dir.join(lock_name)) else {
        return;
    };
    let trash_path = gens_dir.join(format!(".deleting-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&trash_path);
    if fs::rename(dir, &trash_path).is_ok() {
        let _ = fs::remove_dir_all(&trash_path);
    }
}

fn sec_gen_has_complete_assets(gen_dir: &Path, source_label: &str, gen_id: &str) -> Result<()> {
    ensure!(
        gen_dir.is_dir(),
        "{source_label} '{gen_id}' is missing directory at {}; refusing recovery cleanup",
        gen_dir.display()
    );
    let facts_zip = gen_dir.join(COMPANYFACTS_ZIP_FILE);
    let facts_meta = fs::metadata(&facts_zip).with_context(|| {
        format!(
            "{source_label} '{gen_id}' is missing mandatory {} at {}; refusing recovery cleanup",
            COMPANYFACTS_ZIP_FILE,
            facts_zip.display()
        )
    })?;
    ensure!(
        facts_meta.is_file() && facts_meta.len() > 0 && is_valid_zip_file(&facts_zip),
        "{source_label} '{gen_id}' has empty or invalid {} at {}; refusing recovery cleanup",
        COMPANYFACTS_ZIP_FILE,
        facts_zip.display()
    );
    let tickers_path = gen_dir.join(TICKERS_EXCHANGE_FILE);
    let tickers_meta = fs::metadata(&tickers_path).with_context(|| {
        format!(
            "{source_label} '{gen_id}' is missing mandatory {} at {}; refusing recovery cleanup",
            TICKERS_EXCHANGE_FILE,
            tickers_path.display()
        )
    })?;
    ensure!(
        tickers_meta.is_file() && tickers_meta.len() > 0,
        "{source_label} '{gen_id}' has empty or non-file {} at {}; refusing recovery cleanup",
        TICKERS_EXCHANGE_FILE,
        tickers_path.display()
    );
    validate_tickers_exchange_file(&tickers_path, &HashSet::new()).with_context(|| {
        format!(
            "{source_label} '{gen_id}' has invalid {} at {}; refusing recovery cleanup",
            TICKERS_EXCHANGE_FILE,
            tickers_path.display()
        )
    })?;
    Ok(())
}

fn ensure_valid_sec_gen_dir(gens_dir: &Path, gen_id: &str, source_label: &str) -> Result<PathBuf> {
    let trimmed = gen_id.trim();
    ensure!(
        is_valid_gen_dir_name(trimmed),
        "invalid {source_label} '{trimmed}'; refusing recovery cleanup"
    );
    let gen_dir = gens_dir.join(trimmed);
    sec_gen_has_complete_assets(&gen_dir, source_label, trimmed)?;
    Ok(gen_dir)
}

fn cleanup_sec_input_generations(paths: &WorkspacePaths, gens_dir: &Path, extra_keep: &[&str]) {
    cleanup_sec_input_generations_inner(paths, gens_dir, extra_keep, true);
}

fn cleanup_sec_input_generations_inner(
    paths: &WorkspacePaths,
    gens_dir: &Path,
    extra_keep: &[&str],
    prune_unprotected_gens: bool,
) {
    let mut protected: HashSet<String> = HashSet::new();
    if prune_unprotected_gens {
        let Ok(manifest_gen) = read_manifest_gen_id(paths) else {
            return;
        };
        let Ok(db_gen) = read_live_db_sec_input_gen(paths) else {
            return;
        };
        for raw in extra_keep {
            let trimmed = raw.trim();
            if ensure_valid_sec_gen_dir(gens_dir, trimmed, "extra keep SEC input generation")
                .is_err()
            {
                return;
            }
            protected.insert(trimmed.to_owned());
        }
        if let Some(m_gen) = manifest_gen {
            if ensure_valid_sec_gen_dir(gens_dir, &m_gen, "SEC input manifest generation").is_err()
            {
                return;
            }
            protected.insert(m_gen);
        }
        if let Some(db_gen) = db_gen {
            if ensure_valid_sec_gen_dir(gens_dir, &db_gen, "committed SEC input generation")
                .is_err()
            {
                return;
            }
            protected.insert(db_gen);
        }
    }
    let Ok(entries) = fs::read_dir(gens_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !p.is_dir() {
            continue;
        }
        if name.starts_with(".deleting-") {
            let lock_file = if p.join(".stage.lock").is_file() {
                ".stage.lock"
            } else {
                ".gen.lock"
            };
            if let Some(_excl_lock) = try_acquire_exclusive_lock(&p.join(lock_file)) {
                let _ = fs::remove_dir_all(&p);
            }
            continue;
        }
        if name.starts_with(".staging-") {
            remove_dir_under_exclusive_lock(gens_dir, &p, name, ".stage.lock");
            continue;
        }
        if prune_unprotected_gens && name.starts_with("gen-") && !protected.contains(name) {
            remove_dir_under_exclusive_lock(gens_dir, &p, name, ".gen.lock");
        }
    }
}

fn publish_sec_input_manifest_and_links(
    paths: &WorkspacePaths,
    gen_id: &str,
    gen_dir: &Path,
    committed_at_ms: u64,
) -> Result<()> {
    let manifest_path = paths.lake_raw_sec_manifest_path();
    let manifest_needs_write = store::read_json::<SecInputManifest>(&manifest_path)
        .ok()
        .is_none_or(|m| m.generation_id != gen_id || m.generation_dir != gen_id);
    if manifest_needs_write {
        let manifest = SecInputManifest {
            schema_version: 1,
            generation_id: gen_id.to_owned(),
            generation_dir: gen_id.to_owned(),
            committed_at_ms,
        };
        store::write_json_atomic(&manifest_path, &manifest)?;
    }
    let top_dir = paths.lake_raw_sec_dir();
    for file_name in [
        TICKERS_EXCHANGE_FILE,
        SUBMISSIONS_ZIP_FILE,
        SIC_MAP_FILE,
        COMPANYFACTS_ZIP_FILE,
    ] {
        let gen_file = gen_dir.join(file_name);
        if gen_file.is_file() {
            atomic_link_or_copy_replace(&gen_file, &top_dir.join(file_name))?;
        }
    }
    Ok(())
}

fn resolved_paths_for_gen_dir(gen_id: &str, gen_dir: &Path) -> ResolvedSecInputPaths {
    ResolvedSecInputPaths {
        generation_id: Some(gen_id.to_owned()),
        tickers_path: gen_dir.join(TICKERS_EXCHANGE_FILE),
        submissions_zip_path: gen_dir.join(SUBMISSIONS_ZIP_FILE),
        sic_map_path: gen_dir.join(SIC_MAP_FILE),
        facts_zip_path: gen_dir.join(COMPANYFACTS_ZIP_FILE),
    }
}

#[cfg(test)]
fn reconcile_sec_input_generation_on_disk(
    paths: &WorkspacePaths,
    db_input_gen: Option<&str>,
    now_ms: u64,
) -> Result<ResolvedSecInputPaths> {
    let _sync_lock = acquire_sec_sync_lock(paths)?;
    reconcile_sec_input_generation_on_disk_inner(paths, db_input_gen, now_ms, true)
}

fn reconcile_sec_input_generation_on_disk_for_sync_locked(
    paths: &WorkspacePaths,
    db_input_gen: Option<&str>,
    now_ms: u64,
) -> Result<ResolvedSecInputPaths> {
    reconcile_sec_input_generation_on_disk_inner(paths, db_input_gen, now_ms, false)
}

fn reconcile_sec_input_generation_on_disk_inner(
    paths: &WorkspacePaths,
    db_input_gen: Option<&str>,
    now_ms: u64,
    strict_publish_and_prune: bool,
) -> Result<ResolvedSecInputPaths> {
    let gens_dir = paths.lake_raw_sec_generations_dir();
    let top_dir = paths.lake_raw_sec_dir();
    let ensure_gen_dir_only = |gen_id: &str, source_label: &str| -> Result<PathBuf> {
        let trimmed = gen_id.trim();
        ensure!(
            is_valid_gen_dir_name(trimmed),
            "invalid {source_label} '{trimmed}'; refusing recovery cleanup"
        );
        let gen_dir = gens_dir.join(trimmed);
        ensure!(
            gen_dir.is_dir(),
            "{source_label} '{trimmed}' is missing directory at {}; refusing recovery cleanup",
            gen_dir.display()
        );
        Ok(gen_dir)
    };
    if let Some(raw_db_gen) = db_input_gen {
        let gen_id = raw_db_gen.trim();
        let gen_dir = if strict_publish_and_prune {
            let dir =
                ensure_valid_sec_gen_dir(&gens_dir, gen_id, "committed SEC input generation")?;
            publish_sec_input_manifest_and_links(paths, gen_id, &dir, now_ms)?;
            dir
        } else {
            let dir = ensure_gen_dir_only(gen_id, "committed SEC input generation")?;
            if sec_gen_has_complete_assets(&dir, "committed SEC input generation", gen_id).is_ok() {
                publish_sec_input_manifest_and_links(paths, gen_id, &dir, now_ms)?;
            }
            dir
        };
        cleanup_sec_input_generations_inner(paths, &gens_dir, &[gen_id], strict_publish_and_prune);
        return Ok(resolved_paths_for_gen_dir(gen_id, &gen_dir));
    }
    if let Some(manifest_gen) = read_manifest_gen_id(paths)? {
        let gen_dir = if strict_publish_and_prune {
            ensure_valid_sec_gen_dir(&gens_dir, &manifest_gen, "SEC input manifest generation")?
        } else {
            ensure_gen_dir_only(&manifest_gen, "SEC input manifest generation")?
        };
        cleanup_sec_input_generations_inner(
            paths,
            &gens_dir,
            &[manifest_gen.as_str()],
            strict_publish_and_prune,
        );
        return Ok(resolved_paths_for_gen_dir(&manifest_gen, &gen_dir));
    }
    if let Some(db_gen) = read_live_db_sec_input_gen(paths)? {
        let gen_dir = if strict_publish_and_prune {
            let dir =
                ensure_valid_sec_gen_dir(&gens_dir, &db_gen, "committed SEC input generation")?;
            publish_sec_input_manifest_and_links(paths, &db_gen, &dir, now_ms)?;
            dir
        } else {
            let dir = ensure_gen_dir_only(&db_gen, "committed SEC input generation")?;
            if sec_gen_has_complete_assets(&dir, "committed SEC input generation", &db_gen).is_ok()
            {
                publish_sec_input_manifest_and_links(paths, &db_gen, &dir, now_ms)?;
            }
            dir
        };
        cleanup_sec_input_generations_inner(
            paths,
            &gens_dir,
            &[db_gen.as_str()],
            strict_publish_and_prune,
        );
        return Ok(resolved_paths_for_gen_dir(&db_gen, &gen_dir));
    }
    cleanup_sec_input_generations_inner(paths, &gens_dir, &[], strict_publish_and_prune);
    Ok(ResolvedSecInputPaths {
        generation_id: None,
        tickers_path: top_dir.join(TICKERS_EXCHANGE_FILE),
        submissions_zip_path: top_dir.join(SUBMISSIONS_ZIP_FILE),
        sic_map_path: top_dir.join(SIC_MAP_FILE),
        facts_zip_path: top_dir.join(COMPANYFACTS_ZIP_FILE),
    })
}

struct StagedSecGenerationGuard {
    generation_id: String,
    generation_dir: PathBuf,
    staging_dir: PathBuf,
    stage_lock: Option<File>,
    staged_tickers: StagedSecFetch,
    staged_submissions: StagedSecFetch,
    staged_facts: StagedSecFetch,
    sic_map: HashMap<String, SicInfo>,
    active_paths: ResolvedSecInputPaths,
    gen_published: bool,
    finalized: bool,
}

impl StagedSecGenerationGuard {
    fn new(
        paths: &WorkspacePaths,
        generation_id: String,
        staged_tickers: StagedSecFetch,
        staged_submissions: StagedSecFetch,
        staged_facts: StagedSecFetch,
        sic_map: HashMap<String, SicInfo>,
        active_paths: ResolvedSecInputPaths,
    ) -> Self {
        let gens_dir = paths.lake_raw_sec_generations_dir();
        let staging_dir = gens_dir.join(format!(".staging-{generation_id}"));
        let generation_dir = gens_dir.join(&generation_id);
        Self {
            generation_id,
            generation_dir,
            staging_dir,
            stage_lock: None,
            staged_tickers,
            staged_submissions,
            staged_facts,
            sic_map,
            active_paths,
            gen_published: false,
            finalized: false,
        }
    }

    fn promote_before_commit(&mut self) -> Result<()> {
        let _ = fs::remove_dir_all(&self.staging_dir);
        fs::create_dir_all(&self.staging_dir).with_context(|| {
            format!(
                "create SEC generation staging dir {}",
                self.staging_dir.display()
            )
        })?;
        let stage_lock_path = self.staging_dir.join(".stage.lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&stage_lock_path)?;
        lock_file.lock()?;
        self.stage_lock = Some(lock_file);
        let stage_asset =
            |staged: &mut StagedSecFetch, active_src: &Path, dst: &Path| -> Result<()> {
                match staged {
                    StagedSecFetch::Downloaded { tmp_file, .. } => {
                        if let Some(tmp_path) = tmp_file.0.take() {
                            fs::rename(&tmp_path, dst).with_context(|| {
                                format!(
                                    "stage downloaded SEC asset {} -> {}",
                                    tmp_path.display(),
                                    dst.display()
                                )
                            })?;
                        }
                    }
                    StagedSecFetch::NotModified => {
                        if active_src.is_file() {
                            link_or_copy_file(active_src, dst)?;
                        }
                    }
                    StagedSecFetch::Unavailable => {}
                }
                Ok(())
            };
        stage_asset(
            &mut self.staged_tickers,
            &self.active_paths.tickers_path,
            &self.staging_dir.join(TICKERS_EXCHANGE_FILE),
        )?;
        stage_asset(
            &mut self.staged_submissions,
            &self.active_paths.submissions_zip_path,
            &self.staging_dir.join(SUBMISSIONS_ZIP_FILE),
        )?;
        stage_asset(
            &mut self.staged_facts,
            &self.active_paths.facts_zip_path,
            &self.staging_dir.join(COMPANYFACTS_ZIP_FILE),
        )?;
        if !self.sic_map.is_empty() {
            store::write_json_atomic(&self.staging_dir.join(SIC_MAP_FILE), &self.sic_map)?;
        } else if self.active_paths.sic_map_path.is_file() {
            link_or_copy_file(
                &self.active_paths.sic_map_path,
                &self.staging_dir.join(SIC_MAP_FILE),
            )?;
        }
        sec_gen_has_complete_assets(
            &self.staging_dir,
            "staged SEC input generation",
            &self.generation_id,
        )?;
        let _ = fs::remove_dir_all(&self.generation_dir);
        let gen_lock_path = self.staging_dir.join(".gen.lock");
        let _ = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&gen_lock_path);
        self.stage_lock = None;
        let _ = fs::remove_file(&stage_lock_path);
        fs::rename(&self.staging_dir, &self.generation_dir).with_context(|| {
            format!(
                "publish immutable SEC input generation {}",
                self.generation_dir.display()
            )
        })?;
        self.gen_published = true;
        Ok(())
    }

    fn finalize(mut self, paths: &WorkspacePaths, committed_at_ms: u64) -> Result<()> {
        self.finalized = true;
        publish_sec_input_manifest_and_links(
            paths,
            &self.generation_id,
            &self.generation_dir,
            committed_at_ms,
        )?;
        cleanup_sec_input_generations(
            paths,
            &paths.lake_raw_sec_generations_dir(),
            &[self.generation_id.as_str()],
        );
        Ok(())
    }
}

impl Drop for StagedSecGenerationGuard {
    fn drop(&mut self) {
        self.stage_lock = None;
        if !self.finalized {
            let _ = fs::remove_dir_all(&self.staging_dir);
            if self.gen_published {
                let _ = fs::remove_dir_all(&self.generation_dir);
            }
        }
    }
}

fn format_sha256_hex(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn stream_file_len_and_sha256(path: &Path) -> Result<(u64, String)> {
    let file =
        File::open(path).with_context(|| format!("open {} for streaming hash", path.display()))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("read {} for streaming hash", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total = total.saturating_add(n as u64);
    }
    Ok((total, format_sha256_hex(&hasher.finalize())))
}

struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
    len: u64,
}

impl<W: io::Write> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            len: 0,
        }
    }

    fn finish(self) -> Result<(W, String)> {
        let mut inner = self.inner;
        inner.flush()?;
        let cursor = format!(
            "len={};sha256={}",
            self.len,
            format_sha256_hex(&self.hasher.finalize())
        );
        Ok((inner, cursor))
    }
}

impl<W: io::Write> io::Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.len = self.len.saturating_add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn format_file_integrity_cursor(path: &Path) -> Result<String> {
    let (len, sha) = stream_file_len_and_sha256(path)?;
    Ok(format!("len={len};sha256={sha}"))
}

fn verify_file_integrity_cursor(path: &Path, cursor: &str) -> bool {
    let mut exp_len: Option<u64> = None;
    let mut exp_sha: Option<&str> = None;
    for part in cursor.trim().split(';') {
        if let Some(v) = part.strip_prefix("len=") {
            exp_len = v.parse::<u64>().ok();
        } else if let Some(v) = part.strip_prefix("sha256=") {
            exp_sha = (!v.trim().is_empty()).then_some(v.trim());
        }
    }
    let (Some(len), Some(sha)) = (exp_len, exp_sha) else {
        return false;
    };
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() != len {
        return false;
    }
    let Ok((actual_len, actual_sha)) = stream_file_len_and_sha256(path) else {
        return false;
    };
    actual_len == len && actual_sha.eq_ignore_ascii_case(sha)
}

#[cfg_attr(not(test), allow(dead_code))]
fn fetch_sec_file_staged<V>(
    client: &Client,
    spec: &ConditionalFetchSpec<'_>,
    validate_tmp: V,
) -> Result<StagedSecFetch>
where
    V: FnOnce(&Path) -> Result<()>,
{
    fetch_sec_file_staged_with_progress(client, spec, validate_tmp, &mut |_, _| {})
}

fn fetch_sec_file_staged_with_progress<V>(
    client: &Client,
    spec: &ConditionalFetchSpec<'_>,
    validate_tmp: V,
    on_bytes: &mut dyn FnMut(u64, Option<u64>),
) -> Result<StagedSecFetch>
where
    V: FnOnce(&Path) -> Result<()>,
{
    let ConditionalFetchSpec {
        url,
        dest_path,
        watermark_key,
        cached_wm,
        now_ms,
        force_download,
        record_integrity_cursor,
    } = *spec;
    thread::sleep(Duration::from_millis(160));
    let cursor_ok = if record_integrity_cursor {
        cached_wm
            .and_then(|w| w.cursor.as_deref())
            .is_some_and(|c| c.starts_with("len=") && verify_file_integrity_cursor(dest_path, c))
    } else {
        true
    };
    let dest_ready =
        dest_path.is_file() && fs::metadata(dest_path).is_ok_and(|m| m.len() > 0) && cursor_ok;
    let mut req = client.get(url);
    if !force_download
        && dest_ready
        && let Some(wm) = cached_wm
    {
        if let Some(etag) = wm.etag.as_deref().filter(|s| !s.trim().is_empty()) {
            req = req.header("If-None-Match", etag);
        }
        if let Some(lm) = wm.last_modified.as_deref().filter(|s| !s.trim().is_empty()) {
            req = req.header("If-Modified-Since", lm);
        }
    }

    let mut resp = req
        .send()
        .with_context(|| format!("request SEC bulk resource {url}"))?;

    if !force_download && resp.status() == StatusCode::NOT_MODIFIED && dest_ready {
        return Ok(StagedSecFetch::NotModified);
    }

    ensure!(
        resp.status().is_success(),
        "SEC {url} returned HTTP {}",
        resp.status()
    );

    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let last_modified = resp
        .headers()
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let total_bytes = resp.content_length();

    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = store::atomic_temp_path(dest_path);
    let _ = fs::remove_file(&tmp_path);
    let tmp_file = StagedTempFile::new(tmp_path);
    let staged_path = tmp_file
        .as_path()
        .context("missing staged temp file path")?;
    let file =
        File::create(staged_path).with_context(|| format!("create {}", staged_path.display()))?;
    let mut writer = HashingWriter::new(BufWriter::with_capacity(64 * 1024, file));
    let mut buf = vec![0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_emit_bytes: u64 = 0;
    let mut last_emit_at = std::time::Instant::now();
    loop {
        let n = io::Read::read(&mut resp, &mut buf)
            .with_context(|| format!("stream {url} to {}", staged_path.display()))?;
        if n == 0 {
            break;
        }
        io::Write::write_all(&mut writer, &buf[..n])
            .with_context(|| format!("write {url} chunk to {}", staged_path.display()))?;
        downloaded = downloaded.saturating_add(n as u64);
        if downloaded.saturating_sub(last_emit_bytes) >= 2 * 1024 * 1024
            || last_emit_at.elapsed() >= Duration::from_millis(250)
        {
            on_bytes(downloaded, total_bytes);
            last_emit_bytes = downloaded;
            last_emit_at = std::time::Instant::now();
        }
    }
    if downloaded > last_emit_bytes {
        on_bytes(downloaded, total_bytes.or(Some(downloaded)));
    }
    let (inner_writer, streamed_cursor) = writer.finish()?;
    drop(inner_writer);
    validate_tmp(staged_path).with_context(|| format!("validate downloaded payload from {url}"))?;
    let cursor = record_integrity_cursor.then_some(streamed_cursor);

    Ok(StagedSecFetch::Downloaded {
        tmp_file,
        watermark: SyncWatermark {
            source_key: watermark_key.to_owned(),
            cursor,
            etag,
            last_modified,
            synced_at_ms: now_ms,
        },
    })
}

fn is_valid_tickers_exchange_file(
    path: &Path,
    prev_all_ciks: &HashSet<String>,
    expected_cursor: Option<&str>,
) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(cursor) = expected_cursor.filter(|c| c.starts_with("len=")) else {
        return false;
    };
    if !verify_file_integrity_cursor(path, cursor) {
        return false;
    }
    validate_tickers_exchange_file(path, prev_all_ciks).is_ok()
}

fn validate_tickers_exchange_file(path: &Path, prev_all_ciks: &HashSet<String>) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let listed = parse_canonical_tickers_exchange(&bytes, &HashMap::new())?;
    ensure!(
        !listed.is_empty(),
        "company_tickers_exchange payload produced zero Nasdaq/NYSE/CBOE listings"
    );
    if prev_all_ciks.len() >= 10 {
        ensure!(
            listed.len() * 100 > prev_all_ciks.len() * 90,
            "company_tickers_exchange payload shrunk to {}/{} of previous universe (<=90%)",
            listed.len(),
            prev_all_ciks.len()
        );
        let retained = listed
            .iter()
            .filter(|c| prev_all_ciks.contains(&c.cik))
            .count();
        ensure!(
            retained * 100 >= prev_all_ciks.len() * 85,
            "company_tickers_exchange payload retained only {retained}/{} previously listed CIKs (<85%)",
            prev_all_ciks.len()
        );
    }
    Ok(())
}

fn validate_zip_file(path: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("invalid zip {}", path.display()))?;
    Ok(())
}

fn validate_parsed_bundle_coverage(
    snapshot: &[UniverseRow],
    annuals: &[LakeAnnualRow],
    ttms: &[LakeTtmRow],
    pre_state: &PreSyncLakeState,
    expected_parsed_active: &HashSet<String>,
) -> Result<()> {
    if pre_state.parsed_ciks.len() >= 10 {
        ensure!(
            expected_parsed_active.len() * 100 >= pre_state.parsed_ciks.len() * 85,
            "SEC active listed universe retained only {}/{} previously parsed companies (<85%), aborting lake replacement",
            expected_parsed_active.len(),
            pre_state.parsed_ciks.len()
        );
    }
    if expected_parsed_active.len() < 10 {
        return Ok(());
    }
    let mut new_annual_ends: HashMap<&str, Vec<String>> = HashMap::new();
    for a in annuals {
        new_annual_ends
            .entry(a.cik.as_str())
            .or_default()
            .push(a.period_end.clone());
    }
    let ttm_ends: HashMap<&str, &str> = ttms
        .iter()
        .map(|t| (t.cik.as_str(), t.period_end.as_str()))
        .collect();
    let retained_parsed = snapshot
        .iter()
        .filter(|r| {
            if !expected_parsed_active.contains(&r.cik) || r.years_of_data == 0 {
                return false;
            }
            if let Some(prev_ends) = pre_state.prev_annual_period_ends.get(&r.cik) {
                let new_ends = new_annual_ends
                    .get(r.cik.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                !edgar::is_annual_period_coverage_regression(
                    prev_ends,
                    pre_state
                        .prev_ttm_period_ends
                        .get(&r.cik)
                        .map(String::as_str),
                    new_ends,
                    ttm_ends.get(r.cik.as_str()).copied(),
                    r.latest_filing_end.as_deref(),
                )
            } else if let Some(prev) = pre_state.prev_parsed_rows.get(&r.cik) {
                !is_issuer_coverage_regression(
                    prev.years_of_data,
                    pre_state.prev_ttm_ciks.contains(&r.cik),
                    prev.latest_filing_end.as_deref(),
                    r.years_of_data,
                    ttm_ends.contains_key(r.cik.as_str()),
                    r.latest_filing_end.as_deref(),
                )
            } else {
                true
            }
        })
        .count();
    ensure!(
        retained_parsed * 100 >= expected_parsed_active.len() * 90,
        "SEC companyfacts parse retained only {retained_parsed}/{} previously parsed active issuers without period-key regression (<90%), aborting lake replacement",
        expected_parsed_active.len()
    );
    Ok(())
}

fn validate_companyfacts_zip_tmp(
    path: &Path,
    listed_ciks: &HashSet<String>,
    expected_parsed_ciks: &HashSet<String>,
) -> Result<()> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("invalid companyfacts.zip {}", path.display()))?;
    ensure!(!archive.is_empty(), "companyfacts.zip archive is empty");
    let max_verify_bytes = MAX_FACTS_ENTRY_BYTES.saturating_mul(8);
    let mut verified = 0usize;
    let mut covered_prev = 0usize;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .with_context(|| format!("read zip entry header #{i} in {}", path.display()))?;
        let name = entry
            .name()
            .map(|s| s.into_owned())
            .with_context(|| format!("decode zip entry name #{i} in {}", path.display()))?;
        let cik = match_cik_json_entry(&name);
        let Some(cik) = cik else {
            continue;
        };
        if !listed_ciks.is_empty() && !listed_ciks.contains(&cik) {
            continue;
        }
        ensure!(
            entry.size() <= max_verify_bytes,
            "zip entry {name} uncompressed size {} exceeds safety bound {max_verify_bytes}",
            entry.size()
        );
        let n = io::copy(&mut entry.take(max_verify_bytes + 1), &mut io::sink())
            .with_context(|| format!("corrupt deflate stream or CRC for {name}"))?;
        ensure!(
            n <= max_verify_bytes,
            "zip entry {name} inflated past safety bound {max_verify_bytes}"
        );
        verified += 1;
        if expected_parsed_ciks.contains(&cik) {
            covered_prev += 1;
        }
    }
    if !listed_ciks.is_empty() {
        ensure!(
            verified > 0,
            "companyfacts.zip contained zero listed CIK entries"
        );
    }
    if !expected_parsed_ciks.is_empty() {
        ensure!(
            covered_prev * 100 >= expected_parsed_ciks.len() * 95,
            "companyfacts.zip only covered {covered_prev}/{} previously parsed active issuers (<95%)",
            expected_parsed_ciks.len()
        );
    }
    Ok(())
}

fn preserved_raw_issuer_path(paths: &WorkspacePaths, cik: &str, sha256: &str) -> PathBuf {
    paths
        .lake_preserved_raw_sec_dir()
        .join(format!("CIK{cik}-{sha256}.json"))
}

fn legacy_preserved_raw_issuer_path(paths: &WorkspacePaths, cik: &str) -> PathBuf {
    paths
        .lake_preserved_raw_sec_dir()
        .join(format!("CIK{cik}.json"))
}

fn verify_readable_sec_bytes(
    bytes: &[u8],
    expected_sha: Option<&str>,
    cik: &str,
) -> Result<String> {
    ensure!(
        !bytes.is_empty(),
        "raw SEC provenance source for CIK {cik} is empty"
    );
    let actual_sha = edgar::hex_digest(bytes);
    if let Some(exp) = expected_sha.map(str::trim).filter(|s| !s.is_empty()) {
        ensure!(
            actual_sha == exp,
            "raw SEC provenance SHA mismatch for preserved CIK {cik}: expected {exp}, got {actual_sha}"
        );
    }
    let _: serde_json::Value = serde_json::from_slice(bytes)
        .with_context(|| format!("preserved raw SEC source for CIK {cik} is invalid JSON"))?;
    Ok(actual_sha)
}

fn try_promote_preserved_from_local_file(
    paths: &WorkspacePaths,
    src_path: &Path,
    cik: &str,
    expected_sha: Option<&str>,
) -> Result<bool> {
    if !src_path.is_file() {
        return Ok(false);
    }
    let Ok(bytes) = fs::read(src_path) else {
        return Ok(false);
    };
    let Ok(actual_sha) = verify_readable_sec_bytes(&bytes, expected_sha, cik) else {
        return Ok(false);
    };
    let dest = preserved_raw_issuer_path(paths, cik, &actual_sha);
    if src_path != dest {
        fs::create_dir_all(paths.lake_preserved_raw_sec_dir())?;
        let tmp = store::atomic_temp_path(&dest);
        if let Err(err) = fs::write(&tmp, &bytes).and_then(|_| fs::rename(&tmp, &dest)) {
            let _ = fs::remove_file(&tmp);
            return Err(err).with_context(|| {
                format!(
                    "stage preserved raw SEC bytes for CIK {cik} at {}",
                    dest.display()
                )
            });
        }
    }
    Ok(true)
}

fn preserved_sha_already_staged(
    paths: &WorkspacePaths,
    cik: &str,
    expected_sha: Option<&str>,
) -> Result<bool> {
    let norm_exp = expected_sha
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);
    if let Some(ref sha) = norm_exp {
        let sha_path = preserved_raw_issuer_path(paths, cik, sha);
        if try_promote_preserved_from_local_file(paths, &sha_path, cik, Some(sha))? {
            return Ok(true);
        }
        let legacy = legacy_preserved_raw_issuer_path(paths, cik);
        if try_promote_preserved_from_local_file(paths, &legacy, cik, Some(sha))? {
            return Ok(true);
        }
        let raw_sec = paths.sec_raw_path(cik);
        return try_promote_preserved_from_local_file(paths, &raw_sec, cik, Some(sha));
    }

    let preserved_dir = paths.lake_preserved_raw_sec_dir();
    if let Ok(entries) = fs::read_dir(&preserved_dir) {
        let prefix = format!("CIK{cik}-");
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(name) = p.file_name().and_then(|n| n.to_str())
                && p.is_file()
                && name.starts_with(&prefix)
                && let Some(stem_sha) = name
                    .strip_prefix(&prefix)
                    .and_then(|s| s.strip_suffix(".json"))
                && try_promote_preserved_from_local_file(paths, &p, cik, Some(stem_sha))?
            {
                return Ok(true);
            }
        }
    }
    let legacy = legacy_preserved_raw_issuer_path(paths, cik);
    if try_promote_preserved_from_local_file(paths, &legacy, cik, None)? {
        return Ok(true);
    }
    let raw_sec = paths.sec_raw_path(cik);
    try_promote_preserved_from_local_file(paths, &raw_sec, cik, None)
}

fn stage_preserved_raw_issuer_bytes(
    paths: &WorkspacePaths,
    existing_facts_zip: &Path,
    preserved_ciks_with_sha: &HashMap<String, Vec<Option<String>>>,
) -> Result<()> {
    if preserved_ciks_with_sha.is_empty() {
        return Ok(());
    }
    let mut missing_on_disk: HashMap<String, Vec<Option<String>>> = HashMap::new();
    for (cik, expected_shas) in preserved_ciks_with_sha {
        let candidates: &[Option<String>] = if expected_shas.is_empty() {
            &[None]
        } else {
            expected_shas.as_slice()
        };
        let mut needed = Vec::new();
        for sha_opt in candidates {
            if !preserved_sha_already_staged(paths, cik, sha_opt.as_deref())? {
                needed.push(sha_opt.clone());
            }
        }
        if !needed.is_empty() {
            missing_on_disk.insert(cik.clone(), needed);
        }
    }
    if missing_on_disk.is_empty() {
        return Ok(());
    }
    ensure!(
        existing_facts_zip.exists(),
        "cannot publish SEC replacement while {} preserved issuers lack readable raw provenance (active archive {} missing)",
        missing_on_disk.len(),
        existing_facts_zip.display()
    );

    let file = File::open(existing_facts_zip).with_context(|| {
        format!(
            "open active SEC archive {} to preserve {} fallback issuers",
            existing_facts_zip.display(),
            missing_on_disk.len()
        )
    })?;
    let mut archive = ZipArchive::new(BufReader::new(file)).with_context(|| {
        format!(
            "open ZipArchive {} to preserve raw SEC provenance",
            existing_facts_zip.display()
        )
    })?;
    let preserved_dir = paths.lake_preserved_raw_sec_dir();
    fs::create_dir_all(&preserved_dir)?;
    for idx in 0..archive.len() {
        let name = archive
            .name_for_index(idx)
            .with_context(|| {
                format!(
                    "read zip entry header #{idx} in {}",
                    existing_facts_zip.display()
                )
            })?
            .with_context(|| {
                format!(
                    "decode zip entry name #{idx} in {}",
                    existing_facts_zip.display()
                )
            })?
            .into_owned();
        let Some(cik) = match_cik_json_entry(&name) else {
            continue;
        };
        let Some(expected_shas) = missing_on_disk.get(&cik).cloned() else {
            continue;
        };
        let entry = archive
            .by_index(idx)
            .with_context(|| format!("open preserved zip entry {name}"))?;
        let claimed = entry.size();
        let bytes = read_zip_entry_bounded(entry, claimed, MAX_FACTS_ENTRY_BYTES, &cik)
            .map_err(|msg| anyhow::anyhow!(msg))?;
        let mut actual_sha_to_write: Option<String> = None;
        let mut remaining = Vec::new();
        for exp_opt in expected_shas {
            match verify_readable_sec_bytes(&bytes, exp_opt.as_deref(), &cik) {
                Ok(sha) => actual_sha_to_write = Some(sha),
                Err(_) => remaining.push(exp_opt),
            }
        }
        let Some(actual_sha) = actual_sha_to_write else {
            // None of the expected SHAs matched this archive entry; trigger explicit error.
            let first_exp = missing_on_disk
                .get(&cik)
                .and_then(|v| v.first())
                .and_then(|o| o.as_deref());
            verify_readable_sec_bytes(&bytes, first_exp, &cik)?;
            continue;
        };
        let dest = preserved_raw_issuer_path(paths, &cik, &actual_sha);
        let tmp = store::atomic_temp_path(&dest);
        if let Err(err) = fs::write(&tmp, &bytes).and_then(|_| fs::rename(&tmp, &dest)) {
            let _ = fs::remove_file(&tmp);
            return Err(err).with_context(|| {
                format!(
                    "persist preserved raw SEC bytes for CIK {cik} at {}",
                    dest.display()
                )
            });
        }
        if remaining.is_empty() {
            missing_on_disk.remove(&cik);
        } else {
            missing_on_disk.insert(cik, remaining);
        }
        if missing_on_disk.is_empty() {
            break;
        }
    }
    ensure!(
        missing_on_disk.is_empty(),
        "cannot publish SEC replacement: missing readable/SHA-verified raw provenance in {} for preserved CIKs {:?}",
        existing_facts_zip.display(),
        missing_on_disk.keys().collect::<Vec<_>>()
    );
    Ok(())
}

fn prune_preserved_raw_issuer_bytes(
    paths: &WorkspacePaths,
    keep_preserved: &HashMap<String, Vec<Option<String>>>,
) -> Result<()> {
    let preserved_dir = paths.lake_preserved_raw_sec_dir();
    if !preserved_dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&preserved_dir)
        .with_context(|| format!("read preserved SEC dir {}", preserved_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name
            .strip_prefix("CIK")
            .and_then(|s| s.strip_suffix(".json"))
        else {
            continue;
        };
        let (cik, file_sha_opt) = if let Some((c, sha)) = stem.split_once('-') {
            (c, Some(sha))
        } else {
            (stem, None)
        };
        if cik.len() != 10 || !cik.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let should_keep = match keep_preserved.get(cik) {
            Some(expected_shas) => {
                expected_shas.is_empty()
                    || expected_shas.iter().any(|sha_opt| match sha_opt {
                        Some(expected_sha) => file_sha_opt == Some(expected_sha.as_str()),
                        None => true,
                    })
            }
            None => false,
        };
        if !should_keep {
            fs::remove_file(&path)
                .with_context(|| format!("remove superseded preserved file {}", path.display()))?;
        }
    }
    Ok(())
}

fn is_valid_zip_file(path: &Path) -> bool {
    validate_zip_file(path).is_ok()
}

fn read_zip_entry_bounded<R: Read>(
    entry: R,
    claimed_size: u64,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    if claimed_size > max_bytes {
        return Err(format!(
            "{label}: zip uncompressed size {claimed_size} exceeds {max_bytes} byte limit"
        ));
    }
    let reserve = usize::try_from(claimed_size)
        .unwrap_or(0)
        .min(4 * 1024 * 1024);
    let mut buf = Vec::with_capacity(reserve);
    entry
        .take(max_bytes + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("{label}: zip deflate failed: {e}"))?;
    if buf.len() as u64 > max_bytes {
        return Err(format!(
            "{label}: zip stream exceeded {max_bytes} byte limit"
        ));
    }
    Ok(buf)
}

fn merge_fallback_sic(
    target: &mut HashMap<String, SicInfo>,
    fallback: &HashMap<String, SicInfo>,
) -> usize {
    let mut merged = 0usize;
    for (cik, info) in fallback {
        let should_insert = match target.get(cik) {
            None => info.sic.is_some() || info.sic_description.is_some(),
            Some(existing) => existing.sic.is_none() && info.sic.is_some(),
        };
        if should_insert {
            target.insert(cik.clone(), info.clone());
            merged += 1;
        }
    }
    merged
}

#[cfg(test)]
fn merge_and_persist_sic_map(
    sic_cache_path: &Path,
    mut map: HashMap<String, SicInfo>,
    prev_sic_map: &HashMap<String, SicInfo>,
    force_persist: bool,
) -> Result<(HashMap<String, SicInfo>, usize)> {
    let merged = merge_fallback_sic(&mut map, prev_sic_map);
    if force_persist || merged > 0 {
        store::write_json_atomic(sic_cache_path, &map)
            .with_context(|| format!("persist SEC SIC cache to {}", sic_cache_path.display()))?;
    }
    Ok((map, merged))
}

struct SicLoadOutcome {
    map: HashMap<String, SicInfo>,
    downloaded: bool,
    watermark: Option<SyncWatermark>,
    warning: Option<String>,
    staged_submissions: StagedSecFetch,
}

#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn load_or_fetch_sic_map_with_url(
    sub_url: &str,
    sub_zip_path: &Path,
    sic_cache_path: &Path,
    client: &Client,
    cached_wm: Option<&SyncWatermark>,
    listed_ciks: &HashSet<String>,
    expected_sic_ciks: &HashSet<String>,
    prev_sic_map: &HashMap<String, SicInfo>,
    tickers_downloaded: bool,
    now_ms: u64,
) -> Result<SicLoadOutcome> {
    load_or_fetch_sic_map_with_url_and_progress(
        sub_url,
        sub_zip_path,
        sic_cache_path,
        client,
        cached_wm,
        listed_ciks,
        expected_sic_ciks,
        prev_sic_map,
        tickers_downloaded,
        now_ms,
        &mut |_, _| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn load_or_fetch_sic_map_with_url_and_progress(
    sub_url: &str,
    sub_zip_path: &Path,
    sic_cache_path: &Path,
    client: &Client,
    cached_wm: Option<&SyncWatermark>,
    listed_ciks: &HashSet<String>,
    expected_sic_ciks: &HashSet<String>,
    prev_sic_map: &HashMap<String, SicInfo>,
    tickers_downloaded: bool,
    now_ms: u64,
    on_bytes: &mut dyn FnMut(u64, Option<u64>),
) -> Result<SicLoadOutcome> {
    let sub_cursor_ok = cached_wm
        .and_then(|w| w.cursor.as_deref())
        .is_some_and(|c| c.starts_with("len=") && verify_file_integrity_cursor(sub_zip_path, c));
    let sub_zip_valid = sub_zip_path.is_file() && sub_cursor_ok && is_valid_zip_file(sub_zip_path);

    let load_cached = || -> Option<HashMap<String, SicInfo>> {
        if sic_cache_path.is_file()
            && let Ok(cached) = store::read_json::<HashMap<String, SicInfo>>(sic_cache_path)
            && !cached.is_empty()
        {
            Some(cached)
        } else {
            None
        }
    };

    let merge_only = |mut map: HashMap<String, SicInfo>| -> (HashMap<String, SicInfo>, usize) {
        let merged = merge_fallback_sic(&mut map, prev_sic_map);
        (map, merged)
    };

    let validated_sic_map: std::cell::RefCell<Option<HashMap<String, SicInfo>>> =
        std::cell::RefCell::new(None);
    let make_outcome = |map: HashMap<String, SicInfo>,
                        downloaded: bool,
                        watermark: Option<SyncWatermark>,
                        warning: Option<String>,
                        staged_submissions: StagedSecFetch|
     -> SicLoadOutcome {
        SicLoadOutcome {
            map,
            downloaded,
            watermark,
            warning,
            staged_submissions,
        }
    };

    match fetch_sec_file_staged_with_progress(
        client,
        &ConditionalFetchSpec {
            url: sub_url,
            dest_path: sub_zip_path,
            watermark_key: WM_SUBMISSIONS_ZIP,
            cached_wm,
            now_ms,
            force_download: !sub_zip_valid,
            record_integrity_cursor: true,
        },
        |tmp_path| {
            validate_zip_file(tmp_path)?;
            let extracted = extract_sic_from_submissions_zip(tmp_path, listed_ciks)?;
            ensure!(
                !extracted.is_empty(),
                "downloaded submissions.zip yielded zero SIC entries"
            );
            if !expected_sic_ciks.is_empty() {
                let covered = expected_sic_ciks
                    .iter()
                    .filter(|cik| extracted.get(*cik).and_then(|s| s.sic).is_some())
                    .count();
                ensure!(
                    covered * 100 >= expected_sic_ciks.len() * 95,
                    "downloaded submissions.zip only covered {covered}/{} previously classified active issuers (<95%)",
                    expected_sic_ciks.len()
                );
            }
            *validated_sic_map.borrow_mut() = Some(extracted);
            Ok(())
        },
        on_bytes,
    ) {
        Ok(StagedSecFetch::NotModified) => {
            if !tickers_downloaded
                && let Some(cached) = load_cached()
                && expected_sic_ciks
                    .iter()
                    .all(|cik| cached.get(cik).and_then(|s| s.sic).is_some())
            {
                let (merged_map, merged_cnt) = merge_only(cached);
                let warn = (merged_cnt > 0).then(|| {
                    format!("SEC SIC 缓存缺失 {merged_cnt} 条记录，已从 DuckDB 历史分类补全")
                });
                return Ok(make_outcome(
                    merged_map,
                    false,
                    None,
                    warn,
                    StagedSecFetch::NotModified,
                ));
            }
            match extract_sic_from_submissions_zip(sub_zip_path, listed_ciks) {
                Ok(extracted) if !extracted.is_empty() => {
                    let (merged_map, merged_cnt) = merge_only(extracted);
                    let warn = (merged_cnt > 0).then(|| {
                        format!(
                            "SEC submissions.zip 缺失 {merged_cnt} 条已知 SIC 分类，已保留 DuckDB 历史分类"
                        )
                    });
                    Ok(make_outcome(
                        merged_map,
                        false,
                        None,
                        warn,
                        StagedSecFetch::NotModified,
                    ))
                }
                Ok(_) => {
                    if let Some(cached) = load_cached() {
                        let (merged_map, _) = merge_only(cached);
                        return Ok(make_outcome(
                            merged_map,
                            false,
                            None,
                            Some("SEC submissions.zip 未提取到新 SIC 分类，已保留本地 SIC 缓存与历史分类".to_owned()),
                            StagedSecFetch::Unavailable,
                        ));
                    }
                    let (merged_map, _) = merge_only(HashMap::new());
                    let msg = if merged_map.is_empty() {
                        "SEC submissions.zip 未提取到任何 SIC 分类，未分类公司将排除出方案 1 质量池"
                            .to_owned()
                    } else {
                        "SEC submissions.zip 未提取到新 SIC 分类，已保留 DuckDB 已验证 SIC 分类"
                            .to_owned()
                    };
                    Ok(make_outcome(
                        merged_map,
                        false,
                        None,
                        Some(msg),
                        StagedSecFetch::Unavailable,
                    ))
                }
                Err(err) => {
                    if let Some(cached) = load_cached() {
                        let (merged_map, _) = merge_only(cached);
                        return Ok(make_outcome(
                            merged_map,
                            false,
                            None,
                            Some(format!(
                                "读取本地 SEC submissions.zip 提取 SIC 失败，已回退本地缓存与历史分类: {err:#}"
                            )),
                            StagedSecFetch::Unavailable,
                        ));
                    }
                    let (merged_map, _) = merge_only(HashMap::new());
                    let msg = if merged_map.is_empty() {
                        format!(
                            "读取本地 SEC submissions.zip 提取 SIC 失败（未分类公司将排除出方案 1 质量池）: {err:#}"
                        )
                    } else {
                        format!(
                            "读取本地 SEC submissions.zip 提取 SIC 失败，已保留 DuckDB 已验证 SIC 分类: {err:#}"
                        )
                    };
                    Ok(make_outcome(
                        merged_map,
                        false,
                        None,
                        Some(msg),
                        StagedSecFetch::Unavailable,
                    ))
                }
            }
        }
        Ok(staged @ StagedSecFetch::Downloaded { .. }) => {
            let wm = match &staged {
                StagedSecFetch::Downloaded { watermark, .. } => watermark.clone(),
                StagedSecFetch::NotModified | StagedSecFetch::Unavailable => unreachable!(),
            };
            if let Some(extracted) = validated_sic_map.into_inner()
                && !extracted.is_empty()
            {
                let (merged_map, merged_cnt) = merge_only(extracted);
                let warn = (merged_cnt > 0).then(|| {
                    format!(
                        "SEC submissions.zip 缺失 {merged_cnt} 条历史 SIC 分类，已合并保留 DuckDB 已验证分类"
                    )
                });
                return Ok(make_outcome(merged_map, true, Some(wm), warn, staged));
            }
            let staged_sub_path = staged.active_path(sub_zip_path);
            match extract_sic_from_submissions_zip(staged_sub_path, listed_ciks) {
                Ok(extracted) if !extracted.is_empty() => {
                    let (merged_map, merged_cnt) = merge_only(extracted);
                    let warn = (merged_cnt > 0).then(|| {
                        format!(
                            "SEC submissions.zip 缺失 {merged_cnt} 条历史 SIC 分类，已合并保留 DuckDB 已验证分类"
                        )
                    });
                    Ok(make_outcome(merged_map, true, Some(wm), warn, staged))
                }
                Ok(_) => {
                    if let Some(cached) = load_cached() {
                        let (merged_map, _) = merge_only(cached);
                        return Ok(make_outcome(
                            merged_map,
                            false,
                            None,
                            Some("SEC submissions.zip 解析结果为空，已回退本地 SIC 缓存与历史分类（未提交新 watermark）".to_owned()),
                            StagedSecFetch::Unavailable,
                        ));
                    }
                    let (merged_map, _) = merge_only(HashMap::new());
                    Ok(make_outcome(
                        merged_map,
                        false,
                        None,
                        Some(
                            "SEC submissions.zip 解析结果为空，已保留已有 SIC 分类（未分类公司将排除出方案 1 质量池）"
                                .to_owned(),
                        ),
                        StagedSecFetch::Unavailable,
                    ))
                }
                Err(err) => {
                    if let Some(cached) = load_cached() {
                        let (merged_map, _) = merge_only(cached);
                        return Ok(make_outcome(
                            merged_map,
                            false,
                            None,
                            Some(format!(
                                "SEC submissions.zip 提取 SIC 失败，已回退本地缓存与历史分类: {err:#}"
                            )),
                            StagedSecFetch::Unavailable,
                        ));
                    }
                    let (merged_map, _) = merge_only(HashMap::new());
                    Ok(make_outcome(
                        merged_map,
                        false,
                        None,
                        Some(format!(
                            "SEC submissions.zip 提取 SIC 失败，已保留 DuckDB 历史 SIC 分类: {err:#}"
                        )),
                        StagedSecFetch::Unavailable,
                    ))
                }
            }
        }
        Ok(StagedSecFetch::Unavailable) => {
            let (merged_map, _) = merge_only(load_cached().unwrap_or_default());
            Ok(make_outcome(
                merged_map,
                false,
                None,
                None,
                StagedSecFetch::Unavailable,
            ))
        }
        Err(err) => {
            let fallback_staged = if sub_zip_valid {
                StagedSecFetch::NotModified
            } else {
                StagedSecFetch::Unavailable
            };
            if let Some(cached) = load_cached() {
                let (merged_map, _) = merge_only(cached);
                return Ok(make_outcome(
                    merged_map,
                    false,
                    None,
                    Some(format!(
                        "SEC submissions.zip 同步失败，已回退本地 SIC 缓存与历史分类: {err:#}"
                    )),
                    fallback_staged,
                ));
            }
            if sub_zip_valid
                && let Ok(extracted) = extract_sic_from_submissions_zip(sub_zip_path, listed_ciks)
                && !extracted.is_empty()
            {
                let (merged_map, _) = merge_only(extracted);
                return Ok(make_outcome(
                    merged_map,
                    false,
                    None,
                    Some(format!(
                        "SEC submissions.zip 下载失败，已回退本地历史 submissions.zip: {err:#}"
                    )),
                    StagedSecFetch::NotModified,
                ));
            }
            let (merged_map, _) = merge_only(HashMap::new());
            let msg = if merged_map.is_empty() {
                format!(
                    "SEC submissions.zip 不可用且无本地 SIC 缓存，受影响公司将标记 UNCLASSIFIED_SIC 并排除出方案 1 质量池: {err:#}"
                )
            } else {
                format!(
                    "SEC submissions.zip 不可用，已从 DuckDB 保留 {} 条已验证 SIC 分类: {err:#}",
                    merged_map.len()
                )
            };
            Ok(make_outcome(
                merged_map,
                false,
                None,
                Some(msg),
                StagedSecFetch::Unavailable,
            ))
        }
    }
}

pub(crate) fn extract_sic_from_submissions_zip(
    zip_path: &Path,
    listed_ciks: &HashSet<String>,
) -> Result<HashMap<String, SicInfo>> {
    let file = File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("open submissions zip {}", zip_path.display()))?;

    let mut matched_indices: Vec<(usize, String)> = Vec::new();
    for idx in 0..archive.len() {
        let Some(Ok(name)) = archive.name_for_index(idx) else {
            continue;
        };
        if let Some(cik) = match_cik_json_entry(&name)
            && listed_ciks.contains(&cik)
        {
            matched_indices.push((idx, cik));
        }
    }

    let mut out = HashMap::with_capacity(matched_indices.len());
    let mut buffers: Vec<(String, Vec<u8>)> = Vec::with_capacity(CHUNK_SIZE);
    let mut batch_bytes: usize = 0;

    let mut corrupt_entries = 0usize;
    let mut first_corrupt_err: Option<String> = None;

    let flush_sic_batch = |batch: &mut Vec<(String, Vec<u8>)>,
                           out: &mut HashMap<String, SicInfo>,
                           corrupt_entries: &mut usize,
                           first_corrupt_err: &mut Option<String>| {
        if batch.is_empty() {
            return;
        }
        let items = std::mem::take(batch);
        let parsed: Vec<(String, std::result::Result<Option<SicInfo>, String>)> = items
            .into_par_iter()
            .map(|(cik, bytes)| {
                let res =
                    parse_submission_sic_bytes_checked(&bytes).map_err(|e| format!("{cik}: {e}"));
                (cik, res)
            })
            .collect();
        for (cik, res) in parsed {
            match res {
                Ok(Some(info)) => {
                    out.insert(cik, info);
                }
                Ok(None) => {}
                Err(err) => {
                    *corrupt_entries += 1;
                    if first_corrupt_err.is_none() {
                        *first_corrupt_err = Some(err);
                    }
                }
            }
        }
    };

    for (idx, cik) in matched_indices {
        let mut entry = match archive.by_index(idx) {
            Ok(e) => e,
            Err(err) => {
                corrupt_entries += 1;
                if first_corrupt_err.is_none() {
                    first_corrupt_err = Some(format!("{cik}: {err}"));
                }
                continue;
            }
        };
        let claimed = entry.size();
        let buf =
            match read_zip_entry_bounded(&mut entry, claimed, MAX_SUBMISSION_ENTRY_BYTES, &cik) {
                Ok(b) => b,
                Err(err) => {
                    corrupt_entries += 1;
                    if first_corrupt_err.is_none() {
                        first_corrupt_err = Some(err);
                    }
                    continue;
                }
            };
        batch_bytes += buf.len();
        buffers.push((cik, buf));
        if buffers.len() >= CHUNK_SIZE || batch_bytes >= MAX_ZIP_BATCH_BYTES {
            flush_sic_batch(
                &mut buffers,
                &mut out,
                &mut corrupt_entries,
                &mut first_corrupt_err,
            );
            batch_bytes = 0;
        }
    }
    flush_sic_batch(
        &mut buffers,
        &mut out,
        &mut corrupt_entries,
        &mut first_corrupt_err,
    );
    ensure!(
        corrupt_entries == 0,
        "submissions.zip failed decompression or JSON decode on {corrupt_entries} entries ({})",
        first_corrupt_err.as_deref().unwrap_or("unknown error")
    );
    Ok(out)
}

#[cfg(test)]
pub(crate) fn parse_submission_sic_bytes(bytes: &[u8]) -> Option<SicInfo> {
    parse_submission_sic_bytes_checked(bytes).ok().flatten()
}

fn parse_submission_sic_bytes_checked(
    bytes: &[u8],
) -> std::result::Result<Option<SicInfo>, String> {
    #[derive(Deserialize)]
    struct SubmissionMeta {
        #[serde(default)]
        sic: Option<serde_json::Value>,
        #[serde(default, rename = "sicDescription")]
        sic_description: Option<String>,
    }

    let meta: SubmissionMeta =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid submission JSON: {e}"))?;
    let sic = meta.sic.as_ref().and_then(|v| {
        if let Some(n) = v.as_u64() {
            u32::try_from(n).ok().filter(|s| *s > 0 && *s <= 9999)
        } else if let Some(s) = v.as_str() {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<u32>().ok().filter(|n| *n > 0 && *n <= 9999)
            }
        } else {
            None
        }
    });
    let sic_description = meta
        .sic_description
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    if sic.is_none() && sic_description.is_none() {
        return Ok(None);
    }
    Ok(Some(SicInfo {
        sic,
        sic_description,
    }))
}

pub fn is_financial_sic(sic: Option<u32>) -> bool {
    sic.is_some_and(|s| (6000..=6799).contains(&s))
}

pub(crate) fn parse_canonical_tickers_exchange(
    bytes: &[u8],
    watchlist_cik_tickers: &HashMap<String, String>,
) -> Result<Vec<ListedCompany>> {
    #[derive(Deserialize)]
    struct RawTickersExchange {
        fields: Vec<String>,
        data: Vec<Vec<serde_json::Value>>,
    }

    let parsed: RawTickersExchange =
        serde_json::from_slice(bytes).context("parse SEC company_tickers_exchange.json")?;
    let cik_idx = parsed
        .fields
        .iter()
        .position(|f| f.eq_ignore_ascii_case("cik"))
        .context("company_tickers_exchange.json missing 'cik' field")?;
    let name_idx = parsed
        .fields
        .iter()
        .position(|f| f.eq_ignore_ascii_case("name"))
        .context("company_tickers_exchange.json missing 'name' field")?;
    let ticker_idx = parsed
        .fields
        .iter()
        .position(|f| f.eq_ignore_ascii_case("ticker"))
        .context("company_tickers_exchange.json missing 'ticker' field")?;
    let ex_idx = parsed
        .fields
        .iter()
        .position(|f| f.eq_ignore_ascii_case("exchange"))
        .context("company_tickers_exchange.json missing 'exchange' field")?;

    let mut by_cik: BTreeMap<String, Vec<(usize, ListedCompany)>> = BTreeMap::new();
    for (row_idx, row) in parsed.data.iter().enumerate() {
        let Some(ex) = row.get(ex_idx).and_then(|v| v.as_str()).map(str::trim) else {
            continue;
        };
        if !is_major_us_exchange(ex) {
            continue;
        }
        let Some(ticker_raw) = row.get(ticker_idx).and_then(|v| v.as_str()) else {
            continue;
        };
        let ticker = ticker_raw.trim().to_ascii_uppercase();
        if ticker.is_empty() {
            continue;
        }
        let cik_raw = match row.get(cik_idx) {
            Some(v) if v.as_u64().is_some() => v.as_u64().unwrap_or(0).to_string(),
            Some(v) if v.as_str().is_some() => v.as_str().unwrap_or("").to_owned(),
            _ => continue,
        };
        let Ok(cik) = edgar::normalize_cik(&cik_raw) else {
            continue;
        };
        let name = row
            .get(name_idx)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&ticker)
            .to_owned();

        by_cik.entry(cik.clone()).or_default().push((
            row_idx,
            ListedCompany {
                cik,
                ticker,
                name,
                exchange: canonical_exchange_label(ex),
                is_non_common_listing: false,
            },
        ));
    }

    let mut canonical = Vec::with_capacity(by_cik.len());
    for (cik, candidates) in by_cik {
        let sibling_tickers: Vec<String> =
            candidates.iter().map(|(_, c)| c.ticker.clone()).collect();

        let best = candidates.iter().min_by_key(|(row_idx, c)| {
            ticker_preference_key(*row_idx, &c.ticker, &c.name, &sibling_tickers)
        });
        let Some((_, primary)) = best else {
            continue;
        };
        let primary_is_non_common = is_non_common_ticker(&primary.ticker, &sibling_tickers);

        if !primary_is_non_common
            && let Some(preferred) = watchlist_cik_tickers.get(&cik)
            && let Some((_, matched)) = candidates
                .iter()
                .find(|(_, c)| c.ticker.eq_ignore_ascii_case(preferred))
            && !is_non_common_ticker(&matched.ticker, &sibling_tickers)
            && !calculator::is_supplemental_security_ticker(&matched.ticker, &primary.ticker)
            && calculator::are_verified_common_share_class_siblings(
                &matched.ticker,
                &primary.ticker,
                &sibling_tickers,
            )
        {
            let mut chosen = matched.clone();
            chosen.is_non_common_listing = false;
            canonical.push(chosen);
            continue;
        }

        let mut chosen = primary.clone();
        chosen.is_non_common_listing = primary_is_non_common;
        canonical.push(chosen);
    }
    Ok(canonical)
}

fn is_major_us_exchange(ex: &str) -> bool {
    matches!(
        ex.trim().to_ascii_lowercase().as_str(),
        "nasdaq" | "nyse" | "cboe"
    )
}

fn canonical_exchange_label(ex: &str) -> String {
    match ex.trim().to_ascii_lowercase().as_str() {
        "nasdaq" => "Nasdaq".to_owned(),
        "nyse" => "NYSE".to_owned(),
        "cboe" => "CBOE".to_owned(),
        other => other.to_owned(),
    }
}

pub(crate) use crate::calculator::{is_non_common_ticker, is_share_basis_mismatched};

fn row_has_share_basis_mismatch(row: &UniverseRow) -> bool {
    row.warnings_json.contains("\"SHARE_BASIS_MISMATCH\"")
        || is_share_basis_mismatched(&row.ticker, false)
}

fn ticker_preference_key(
    row_idx: usize,
    ticker: &str,
    issuer_name: &str,
    sibling_tickers: &[String],
) -> (u8, u8, u8, usize) {
    let non_common_penalty = u8::from(is_non_common_ticker(ticker, sibling_tickers));
    let class_punct_penalty = u8::from(ticker.contains('-') || ticker.contains('.'));
    let name_mismatch_penalty =
        u8::from(!calculator::ticker_matches_issuer_name(ticker, issuer_name));
    (
        non_common_penalty,
        class_punct_penalty,
        name_mismatch_penalty,
        row_idx,
    )
}

pub(crate) fn canonical_ticker_for_cik_in_tickers_exchange(
    bytes: &[u8],
    target_cik: &str,
) -> Option<String> {
    let listed = parse_canonical_tickers_exchange(bytes, &HashMap::new()).ok()?;
    listed
        .into_iter()
        .find(|co| co.cik == target_cik)
        .map(|co| co.ticker)
}

fn match_cik_json_entry(entry_name: &str) -> Option<String> {
    let base = entry_name.rsplit('/').next().unwrap_or(entry_name);
    let stem = base.strip_prefix("CIK")?.strip_suffix(".json")?;
    if stem.len() == 10 && stem.chars().all(|c| c.is_ascii_digit()) {
        Some(stem.to_owned())
    } else {
        None
    }
}

struct WatchlistLookup {
    by_ticker: HashMap<String, CompanyEntry>,
    by_cik: HashMap<String, CompanyEntry>,
    cik_to_ticker: HashMap<String, String>,
}

impl WatchlistLookup {
    fn from_opt(wl: Option<&WatchlistFile>) -> Self {
        let mut by_ticker = HashMap::new();
        let mut by_cik = HashMap::new();
        let mut cik_to_ticker = HashMap::new();
        if let Some(file) = wl {
            let mut siblings_by_cik: HashMap<String, Vec<String>> = HashMap::new();
            for co in &file.companies {
                let upper = co.ticker.trim().to_ascii_uppercase();
                if let Some(cik) = co.cik.as_deref().and_then(|c| edgar::normalize_cik(c).ok()) {
                    siblings_by_cik.entry(cik).or_default().push(upper);
                }
            }
            for co in &file.companies {
                let upper = co.ticker.trim().to_ascii_uppercase();
                by_ticker.insert(upper.clone(), co.clone());
                if let Some(cik) = co.cik.as_deref().and_then(|c| edgar::normalize_cik(c).ok()) {
                    let siblings = siblings_by_cik.get(&cik).map(Vec::as_slice).unwrap_or(&[]);
                    let is_supplemental = calculator::is_supplemental_security_ticker(&upper, "")
                        || calculator::is_non_common_ticker(&upper, siblings);
                    if !is_supplemental && !by_cik.contains_key(&cik) {
                        cik_to_ticker.insert(cik.clone(), upper);
                        by_cik.insert(cik, co.clone());
                    }
                }
            }
        }
        Self {
            by_ticker,
            by_cik,
            cik_to_ticker,
        }
    }

    fn find(&self, ticker: &str, cik: &str) -> Option<&CompanyEntry> {
        self.by_ticker
            .get(&ticker.to_ascii_uppercase())
            .or_else(|| {
                self.by_cik
                    .get(cik)
                    .filter(|co| !calculator::is_supplemental_security_ticker(&co.ticker, ticker))
            })
    }
}

struct ParsedBundle {
    matched_entries: usize,
    parsed_companies: usize,
    skipped_companies: usize,
    skip_samples: Vec<SkipDiagnostic>,
    companies: Vec<LakeCompanyRow>,
    annuals: Vec<LakeAnnualRow>,
    ttms: Vec<LakeTtmRow>,
    snapshot: Vec<UniverseRow>,
}

enum ChunkParseOutcome {
    Parsed {
        annuals: Vec<LakeAnnualRow>,
        ttm: Option<Box<LakeTtmRow>>,
        universe: UniverseRow,
    },
    Skipped {
        skip: SkipDiagnostic,
        universe: UniverseRow,
    },
}

fn parse_companyfacts_zip_streaming<F: FnMut(&str)>(
    zip_path: &Path,
    listed: &[ListedCompany],
    sic_map: &HashMap<String, SicInfo>,
    watchlist: &WatchlistLookup,
    cached_prices: &HashMap<String, PriceSnapshot>,
    now_ms: u64,
    on_progress: &mut F,
) -> Result<ParsedBundle> {
    let file = File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("open companyfacts zip {}", zip_path.display()))?;

    let listed_by_cik: HashMap<&str, &ListedCompany> =
        listed.iter().map(|c| (c.cik.as_str(), c)).collect();

    let mut target_entries: Vec<(usize, ListedCompany)> = Vec::with_capacity(listed.len());
    let mut matched_ciks: HashSet<String> = HashSet::with_capacity(listed.len());
    for idx in 0..archive.len() {
        let Some(Ok(name)) = archive.name_for_index(idx) else {
            continue;
        };
        if let Some(cik) = match_cik_json_entry(&name)
            && let Some(co) = listed_by_cik.get(cik.as_str())
            && matched_ciks.insert(cik)
        {
            target_entries.push((idx, (*co).clone()));
        }
    }

    let matched_entries = target_entries.len();
    let mut companies = Vec::with_capacity(listed.len());
    let mut annuals = Vec::with_capacity(matched_entries * 6);
    let mut ttms = Vec::with_capacity(matched_entries);
    let mut snapshot = Vec::with_capacity(listed.len());
    let mut skip_samples = Vec::new();
    let mut parsed_companies = 0usize;
    let mut skipped_companies = 0usize;

    for co in listed {
        let sic_info = sic_map.get(&co.cik);
        let sic = sic_info.and_then(|s| s.sic);
        let sic_description = sic_info.and_then(|s| s.sic_description.clone());
        companies.push(LakeCompanyRow {
            cik: co.cik.clone(),
            ticker: co.ticker.clone(),
            name: co.name.clone(),
            exchange: co.exchange.clone(),
            sic,
            sic_description,
            is_financial: is_financial_sic(sic),
            is_active: true,
        });
    }

    let total_targets = target_entries.len();
    let mut processed = 0usize;
    let mut raw_items: Vec<(ListedCompany, Result<Vec<u8>, String>)> =
        Vec::with_capacity(CHUNK_SIZE);
    let mut batch_bytes: usize = 0;

    let mut flush_facts_batch = |items: &mut Vec<(ListedCompany, Result<Vec<u8>, String>)>,
                                 done_count: usize| {
        if items.is_empty() {
            return;
        }
        if total_targets > 0 {
            on_progress(&format!(
                "流式解压并并行解析 XBRL ({done_count}/{total_targets})..."
            ));
        }
        let batch = std::mem::take(items);
        let outcomes: Vec<ChunkParseOutcome> = batch
            .into_par_iter()
            .map(|(co, bytes_res)| {
                let sic_info = sic_map.get(&co.cik);
                let wl_entry = watchlist.find(&co.ticker, &co.cik);
                let price_snap = cached_prices.get(&co.ticker);
                match bytes_res {
                    Ok(bytes) => parse_single_company_bytes(
                        &co, &bytes, sic_info, wl_entry, price_snap, now_ms,
                    ),
                    Err(reason) => ChunkParseOutcome::Skipped {
                        universe: build_unparsed_universe_row(&co, sic_info, wl_entry, price_snap),
                        skip: SkipDiagnostic {
                            ticker: co.ticker,
                            cik: co.cik,
                            reason,
                        },
                    },
                }
            })
            .collect();

        for outcome in outcomes {
            match outcome {
                ChunkParseOutcome::Parsed {
                    annuals: mut co_annuals,
                    ttm,
                    universe,
                } => {
                    parsed_companies += 1;
                    annuals.append(&mut co_annuals);
                    if let Some(t) = ttm {
                        ttms.push(*t);
                    }
                    snapshot.push(universe);
                }
                ChunkParseOutcome::Skipped { skip, universe } => {
                    skipped_companies += 1;
                    if skip_samples.len() < MAX_SKIP_SAMPLES {
                        skip_samples.push(skip);
                    }
                    snapshot.push(universe);
                }
            }
        }
    };

    let mut zip_io_errors = 0usize;
    let mut first_zip_err: Option<String> = None;
    for (idx, co) in target_entries {
        let read_res = (|| -> Result<Vec<u8>, String> {
            let entry = archive
                .by_index(idx)
                .map_err(|e| format!("zip entry read failed: {e}"))?;
            let claimed = entry.size();
            read_zip_entry_bounded(entry, claimed, MAX_FACTS_ENTRY_BYTES, &co.ticker)
        })();
        match &read_res {
            Ok(buf) => batch_bytes += buf.len(),
            Err(err) => {
                zip_io_errors += 1;
                if first_zip_err.is_none() {
                    first_zip_err = Some(format!("{} ({}): {err}", co.ticker, co.cik));
                }
            }
        }
        processed += 1;
        raw_items.push((co, read_res));
        if raw_items.len() >= CHUNK_SIZE || batch_bytes >= MAX_ZIP_BATCH_BYTES {
            flush_facts_batch(&mut raw_items, processed);
            batch_bytes = 0;
        }
    }
    flush_facts_batch(&mut raw_items, processed);

    ensure!(
        zip_io_errors == 0,
        "SEC companyfacts.zip decompression failed on {zip_io_errors} entries (refusing partial lakehouse replacement): {}",
        first_zip_err.as_deref().unwrap_or("unknown zip error")
    );
    if matched_entries > 0 {
        ensure!(
            parsed_companies > 0,
            "SEC companyfacts.zip parsed zero listed companies out of {matched_entries} matched entries"
        );
    }

    // Include listed companies not present in companyfacts.zip at all.
    for co in listed {
        if !matched_ciks.contains(&co.cik) {
            let sic_info = sic_map.get(&co.cik);
            let wl_entry = watchlist.find(&co.ticker, &co.cik);
            let price_snap = cached_prices.get(&co.ticker);
            skipped_companies += 1;
            if skip_samples.len() < MAX_SKIP_SAMPLES {
                skip_samples.push(SkipDiagnostic {
                    ticker: co.ticker.clone(),
                    cik: co.cik.clone(),
                    reason: "missing from companyfacts.zip".to_owned(),
                });
            }
            snapshot.push(build_unparsed_universe_row(
                co, sic_info, wl_entry, price_snap,
            ));
        }
    }

    Ok(ParsedBundle {
        matched_entries,
        parsed_companies,
        skipped_companies,
        skip_samples,
        companies,
        annuals,
        ttms,
        snapshot,
    })
}

pub(crate) fn default_assumptions() -> ValuationAssumptions {
    ValuationAssumptions {
        multiple_low: Decimal::from(18),
        multiple_high: Decimal::from(22),
        target_return_pct: Decimal::from(10),
        expected_growth_pct: Decimal::from(8),
        strike_price_override: None,
    }
}

fn default_company_entry(co: &ListedCompany) -> CompanyEntry {
    CompanyEntry {
        ticker: co.ticker.clone(),
        name: co.name.clone(),
        bucket: Bucket::Watch,
        source: DataSource::SecEdgar,
        cik: Some(co.cik.clone()),
        currency: "USD".to_owned(),
        circle: "US Equity".to_owned(),
        too_hard_reason: None,
        assumptions: default_assumptions(),
    }
}

fn parse_single_company_bytes(
    co: &ListedCompany,
    raw_bytes: &[u8],
    sic_info: Option<&SicInfo>,
    wl_entry: Option<&CompanyEntry>,
    cached_price: Option<&PriceSnapshot>,
    now_ms: u64,
) -> ChunkParseOutcome {
    let source_url = format!(
        "https://data.sec.gov/api/xbrl/companyfacts/CIK{}.json",
        co.cik
    );
    let facts = match edgar::parse_companyfacts_bytes(
        raw_bytes,
        &co.ticker,
        &co.cik,
        &source_url,
        now_ms,
    ) {
        Ok(f) => f,
        Err(err) => {
            return ChunkParseOutcome::Skipped {
                universe: build_unparsed_universe_row(co, sic_info, wl_entry, cached_price),
                skip: SkipDiagnostic {
                    ticker: co.ticker.clone(),
                    cik: co.cik.clone(),
                    reason: format!("{err:#}"),
                },
            };
        }
    };

    let entry = wl_entry
        .cloned()
        .unwrap_or_else(|| default_company_entry(co));
    let report = calculator::build_report(&entry, Some(&facts), cached_price, now_ms);

    let (annuals, ttm) = extract_lake_facts_rows(co, &facts, &report);
    let universe = build_parsed_universe_row(co, sic_info, wl_entry, Some(&facts), &report, now_ms);

    ChunkParseOutcome::Parsed {
        annuals,
        ttm: ttm.map(Box::new),
        universe,
    }
}

pub(crate) fn clear_out_of_pool_quote_fields(rows: &mut [UniverseRow]) {
    for row in rows {
        if !row.in_price_pool {
            row.latest_price = None;
            row.distance_to_strike_pct = None;
            row.strict_fcf_yield_pct = None;
        }
    }
}

pub(crate) fn extract_lake_facts_rows(
    co: &ListedCompany,
    facts: &RawCompanyFactsSummary,
    report: &MungerReport,
) -> (Vec<LakeAnnualRow>, Option<LakeTtmRow>) {
    let mut annuals = Vec::with_capacity(report.annual_rows.len());
    for (raw_a, metric_a) in facts.annuals.iter().zip(report.annual_rows.iter()) {
        let provenance_json =
            serde_json::to_string(&raw_a.provenance).unwrap_or_else(|_| "[]".to_owned());
        let cash_absent = calculator::has_absent_provenance(&raw_a.provenance, &["cash_and_st"]);
        annuals.push(LakeAnnualRow {
            cik: co.cik.clone(),
            ticker: co.ticker.clone(),
            fiscal_year: raw_a.fiscal_year,
            label: metric_a.label.clone(),
            period_end: metric_a.period_end.clone(),
            filed: raw_a.filed.clone(),
            revenue: metric_a.revenue,
            gross_profit: metric_a.gross_profit,
            gross_margin_pct: metric_a.gross_margin_pct,
            ebit: metric_a.ebit,
            nopat: metric_a.nopat,
            invested_capital: metric_a.invested_capital,
            roic_pct: metric_a.roic_pct,
            roic_note: metric_a.roic_note.clone(),
            roic_total_capital_pct: metric_a.roic_total_capital_pct,
            ocf: metric_a.ocf,
            capex: metric_a.capex,
            fcf: metric_a.fcf,
            sbc: metric_a.sbc,
            strict_owner_earnings: metric_a.strict_owner_earnings,
            sbc_to_ocf_pct: metric_a.sbc_to_ocf_pct,
            buybacks: metric_a.buybacks,
            cash_and_st: (!cash_absent).then_some(metric_a.cash_and_st).flatten(),
            total_debt: metric_a.total_debt,
            net_debt: metric_a.net_debt,
            equity: metric_a.equity,
            diluted_shares: metric_a.diluted_shares,
            ending_shares: raw_a.ending_shares,
            raw_sha256: facts.raw_sha256.clone(),
            provenance_json,
        });
    }

    let ttm = match (facts.ttm.as_ref(), report.ttm_row.as_ref()) {
        (Some(raw_t), Some(metric_t)) => {
            let provenance_json =
                serde_json::to_string(&raw_t.provenance).unwrap_or_else(|_| "[]".to_owned());
            let ttm_tax_absent = calculator::has_absent_provenance(
                &raw_t.provenance,
                &["ttm_tax_expense", "tax_expense"],
            );
            let ttm_sbc_absent =
                calculator::has_absent_provenance(&raw_t.provenance, &["ttm_sbc", "sbc"]);
            let ttm_buybacks_absent =
                calculator::has_absent_provenance(&raw_t.provenance, &["ttm_buybacks", "buybacks"]);
            let ttm_debt_absent = calculator::has_absent_provenance(
                &raw_t.provenance,
                &["ttm_total_debt", "total_debt"],
            );
            let ttm_cash_absent = calculator::has_absent_provenance(
                &raw_t.provenance,
                &["ttm_cash_and_st", "cash_and_st"],
            );
            let ytd_rev_absent =
                calculator::has_absent_provenance(&raw_t.provenance, &["ytd_revenue"]);
            let ytd_ocf_absent = calculator::has_absent_provenance(&raw_t.provenance, &["ytd_ocf"]);
            let ytd_capex_absent =
                calculator::has_absent_provenance(&raw_t.provenance, &["ytd_capex"]);
            let ytd_sbc_absent = calculator::has_absent_provenance(&raw_t.provenance, &["ytd_sbc"]);
            let ytd_buybacks_absent =
                calculator::has_absent_provenance(&raw_t.provenance, &["ytd_buybacks"]);
            let roic_available = !ttm_tax_absent && !ttm_debt_absent && !ttm_cash_absent;
            let total_cap_roic_available = !ttm_tax_absent && !ttm_debt_absent;
            Some(LakeTtmRow {
                cik: co.cik.clone(),
                ticker: co.ticker.clone(),
                label: metric_t.label.clone(),
                period_end: metric_t.period_end.clone(),
                revenue: metric_t.revenue,
                gross_profit: metric_t.gross_profit,
                gross_margin_pct: metric_t.gross_margin_pct,
                ebit: metric_t.ebit,
                nopat: (!ttm_tax_absent).then_some(metric_t.nopat).flatten(),
                invested_capital: (!ttm_debt_absent && !ttm_cash_absent)
                    .then_some(metric_t.invested_capital)
                    .flatten(),
                roic_pct: roic_available.then_some(metric_t.roic_pct).flatten(),
                roic_note: metric_t.roic_note.clone(),
                roic_total_capital_pct: total_cap_roic_available
                    .then_some(metric_t.roic_total_capital_pct)
                    .flatten(),
                ocf: metric_t.ocf,
                capex: metric_t.capex,
                fcf: metric_t.fcf,
                sbc: (!ttm_sbc_absent).then_some(metric_t.sbc).flatten(),
                strict_owner_earnings: (!ttm_sbc_absent)
                    .then_some(metric_t.strict_owner_earnings)
                    .flatten(),
                sbc_to_ocf_pct: (!ttm_sbc_absent)
                    .then_some(metric_t.sbc_to_ocf_pct)
                    .flatten(),
                buybacks: (!ttm_buybacks_absent)
                    .then_some(metric_t.buybacks)
                    .flatten(),
                cash_and_st: (!ttm_cash_absent).then_some(metric_t.cash_and_st).flatten(),
                total_debt: (!ttm_debt_absent).then_some(metric_t.total_debt).flatten(),
                net_debt: (!ttm_debt_absent && !ttm_cash_absent)
                    .then_some(metric_t.net_debt)
                    .flatten(),
                equity: metric_t.equity,
                diluted_shares: metric_t.diluted_shares,
                ytd_period_label: raw_t.latest_ytd.period_label.clone(),
                ytd_start: raw_t.latest_ytd.start.clone(),
                ytd_end: raw_t.latest_ytd.end.clone(),
                ytd_revenue: (!ytd_rev_absent)
                    .then_some(raw_t.latest_ytd.revenue)
                    .flatten(),
                ytd_ocf: (!ytd_ocf_absent).then_some(raw_t.latest_ytd.ocf).flatten(),
                ytd_capex: (!ytd_capex_absent)
                    .then_some(raw_t.latest_ytd.capex)
                    .flatten(),
                ytd_sbc: (!ytd_sbc_absent).then_some(raw_t.latest_ytd.sbc).flatten(),
                ytd_buybacks: (!ytd_buybacks_absent)
                    .then_some(raw_t.latest_ytd.buybacks)
                    .flatten(),
                latest_quarter_label: raw_t.latest_ytd.latest_quarter_label.clone(),
                latest_quarter_ocf: (!ytd_ocf_absent)
                    .then_some(raw_t.latest_ytd.latest_quarter_ocf)
                    .flatten(),
                latest_quarter_capex: (!ytd_capex_absent)
                    .then_some(raw_t.latest_ytd.latest_quarter_capex)
                    .flatten(),
                raw_sha256: facts.raw_sha256.clone(),
                provenance_json,
            })
        }
        _ => None,
    };

    (annuals, ttm)
}

fn is_filing_stale(latest_filing_end: Option<&str>, now_ms: u64) -> bool {
    let Some(end_iso) = latest_filing_end.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let Ok(end_days) = edgar::parse_date_days(end_iso) else {
        return true;
    };
    let now_days = (now_ms / 86_400_000) as i64;
    now_days.saturating_sub(end_days) > MAX_FILING_STALENESS_DAYS
}

pub(crate) fn build_parsed_universe_row(
    co: &ListedCompany,
    sic_info: Option<&SicInfo>,
    wl_entry: Option<&CompanyEntry>,
    facts: Option<&RawCompanyFactsSummary>,
    report: &MungerReport,
    now_ms: u64,
) -> UniverseRow {
    let latest_prov: &[crate::model::FieldProvenance] = facts
        .and_then(|f| {
            f.ttm
                .as_ref()
                .map(|t| t.provenance.as_slice())
                .or_else(|| f.annuals.last().map(|a| a.provenance.as_slice()))
        })
        .unwrap_or(&[]);
    build_parsed_universe_row_with_latest_prov(co, sic_info, wl_entry, latest_prov, report, now_ms)
}

pub(crate) fn build_parsed_universe_row_with_latest_prov(
    co: &ListedCompany,
    sic_info: Option<&SicInfo>,
    wl_entry: Option<&CompanyEntry>,
    latest_prov: &[crate::model::FieldProvenance],
    report: &MungerReport,
    now_ms: u64,
) -> UniverseRow {
    let sic = sic_info.and_then(|s| s.sic);
    let sic_description = sic_info.and_then(|s| s.sic_description.clone());
    let is_financial = is_financial_sic(sic);
    let years_of_data = report.annual_rows.len();

    let sbc_unavailable = calculator::has_absent_provenance(latest_prov, &["ttm_sbc", "sbc"]);

    let latest_row = report
        .ttm_row
        .as_ref()
        .or_else(|| report.annual_rows.last());
    let strict_owner_earnings = (!sbc_unavailable)
        .then(|| latest_row.and_then(|r| r.strict_owner_earnings))
        .flatten();
    let sbc_to_ocf_pct = latest_row.and_then(|r| r.sbc_to_ocf_pct);
    let primary_val = report
        .ttm_valuation
        .as_ref()
        .or(report.fy_valuation.as_ref());

    let share_basis_mismatch = is_share_basis_mismatched(&co.ticker, co.is_non_common_listing)
        || report
            .warnings
            .iter()
            .any(|w| w.code == "SHARE_BASIS_MISMATCH");
    let consecutive_5y =
        report.annual_rows.len() >= 5 && calculator::last_5_years_consecutive(&report.annual_rows);
    let stale_financials = report.annual_rows.is_empty()
        || is_filing_stale(report.latest_filing_end.as_deref(), now_ms);
    let has_5y_roic = consecutive_5y
        && report
            .annual_rows
            .iter()
            .rev()
            .take(5)
            .all(|r| r.roic_pct.is_some());
    let has_5y_gm = consecutive_5y
        && report
            .annual_rows
            .iter()
            .rev()
            .take(5)
            .all(|r| r.gross_margin_pct.is_some());

    let mut warnings = report.warnings.clone();
    if share_basis_mismatch && !warnings.iter().any(|w| w.code == "SHARE_BASIS_MISMATCH") {
        warnings.push(crate::model::WarningFlag {
            code: "SHARE_BASIS_MISMATCH".to_owned(),
            severity: WarningSeverity::Amber,
            title: "股本口径与报价类别不一致".to_owned(),
            detail: format!(
                "{} 属于非普通股或与财报稀释股本口径不一致（如 BRK-B），已抑制每股估值",
                co.ticker
            ),
        });
    }
    if years_of_data >= 5 && !consecutive_5y {
        warnings.push(crate::model::WarningFlag {
            code: "NON_CONSECUTIVE_5Y".to_owned(),
            severity: WarningSeverity::Info,
            title: "最近 5 年财报年份不连续".to_owned(),
            detail: "最近 5 份年报存在缺失年份断层，已抑制 5 年均值与方案 1 质量池资格".to_owned(),
        });
    }
    if stale_financials {
        warnings.push(crate::model::WarningFlag {
            code: "STALE_FINANCIALS".to_owned(),
            severity: WarningSeverity::Amber,
            title: "最新有效财报已过期".to_owned(),
            detail: format!(
                "最近有效财报期 ({}) 距今已超过 {MAX_FILING_STALENESS_DAYS} 天，已从方案 1 活跃质量池排除",
                report.latest_filing_end.as_deref().unwrap_or("未知")
            ),
        });
    }
    if sic.is_none() {
        warnings.push(crate::model::WarningFlag {
            code: "UNCLASSIFIED_SIC".to_owned(),
            severity: WarningSeverity::Info,
            title: "缺少 SEC SIC 行业分类".to_owned(),
            detail: "缺少 SEC SIC 行业分类，已从方案 1 自动质量池排除（避免未识别金融股混入）"
                .to_owned(),
        });
    }

    let warnings_count = warnings.len();
    let warnings_json = serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".to_owned());

    let (strike_price, distance_to_strike_pct, strict_fcf_yield_pct) =
        if share_basis_mismatch || sbc_unavailable {
            (None, None, None)
        } else {
            (
                primary_val.and_then(|v| v.strike_price),
                primary_val.and_then(|v| v.distance_to_strike_pct),
                primary_val.and_then(|v| v.strict_fcf_yield_mcap_pct),
            )
        };

    let mut row = UniverseRow {
        ticker: co.ticker.clone(),
        cik: co.cik.clone(),
        name: co.name.clone(),
        exchange: co.exchange.clone(),
        sic,
        sic_description,
        is_financial,
        latest_filing_end: report.latest_filing_end.clone(),
        years_of_data,
        quality_score: Decimal::ZERO,
        in_price_pool: wl_entry.is_some(),
        in_watchlist: wl_entry.is_some(),
        bucket_opt: wl_entry.map(|w| w.bucket),
        latest_price: report.price.as_ref().map(|p| p.price),
        strike_price,
        distance_to_strike_pct,
        strict_fcf_yield_pct,
        roic_5y_avg_pct: has_5y_roic.then_some(report.roic_5y_avg_pct).flatten(),
        roic_5y_stddev_pct: has_5y_roic.then_some(report.roic_5y_stddev_pct).flatten(),
        gross_margin_5y_avg_pct: has_5y_gm
            .then_some(report.gross_margin_5y_avg_pct)
            .flatten(),
        gross_margin_5y_stddev_pct: has_5y_gm
            .then_some(report.gross_margin_5y_stddev_pct)
            .flatten(),
        share_change_3y_pct: report.share_change_3y_pct,
        sbc_to_ocf_pct,
        net_debt_to_oe: report.net_debt_to_owner_earnings,
        strict_owner_earnings,
        warnings_count,
        warnings_json,
        raw_sha256: report.raw_sha256.clone(),
    };
    row.quality_score = compute_quality_score_with_warnings(&row, report, &warnings);
    row
}

fn build_unparsed_universe_row(
    co: &ListedCompany,
    sic_info: Option<&SicInfo>,
    wl_entry: Option<&CompanyEntry>,
    cached_price: Option<&PriceSnapshot>,
) -> UniverseRow {
    let sic = sic_info.and_then(|s| s.sic);
    let mut warnings = Vec::new();
    if is_share_basis_mismatched(&co.ticker, co.is_non_common_listing) {
        warnings.push(crate::model::WarningFlag {
            code: "SHARE_BASIS_MISMATCH".to_owned(),
            severity: WarningSeverity::Amber,
            title: "股本口径与报价类别不一致".to_owned(),
            detail: format!(
                "{} 属于非普通股或与财报稀释股本口径不一致（如 BRK-B），已抑制每股估值",
                co.ticker
            ),
        });
    }
    let warnings_count = warnings.len();
    let warnings_json = serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".to_owned());

    UniverseRow {
        ticker: co.ticker.clone(),
        cik: co.cik.clone(),
        name: co.name.clone(),
        exchange: co.exchange.clone(),
        sic,
        sic_description: sic_info.and_then(|s| s.sic_description.clone()),
        is_financial: is_financial_sic(sic),
        latest_filing_end: None,
        years_of_data: 0,
        quality_score: Decimal::ZERO,
        in_price_pool: wl_entry.is_some(),
        in_watchlist: wl_entry.is_some(),
        bucket_opt: wl_entry.map(|w| w.bucket),
        latest_price: cached_price.map(|p| p.price),
        strike_price: None,
        distance_to_strike_pct: None,
        strict_fcf_yield_pct: None,
        roic_5y_avg_pct: None,
        roic_5y_stddev_pct: None,
        gross_margin_5y_avg_pct: None,
        gross_margin_5y_stddev_pct: None,
        share_change_3y_pct: None,
        sbc_to_ocf_pct: None,
        net_debt_to_oe: None,
        strict_owner_earnings: None,
        warnings_count,
        warnings_json,
        raw_sha256: None,
    }
}

#[allow(dead_code)]
pub fn compute_quality_score(row: &UniverseRow, report: &MungerReport) -> Decimal {
    compute_quality_score_with_warnings(row, report, &report.warnings)
}

fn compute_quality_score_with_warnings(
    row: &UniverseRow,
    report: &MungerReport,
    warnings: &[crate::model::WarningFlag],
) -> Decimal {
    if row.is_financial
        || row.years_of_data < 5
        || warnings.iter().any(|w| w.code == "NON_CONSECUTIVE_5Y")
    {
        return Decimal::ZERO;
    }

    let mut score = Decimal::ZERO;

    // 1. Data history (15 pts; strictly requires >= 5 valid fiscal years)
    score += Decimal::from(15);

    // 2. Positive strict owner earnings (0..15)
    let positive_oe = row
        .strict_owner_earnings
        .is_some_and(|oe| oe > Decimal::ZERO);
    if positive_oe {
        score += Decimal::from(15);
    }

    // 3. 5Y ROIC average (0..25)
    if let Some(roic) = row.roic_5y_avg_pct
        && roic > Decimal::ZERO
    {
        let roic_pts = if roic >= Decimal::from(30) {
            Decimal::from(25)
        } else if roic >= Decimal::from(20) {
            Decimal::from(20) + ((roic - Decimal::from(20)) / Decimal::from(2))
        } else if roic >= Decimal::from(15) {
            Decimal::from(15) + (roic - Decimal::from(15))
        } else if roic >= Decimal::from(10) {
            Decimal::from(8) + ((roic - Decimal::from(10)) * Decimal::new(14, 1))
        } else {
            (roic * Decimal::new(8, 1)).min(Decimal::from(8))
        };
        score += roic_pts;
    } else if positive_oe {
        // Negative invested capital (e.g. high buybacks or net cash) while earning positive OE:
        // check total capital ROIC as modest secondary quality signal (unavailable when tax_expense is absent).
        let latest_tot_roic = report
            .ttm_row
            .as_ref()
            .or_else(|| report.annual_rows.last())
            .and_then(|r| r.roic_total_capital_pct);
        if let Some(tot) = latest_tot_roic
            && tot >= Decimal::from(15)
        {
            score += Decimal::from(14);
        }
    }

    // 4. ROIC stability (0..10)
    if let (Some(avg), Some(std)) = (row.roic_5y_avg_pct, row.roic_5y_stddev_pct)
        && avg > Decimal::ZERO
    {
        score += if std <= Decimal::from(3) {
            Decimal::from(10)
        } else if std <= Decimal::from(8) {
            Decimal::from(7)
        } else if std <= Decimal::from(15) {
            Decimal::from(4)
        } else {
            Decimal::ONE
        };
    }

    // 5. Gross margin stability (0..10)
    if let Some(gm_std) = row.gross_margin_5y_stddev_pct {
        score += if gm_std <= Decimal::new(15, 1) {
            Decimal::from(10)
        } else if gm_std <= Decimal::from(3) {
            Decimal::from(7)
        } else if gm_std <= Decimal::from(5) {
            Decimal::from(4)
        } else {
            Decimal::ZERO
        };
    }

    // 6. 3Y share dilution / buyback discipline (0..10)
    if let Some(sh3) = row.share_change_3y_pct {
        score += if sh3 <= Decimal::ZERO {
            Decimal::from(10)
        } else if sh3 <= Decimal::from(2) {
            Decimal::from(7)
        } else if sh3 <= Decimal::from(5) {
            Decimal::from(3)
        } else {
            Decimal::ZERO
        };
    }

    // 7. SBC / OCF discipline (0..10; skipped when SBC is unavailable/absent)
    if positive_oe && let Some(sbc_pct) = row.sbc_to_ocf_pct {
        score += if sbc_pct <= Decimal::from(5) {
            Decimal::from(10)
        } else if sbc_pct <= Decimal::from(15) {
            Decimal::from(7)
        } else if sbc_pct <= Decimal::from(25) {
            Decimal::from(4)
        } else {
            Decimal::ZERO
        };
    }

    // 8. Balance sheet leverage: Net debt / OE (0..5; skipped when total_debt is unavailable/absent)
    if report.net_debt.is_some_and(|nd| nd <= Decimal::ZERO) {
        score += Decimal::from(5);
    } else if let Some(nd_oe) = row.net_debt_to_oe {
        score += if nd_oe <= Decimal::ONE {
            Decimal::from(5)
        } else if nd_oe <= Decimal::new(25, 1) {
            Decimal::from(3)
        } else if nd_oe <= Decimal::from(4) {
            Decimal::ONE
        } else {
            Decimal::ZERO
        };
    }

    // 9. Warning penalty
    for w in warnings {
        match w.severity {
            WarningSeverity::Red => score -= Decimal::from(15),
            WarningSeverity::Amber => score -= Decimal::from(3),
            WarningSeverity::Info => {}
        }
    }

    if score < Decimal::ZERO {
        Decimal::ZERO
    } else if score > Decimal::from(100) {
        Decimal::from(100)
    } else {
        score.round_dp(2)
    }
}

fn is_scheme1_eligible(row: &UniverseRow, now_ms: u64) -> bool {
    !row.is_financial
        && row.sic.is_some()
        && row.years_of_data >= 5
        && !row.warnings_json.contains("\"NON_CONSECUTIVE_5Y\"")
        && !is_filing_stale(row.latest_filing_end.as_deref(), now_ms)
        && row.strict_owner_earnings.is_some_and(|v| v > Decimal::ZERO)
        && !row_has_share_basis_mismatch(row)
}

pub(crate) fn assign_scheme1_quality_pool(
    snapshot: &mut [UniverseRow],
    prices_top: usize,
    now_ms: u64,
) -> usize {
    for row in snapshot.iter_mut() {
        row.in_price_pool = row.in_watchlist;
    }

    let mut ranked_indices: Vec<usize> = snapshot
        .iter()
        .enumerate()
        .filter(|(_, r)| is_scheme1_eligible(r, now_ms))
        .map(|(i, _)| i)
        .collect();

    ranked_indices.sort_by(|&a, &b| {
        let ra = &snapshot[a];
        let rb = &snapshot[b];
        rb.quality_score
            .cmp(&ra.quality_score)
            .then_with(|| ra.ticker.cmp(&rb.ticker))
    });

    for &idx in ranked_indices.iter().take(prices_top) {
        snapshot[idx].in_price_pool = true;
    }

    snapshot.iter().filter(|r| r.in_price_pool).count()
}

fn resolve_price_snapshot_with_cache(
    mkt_client: &Client,
    paths: &WorkspacePaths,
    ticker: &str,
    force_refresh: bool,
    now_ms: u64,
    consecutive_rate_limits: &mut usize,
    report: &mut UniverseSyncReport,
) -> Option<(PriceSnapshot, bool)> {
    report.prices_attempted += 1;
    let cached_opt = store::load_cached_price(paths, ticker)
        .filter(|c| market::validate_quote_session(c, now_ms).is_ok());
    let cache_is_fresh = !force_refresh
        && cached_opt
            .as_ref()
            .is_some_and(|c| now_ms.saturating_sub(c.fetched_at_ms) < PRICE_CACHE_FRESH_MS);

    if cache_is_fresh {
        report.prices_cached_fallback += 1;
        return cached_opt.map(|c| (c, false));
    }
    if *consecutive_rate_limits >= 3 {
        if cached_opt.is_some() {
            report.prices_cached_fallback += 1;
        } else {
            report.prices_failed += 1;
        }
        return cached_opt.map(|c| (c, false));
    }

    thread::sleep(Duration::from_millis(140));
    match market::fetch_price_snapshot_with_status(
        mkt_client,
        ticker,
        &paths.price_path(ticker),
        now_ms,
    ) {
        Ok((snap, accepted)) => {
            *consecutive_rate_limits = 0;
            if accepted {
                report.prices_succeeded += 1;
                Some((snap, true))
            } else {
                report.prices_cached_fallback += 1;
                Some((snap, false))
            }
        }
        Err(err) => {
            let msg = format!("{ticker}: {err:#}");
            if msg.contains("429") || msg.contains("Too Many Requests") {
                *consecutive_rate_limits += 1;
                thread::sleep(Duration::from_secs(2));
            }
            if report.price_error_samples.len() < MAX_PRICE_ERR_SAMPLES {
                report.price_error_samples.push(msg);
            }
            if cached_opt.is_some() {
                report.prices_cached_fallback += 1;
            } else {
                report.prices_failed += 1;
            }
            cached_opt.map(|c| (c, false))
        }
    }
}

struct PoolPriceSyncBatch {
    price_rows: Vec<LakePriceDailyRow>,
    updated_snapshot_rows: Vec<QuoteSnapshotUpdate>,
    expected_assumptions_digests: HashMap<String, String>,
    expected_assumptions_revs: HashMap<String, u64>,
    price_watermarks: Vec<SyncWatermark>,
    live_watchlist: Option<WatchlistFile>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn fetch_pool_prices_lock_free(
    paths: &WorkspacePaths,
    rows: &[UniverseRow],
    watchlist_file: Option<&WatchlistFile>,
    watchlist: &WatchlistLookup,
    basis_map: &HashMap<
        String,
        (
            &'static str,
            crate::model::AnnualMetricRow,
            calculator::ValuationGuards,
        ),
    >,
    assump_revs: &HashMap<String, u64>,
    force_refresh: bool,
    now_ms: u64,
    report: &mut UniverseSyncReport,
) -> Result<PoolPriceSyncBatch> {
    fetch_pool_prices_lock_free_with_progress(
        paths,
        rows,
        watchlist_file,
        watchlist,
        basis_map,
        assump_revs,
        force_refresh,
        now_ms,
        report,
        &mut |_, _| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn fetch_pool_prices_lock_free_with_progress(
    paths: &WorkspacePaths,
    rows: &[UniverseRow],
    watchlist_file: Option<&WatchlistFile>,
    watchlist: &WatchlistLookup,
    basis_map: &HashMap<
        String,
        (
            &'static str,
            crate::model::AnnualMetricRow,
            calculator::ValuationGuards,
        ),
    >,
    assump_revs: &HashMap<String, u64>,
    force_refresh: bool,
    now_ms: u64,
    report: &mut UniverseSyncReport,
    on_quote_progress: &mut dyn FnMut(usize, usize),
) -> Result<PoolPriceSyncBatch> {
    let mut pool_indices: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.in_price_pool)
        .map(|(i, _)| i)
        .collect();
    pool_indices.sort_by(|&a, &b| {
        let ra = &rows[a];
        let rb = &rows[b];
        rb.in_watchlist
            .cmp(&ra.in_watchlist)
            .then_with(|| rb.quality_score.cmp(&ra.quality_score))
            .then_with(|| ra.ticker.cmp(&rb.ticker))
    });

    let mkt_client = market::market_http_client()?;
    let mut consecutive_rate_limits = 0usize;
    let mut price_rows: Vec<LakePriceDailyRow> = Vec::with_capacity(pool_indices.len());
    let mut pending_pool_snaps: Vec<(&UniverseRow, PriceSnapshot)> =
        Vec::with_capacity(pool_indices.len());
    let mut synced_tickers: HashSet<String> = HashSet::new();
    let mut latest_live_trade_date: Option<String> = None;
    let total_pool = pool_indices.len();

    for (loop_idx, &idx) in pool_indices.iter().enumerate() {
        let row = &rows[idx];
        synced_tickers.insert(row.ticker.to_ascii_uppercase());
        let done_count = loop_idx + 1;
        let maybe_snap = resolve_price_snapshot_with_cache(
            &mkt_client,
            paths,
            &row.ticker,
            force_refresh,
            now_ms,
            &mut consecutive_rate_limits,
            report,
        );
        if done_count % 10 == 0 || done_count == total_pool {
            on_quote_progress(done_count, total_pool);
        }
        let Some((snap, is_live)) = maybe_snap else {
            continue;
        };

        let Some(trade_date) = market::trade_date_from_snapshot(&snap) else {
            continue;
        };
        if is_live
            && latest_live_trade_date
                .as_ref()
                .is_none_or(|prev| trade_date > *prev)
        {
            latest_live_trade_date = Some(trade_date.clone());
        }
        price_rows.push(LakePriceDailyRow {
            symbol: snap.symbol.clone(),
            trade_date,
            price: snap.price,
            previous_close: snap.previous_close,
            currency: snap.currency.clone(),
            exchange: snap.exchange.clone(),
            market_time_ms: snap.market_time_ms,
            fetched_at_ms: snap.fetched_at_ms,
            source_url: snap.source_url.clone(),
        });
        if !snap.symbol.eq_ignore_ascii_case(&row.ticker) {
            continue;
        }
        pending_pool_snaps.push((row, snap));
    }

    // Re-read live watchlist.json after potentially long quote network fetching and propagate
    // any parse/read failure instead of silently falling back to stale initial assumptions.
    let live_watchlist = load_watchlist_strict(paths)?;
    let live_lookup = live_watchlist
        .as_ref()
        .map(|wl| WatchlistLookup::from_opt(Some(wl)));
    let effective_lookup = live_lookup.as_ref().unwrap_or(watchlist);

    let rows_by_cik: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.cik.as_str(), i))
        .collect();
    let rows_by_ticker: HashMap<String, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.ticker.to_ascii_uppercase(), i))
        .collect();

    // Ensure newly admitted SEC watchlist companies, non-SEC watchlist items (e.g. DSY.PA, 1810.HK),
    // and the configured benchmark (e.g. ^GSPC) are synced into prices_daily, AND attach fetched
    // quotes for newly admitted SEC companies to snapshot updates.
    if let Some(wl) = live_watchlist.as_ref().or(watchlist_file) {
        let extra_symbols = wl
            .companies
            .iter()
            .map(|c| (Some(c), c.ticker.as_str()))
            .chain(std::iter::once((None, wl.benchmark_symbol.as_str())));
        for (wl_entry_opt, sym) in extra_symbols {
            let upper = sym.trim().to_ascii_uppercase();
            if upper.is_empty() || !synced_tickers.insert(upper.clone()) {
                continue;
            }
            let Some((snap, is_live)) = resolve_price_snapshot_with_cache(
                &mkt_client,
                paths,
                &upper,
                force_refresh,
                now_ms,
                &mut consecutive_rate_limits,
                report,
            ) else {
                continue;
            };
            let Some(trade_date) = market::trade_date_from_snapshot(&snap) else {
                continue;
            };
            if is_live
                && latest_live_trade_date
                    .as_ref()
                    .is_none_or(|prev| trade_date > *prev)
            {
                latest_live_trade_date = Some(trade_date.clone());
            }
            price_rows.push(LakePriceDailyRow {
                symbol: snap.symbol.clone(),
                trade_date,
                price: snap.price,
                previous_close: snap.previous_close,
                currency: snap.currency.clone(),
                exchange: snap.exchange.clone(),
                market_time_ms: snap.market_time_ms,
                fetched_at_ms: snap.fetched_at_ms,
                source_url: snap.source_url.clone(),
            });
            if let Some(wl_co) = wl_entry_opt {
                let matched_row_idx = wl_co
                    .cik
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .and_then(|c| edgar::normalize_cik(c).ok())
                    .and_then(|norm_cik| rows_by_cik.get(norm_cik.as_str()).copied())
                    .or_else(|| rows_by_ticker.get(&upper).copied())
                    .filter(|&idx| {
                        rows[idx].ticker.eq_ignore_ascii_case(&snap.symbol)
                            && rows[idx].ticker.eq_ignore_ascii_case(&wl_co.ticker)
                    });
                if let Some(idx) = matched_row_idx {
                    pending_pool_snaps.push((&rows[idx], snap));
                }
            }
        }
    }

    let mut updated_snapshot_rows: Vec<QuoteSnapshotUpdate> =
        Vec::with_capacity(pending_pool_snaps.len());
    let mut expected_assumptions_digests: HashMap<String, String> =
        HashMap::with_capacity(pending_pool_snaps.len());
    let mut expected_assumptions_revs: HashMap<String, u64> =
        HashMap::with_capacity(pending_pool_snaps.len());
    for (row, snap) in pending_pool_snaps {
        if !snap.symbol.eq_ignore_ascii_case(&row.ticker) {
            continue;
        }
        let expected_basis_period_end = basis_map
            .get(&row.cik)
            .map(|(_, metric_row, _)| metric_row.period_end.clone());
        let mut quote_update = QuoteSnapshotUpdate {
            cik: row.cik.clone(),
            expected_ticker: snap.symbol.clone(),
            expected_raw_sha256: row.raw_sha256.clone(),
            expected_latest_filing_end: row.latest_filing_end.clone(),
            expected_basis_period_end,
            quote_market_time_ms: snap.market_time_ms,
            quote_fetched_at_ms: snap.fetched_at_ms,
            latest_price: Some(snap.price),
            strike_price: row.strike_price,
            distance_to_strike_pct: None,
            strict_fcf_yield_pct: None,
        };
        let initial_assump = watchlist
            .find(&row.ticker, &row.cik)
            .map(|w| w.assumptions.clone())
            .unwrap_or_else(default_assumptions);
        let assumptions = effective_lookup
            .find(&row.ticker, &row.cik)
            .map(|w| w.assumptions.clone())
            .unwrap_or_else(default_assumptions);
        expected_assumptions_digests
            .insert(row.cik.clone(), valuation_assumptions_digest(&assumptions));
        if assumptions == initial_assump
            && let Some(&rev) = assump_revs.get(&row.cik)
            && rev > 0
        {
            expected_assumptions_revs.insert(row.cik.clone(), rev);
        }
        if row_has_share_basis_mismatch(row) {
            quote_update.strike_price = None;
            quote_update.distance_to_strike_pct = None;
            quote_update.strict_fcf_yield_pct = None;
        } else if let Some((basis_label, metric_row, guards)) = basis_map.get(&row.cik) {
            let val = calculator::compute_valuation(
                basis_label,
                metric_row,
                Some(&snap),
                &assumptions,
                *guards,
            );
            quote_update.strike_price = val.strike_price;
            quote_update.distance_to_strike_pct = val.distance_to_strike_pct;
            quote_update.strict_fcf_yield_pct = val.strict_fcf_yield_mcap_pct;
        }
        updated_snapshot_rows.push(quote_update);
    }

    // Only advance the Yahoo price watermark when at least one live quote succeeded with a session trade date.
    let should_advance_watermark = report.prices_succeeded > 0 && latest_live_trade_date.is_some();
    let mut price_watermarks = Vec::new();
    if should_advance_watermark {
        price_watermarks.push(SyncWatermark {
            source_key: WM_YAHOO_PRICES.to_owned(),
            cursor: latest_live_trade_date,
            etag: None,
            last_modified: None,
            synced_at_ms: now_ms,
        });
    }

    Ok(PoolPriceSyncBatch {
        price_rows,
        updated_snapshot_rows,
        expected_assumptions_digests,
        expected_assumptions_revs,
        price_watermarks,
        live_watchlist,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn canonical_tickers_dedup_filters_exchanges_and_avoids_warrants_and_preferreds() {
        let json = serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [
                [320193, "Apple Inc.", "AAPL", "Nasdaq"],
                [320193, "Apple Inc.", "AAPL-WT", "Nasdaq"],
                [320193, "Apple Inc.", "AAPL-WTA", "Nasdaq"],
                [320193, "Apple Inc.", "AAPL.WS", "NYSE"],
                [1652044, "Alphabet Inc.", "GOOG", "Nasdaq"],
                [1652044, "Alphabet Inc.", "GOOGL", "Nasdaq"],
                [1166691, "COMCAST CORP", "CMCSA", "Nasdaq"],
                [1166691, "COMCAST CORP", "CCZ", "NYSE"],
                [936340, "DTE ENERGY CO", "DTE", "NYSE"],
                [936340, "DTE ENERGY CO", "DTB", "NYSE"],
                [936340, "DTE ENERGY CO", "DTW", "NYSE"],
                [19617, "JPMORGAN CHASE & CO", "JPM", "NYSE"],
                [19617, "JPMORGAN CHASE & CO", "JPM-PC", "NYSE"],
                [19617, "JPMORGAN CHASE & CO", "AMJB", "NYSE"],
                [70858, "BANK OF AMERICA CORP", "BAC-PB", "NYSE"],
                [70858, "BANK OF AMERICA CORP", "BAC", "NYSE"],
                [12345, "ACME SPAC INC", "ACMEW", "Nasdaq"],
                [12345, "ACME SPAC INC", "ACME", "Nasdaq"],
                [555555, "TRITON PREF ONLY", "TRTN-PA", "NYSE"],
                [99999, "OTC Penny Co", "OTCP", "OTC"],
                [88888, "Null Ex Co", "NULX", null]
            ]
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let mut wl_pref = HashMap::new();
        wl_pref.insert("0001652044".to_owned(), "GOOGL".to_owned());

        let listed = parse_canonical_tickers_exchange(&bytes, &wl_pref).unwrap();
        let map: HashMap<String, ListedCompany> =
            listed.into_iter().map(|c| (c.cik.clone(), c)).collect();

        assert_eq!(map.len(), 8);
        assert_eq!(map["0000320193"].ticker, "AAPL");
        assert!(!map["0000320193"].is_non_common_listing);
        assert_eq!(map["0001652044"].ticker, "GOOGL");
        assert_eq!(map["0001166691"].ticker, "CMCSA");
        assert_eq!(map["0000936340"].ticker, "DTE");
        assert_eq!(map["0000019617"].ticker, "JPM");
        assert_eq!(map["0000070858"].ticker, "BAC");
        assert_eq!(map["0000012345"].ticker, "ACME");
        assert_eq!(map["0000555555"].ticker, "TRTN-PA");
        assert!(map["0000555555"].is_non_common_listing);
        assert!(!map.contains_key("0000099999"));
        assert!(!map.contains_key("0000088888"));
    }

    #[test]
    fn financial_sic_tagging_and_submission_sic_parsing_work() {
        assert!(!is_financial_sic(None));
        assert!(!is_financial_sic(Some(5999)));
        assert!(is_financial_sic(Some(6000)));
        assert!(is_financial_sic(Some(6021)));
        assert!(is_financial_sic(Some(6799)));
        assert!(!is_financial_sic(Some(6800)));
        assert!(!is_financial_sic(Some(7372)));

        let sub_str = serde_json::json!({
            "cik": "0000070858",
            "sic": "6021",
            "sicDescription": "National Commercial Banks",
            "filings": { "recent": { "form": ["10-K"] } }
        });
        let info = parse_submission_sic_bytes(&serde_json::to_vec(&sub_str).unwrap()).unwrap();
        assert_eq!(info.sic, Some(6021));
        assert_eq!(
            info.sic_description.as_deref(),
            Some("National Commercial Banks")
        );
        assert!(is_financial_sic(info.sic));

        let sub_empty = serde_json::json!({ "sic": "", "sicDescription": "" });
        assert!(parse_submission_sic_bytes(&serde_json::to_vec(&sub_empty).unwrap()).is_none());
    }

    fn sample_companyfacts_json_years(
        cik: u64,
        rev_base: i64,
        years: usize,
        include_optional_tags: bool,
    ) -> Vec<u8> {
        let all_years = [2020_i64, 2021, 2022, 2023, 2024];
        let selected = &all_years[all_years.len() - years..];

        let dur = |y: i64, val: i64| {
            serde_json::json!({
                "start": format!("{y}-01-01"),
                "end": format!("{y}-12-31"),
                "val": val,
                "form": "10-K",
                "filed": format!("{}-02-10", y + 1),
            })
        };
        let inst = |y: i64, val: i64| {
            serde_json::json!({
                "end": format!("{y}-12-31"),
                "val": val,
                "form": "10-K",
                "filed": format!("{}-02-10", y + 1),
            })
        };

        let mut us_gaap = serde_json::Map::new();
        us_gaap.insert(
            "Revenues".to_owned(),
            serde_json::json!({ "units": { "USD": selected.iter().enumerate().map(|(i, &y)| dur(y, rev_base + (i as i64) * 100)).collect::<Vec<_>>() } }),
        );
        us_gaap.insert(
            "GrossProfit".to_owned(),
            serde_json::json!({ "units": { "USD": selected.iter().enumerate().map(|(i, &y)| dur(y, 630 + (i as i64) * 70)).collect::<Vec<_>>() } }),
        );
        us_gaap.insert(
            "OperatingIncomeLoss".to_owned(),
            serde_json::json!({ "units": { "USD": selected.iter().enumerate().map(|(i, &y)| dur(y, 270 + (i as i64) * 30)).collect::<Vec<_>>() } }),
        );
        us_gaap.insert(
            "NetCashProvidedByUsedInOperatingActivities".to_owned(),
            serde_json::json!({ "units": { "USD": selected.iter().enumerate().map(|(i, &y)| dur(y, 290 + (i as i64) * 30)).collect::<Vec<_>>() } }),
        );
        us_gaap.insert(
            "PaymentsToAcquirePropertyPlantAndEquipment".to_owned(),
            serde_json::json!({ "units": { "USD": selected.iter().map(|&y| dur(y, 40)).collect::<Vec<_>>() } }),
        );
        us_gaap.insert(
            "CashCashEquivalentsAndShortTermInvestments".to_owned(),
            serde_json::json!({ "units": { "USD": selected.iter().enumerate().map(|(i, &y)| inst(y, 150 + (i as i64) * 50)).collect::<Vec<_>>() } }),
        );
        us_gaap.insert(
            "StockholdersEquity".to_owned(),
            serde_json::json!({ "units": { "USD": selected.iter().enumerate().map(|(i, &y)| inst(y, 950 + (i as i64) * 50)).collect::<Vec<_>>() } }),
        );
        us_gaap.insert(
            "WeightedAverageNumberOfDilutedSharesOutstanding".to_owned(),
            serde_json::json!({ "units": { "shares": selected.iter().enumerate().map(|(i, &y)| dur(y, 102 - (i as i64) * 2)).collect::<Vec<_>>() } }),
        );

        if include_optional_tags {
            us_gaap.insert(
                "IncomeTaxExpenseBenefit".to_owned(),
                serde_json::json!({ "units": { "USD": selected.iter().enumerate().map(|(i, &y)| dur(y, 54 + (i as i64) * 6)).collect::<Vec<_>>() } }),
            );
            us_gaap.insert(
                "ShareBasedCompensation".to_owned(),
                serde_json::json!({ "units": { "USD": selected.iter().map(|&y| dur(y, 20)).collect::<Vec<_>>() } }),
            );
            us_gaap.insert(
                "LongTermDebtNoncurrent".to_owned(),
                serde_json::json!({ "units": { "USD": selected.iter().map(|&y| inst(y, 100)).collect::<Vec<_>>() } }),
            );
        }

        let val = serde_json::json!({
            "cik": cik,
            "entityName": "Sample Compounder Inc.",
            "facts": {
                "us-gaap": serde_json::Value::Object(us_gaap),
            }
        });
        serde_json::to_vec(&val).unwrap()
    }

    fn sample_companyfacts_json(cik: u64, rev_base: i64) -> Vec<u8> {
        sample_companyfacts_json_years(cik, rev_base, 5, true)
    }

    fn make_test_zip_bytes(entry_name: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut out);
            let mut zw = ZipWriter::new(cursor);
            zw.start_file(entry_name, SimpleFileOptions::default())
                .unwrap();
            zw.write_all(payload).unwrap();
            zw.finish().unwrap();
        }
        out
    }

    fn minimal_valid_tickers_exchange_bytes(cik: u64, ticker: &str, name: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [[cik, name, ticker, "Nasdaq"]]
        }))
        .unwrap()
    }

    fn seed_valid_sec_gen_dir(gen_dir: &Path, label: &str) {
        fs::create_dir_all(gen_dir).unwrap();
        fs::write(
            gen_dir.join(COMPANYFACTS_ZIP_FILE),
            make_test_zip_bytes("CIK0000320193.json", label.as_bytes()),
        )
        .unwrap();
        fs::write(
            gen_dir.join(TICKERS_EXCHANGE_FILE),
            minimal_valid_tickers_exchange_bytes(320193, "AAPL", "Apple Inc."),
        )
        .unwrap();
    }

    #[test]
    fn streaming_companyfacts_zip_parse_matches_calculator_build_report_exactly() {
        let dir = std::env::temp_dir().join(format!(
            "m3-universe-zip-test-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("companyfacts.zip");

        let aapl_bytes = sample_companyfacts_json(320193, 900);
        let bad_bytes = br#"{"cik": 999999, "facts": {}}"#.to_vec();
        {
            let file = File::create(&zip_path).unwrap();
            let mut zw = ZipWriter::new(file);
            let opts = SimpleFileOptions::default();
            zw.start_file("CIK0000320193.json", opts).unwrap();
            zw.write_all(&aapl_bytes).unwrap();
            zw.start_file("CIK0000999999.json", opts).unwrap();
            zw.write_all(&bad_bytes).unwrap();
            zw.start_file("CIK0000777777.json", opts).unwrap();
            zw.write_all(&sample_companyfacts_json(777777, 500))
                .unwrap();
            zw.finish().unwrap();
        }

        let listed = vec![
            ListedCompany {
                cik: "0000320193".into(),
                ticker: "AAPL".into(),
                name: "Apple Inc.".into(),
                exchange: "Nasdaq".into(),
                is_non_common_listing: false,
            },
            ListedCompany {
                cik: "0000999999".into(),
                ticker: "BAD".into(),
                name: "Bad Co.".into(),
                exchange: "NYSE".into(),
                is_non_common_listing: false,
            },
            ListedCompany {
                cik: "0000888888".into(),
                ticker: "MISS".into(),
                name: "Missing Zip Co.".into(),
                exchange: "CBOE".into(),
                is_non_common_listing: false,
            },
        ];

        let mut sic_map = HashMap::new();
        sic_map.insert(
            "0000320193".to_owned(),
            SicInfo {
                sic: Some(3571),
                sic_description: Some("Electronic Computers".to_owned()),
            },
        );

        let mut cached_prices = HashMap::new();
        let price_snap = PriceSnapshot {
            schema_version: 1,
            symbol: "AAPL".into(),
            price: Decimal::from(50),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(1_735_603_200_000),
            fetched_at_ms: 100,
            source_url: "https://price.test".into(),
        };
        cached_prices.insert("AAPL".to_owned(), price_snap.clone());

        let watchlist = WatchlistLookup::from_opt(None);
        let mut progress_msgs = Vec::new();
        let bundle = parse_companyfacts_zip_streaming(
            &zip_path,
            &listed,
            &sic_map,
            &watchlist,
            &cached_prices,
            1000,
            &mut |msg| progress_msgs.push(msg.to_owned()),
        )
        .unwrap();

        assert_eq!(bundle.matched_entries, 2); // CIK 777777 ignored cleanly!
        assert_eq!(bundle.parsed_companies, 1);
        assert_eq!(bundle.skipped_companies, 2); // BAD + MISS

        // Parity check against direct edgar::parse_companyfacts_bytes + calculator::build_report
        let direct_facts = edgar::parse_companyfacts_bytes(
            &aapl_bytes,
            "AAPL",
            "0000320193",
            "https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json",
            1000,
        )
        .unwrap();
        let direct_entry = default_company_entry(&listed[0]);
        let direct_report =
            calculator::build_report(&direct_entry, Some(&direct_facts), Some(&price_snap), 1000);
        let direct_val = direct_report.fy_valuation.as_ref().unwrap();

        let aapl_row = bundle
            .snapshot
            .iter()
            .find(|r| r.ticker == "AAPL")
            .unwrap()
            .clone();
        assert_eq!(aapl_row.years_of_data, 5);
        assert_eq!(aapl_row.roic_5y_avg_pct, direct_report.roic_5y_avg_pct);
        assert_eq!(
            aapl_row.gross_margin_5y_avg_pct,
            direct_report.gross_margin_5y_avg_pct
        );
        assert_eq!(
            aapl_row.share_change_3y_pct,
            direct_report.share_change_3y_pct
        );
        assert_eq!(aapl_row.strike_price, direct_val.strike_price);
        assert_eq!(
            aapl_row.distance_to_strike_pct,
            direct_val.distance_to_strike_pct
        );
        assert_eq!(
            aapl_row.strict_fcf_yield_pct,
            direct_val.strict_fcf_yield_mcap_pct
        );

        // Verify shared calculator::compute_valuation parity with updated price
        let new_price_snap = PriceSnapshot {
            price: Decimal::from(65),
            ..price_snap
        };
        let direct_reprice = calculator::build_report(
            &direct_entry,
            Some(&direct_facts),
            Some(&new_price_snap),
            1000,
        );
        let direct_reprice_val = direct_reprice.fy_valuation.as_ref().unwrap();
        let latest_metric_row = direct_report.annual_rows.last().unwrap();
        let recomputed_val = calculator::compute_valuation(
            "FY",
            latest_metric_row,
            Some(&new_price_snap),
            &direct_entry.assumptions,
            calculator::ValuationGuards::default(),
        );
        assert_eq!(recomputed_val.strike_price, direct_reprice_val.strike_price);
        assert_eq!(
            recomputed_val.distance_to_strike_pct,
            direct_reprice_val.distance_to_strike_pct
        );
        assert_eq!(
            recomputed_val.strict_fcf_yield_mcap_pct,
            direct_reprice_val.strict_fcf_yield_mcap_pct
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_common_and_brk_b_suppress_per_share_valuation_and_emit_warnings() {
        let facts_bytes = sample_companyfacts_json(1067983, 1000);
        let sic_info = SicInfo {
            sic: Some(7372),
            sic_description: Some("Software".to_owned()),
        };
        let price_snap = PriceSnapshot {
            schema_version: 1,
            symbol: "BRK-B".into(),
            price: Decimal::from(450),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NYQ".into(),
            market_time_ms: Some(1_735_603_200_000),
            fetched_at_ms: 1000,
            source_url: "https://price.test".into(),
        };

        for (ticker, cik, non_common) in [
            ("BRK-B", "0001067983", false),
            ("TRTN-PA", "0000555555", true),
        ] {
            let co = ListedCompany {
                cik: cik.to_owned(),
                ticker: ticker.to_owned(),
                name: ticker.to_owned(),
                exchange: "NYSE".to_owned(),
                is_non_common_listing: non_common,
            };
            let outcome = parse_single_company_bytes(
                &co,
                &facts_bytes,
                Some(&sic_info),
                None,
                Some(&price_snap),
                1000,
            );
            let ChunkParseOutcome::Parsed { mut universe, .. } = outcome else {
                panic!("expected parsed outcome for {ticker}");
            };
            assert_eq!(universe.latest_price, Some(Decimal::from(450)));
            assert!(universe.strike_price.is_none());
            assert!(universe.distance_to_strike_pct.is_none());
            assert!(universe.strict_fcf_yield_pct.is_none());
            assert!(universe.warnings_json.contains("SHARE_BASIS_MISMATCH"));

            let pool_count =
                assign_scheme1_quality_pool(std::slice::from_mut(&mut universe), 10, 1000);
            assert_eq!(pool_count, 0);
            assert!(!universe.in_price_pool);
        }
    }

    #[test]
    fn short_history_under_5y_and_unclassified_sic_excluded_from_5y_metrics_and_scheme1() {
        let four_y_bytes = sample_companyfacts_json_years(111111, 1000, 4, true);
        let five_y_bytes = sample_companyfacts_json_years(222222, 1000, 5, true);
        let classified_sic = SicInfo {
            sic: Some(7372),
            sic_description: Some("Software".to_owned()),
        };

        let short_co = ListedCompany {
            cik: "0000111111".into(),
            ticker: "FOUR".into(),
            name: "Four Year Co".into(),
            exchange: "Nasdaq".into(),
            is_non_common_listing: false,
        };
        let short_outcome = parse_single_company_bytes(
            &short_co,
            &four_y_bytes,
            Some(&classified_sic),
            None,
            None,
            1000,
        );
        let ChunkParseOutcome::Parsed {
            universe: short_row,
            ..
        } = short_outcome
        else {
            panic!("expected parsed 4y row");
        };
        assert_eq!(short_row.years_of_data, 4);
        assert!(short_row.roic_5y_avg_pct.is_none());
        assert!(short_row.gross_margin_5y_avg_pct.is_none());
        assert_eq!(short_row.quality_score, Decimal::ZERO);

        let uncls_co = ListedCompany {
            cik: "0000222222".into(),
            ticker: "UNCLS".into(),
            name: "Unclassified Co".into(),
            exchange: "NYSE".into(),
            is_non_common_listing: false,
        };
        let uncls_outcome =
            parse_single_company_bytes(&uncls_co, &five_y_bytes, None, None, None, 1000);
        let ChunkParseOutcome::Parsed {
            universe: uncls_row,
            ..
        } = uncls_outcome
        else {
            panic!("expected parsed unclassified row");
        };
        assert!(uncls_row.sic.is_none());
        assert!(uncls_row.warnings_json.contains("UNCLASSIFIED_SIC"));

        let valid_co = ListedCompany {
            cik: "0000333333".into(),
            ticker: "GOOD".into(),
            name: "Good 5Y Co".into(),
            exchange: "NYSE".into(),
            is_non_common_listing: false,
        };
        let valid_outcome = parse_single_company_bytes(
            &valid_co,
            &five_y_bytes,
            Some(&classified_sic),
            None,
            None,
            1000,
        );
        let ChunkParseOutcome::Parsed {
            universe: valid_row,
            ..
        } = valid_outcome
        else {
            panic!("expected parsed valid row");
        };

        let mut rows = vec![short_row, uncls_row, valid_row];
        let count = assign_scheme1_quality_pool(&mut rows, 10, 1000);
        assert_eq!(count, 1);
        assert!(!rows[0].in_price_pool);
        assert!(!rows[1].in_price_pool);
        assert!(rows[2].in_price_pool);
    }

    #[test]
    fn missing_sbc_debt_and_tax_do_not_receive_zero_quality_bonuses() {
        let complete_bytes = sample_companyfacts_json_years(444444, 1000, 5, true);
        let absent_bytes = sample_companyfacts_json_years(555555, 1000, 5, false);
        let sic_info = SicInfo {
            sic: Some(7372),
            sic_description: Some("Software".to_owned()),
        };

        let co_ok = ListedCompany {
            cik: "0000444444".into(),
            ticker: "FULL".into(),
            name: "Full Tags Co".into(),
            exchange: "Nasdaq".into(),
            is_non_common_listing: false,
        };
        let co_abs = ListedCompany {
            cik: "0000555555".into(),
            ticker: "ABST".into(),
            name: "Absent Tags Co".into(),
            exchange: "Nasdaq".into(),
            is_non_common_listing: false,
        };

        let ChunkParseOutcome::Parsed {
            universe: ok_row, ..
        } = parse_single_company_bytes(&co_ok, &complete_bytes, Some(&sic_info), None, None, 1000)
        else {
            panic!("expected ok row");
        };
        let ChunkParseOutcome::Parsed {
            universe: abs_row, ..
        } = parse_single_company_bytes(&co_abs, &absent_bytes, Some(&sic_info), None, None, 1000)
        else {
            panic!("expected absent row");
        };

        assert!(abs_row.roic_5y_avg_pct.is_none());
        assert!(abs_row.strict_owner_earnings.is_none());
        assert!(abs_row.strike_price.is_none());
        assert!(abs_row.sbc_to_ocf_pct.is_none());
        assert!(abs_row.net_debt_to_oe.is_none());
        assert!(abs_row.warnings_json.contains("DATA_UNAVAILABLE"));
        // Max without OE (>0), ROIC, SBC, and NetDebt points is 15 (history) + 10 (GM stability) + 10 (shares) = 35.
        assert!(abs_row.quality_score <= Decimal::from(35));
        assert!(abs_row.quality_score < ok_row.quality_score);
    }

    #[test]
    fn bounded_zip_reader_rejects_oversized_entries() {
        let small = vec![b'x'; 32];
        assert!(read_zip_entry_bounded(small.as_slice(), 100, 50, "test").is_err());
        assert!(read_zip_entry_bounded(small.as_slice(), 10, 20, "test").is_err());
        let ok = read_zip_entry_bounded(small.as_slice(), 32, 32, "test").unwrap();
        assert_eq!(ok.len(), 32);
    }

    #[test]
    fn watchlist_non_sec_tickers_are_priced_and_all_failures_skip_watermark() {
        let dir = std::env::temp_dir().join(format!(
            "m3-universe-wl-price-test-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let price_file = paths.price_path("DSY.PA");
        fs::create_dir_all(price_file.parent().unwrap()).unwrap();

        let snap_dsy = PriceSnapshot {
            schema_version: 1,
            symbol: "DSY.PA".into(),
            price: Decimal::new(3540, 2),
            previous_close: None,
            currency: "EUR".into(),
            exchange: "PAR".into(),
            market_time_ms: Some(1_795_100_000_000), // 2026-11-19 Paris
            fetched_at_ms: 1_795_200_000_000,
            source_url: "https://price.test/dsy".into(),
        };
        store::write_json_atomic(&price_file, &snap_dsy).unwrap();

        let snap_gspc = PriceSnapshot {
            schema_version: 1,
            symbol: "^GSPC".into(),
            price: Decimal::new(732_363, 2),
            previous_close: None,
            currency: "USD".into(),
            exchange: "SNP".into(),
            market_time_ms: Some(1_795_122_000_000), // 2026-11-19 ET
            fetched_at_ms: 1_795_200_000_000,
            source_url: "https://price.test/gspc".into(),
        };
        store::write_json_atomic(&paths.price_path("^GSPC"), &snap_gspc).unwrap();

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".to_owned(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "DSY.PA".into(),
                name: "Dassault Systemes".into(),
                bucket: Bucket::Core,
                source: DataSource::Manual,
                cik: None,
                currency: "EUR".into(),
                circle: "Europe Software".into(),
                too_hard_reason: None,
                assumptions: default_assumptions(),
            }],
        };
        let wl_lookup = WatchlistLookup::from_opt(Some(&wl));
        let basis_map = HashMap::new();
        let assump_revs = HashMap::new();
        let mut report = UniverseSyncReport::default();

        let batch = fetch_pool_prices_lock_free(
            &paths,
            &[],
            Some(&wl),
            &wl_lookup,
            &basis_map,
            &assump_revs,
            false,
            1_795_201_000_000,
            &mut report,
        )
        .unwrap();
        let prices = batch.price_rows;
        let updates = batch.updated_snapshot_rows;
        let watermarks = batch.price_watermarks;

        assert_eq!(prices.len(), 2);
        let by_sym: HashMap<&str, &LakePriceDailyRow> =
            prices.iter().map(|p| (p.symbol.as_str(), p)).collect();
        assert_eq!(by_sym["DSY.PA"].trade_date, "2026-11-19");
        assert_eq!(by_sym["^GSPC"].trade_date, "2026-11-19");
        assert!(updates.is_empty());
        // Cached-only runs with zero live fetches do not advance WM_YAHOO_PRICES watermark.
        assert!(watermarks.is_empty());

        // Corrupt watchlist must error rather than silently empty.
        fs::create_dir_all(paths.watchlist_path().parent().unwrap()).unwrap();
        fs::write(paths.watchlist_path(), "{corrupt json").unwrap();
        assert!(load_watchlist_strict(&paths).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scheme1_quality_pool_ranks_non_financials_and_always_includes_watchlist() {
        let mk = |ticker: &str,
                  cik: &str,
                  sic: Option<u32>,
                  is_financial: bool,
                  years: usize,
                  oe: i64,
                  score: i64,
                  in_watchlist: bool| UniverseRow {
            ticker: ticker.to_owned(),
            cik: cik.to_owned(),
            name: ticker.to_owned(),
            exchange: "Nasdaq".to_owned(),
            sic,
            sic_description: None,
            is_financial,
            latest_filing_end: Some("2025-12-31".to_owned()),
            years_of_data: years,
            quality_score: Decimal::from(score),
            in_price_pool: false,
            in_watchlist,
            bucket_opt: in_watchlist.then_some(Bucket::Watch),
            latest_price: None,
            strike_price: None,
            distance_to_strike_pct: None,
            strict_fcf_yield_pct: None,
            roic_5y_avg_pct: (years >= 5).then_some(Decimal::from(20)),
            roic_5y_stddev_pct: None,
            gross_margin_5y_avg_pct: None,
            gross_margin_5y_stddev_pct: None,
            share_change_3y_pct: None,
            sbc_to_ocf_pct: None,
            net_debt_to_oe: None,
            strict_owner_earnings: Some(Decimal::from(oe)),
            warnings_count: 0,
            warnings_json: "[]".to_owned(),
            raw_sha256: Some("abc".to_owned()),
        };

        let nov_2026_ms = 1_795_400_000_000u64;
        let mut stale_row = mk("VNT", "0001786842", Some(3824), false, 5, 500, 91, false);
        stale_row.latest_filing_end = Some("2023-09-29".to_owned());

        let mut gapped_row = mk("GTEC", "0001735041", Some(3560), false, 5, 500, 88, false);
        gapped_row.warnings_json = serde_json::to_string(&vec![crate::model::WarningFlag {
            code: "NON_CONSECUTIVE_5Y".to_owned(),
            severity: WarningSeverity::Info,
            title: "Gap".to_owned(),
            detail: "Gap".to_owned(),
        }])
        .unwrap();

        let mut rows = vec![
            mk("BANK", "0000000001", Some(6021), true, 6, 500, 99, false),
            mk("FOUR", "0000000002", Some(7372), false, 4, 500, 95, false),
            mk("UNCLS", "0000000007", None, false, 6, 500, 96, false),
            stale_row,
            gapped_row,
            mk("QUAL1", "0000000003", Some(7372), false, 6, 500, 90, false),
            mk("QUAL2", "0000000004", Some(7372), false, 5, 300, 80, false),
            mk("QUAL3", "0000000005", Some(7372), false, 5, 200, 70, false),
            mk("WATCH", "0000000006", Some(7372), false, 1, -10, 10, true),
        ];

        let count = assign_scheme1_quality_pool(&mut rows, 2, nov_2026_ms);
        // Top 2 eligible non-financials (>=5 consecutive fresh years, classified SIC, OE > 0) are QUAL1, QUAL2; plus WATCH
        assert_eq!(count, 3);
        let in_pool: HashSet<&str> = rows
            .iter()
            .filter(|r| r.in_price_pool)
            .map(|r| r.ticker.as_str())
            .collect();
        assert!(in_pool.contains("QUAL1"));
        assert!(in_pool.contains("QUAL2"));
        assert!(in_pool.contains("WATCH"));
        assert!(!in_pool.contains("BANK"));
        assert!(!in_pool.contains("FOUR"));
        assert!(!in_pool.contains("UNCLS"));
        assert!(!in_pool.contains("VNT"));
        assert!(!in_pool.contains("GTEC"));
        assert!(!in_pool.contains("QUAL3"));

        // Even with large prices_top, ineligible/stale/gapped rows are never backfilled into the quality pool
        let count_large = assign_scheme1_quality_pool(&mut rows, 100, nov_2026_ms);
        assert_eq!(count_large, 4);
    }

    #[test]
    fn clear_out_of_pool_quote_fields_drops_quotes_only_for_excluded_rows() {
        let mut out_row = UniverseRow {
            ticker: "VNT".to_owned(),
            cik: "0001786842".to_owned(),
            name: "Vontier".to_owned(),
            exchange: "NYSE".to_owned(),
            sic: Some(3824),
            sic_description: None,
            is_financial: false,
            latest_filing_end: Some("2023-09-29".to_owned()),
            years_of_data: 5,
            quality_score: Decimal::from(50),
            in_price_pool: false,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: Some(Decimal::from(35)),
            strike_price: Some(Decimal::from(40)),
            distance_to_strike_pct: Some(Decimal::new(-125, 1)),
            strict_fcf_yield_pct: Some(Decimal::from(6)),
            roic_5y_avg_pct: None,
            roic_5y_stddev_pct: None,
            gross_margin_5y_avg_pct: None,
            gross_margin_5y_stddev_pct: None,
            share_change_3y_pct: None,
            sbc_to_ocf_pct: None,
            net_debt_to_oe: None,
            strict_owner_earnings: Some(Decimal::from(500)),
            warnings_count: 1,
            warnings_json: "[]".to_owned(),
            raw_sha256: Some("abc".to_owned()),
        };
        let mut in_row = out_row.clone();
        in_row.ticker = "QUAL".to_owned();
        in_row.cik = "0000000001".to_owned();
        in_row.in_price_pool = true;

        let mut rows = vec![out_row.clone(), in_row.clone()];
        clear_out_of_pool_quote_fields(&mut rows);

        out_row = rows[0].clone();
        in_row = rows[1].clone();
        assert!(out_row.latest_price.is_none());
        assert!(out_row.distance_to_strike_pct.is_none());
        assert!(out_row.strict_fcf_yield_pct.is_none());
        assert_eq!(out_row.strike_price, Some(Decimal::from(40)));
        assert_eq!(in_row.latest_price, Some(Decimal::from(35)));
        assert_eq!(in_row.distance_to_strike_pct, Some(Decimal::new(-125, 1)));
        assert_eq!(in_row.strict_fcf_yield_pct, Some(Decimal::from(6)));
    }

    #[test]
    fn validate_companyfacts_zip_tmp_rejects_corrupt_member_crc() {
        let dir = std::env::temp_dir().join(format!(
            "m3-zip-corrupt-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("companyfacts.zip");
        let payload = br#"{"entityName":"Apple","facts":{}}"#.repeat(8);
        {
            let f = File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("CIK0000320193.json", opts).unwrap();
            io::Write::write_all(&mut zw, &payload).unwrap();
            zw.finish().unwrap();
        }
        // Flip a byte inside the compressed deflate stream without touching EOCD or CD headers
        // so ZipArchive::new succeeds while entry decompression fails CRC verification.
        let mut raw = fs::read(&zip_path).unwrap();
        let cd_offset = raw
            .windows(4)
            .position(|w| w == b"PK\x01\x02")
            .expect("central directory header present");
        let mid = (48 + cd_offset) / 2;
        assert!(mid > 48 && mid < cd_offset);
        raw[mid] ^= 0xFF;
        fs::write(&zip_path, &raw).unwrap();

        assert!(validate_zip_file(&zip_path).is_ok());
        let mut listed = HashSet::new();
        listed.insert("0000320193".to_owned());
        assert!(validate_companyfacts_zip_tmp(&zip_path, &listed, &HashSet::new()).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_archive_and_bundle_coverage_guards_reject_truncation_and_bad_json() {
        let dir = std::env::temp_dir().join(format!(
            "m3-coverage-guard-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();

        // 1. Valid companyfacts.zip covering only 1 of 2 expected active parsed CIKs (<95%) must fail.
        let facts_zip = dir.join("companyfacts.zip");
        {
            let f = File::create(&facts_zip).unwrap();
            let mut zw = ZipWriter::new(f);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("CIK0000320193.json", opts).unwrap();
            zw.write_all(br#"{"entityName":"Apple","facts":{}}"#)
                .unwrap();
            zw.finish().unwrap();
        }
        let mut listed = HashSet::new();
        listed.insert("0000320193".to_owned());
        listed.insert("0000001001".to_owned());
        assert!(validate_companyfacts_zip_tmp(&facts_zip, &listed, &listed).is_err());
        let mut subset = HashSet::new();
        subset.insert("0000320193".to_owned());
        assert!(validate_companyfacts_zip_tmp(&facts_zip, &listed, &subset).is_ok());

        // 2. Corrupt submission JSON inside submissions.zip must fail validation instead of silently dropping.
        let sub_zip = dir.join("submissions.zip");
        {
            let f = File::create(&sub_zip).unwrap();
            let mut zw = ZipWriter::new(f);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("CIK0000320193.json", opts).unwrap();
            zw.write_all(b"{not valid json").unwrap();
            zw.finish().unwrap();
        }
        assert!(extract_sic_from_submissions_zip(&sub_zip, &subset).is_err());

        // 3. validate_parsed_bundle_coverage rejects <90% retention when >=10 active parsed issuers exist.
        let expected_ten: HashSet<String> = (1..=10).map(|i| format!("{i:010}")).collect();
        let make_row = |cik: String, years: usize| UniverseRow {
            ticker: format!("T{cik}"),
            cik,
            name: "Co".into(),
            exchange: "NYSE".into(),
            sic: Some(3571),
            sic_description: None,
            is_financial: false,
            latest_filing_end: (years > 0).then(|| "2025-12-31".into()),
            years_of_data: years,
            quality_score: Decimal::from(50),
            in_price_pool: false,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: None,
            strike_price: None,
            distance_to_strike_pct: None,
            strict_fcf_yield_pct: None,
            roic_5y_avg_pct: None,
            roic_5y_stddev_pct: None,
            gross_margin_5y_avg_pct: None,
            gross_margin_5y_stddev_pct: None,
            share_change_3y_pct: None,
            sbc_to_ocf_pct: None,
            net_debt_to_oe: None,
            strict_owner_earnings: None,
            warnings_count: 0,
            warnings_json: "[]".into(),
            raw_sha256: (years > 0).then(|| "sha".into()),
        };
        let empty_pre = PreSyncLakeState::default();
        let eight_of_ten: Vec<UniverseRow> = (1..=10)
            .map(|i| make_row(format!("{i:010}"), if i <= 8 { 5 } else { 0 }))
            .collect();
        assert!(
            validate_parsed_bundle_coverage(&eight_of_ten, &[], &[], &empty_pre, &expected_ten)
                .is_err()
        );
        let nine_of_ten: Vec<UniverseRow> = (1..=10)
            .map(|i| make_row(format!("{i:010}"), if i <= 9 { 5 } else { 0 }))
            .collect();
        assert!(
            validate_parsed_bundle_coverage(&nine_of_ten, &[], &[], &empty_pre, &expected_ten)
                .is_ok()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_lake_facts_rows_preserves_ytd_sbc_when_ttm_sbc_and_buybacks_absent() {
        let dur = |start: &str, end: &str, val: i64, fy: i32, fp: &str, form: &str, filed: &str| {
            serde_json::json!({
                "start": start, "end": end, "val": val, "fy": fy, "fp": fp, "form": form, "filed": filed
            })
        };
        let inst = |end: &str, val: i64, fy: i32, fp: &str, form: &str, filed: &str| {
            serde_json::json!({
                "end": end, "val": val, "fy": fy, "fp": fp, "form": form, "filed": filed
            })
        };
        let raw = serde_json::json!({
            "cik": 832428,
            "entityName": "SSP TEST CORP",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 2_000_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2025-01-01", "2025-09-30", 1_450_000_000, 2025, "Q3", "10-Q", "2025-11-05"),
                        dur("2026-01-01", "2026-09-30", 1_550_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "GrossProfit": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 800_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2025-01-01", "2025-09-30", 580_000_000, 2025, "Q3", "10-Q", "2025-11-05"),
                        dur("2026-01-01", "2026-09-30", 620_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "OperatingIncomeLoss": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 300_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2025-01-01", "2025-09-30", 210_000_000, 2025, "Q3", "10-Q", "2025-11-05"),
                        dur("2026-01-01", "2026-09-30", 230_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "IncomeTaxExpenseBenefit": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 60_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2025-01-01", "2025-09-30", 42_000_000, 2025, "Q3", "10-Q", "2025-11-05"),
                        dur("2026-01-01", "2026-09-30", 46_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 280_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2025-01-01", "2025-09-30", 195_000_000, 2025, "Q3", "10-Q", "2025-11-05"),
                        dur("2026-01-01", "2026-09-30", 215_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 260_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2025-01-01", "2025-09-30", 180_000_000, 2025, "Q3", "10-Q", "2025-11-05"),
                        dur("2026-01-01", "2026-09-30", 200_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 50_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2025-01-01", "2025-09-30", 35_000_000, 2025, "Q3", "10-Q", "2025-11-05"),
                        dur("2026-01-01", "2026-09-30", 40_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    // FY25 SBC + Q3 2026 YTD SBC ($15.7M) present, prior Q3 2025 YTD SBC missing!
                    "ShareBasedCompensation": { "units": { "USD": [
                        dur("2025-01-01", "2025-12-31", 20_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2026-01-01", "2026-09-30", 15_700_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                        inst("2025-12-31", 120_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        inst("2026-09-30", 140_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "LongTermDebt": { "units": { "USD": [
                        inst("2025-12-31", 400_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        inst("2026-09-30", 380_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "StockholdersEquity": { "units": { "USD": [
                        inst("2025-12-31", 900_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        inst("2026-09-30", 950_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } },
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        dur("2025-01-01", "2025-12-31", 85_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        dur("2026-01-01", "2026-09-30", 85_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } }
                },
                "dei": {
                    "EntityCommonStockSharesOutstanding": { "units": { "shares": [
                        inst("2025-12-31", 85_000_000, 2025, "FY", "10-K", "2026-02-20"),
                        inst("2026-09-30", 85_000_000, 2026, "Q3", "10-Q", "2026-11-05")
                    ] } }
                }
            }
        });
        let raw_bytes = serde_json::to_vec(&raw).unwrap();
        let facts = edgar::parse_companyfacts_bytes(
            &raw_bytes,
            "SSP",
            "0000832428",
            "https://sec.test",
            1_795_400_000_000,
        )
        .unwrap();
        let co = ListedCompany {
            cik: "0000832428".into(),
            ticker: "SSP".into(),
            name: "SSP TEST CORP".into(),
            exchange: "Nasdaq".into(),
            is_non_common_listing: false,
        };
        let entry = default_company_entry(&co);
        let report = calculator::build_report(&entry, Some(&facts), None, 1_795_400_000_000);
        assert!(
            !report.warnings.iter().any(|w| w.code == "BUYBACKS_HALTED"),
            "absent buybacks must not trigger BUYBACKS_HALTED: {:?}",
            report.warnings
        );

        let (_annuals, ttm_opt) = extract_lake_facts_rows(&co, &facts, &report);
        let ttm = ttm_opt.expect("TTM row generated");
        assert!(ttm.sbc.is_none());
        assert!(ttm.strict_owner_earnings.is_none());
        assert_eq!(ttm.ytd_sbc, Some(Decimal::from(15_700_000i64)));
        assert!(ttm.buybacks.is_none());
        assert!(ttm.ytd_buybacks.is_none());
    }

    #[test]
    fn resolve_price_snapshot_with_cache_marks_cache_hit_non_live() {
        let dir = std::env::temp_dir().join(format!(
            "m3-cache-live-flag-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        let snap = PriceSnapshot {
            schema_version: 1,
            symbol: "DSY.PA".into(),
            price: Decimal::new(3540, 2),
            previous_close: None,
            currency: "EUR".into(),
            exchange: "PAR".into(),
            market_time_ms: Some(1_795_100_000_000),
            fetched_at_ms: 1_795_200_000_000,
            source_url: "https://price.test/dsy".into(),
        };
        let price_file = paths.price_path("DSY.PA");
        fs::create_dir_all(price_file.parent().unwrap()).unwrap();
        store::write_json_atomic(&price_file, &snap).unwrap();

        let client = market::market_http_client().unwrap();
        let mut rate_limits = 0usize;
        let mut report = UniverseSyncReport::default();
        let resolved = resolve_price_snapshot_with_cache(
            &client,
            &paths,
            "DSY.PA",
            false,
            1_795_201_000_000,
            &mut rate_limits,
            &mut report,
        );
        let (resolved_snap, is_live) = resolved.expect("cached quote returned");
        assert!(!is_live);
        assert_eq!(resolved_snap.symbol, "DSY.PA");
        assert_eq!(report.prices_succeeded, 0);
        assert_eq!(report.prices_cached_fallback, 1);

        // Stale >10d market session cache must NOT be treated as a valid cache fallback even when fetched_at_ms is fresh.
        let stale_cached = PriceSnapshot {
            market_time_ms: Some(1_795_201_000_000 - 15 * 86_400_000),
            fetched_at_ms: 1_795_200_500_000,
            ..snap.clone()
        };
        store::write_json_atomic(&price_file, &stale_cached).unwrap();
        // Force short-circuit rate-limit gate so no live Yahoo HTTP request runs in unit test.
        let mut rate_limited = 3usize;
        let mut stale_report = UniverseSyncReport::default();
        let stale_resolved = resolve_price_snapshot_with_cache(
            &client,
            &paths,
            "DSY.PA",
            false,
            1_795_201_000_000,
            &mut rate_limited,
            &mut stale_report,
        );
        assert!(stale_resolved.is_none());
        assert_eq!(stale_report.prices_cached_fallback, 0);
        assert_eq!(stale_report.prices_failed, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_tickers_index_sic_fallback_and_preserved_raw_bytes_invariants_hold() {
        let dir = std::env::temp_dir().join(format!(
            "m3-sync-invariants-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.lake_raw_sec_dir()).unwrap();

        // 1. Staged company_tickers_exchange.json <= 90% of previous active CIKs is rejected.
        let tickers_file = paths
            .lake_raw_sec_dir()
            .join("company_tickers_exchange.json");
        let small_json = serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": (1..=9).map(|i| serde_json::json!([i, format!("Co{i}"), format!("T{i}"), "Nasdaq"])).collect::<Vec<_>>()
        });
        fs::write(&tickers_file, serde_json::to_vec(&small_json).unwrap()).unwrap();
        let prev_ten: HashSet<String> = (1..=10).map(|i| format!("{i:010}")).collect();
        assert!(validate_tickers_exchange_file(&tickers_file, &prev_ten).is_err());
        let ok_json = serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": (1..=10).map(|i| serde_json::json!([i, format!("Co{i}"), format!("T{i}"), "Nasdaq"])).collect::<Vec<_>>()
        });
        fs::write(&tickers_file, serde_json::to_vec(&ok_json).unwrap()).unwrap();
        assert!(validate_tickers_exchange_file(&tickers_file, &prev_ten).is_ok());

        // 2. merge_fallback_sic preserves verified DuckDB SIC classifications when submissions fails or omits entries.
        let mut partial_sic: HashMap<String, SicInfo> = HashMap::new();
        partial_sic.insert(
            "0000000001".to_owned(),
            SicInfo {
                sic: None,
                sic_description: Some("Software".to_owned()),
            },
        );
        let mut prev_sic: HashMap<String, SicInfo> = HashMap::new();
        prev_sic.insert(
            "0000000001".to_owned(),
            SicInfo {
                sic: Some(7372),
                sic_description: Some("Software".to_owned()),
            },
        );
        prev_sic.insert(
            "0000000002".to_owned(),
            SicInfo {
                sic: Some(3571),
                sic_description: Some("Computers".to_owned()),
            },
        );
        let merged_cnt = merge_fallback_sic(&mut partial_sic, &prev_sic);
        assert_eq!(merged_cnt, 2);
        assert_eq!(partial_sic["0000000001"].sic, Some(7372));
        assert_eq!(partial_sic["0000000002"].sic, Some(3571));

        // 3. stage_preserved_raw_issuer_bytes persists old raw bytes by (cik, sha256) before zip
        // replacement and leaves them untouched until prune_preserved_raw_issuer_bytes runs after commit.
        let old_zip = paths.lake_raw_sec_dir().join("companyfacts.zip");
        let keep_bytes = br#"{"cik":111111,"entityName":"Keep Co","facts":{}}"#;
        let keep_sha = edgar::hex_digest(keep_bytes);
        {
            let f = File::create(&old_zip).unwrap();
            let mut zw = ZipWriter::new(f);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("CIK0000111111.json", opts).unwrap();
            zw.write_all(keep_bytes).unwrap();
            zw.finish().unwrap();
        }
        let mut fail_ciks = HashMap::new();
        fail_ciks.insert("0000111111".to_owned(), vec![Some(keep_sha.clone())]);
        stage_preserved_raw_issuer_bytes(&paths, &old_zip, &fail_ciks).unwrap();
        let preserved_file = preserved_raw_issuer_path(&paths, "0000111111", &keep_sha);
        assert_eq!(fs::read(&preserved_file).unwrap(), keep_bytes);

        // Staging wrong SHA fails loudly without swallowing error.
        let mut bad_sha_ciks = HashMap::new();
        bad_sha_ciks.insert("0000111111".to_owned(), vec![Some("deadbeef".to_owned())]);
        assert!(stage_preserved_raw_issuer_bytes(&paths, &old_zip, &bad_sha_ciks).is_err());

        // Still on disk while kept; pruned only after commit when unreferenced.
        prune_preserved_raw_issuer_bytes(&paths, &fail_ciks).unwrap();
        assert!(preserved_file.exists());
        prune_preserved_raw_issuer_bytes(&paths, &HashMap::new()).unwrap();
        assert!(!preserved_file.exists());

        // 4. StagedTempFile cleans up staged temp file automatically on early error/drop before commit().
        let staged_tmp_path = paths.lake_raw_sec_dir().join(".staged.companyfacts.tmp");
        fs::write(&staged_tmp_path, b"partial-zip").unwrap();
        {
            let _staged = StagedSecFetch::Downloaded {
                tmp_file: StagedTempFile::new(staged_tmp_path.clone()),
                watermark: SyncWatermark {
                    source_key: WM_COMPANYFACTS_ZIP.into(),
                    cursor: None,
                    etag: None,
                    last_modified: None,
                    synced_at_ms: 1,
                },
            };
            assert!(staged_tmp_path.exists());
        }
        assert!(!staged_tmp_path.exists());

        // 5. read_pre_sync_state propagates DuckDB errors instead of wiping baseline state.
        let bad_dir = dir.join("corrupt-lake");
        let bad_paths = WorkspacePaths::from_root(bad_dir.clone());
        fs::create_dir_all(bad_paths.lake_dir()).unwrap();
        fs::write(bad_paths.lake_db_path(), b"not a duckdb database").unwrap();
        assert!(read_pre_sync_state(&bad_paths).is_err());

        // 6. merge_and_persist_sic_map propagates SIC cache write errors instead of ignoring them.
        let sic_err_dir = dir.join("sic-write-err");
        let sic_err_paths = WorkspacePaths::from_root(sic_err_dir.clone());
        let bad_cache_dir = sic_err_paths.lake_raw_sec_dir().join("sic_map.json");
        fs::create_dir_all(&bad_cache_dir).unwrap();
        let mut fresh_sic = HashMap::new();
        fresh_sic.insert(
            "0000320193".to_owned(),
            SicInfo {
                sic: Some(3571),
                sic_description: Some("Electronic Computers".to_owned()),
            },
        );
        assert!(
            merge_and_persist_sic_map(&bad_cache_dir, fresh_sic, &HashMap::new(), true).is_err()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn staged_zip_rollback_per_issuer_regression_and_live_watchlist_assumptions_work() {
        let dir = std::env::temp_dir().join(format!("m3-uni-guards-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.lake_raw_sec_dir()).unwrap();

        // 1. StagedSecGenerationGuard preserves active generation on rollback before finalize(),
        //    and recovers a committed generation via DuckDB WM_SEC_INPUT_GEN pointer after a crash.
        let gen1_dir = paths.lake_raw_sec_generations_dir().join("gen-001");
        fs::create_dir_all(&gen1_dir).unwrap();
        let active_zip = gen1_dir.join(COMPANYFACTS_ZIP_FILE);
        let active_tickers = gen1_dir.join(TICKERS_EXCHANGE_FILE);
        let g1_zip_bytes = make_test_zip_bytes("CIK0000320193.json", b"old-active-generation");
        let g1_tickers_bytes = minimal_valid_tickers_exchange_bytes(320193, "AAPL", "Apple Inc.");
        fs::write(&active_zip, &g1_zip_bytes).unwrap();
        fs::write(&active_tickers, &g1_tickers_bytes).unwrap();
        publish_sec_input_manifest_and_links(&paths, "gen-001", &gen1_dir, 1).unwrap();
        let resolved_g1 = ResolvedSecInputPaths {
            generation_id: Some("gen-001".into()),
            tickers_path: active_tickers.clone(),
            submissions_zip_path: gen1_dir.join(SUBMISSIONS_ZIP_FILE),
            sic_map_path: gen1_dir.join(SIC_MAP_FILE),
            facts_zip_path: active_zip.clone(),
        };
        let staged_zip = paths.lake_raw_sec_dir().join(".staged.new.zip");
        let staged_tickers = paths.lake_raw_sec_dir().join(".staged.tickers.json");
        fs::write(
            &staged_zip,
            make_test_zip_bytes("CIK0000320193.json", b"new-staged-generation"),
        )
        .unwrap();
        fs::write(
            &staged_tickers,
            minimal_valid_tickers_exchange_bytes(320193, "AAPL", "Apple Inc."),
        )
        .unwrap();
        {
            let mut guard = StagedSecGenerationGuard::new(
                &paths,
                "gen-002".into(),
                StagedSecFetch::Downloaded {
                    tmp_file: StagedTempFile::new(staged_tickers),
                    watermark: SyncWatermark {
                        source_key: WM_TICKERS_EXCHANGE.into(),
                        cursor: None,
                        etag: None,
                        last_modified: None,
                        synced_at_ms: 2,
                    },
                },
                StagedSecFetch::NotModified,
                StagedSecFetch::Downloaded {
                    tmp_file: StagedTempFile::new(staged_zip),
                    watermark: SyncWatermark {
                        source_key: WM_COMPANYFACTS_ZIP.into(),
                        cursor: None,
                        etag: None,
                        last_modified: None,
                        synced_at_ms: 2,
                    },
                },
                HashMap::new(),
                resolved_g1.clone(),
            );
            guard.promote_before_commit().unwrap();
            // Active files remain untouched on disk until DuckDB commits & finalize() swaps pointer!
            assert_eq!(
                fs::read(paths.resolve_active_sec_raw_file(COMPANYFACTS_ZIP_FILE)).unwrap(),
                g1_zip_bytes
            );
            assert_eq!(
                fs::read(paths.resolve_active_sec_raw_file(TICKERS_EXCHANGE_FILE)).unwrap(),
                g1_tickers_bytes
            );
            // Drop without finalize() simulates DB tx.commit() failure -> cleans gen-002 & keeps gen-001 intact!
        }
        assert_eq!(
            fs::read(paths.resolve_active_sec_raw_file(COMPANYFACTS_ZIP_FILE)).unwrap(),
            g1_zip_bytes
        );
        assert!(
            !paths
                .lake_raw_sec_generations_dir()
                .join("gen-002")
                .exists()
        );

        // Simulate crash AFTER tx.commit() (DB watermark points to gen-003) BEFORE finalize():
        // reconcile_sec_input_generation_on_disk repairs CURRENT.json and top-level hardlinks to gen-003.
        let staged_zip_3 = paths.lake_raw_sec_dir().join(".staged.3.zip");
        let g3_zip_bytes = make_test_zip_bytes("CIK0000320193.json", b"committed-gen-003");
        fs::write(&staged_zip_3, &g3_zip_bytes).unwrap();
        let mut guard3 = StagedSecGenerationGuard::new(
            &paths,
            "gen-003".into(),
            StagedSecFetch::NotModified,
            StagedSecFetch::NotModified,
            StagedSecFetch::Downloaded {
                tmp_file: StagedTempFile::new(staged_zip_3),
                watermark: SyncWatermark {
                    source_key: WM_COMPANYFACTS_ZIP.into(),
                    cursor: None,
                    etag: None,
                    last_modified: None,
                    synced_at_ms: 3,
                },
            },
            HashMap::new(),
            resolved_g1,
        );
        guard3.promote_before_commit().unwrap();
        // Mark finalized without running publish/cleanup to simulate SIGKILL right after tx.commit()!
        guard3.finalized = true;
        drop(guard3);
        let recovered = reconcile_sec_input_generation_on_disk(&paths, Some("gen-003"), 3).unwrap();
        assert_eq!(recovered.generation_id.as_deref(), Some("gen-003"));
        assert_eq!(
            fs::read(paths.resolve_active_sec_raw_file(COMPANYFACTS_ZIP_FILE)).unwrap(),
            g3_zip_bytes
        );
        assert_eq!(
            fs::read(paths.lake_raw_sec_dir().join(COMPANYFACTS_ZIP_FILE)).unwrap(),
            g3_zip_bytes
        );
        assert!(!gen1_dir.exists());

        // 2. validate_parsed_bundle_coverage detects per-issuer coverage regression (5y -> 2y).
        let expected_ten: HashSet<String> = (1..=10).map(|i| format!("{i:010}")).collect();
        let make_row = |cik: String, years: usize| UniverseRow {
            ticker: format!("T{cik}"),
            cik,
            name: "Co".into(),
            exchange: "NYSE".into(),
            sic: Some(3571),
            sic_description: None,
            is_financial: false,
            latest_filing_end: Some("2025-12-31".into()),
            years_of_data: years,
            quality_score: Decimal::from(50),
            in_price_pool: false,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: None,
            strike_price: None,
            distance_to_strike_pct: None,
            strict_fcf_yield_pct: None,
            roic_5y_avg_pct: None,
            roic_5y_stddev_pct: None,
            gross_margin_5y_avg_pct: None,
            gross_margin_5y_stddev_pct: None,
            share_change_3y_pct: None,
            sbc_to_ocf_pct: None,
            net_debt_to_oe: None,
            strict_owner_earnings: None,
            warnings_count: 0,
            warnings_json: "[]".into(),
            raw_sha256: Some("sha".into()),
        };
        let mut pre_state = PreSyncLakeState::default();
        for cik in &expected_ten {
            pre_state
                .prev_parsed_rows
                .insert(cik.clone(), make_row(cik.clone(), 5));
        }
        let eight_retained_two_regressed: Vec<UniverseRow> = (1..=10)
            .map(|i| make_row(format!("{i:010}"), if i <= 8 { 5 } else { 2 }))
            .collect();
        assert!(
            validate_parsed_bundle_coverage(
                &eight_retained_two_regressed,
                &[],
                &[],
                &pre_state,
                &expected_ten
            )
            .is_err()
        );

        // 3. fetch_pool_prices_lock_free reloads live watchlist.json assumptions & suppresses warrant strike.
        let now_ms = 1_795_420_000_000u64;
        let snap = PriceSnapshot {
            schema_version: 1,
            symbol: "LIVE".into(),
            price: Decimal::from(100),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(now_ms - 3_600_000),
            fetched_at_ms: now_ms,
            source_url: "https://price.test".into(),
        };
        store::write_json_atomic(&paths.price_path("LIVE"), &snap).unwrap();
        let warrant_snap = PriceSnapshot {
            symbol: "WARR-WT".into(),
            ..snap.clone()
        };
        store::write_json_atomic(&paths.price_path("WARR-WT"), &warrant_snap).unwrap();
        let neww_snap = PriceSnapshot {
            symbol: "NEWW".into(),
            price: Decimal::from(80),
            ..snap
        };
        store::write_json_atomic(&paths.price_path("NEWW"), &neww_snap).unwrap();

        let initial_wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "LIVE".into(),
                name: "Live Co".into(),
                bucket: Bucket::Core,
                source: DataSource::SecEdgar,
                cik: Some("0000123123".into()),
                currency: "USD".into(),
                circle: "Tech".into(),
                too_hard_reason: None,
                assumptions: ValuationAssumptions {
                    multiple_low: Decimal::from(10),
                    multiple_high: Decimal::from(12),
                    target_return_pct: Decimal::from(10),
                    expected_growth_pct: Decimal::from(8),
                    strike_price_override: None,
                },
            }],
        };
        let mut live_wl = initial_wl.clone();
        live_wl.companies[0].assumptions.multiple_low = Decimal::from(25);
        live_wl.companies.push(CompanyEntry {
            ticker: "NEWW".into(),
            name: "Newly Admitted Co".into(),
            bucket: Bucket::Watch,
            source: DataSource::SecEdgar,
            cik: Some("0000789789".into()),
            currency: "USD".into(),
            circle: "Tech".into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(15),
                multiple_high: Decimal::from(18),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        });
        store::save_watchlist(&paths, &live_wl).unwrap();

        let mut live_row = make_row("0000123123".into(), 5);
        live_row.ticker = "LIVE".into();
        live_row.in_price_pool = true;
        let mut warr_row = make_row("0000456456".into(), 5);
        warr_row.ticker = "WARR-WT".into();
        warr_row.in_price_pool = true;
        let mut neww_row = make_row("0000789789".into(), 5);
        neww_row.ticker = "NEWW".into();
        neww_row.in_price_pool = false;

        let metric = crate::model::AnnualMetricRow {
            label: "TTM".into(),
            period_end: "2026-09-30".into(),
            revenue: Decimal::from(1000),
            gross_profit: Decimal::from(500),
            gross_margin_pct: Some(Decimal::from(50)),
            ebit: Decimal::from(200),
            nopat: Some(Decimal::from(160)),
            invested_capital: Some(Decimal::from(800)),
            roic_pct: Some(Decimal::from(20)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(20)),
            ocf: Decimal::from(150),
            capex: Decimal::from(20),
            fcf: Decimal::from(130),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(120)),
            sbc_to_ocf_pct: Some(Decimal::new(66, 1)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(50)),
            total_debt: Some(Decimal::ZERO),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(800),
            diluted_shares: Decimal::from(10),
        };
        let mut basis_map = HashMap::new();
        basis_map.insert(
            "0000123123".into(),
            (
                "TTM",
                metric.clone(),
                calculator::ValuationGuards::default(),
            ),
        );
        basis_map.insert(
            "0000456456".into(),
            (
                "TTM",
                metric.clone(),
                calculator::ValuationGuards::default(),
            ),
        );
        basis_map.insert(
            "0000789789".into(),
            ("TTM", metric, calculator::ValuationGuards::default()),
        );
        let init_lookup = WatchlistLookup::from_opt(Some(&initial_wl));
        let assump_revs = HashMap::new();
        let mut rep = UniverseSyncReport::default();
        let batch = fetch_pool_prices_lock_free(
            &paths,
            &[live_row, warr_row, neww_row],
            Some(&initial_wl),
            &init_lookup,
            &basis_map,
            &assump_revs,
            false,
            now_ms,
            &mut rep,
        )
        .unwrap();

        let by_cik: HashMap<&str, &QuoteSnapshotUpdate> = batch
            .updated_snapshot_rows
            .iter()
            .map(|u| (u.cik.as_str(), u))
            .collect();
        // 12 strict OE/sh * 25 (updated live watchlist) = 300, NOT 12 * 10 = 120.
        assert_eq!(by_cik["0000123123"].strike_price, Some(Decimal::from(300)));
        assert_eq!(
            batch.expected_assumptions_digests.get("0000123123"),
            Some(&valuation_assumptions_digest(
                &live_wl.companies[0].assumptions
            ))
        );
        // Newly admitted watchlist SEC row (NEWW) added during quote fetch also gets observation + strike (12 * 15 = 180).
        assert_eq!(by_cik["0000789789"].latest_price, Some(Decimal::from(80)));
        assert_eq!(by_cik["0000789789"].strike_price, Some(Decimal::from(180)));
        assert!(by_cik["0000456456"].strike_price.is_none());

        // 4. Corrupt live watchlist.json during quote sync must propagate error, never silently fall back.
        fs::write(paths.watchlist_path(), b"{broken-watchlist").unwrap();
        let empty_rows: [UniverseRow; 0] = [];
        let empty_basis = HashMap::new();
        assert!(
            fetch_pool_prices_lock_free(
                &paths,
                &empty_rows,
                Some(&initial_wl),
                &init_lookup,
                &empty_basis,
                &assump_revs,
                false,
                now_ms,
                &mut rep,
            )
            .is_err()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_char_warrants_sbcww_pgyww_and_missing_preserved_source_fail_loudly() {
        // 1. Canonical picker prefers SBC/PGY over SBCWW/PGYWW; watchlist selection of warrants emits SHARE_BASIS_MISMATCH.
        let te_bytes = serde_json::to_vec(&serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [
                [1930313, "SBC Medical Group Holdings Inc", "SBCWW", "Nasdaq"],
                [1930313, "SBC Medical Group Holdings Inc", "SBC", "Nasdaq"],
                [1883085, "Pagaya Technologies Ltd.", "PGY", "Nasdaq"],
                [1883085, "Pagaya Technologies Ltd.", "PGYWW", "Nasdaq"],
                [9991111, "Grainger Inc", "GWW", "NYSE"]
            ]
        }))
        .unwrap();
        let canonical = parse_canonical_tickers_exchange(&te_bytes, &HashMap::new()).unwrap();
        let tickers: HashMap<&str, &ListedCompany> =
            canonical.iter().map(|c| (c.ticker.as_str(), c)).collect();
        assert!(tickers.contains_key("SBC"));
        assert!(!tickers.contains_key("SBCWW"));
        assert!(tickers.contains_key("PGY"));
        assert!(!tickers.contains_key("PGYWW"));
        assert!(tickers.contains_key("GWW"));
        assert!(!tickers["GWW"].is_non_common_listing);

        let wl_pref = HashMap::from([
            ("0001930313".to_owned(), "SBCWW".to_owned()),
            ("0001883085".to_owned(), "PGYWW".to_owned()),
        ]);
        let raw_warr_bytes = serde_json::to_vec(&serde_json::json!({
            "entityName": "SBC Medical Group Holdings Inc",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":60000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":20000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":25000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":5000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                        {"end":"2025-12-31","val":15000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":80000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-25"}
                    ]}}
                }
            }
        }))
        .unwrap();
        let forced = parse_canonical_tickers_exchange(&te_bytes, &wl_pref).unwrap();
        for c in &forced {
            if c.ticker == "SBCWW" || c.ticker == "PGYWW" {
                assert!(c.is_non_common_listing);
                assert!(is_share_basis_mismatched(&c.ticker, false));
                let ChunkParseOutcome::Parsed { universe: urow, .. } = parse_single_company_bytes(
                    c,
                    &raw_warr_bytes,
                    None,
                    None,
                    None,
                    1_795_400_000_000,
                ) else {
                    panic!("expected parsed warrant company row for {}", c.ticker);
                };
                assert!(urow.warnings_json.contains("\"SHARE_BASIS_MISMATCH\""));
                assert!(urow.strike_price.is_none());
            }
        }
        assert!(is_share_basis_mismatched("XYZWS", false));
        assert!(is_share_basis_mismatched("HQWWW", false));
        assert!(!is_share_basis_mismatched("GWW", false));
        assert!(!is_share_basis_mismatched("FLWS", false));

        // 2. stage_preserved_raw_issuer_bytes must fail loudly if a required (CIK, SHA-256) lacks a readable source.
        let dir = std::env::temp_dir().join(format!("m3-u-preserve-req-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("data/munger/lake/raw/sec")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        let missing_zip = paths.lake_raw_sec_dir().join("companyfacts.zip");
        let req = HashMap::from([("0000123456".to_owned(), vec![Some("deadbeef".to_owned())])]);
        assert!(stage_preserved_raw_issuer_bytes(&paths, &missing_zip, &req).is_err());

        // 3. StagedSecGenerationGuard without pre-existing zip cleans up staged generation if dropped before finalize().
        let staged_tmp = paths.lake_raw_sec_dir().join("companyfacts.zip.tmp");
        let staged_tickers_tmp = paths.lake_raw_sec_dir().join("tickers.json.tmp");
        fs::write(
            &staged_tmp,
            make_test_zip_bytes("CIK0000320193.json", b"staged-zip"),
        )
        .unwrap();
        fs::write(
            &staged_tickers_tmp,
            minimal_valid_tickers_exchange_bytes(320193, "AAPL", "Apple Inc."),
        )
        .unwrap();
        let staged = StagedSecFetch::Downloaded {
            tmp_file: StagedTempFile::new(staged_tmp),
            watermark: SyncWatermark {
                source_key: WM_COMPANYFACTS_ZIP.into(),
                cursor: None,
                etag: None,
                last_modified: None,
                synced_at_ms: 1,
            },
        };
        let staged_tickers = StagedSecFetch::Downloaded {
            tmp_file: StagedTempFile::new(staged_tickers_tmp),
            watermark: SyncWatermark {
                source_key: WM_TICKERS_EXCHANGE.into(),
                cursor: None,
                etag: None,
                last_modified: None,
                synced_at_ms: 1,
            },
        };
        let empty_resolved = ResolvedSecInputPaths {
            generation_id: None,
            tickers_path: paths.lake_raw_sec_dir().join(TICKERS_EXCHANGE_FILE),
            submissions_zip_path: paths.lake_raw_sec_dir().join(SUBMISSIONS_ZIP_FILE),
            sic_map_path: paths.lake_raw_sec_dir().join(SIC_MAP_FILE),
            facts_zip_path: missing_zip.clone(),
        };
        {
            let mut guard = StagedSecGenerationGuard::new(
                &paths,
                "gen-empty".into(),
                staged_tickers,
                StagedSecFetch::NotModified,
                staged,
                HashMap::new(),
                empty_resolved,
            );
            guard.promote_before_commit().unwrap();
            assert!(
                paths
                    .lake_raw_sec_generations_dir()
                    .join("gen-empty")
                    .exists()
            );
            assert!(!missing_zip.exists());
        }
        assert!(
            !paths
                .lake_raw_sec_generations_dir()
                .join("gen-empty")
                .exists()
        );
        assert!(!missing_zip.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round12_preferred_quote_isolation_reader_pin_and_ticker_304_guards_work() {
        let dir = std::env::temp_dir().join(format!("m3-uni-r12-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.lake_raw_sec_dir()).unwrap();
        let now_ms = 1_795_440_000_000u64;

        // 1. Preferred ticker sharing a CIK (ALB-PA -> ALB's CIK 0000915913) must write ALB-PA
        //    to prices_daily while NOT attaching ALB-PA's quote or assumptions onto common ALB!
        let mut alb_row = UniverseRow {
            ticker: "ALB".into(),
            cik: "0000915913".into(),
            name: "Albemarle Corp".into(),
            exchange: "NYSE".into(),
            sic: Some(2800),
            sic_description: Some("Chemicals".into()),
            is_financial: false,
            latest_filing_end: Some("2025-12-31".into()),
            years_of_data: 5,
            quality_score: Decimal::from(60),
            in_price_pool: true,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: None,
            strike_price: None,
            distance_to_strike_pct: None,
            strict_fcf_yield_pct: None,
            roic_5y_avg_pct: Some(Decimal::from(15)),
            roic_5y_stddev_pct: Some(Decimal::from(2)),
            gross_margin_5y_avg_pct: Some(Decimal::from(35)),
            gross_margin_5y_stddev_pct: Some(Decimal::from(2)),
            share_change_3y_pct: Some(Decimal::ZERO),
            sbc_to_ocf_pct: Some(Decimal::from(3)),
            net_debt_to_oe: Some(Decimal::from(1)),
            strict_owner_earnings: Some(Decimal::from(200)),
            warnings_count: 0,
            warnings_json: "[]".into(),
            raw_sha256: Some("sha-alb".into()),
        };
        alb_row.in_watchlist = false;
        let alb_snap = PriceSnapshot {
            schema_version: 1,
            symbol: "ALB".into(),
            price: Decimal::from(90),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NYSE".into(),
            market_time_ms: Some(now_ms - 3_600_000),
            fetched_at_ms: now_ms,
            source_url: "https://price.test/ALB".into(),
        };
        let alb_pa_snap = PriceSnapshot {
            symbol: "ALB-PA".into(),
            price: Decimal::from(48),
            ..alb_snap.clone()
        };
        store::write_json_atomic(&paths.price_path("ALB"), &alb_snap).unwrap();
        store::write_json_atomic(&paths.price_path("ALB-PA"), &alb_pa_snap).unwrap();

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "ALB-PA".into(),
                name: "Albemarle Pref".into(),
                bucket: Bucket::TooHard,
                source: DataSource::SecEdgar,
                cik: Some("0000915913".into()),
                currency: "USD".into(),
                circle: "Chem".into(),
                too_hard_reason: None,
                assumptions: ValuationAssumptions {
                    multiple_low: Decimal::from(12),
                    multiple_high: Decimal::from(15),
                    target_return_pct: Decimal::from(10),
                    expected_growth_pct: Decimal::from(8),
                    strike_price_override: None,
                },
            }],
        };
        store::save_watchlist(&paths, &wl).unwrap();
        let wl_lookup = WatchlistLookup::from_opt(Some(&wl));
        assert!(wl_lookup.find("ALB", "0000915913").is_none());

        let metric = crate::model::AnnualMetricRow {
            label: "FY25".into(),
            period_end: "2025-12-31".into(),
            revenue: Decimal::from(1000),
            gross_profit: Decimal::from(350),
            gross_margin_pct: Some(Decimal::from(35)),
            ebit: Decimal::from(250),
            nopat: Some(Decimal::from(200)),
            invested_capital: Some(Decimal::from(1200)),
            roic_pct: Some(Decimal::new(1667, 2)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::new(1667, 2)),
            ocf: Decimal::from(240),
            capex: Decimal::from(30),
            fcf: Decimal::from(210),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(200)),
            sbc_to_ocf_pct: Some(Decimal::from(4)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(200)),
            net_debt: Some(Decimal::from(100)),
            equity: Decimal::from(1000),
            diluted_shares: Decimal::from(10),
        };
        let basis = HashMap::from([(
            "0000915913".into(),
            ("FY25", metric, calculator::ValuationGuards::default()),
        )]);
        let mut rep = UniverseSyncReport::default();
        // Force ALB-PA supplemental quote ONLY (mark ALB not in pool so only extra_symbols runs for ALB-PA).
        alb_row.in_price_pool = false;
        let batch = fetch_pool_prices_lock_free(
            &paths,
            &[alb_row],
            Some(&wl),
            &wl_lookup,
            &basis,
            &HashMap::new(),
            false,
            now_ms,
            &mut rep,
        )
        .unwrap();
        assert!(batch.price_rows.iter().any(|p| p.symbol == "ALB-PA"));
        assert!(
            batch.updated_snapshot_rows.is_empty(),
            "ALB-PA preferred quote must not attach to ALB common stock"
        );

        // 2. Active reader lock pin retains older SEC generation & missing DB gen fails cleanly without deleting CURRENT.json generation.
        let g1 = paths.lake_raw_sec_generations_dir().join("gen-001");
        let g2 = paths.lake_raw_sec_generations_dir().join("gen-002");
        seed_valid_sec_gen_dir(&g1, "g1");
        seed_valid_sec_gen_dir(&g2, "g2");
        publish_sec_input_manifest_and_links(&paths, "gen-001", &g1, 1).unwrap();
        let reader_pin = paths
            .acquire_sec_generation_read_pin(&g1.join(COMPANYFACTS_ZIP_FILE))
            .expect("reader pin acquired");
        let recovered_g2 =
            reconcile_sec_input_generation_on_disk(&paths, Some("gen-002"), 2).unwrap();
        assert_eq!(recovered_g2.generation_id.as_deref(), Some("gen-002"));
        assert!(
            g1.exists(),
            "pinned generation must not be deleted while active reader holds shared lock"
        );
        drop(reader_pin);
        assert!(reconcile_sec_input_generation_on_disk(&paths, Some("gen-missing"), 3).is_err());
        assert!(
            g2.exists(),
            "committed generation must survive failed recovery"
        );
        let fallback_manifest = reconcile_sec_input_generation_on_disk(&paths, None, 4).unwrap();
        assert_eq!(fallback_manifest.generation_id.as_deref(), Some("gen-002"));
        assert!(g2.exists());

        // 3. Cached ticker index truncation fails integrity cursor / coverage verification before HTTP 304,
        //    and validate_parsed_bundle_coverage blocks universe shrinkage bypass even below 10 active rows.
        let te_full = serde_json::to_vec(&serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": (1..=12).map(|i| serde_json::json!([i, format!("Co {i}"), format!("T{i}"), "NYSE"])).collect::<Vec<_>>()
        }))
        .unwrap();
        let te_path = paths.lake_raw_sec_dir().join("test_tickers.json");
        fs::write(&te_path, &te_full).unwrap();
        let cursor = format_file_integrity_cursor(&te_path).unwrap();
        let all_ciks: HashSet<String> = (1..=12).map(|i| format!("{i:010}")).collect();
        assert!(is_valid_tickers_exchange_file(
            &te_path,
            &all_ciks,
            Some(&cursor)
        ));
        let te_trunc = serde_json::to_vec(&serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [[1, "Co 1", "T1", "NYSE"]]
        }))
        .unwrap();
        fs::write(&te_path, &te_trunc).unwrap();
        assert!(!verify_file_integrity_cursor(&te_path, &cursor));
        assert!(!is_valid_tickers_exchange_file(
            &te_path,
            &all_ciks,
            Some(&cursor)
        ));

        let pre_state = PreSyncLakeState {
            parsed_ciks: (1..=12).map(|i| format!("{i:010}")).collect(),
            ..Default::default()
        };
        let active_two = HashSet::from(["0000000001".to_owned(), "0000000002".to_owned()]);
        assert!(validate_parsed_bundle_coverage(&[], &[], &[], &pre_state, &active_two).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round13_warrant_watchlist_order_gc_lock_and_zip_cursor_304_work() {
        use std::io::{BufRead, Write};
        use std::net::TcpListener;

        let dir = std::env::temp_dir().join(format!("m3-uni-r13-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.lake_raw_sec_dir()).unwrap();
        let now_ms = 1_795_440_000_000u64;

        // 1. PGY + PGYWW in BOTH watchlist orders (and PGYWW-only) must never displace PGY
        //    as canonical ticker or attach PGYWW watchlist assumptions onto PGY in pool selection.
        let pgy_sec = serde_json::to_vec(&serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [
                [1883085, "Pagaya Technologies Ltd.", "PGYWW", "Nasdaq"],
                [1883085, "Pagaya Technologies Ltd.", "PGY", "Nasdaq"]
            ]
        }))
        .unwrap();
        let make_co = |ticker: &str, mult: i64| CompanyEntry {
            ticker: ticker.into(),
            name: format!("{ticker} Co"),
            bucket: Bucket::Watch,
            source: DataSource::SecEdgar,
            cik: Some("0001883085".into()),
            currency: "USD".into(),
            circle: "Test".into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(mult),
                multiple_high: Decimal::from(mult + 5),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        };
        for companies in [
            vec![make_co("PGYWW", 5), make_co("PGY", 20)],
            vec![make_co("PGY", 20), make_co("PGYWW", 5)],
        ] {
            let wl = WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies,
            };
            let lookup = WatchlistLookup::from_opt(Some(&wl));
            assert_eq!(
                lookup.cik_to_ticker.get("0001883085").map(String::as_str),
                Some("PGY")
            );
            let listed = parse_canonical_tickers_exchange(&pgy_sec, &lookup.cik_to_ticker).unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].ticker, "PGY");
            assert!(!listed[0].is_non_common_listing);
            assert_eq!(
                lookup.find("PGY", "0001883085").map(|c| c.ticker.as_str()),
                Some("PGY")
            );
            assert_eq!(
                lookup
                    .find("PGYWW", "0001883085")
                    .map(|c| c.ticker.as_str()),
                Some("PGYWW")
            );

            let mut rows = vec![UniverseRow {
                ticker: listed[0].ticker.clone(),
                cik: listed[0].cik.clone(),
                name: listed[0].name.clone(),
                exchange: listed[0].exchange.clone(),
                sic: Some(7372),
                sic_description: Some("Software".into()),
                is_financial: false,
                latest_filing_end: Some("2025-12-31".into()),
                years_of_data: 5,
                quality_score: Decimal::from(40),
                in_price_pool: false,
                in_watchlist: lookup.find(&listed[0].ticker, &listed[0].cik).is_some(),
                bucket_opt: lookup
                    .find(&listed[0].ticker, &listed[0].cik)
                    .map(|c| c.bucket),
                latest_price: None,
                strike_price: None,
                distance_to_strike_pct: None,
                strict_fcf_yield_pct: None,
                roic_5y_avg_pct: Some(Decimal::from(15)),
                roic_5y_stddev_pct: Some(Decimal::from(2)),
                gross_margin_5y_avg_pct: Some(Decimal::from(50)),
                gross_margin_5y_stddev_pct: Some(Decimal::from(2)),
                share_change_3y_pct: Some(Decimal::ZERO),
                sbc_to_ocf_pct: Some(Decimal::from(5)),
                net_debt_to_oe: Some(Decimal::ZERO),
                strict_owner_earnings: Some(Decimal::from(100)),
                warnings_count: 0,
                warnings_json: "[]".into(),
                raw_sha256: Some("sha-pgy".into()),
            }];
            let pool_count = assign_scheme1_quality_pool(&mut rows, 1, now_ms);
            assert_eq!(pool_count, 1);
            assert!(rows[0].in_price_pool);
            assert!(rows[0].in_watchlist);
            assert_eq!(rows[0].ticker, "PGY");
        }
        // Even if caller passes PGYWW directly in cik_to_ticker override, PGY wins over warrant sibling.
        let forced = parse_canonical_tickers_exchange(
            &pgy_sec,
            &HashMap::from([("0001883085".to_owned(), "PGYWW".to_owned())]),
        )
        .unwrap();
        assert_eq!(forced[0].ticker, "PGY");
        assert!(!forced[0].is_non_common_listing);

        // 2. resolved_paths_for_gen_dir does not fall back to top_dir when a file is absent in gen_dir.
        let top_dir = paths.lake_raw_sec_dir();
        fs::write(top_dir.join(SUBMISSIONS_ZIP_FILE), b"stale-top-sub").unwrap();
        let gen1 = paths.lake_raw_sec_generations_dir().join("gen-001");
        let gen2 = paths.lake_raw_sec_generations_dir().join("gen-002");
        fs::create_dir_all(&gen1).unwrap();
        fs::create_dir_all(&gen2).unwrap();
        let gen1_resolved = resolved_paths_for_gen_dir("gen-001", &gen1);
        assert_eq!(
            gen1_resolved.submissions_zip_path,
            gen1.join(SUBMISSIONS_ZIP_FILE)
        );
        seed_valid_sec_gen_dir(&gen1, "g1-zip");
        seed_valid_sec_gen_dir(&gen2, "g2-zip");
        publish_sec_input_manifest_and_links(&paths, "gen-002", &gen2, 2).unwrap();
        // Unlocked gen-001 is moved to .deleting-* & removed under exclusive lock; reader pin returns None.
        cleanup_sec_input_generations(&paths, &paths.lake_raw_sec_generations_dir(), &["gen-002"]);
        assert!(!gen1.exists());
        assert!(
            paths
                .acquire_sec_generation_read_pin(&gen1.join(COMPANYFACTS_ZIP_FILE))
                .is_none()
        );

        // 3. Bulk ZIP fetch requires streaming len=...;sha256=... cursor before sending If-None-Match / accepting 304.
        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut zw = zip::ZipWriter::new(cursor);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("CIK0001883085.json", opts).unwrap();
            zw.write_all(b"{\"entityName\":\"Pagaya\"}").unwrap();
            zw.finish().unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_zip = zip_bytes.clone();
        let server_handle = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = io::BufReader::new(stream.try_clone().unwrap());
                let mut saw_if_none_match = false;
                loop {
                    let mut line = String::new();
                    let n = reader.read_line(&mut line).unwrap();
                    if n == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("if-none-match:") {
                        saw_if_none_match = true;
                    }
                }
                if saw_if_none_match {
                    stream
                        .write_all(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
                } else {
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        server_zip.len()
                    );
                    stream.write_all(hdr.as_bytes()).unwrap();
                    stream.write_all(&server_zip).unwrap();
                }
                stream.flush().unwrap();
            }
        });

        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let url = format!("http://{addr}/submissions.zip");
        let cached_zip_path = top_dir.join("test_submissions.zip");
        fs::write(&cached_zip_path, &zip_bytes).unwrap();

        // Case A: watermark lacks integrity cursor (cursor: None) -> refuses conditional 304, downloads & emits streaming cursor.
        let wm_no_cursor = SyncWatermark {
            source_key: WM_SUBMISSIONS_ZIP.into(),
            cursor: None,
            etag: Some("\"v1\"".into()),
            last_modified: None,
            synced_at_ms: 1,
        };
        let res_no_cursor = fetch_sec_file_staged(
            &client,
            &ConditionalFetchSpec {
                url: &url,
                dest_path: &cached_zip_path,
                watermark_key: WM_SUBMISSIONS_ZIP,
                cached_wm: Some(&wm_no_cursor),
                now_ms: 2,
                force_download: false,
                record_integrity_cursor: true,
            },
            validate_zip_file,
        )
        .unwrap();
        let expected_cursor = format_file_integrity_cursor(&cached_zip_path).unwrap();
        let recorded_wm = match res_no_cursor {
            StagedSecFetch::Downloaded { watermark, .. } => watermark,
            StagedSecFetch::NotModified | StagedSecFetch::Unavailable => {
                panic!("expected download when cursor is missing")
            }
        };
        assert_eq!(
            recorded_wm.cursor.as_deref(),
            Some(expected_cursor.as_str())
        );

        // Case B: valid cursor matches on-disk ZIP -> sends If-None-Match & accepts 304 Not Modified.
        let res_valid = fetch_sec_file_staged(
            &client,
            &ConditionalFetchSpec {
                url: &url,
                dest_path: &cached_zip_path,
                watermark_key: WM_SUBMISSIONS_ZIP,
                cached_wm: Some(&recorded_wm),
                now_ms: 3,
                force_download: false,
                record_integrity_cursor: true,
            },
            validate_zip_file,
        )
        .unwrap();
        assert!(matches!(res_valid, StagedSecFetch::NotModified));

        // Case C: truncated cached ZIP fails verify_file_integrity_cursor -> forces fresh download.
        fs::write(&cached_zip_path, &zip_bytes[..zip_bytes.len() / 2]).unwrap();
        let res_corrupt = fetch_sec_file_staged(
            &client,
            &ConditionalFetchSpec {
                url: &url,
                dest_path: &cached_zip_path,
                watermark_key: WM_SUBMISSIONS_ZIP,
                cached_wm: Some(&recorded_wm),
                now_ms: 4,
                force_download: false,
                record_integrity_cursor: true,
            },
            validate_zip_file,
        )
        .unwrap();
        assert!(matches!(res_corrupt, StagedSecFetch::Downloaded { .. }));
        server_handle.join().unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round14_submissions_guard_pointer_gc_safety_and_debenture_separation() {
        use std::io::BufRead;
        let dir = std::env::temp_dir().join(format!("m3-universe-r14-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.lake_raw_sec_generations_dir()).unwrap();

        // 1. Fix 3: Failed submissions.zip download with missing sic_map.json and unverified local
        //    submissions.zip must NOT reparse or stage unverified bytes into the next generation,
        //    while preserving previously verified DuckDB SIC classifications.
        let mut stale_sub_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut stale_sub_bytes);
            let mut zw = zip::ZipWriter::new(cursor);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("CIK0000320193.json", opts).unwrap();
            zw.write_all(br#"{"sic":9999,"sicDescription":"UNVERIFIED"}"#)
                .unwrap();
            zw.finish().unwrap();
        }
        let gen1 = paths.lake_raw_sec_generations_dir().join("gen-001");
        seed_valid_sec_gen_dir(&gen1, "pk-stub");
        let unverified_sub_path = gen1.join(SUBMISSIONS_ZIP_FILE);
        let sic_cache_path = gen1.join(SIC_MAP_FILE);
        fs::write(&unverified_sub_path, &stale_sub_bytes).unwrap();
        publish_sec_input_manifest_and_links(&paths, "gen-001", &gen1, 1).unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_handle = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    let n = reader.read_line(&mut line).unwrap();
                    if n == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                }
                stream
                    .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
                stream.flush().unwrap();
            }
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let bad_url = format!("http://{addr}/submissions.zip");
        let listed_ciks = HashSet::from(["0000320193".to_owned()]);
        let prev_sic = HashMap::from([(
            "0000320193".to_owned(),
            SicInfo {
                sic: Some(3571),
                sic_description: Some("ELECTRONIC COMPUTERS".to_owned()),
            },
        )]);

        let bad_cursor_wm = SyncWatermark {
            source_key: WM_SUBMISSIONS_ZIP.into(),
            cursor: Some("len=999;sha256=deadbeef".into()),
            etag: None,
            last_modified: None,
            synced_at_ms: 1,
        };
        let outcome_unverified = load_or_fetch_sic_map_with_url(
            &bad_url,
            &unverified_sub_path,
            &sic_cache_path,
            &client,
            Some(&bad_cursor_wm),
            &listed_ciks,
            &listed_ciks,
            &prev_sic,
            false,
            2,
        )
        .unwrap();
        assert!(!outcome_unverified.downloaded);
        assert!(matches!(
            outcome_unverified.staged_submissions,
            StagedSecFetch::Unavailable
        ));
        assert_eq!(
            outcome_unverified.map.get("0000320193").and_then(|s| s.sic),
            Some(3571),
            "previously verified DuckDB SIC must be preserved instead of parsing unverified ZIP"
        );

        let gen2 = paths.lake_raw_sec_generations_dir().join("gen-002");
        let mut guard2 = StagedSecGenerationGuard::new(
            &paths,
            "gen-002".into(),
            StagedSecFetch::NotModified,
            outcome_unverified.staged_submissions,
            StagedSecFetch::NotModified,
            outcome_unverified.map,
            resolved_paths_for_gen_dir("gen-001", &gen1),
        );
        guard2.promote_before_commit().unwrap();
        assert!(
            !gen2.join(SUBMISSIONS_ZIP_FILE).exists(),
            "unverified submissions.zip must never be staged into the next generation"
        );
        guard2.finalize(&paths, 2).unwrap();

        // When cursor matches verified ZIP bytes, failed download may safely reuse NotModified.
        let gen2_sub = gen2.join(SUBMISSIONS_ZIP_FILE);
        fs::write(&gen2_sub, &stale_sub_bytes).unwrap();
        let good_cursor = format_file_integrity_cursor(&gen2_sub).unwrap();
        let good_wm = SyncWatermark {
            source_key: WM_SUBMISSIONS_ZIP.into(),
            cursor: Some(good_cursor),
            etag: None,
            last_modified: None,
            synced_at_ms: 2,
        };
        let outcome_verified = load_or_fetch_sic_map_with_url(
            &bad_url,
            &gen2_sub,
            &gen2.join(SIC_MAP_FILE),
            &client,
            Some(&good_wm),
            &listed_ciks,
            &listed_ciks,
            &prev_sic,
            false,
            3,
        )
        .unwrap();
        assert!(matches!(
            outcome_verified.staged_submissions,
            StagedSecFetch::NotModified
        ));
        server_handle.join().unwrap();

        // 2. Fix 4: Malformed/invalid CURRENT.json or missing target generation must error BEFORE GC
        //    and NEVER wipe gen-* directories or fall back to legacy flat paths.
        let gen_keep = paths.lake_raw_sec_generations_dir().join("gen-009");
        seed_valid_sec_gen_dir(&gen_keep, "keep");
        fs::write(paths.lake_raw_sec_manifest_path(), b"{malformed-json").unwrap();
        assert!(
            reconcile_sec_input_generation_on_disk(&paths, None, 10).is_err(),
            "malformed CURRENT.json must fail recovery instead of entering legacy mode"
        );
        assert!(
            gen_keep.exists() && gen2.exists(),
            "generations must NOT be deleted when CURRENT.json is malformed"
        );
        cleanup_sec_input_generations(&paths, &paths.lake_raw_sec_generations_dir(), &[]);
        assert!(
            gen_keep.exists() && gen2.exists(),
            "GC must abort without deleting generations when CURRENT.json is unreadable"
        );
        store::write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &SecInputManifest {
                schema_version: 1,
                generation_id: "gen-missing".into(),
                generation_dir: "gen-missing".into(),
                committed_at_ms: 10,
            },
        )
        .unwrap();
        assert!(
            reconcile_sec_input_generation_on_disk(&paths, None, 11).is_err(),
            "CURRENT.json naming a missing directory must error before cleanup"
        );
        assert!(gen_keep.exists() && gen2.exists());

        // Only when CURRENT.json is genuinely NotFound (and no DB pointer) does legacy mode enter.
        let _ = fs::remove_file(paths.lake_raw_sec_manifest_path());
        let legacy_paths = reconcile_sec_input_generation_on_disk(&paths, None, 12).unwrap();
        assert_eq!(legacy_paths.generation_id, None);
        assert_eq!(
            legacy_paths.facts_zip_path,
            paths.lake_raw_sec_dir().join(COMPANYFACTS_ZIP_FILE)
        );

        // 3. Fix 5: Watchlisting CCZ (Comcast exchangeable debenture) must NOT displace CMCSA as
        //    canonical issuer ticker, nor leak watchlist metadata onto CMCSA.
        let comcast_sec = serde_json::to_vec(&serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [
                [1166691, "COMCAST CORP", "CMCSA", "Nasdaq"],
                [1166691, "COMCAST CORP", "CCZ", "NYSE"],
                [1652044, "Alphabet Inc.", "GOOG", "Nasdaq"],
                [1652044, "Alphabet Inc.", "GOOGL", "Nasdaq"]
            ]
        }))
        .unwrap();
        let wl_ccz = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "CCZ".into(),
                name: "Comcast Exchangeable Debenture".into(),
                bucket: Bucket::Core,
                source: DataSource::SecEdgar,
                cik: Some("0001166691".into()),
                currency: "USD".into(),
                circle: "Media".into(),
                too_hard_reason: None,
                assumptions: ValuationAssumptions {
                    multiple_low: Decimal::from(15),
                    multiple_high: Decimal::from(20),
                    target_return_pct: Decimal::from(10),
                    expected_growth_pct: Decimal::from(8),
                    strike_price_override: None,
                },
            }],
        };
        let lookup_ccz = WatchlistLookup::from_opt(Some(&wl_ccz));
        let listed =
            parse_canonical_tickers_exchange(&comcast_sec, &lookup_ccz.cik_to_ticker).unwrap();
        let by_cik: HashMap<String, ListedCompany> =
            listed.into_iter().map(|c| (c.cik.clone(), c)).collect();
        assert_eq!(by_cik["0001166691"].ticker, "CMCSA");
        assert!(!by_cik["0001166691"].is_non_common_listing);
        assert!(
            lookup_ccz.find("CMCSA", "0001166691").is_none(),
            "debenture CCZ watchlist entry must stay separated from canonical CMCSA"
        );
        assert_eq!(
            lookup_ccz
                .find("CCZ", "0001166691")
                .map(|c| c.ticker.as_str()),
            Some("CCZ")
        );

        let listed_googl = parse_canonical_tickers_exchange(
            &comcast_sec,
            &HashMap::from([("0001652044".to_owned(), "GOOGL".to_owned())]),
        )
        .unwrap();
        let googl_map: HashMap<String, ListedCompany> = listed_googl
            .into_iter()
            .map(|c| (c.cik.clone(), c))
            .collect();
        assert_eq!(googl_map["0001652044"].ticker, "GOOGL");
        assert_eq!(googl_map["0001166691"].ticker, "CMCSA");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round15_gc_pointer_and_row_order_debenture_regressions() {
        let dir = std::env::temp_dir().join(format!("m3-universe-r15-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.lake_raw_sec_generations_dir()).unwrap();

        // 1. Existing good generation gen-009 (with valid ZIP & tickers) must survive every broken pointer/asset mode:
        let gen_good = paths.lake_raw_sec_generations_dir().join("gen-009");
        seed_valid_sec_gen_dir(&gen_good, "good-facts");

        // Mode A: CURRENT.json with generation_dir = "." (dot traversal / generations root).
        store::write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &SecInputManifest {
                schema_version: 1,
                generation_id: ".".into(),
                generation_dir: ".".into(),
                committed_at_ms: 1,
            },
        )
        .unwrap();
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 10).is_err());
        cleanup_sec_input_generations(&paths, &paths.lake_raw_sec_generations_dir(), &[]);
        assert!(gen_good.exists(), "gen-009 must survive generation_dir='.'");

        // Mode B: CURRENT.json with mismatched generation_id != generation_dir.
        store::write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &SecInputManifest {
                schema_version: 1,
                generation_id: "gen-001".into(),
                generation_dir: "gen-009".into(),
                committed_at_ms: 2,
            },
        )
        .unwrap();
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 11).is_err());
        cleanup_sec_input_generations(&paths, &paths.lake_raw_sec_generations_dir(), &[]);
        assert!(
            gen_good.exists(),
            "gen-009 must survive manifest id/dir mismatch"
        );

        // Mode C: CURRENT.json points to gen-empty dir missing mandatory companyfacts.zip.
        let gen_empty = paths.lake_raw_sec_generations_dir().join("gen-empty");
        fs::create_dir_all(&gen_empty).unwrap();
        store::write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &SecInputManifest {
                schema_version: 1,
                generation_id: "gen-empty".into(),
                generation_dir: "gen-empty".into(),
                committed_at_ms: 3,
            },
        )
        .unwrap();
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 12).is_err());
        assert!(
            gen_good.exists(),
            "gen-009 must survive when CURRENT.json target lacks companyfacts.zip"
        );
        let _ = fs::remove_file(paths.lake_raw_sec_manifest_path());
        let _ = fs::remove_dir_all(&gen_empty);

        // Mode D & E: DuckDB WM_SEC_INPUT_GEN row exists with NULL or blank cursor (CURRENT.json absent).
        {
            let db = LakeDb::open(&paths).unwrap();
            db.write_watermark(&SyncWatermark {
                source_key: WM_SEC_INPUT_GEN.to_owned(),
                cursor: None,
                etag: None,
                last_modified: None,
                synced_at_ms: 4,
            })
            .unwrap();
        }
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 13).is_err());
        cleanup_sec_input_generations(&paths, &paths.lake_raw_sec_generations_dir(), &[]);
        assert!(
            gen_good.exists(),
            "gen-009 must survive NULL WM_SEC_INPUT_GEN"
        );
        {
            let db = LakeDb::open(&paths).unwrap();
            db.write_watermark(&SyncWatermark {
                source_key: WM_SEC_INPUT_GEN.to_owned(),
                cursor: Some("   ".to_owned()),
                etag: None,
                last_modified: None,
                synced_at_ms: 5,
            })
            .unwrap();
        }
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 14).is_err());
        assert!(
            gen_good.exists(),
            "gen-009 must survive blank WM_SEC_INPUT_GEN"
        );

        // 2. Reversed SEC row ordering keeps canonical common equity (CMCSA/DTE/JPM) ahead of debentures/notes.
        let reversed_sec = serde_json::to_vec(&serde_json::json!({
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [
                [1166691, "COMCAST CORP", "CCZ", "NYSE"],
                [1166691, "COMCAST CORP", "CMCSA", "Nasdaq"],
                [936340, "DTE ENERGY CO", "DTB", "NYSE"],
                [936340, "DTE ENERGY CO", "DTW", "NYSE"],
                [936340, "DTE ENERGY CO", "DTE", "NYSE"],
                [19617, "JPMORGAN CHASE & CO", "AMJB", "NYSE"],
                [19617, "JPMORGAN CHASE & CO", "JPM-PC", "NYSE"],
                [19617, "JPMORGAN CHASE & CO", "JPM", "NYSE"]
            ]
        }))
        .unwrap();
        let listed = parse_canonical_tickers_exchange(
            &reversed_sec,
            &HashMap::from([("0001166691".to_owned(), "CCZ".to_owned())]),
        )
        .unwrap();
        let by_cik: HashMap<String, ListedCompany> =
            listed.into_iter().map(|c| (c.cik.clone(), c)).collect();
        assert_eq!(by_cik["0001166691"].ticker, "CMCSA");
        assert!(!by_cik["0001166691"].is_non_common_listing);
        assert_eq!(by_cik["0000936340"].ticker, "DTE");
        assert!(!by_cik["0000936340"].is_non_common_listing);
        assert_eq!(by_cik["0000019617"].ticker, "JPM");
        assert!(!by_cik["0000019617"].is_non_common_listing);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round16_asset_validation_deferred_gc_repair_and_dangling_duckdb_symlink_safety() {
        let dir = std::env::temp_dir().join(format!("m3-universe-r16-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let gens_dir = paths.lake_raw_sec_generations_dir();
        fs::create_dir_all(&gens_dir).unwrap();

        // 1. Zero-byte companyfacts.zip, truncated companyfacts.zip, and missing/empty company_tickers_exchange.json
        //    must all fail strict recovery validation WITHOUT pruning existing valid gen-001.
        let gen1 = gens_dir.join("gen-001");
        seed_valid_sec_gen_dir(&gen1, "gen-001-complete");

        let gen_zero = gens_dir.join("gen-002");
        fs::create_dir_all(&gen_zero).unwrap();
        fs::write(gen_zero.join(COMPANYFACTS_ZIP_FILE), b"").unwrap();
        fs::write(
            gen_zero.join(TICKERS_EXCHANGE_FILE),
            minimal_valid_tickers_exchange_bytes(320193, "AAPL", "Apple Inc."),
        )
        .unwrap();
        store::write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &SecInputManifest {
                schema_version: 1,
                generation_id: "gen-002".into(),
                generation_dir: "gen-002".into(),
                committed_at_ms: 2,
            },
        )
        .unwrap();
        assert!(
            reconcile_sec_input_generation_on_disk(&paths, None, 10).is_err(),
            "zero-byte companyfacts.zip must fail recovery"
        );
        cleanup_sec_input_generations(&paths, &gens_dir, &[]);
        assert!(gen1.exists(), "gen-001 must survive zero-byte gen-002");

        // Valid ZIP header truncated mid-archive also fails recovery without deleting gen-001.
        let valid_zip =
            make_test_zip_bytes("CIK0000320193.json", &sample_companyfacts_json(320193, 900));
        let truncated_zip = &valid_zip[..valid_zip.len() / 2];
        fs::write(gen_zero.join(COMPANYFACTS_ZIP_FILE), truncated_zip).unwrap();
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 11).is_err());
        assert!(gen1.exists(), "gen-001 must survive truncated gen-002");

        // Missing or empty company_tickers_exchange.json also fails recovery without deleting gen-001.
        fs::write(gen_zero.join(COMPANYFACTS_ZIP_FILE), &valid_zip).unwrap();
        fs::write(gen_zero.join(TICKERS_EXCHANGE_FILE), b"").unwrap();
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 12).is_err());
        let _ = fs::remove_file(gen_zero.join(TICKERS_EXCHANGE_FILE));
        assert!(reconcile_sec_input_generation_on_disk(&paths, None, 13).is_err());
        assert!(gen1.exists(), "gen-001 must survive incomplete gen-002");
        let _ = fs::remove_file(paths.lake_raw_sec_manifest_path());
        let _ = fs::remove_dir_all(&gen_zero);

        // 2. Dangling DuckDB symlink must fail pre-sync read-only check WITHOUT creating the symlink target
        //    and WITHOUT pruning gen-001 when CURRENT.json is absent.
        #[cfg(unix)]
        {
            fs::create_dir_all(paths.lake_dir()).unwrap();
            let dangling_target = dir.join("missing-duckdb-target.duckdb");
            let _ = fs::remove_file(&dangling_target);
            let _ = fs::remove_file(paths.lake_db_path());
            std::os::unix::fs::symlink(&dangling_target, paths.lake_db_path()).unwrap();
            let dead_urls = SecFeedUrls {
                tickers_exchange_url: "http://127.0.0.1:9/tickers.json",
                submissions_zip_url: "http://127.0.0.1:9/submissions.zip",
                companyfacts_zip_url: "http://127.0.0.1:9/companyfacts.zip",
            };
            let res = sync_universe_with_sec_urls(
                &paths,
                &UniverseSyncOptions {
                    full_sec: false,
                    prices_top: 10,
                    skip_prices: true,
                },
                &dead_urls,
                |_| {},
            );
            assert!(
                res.is_err(),
                "dangling lake.duckdb symlink must abort sync before recovery cleanup"
            );
            assert!(
                !dangling_target.exists(),
                "dangling DuckDB symlink target must stay absent"
            );
            assert!(
                gen1.exists(),
                "gen-001 must survive dangling DuckDB symlink failure"
            );
            let _ = fs::remove_file(paths.lake_db_path());
        }

        // 3. Two-generation sync repair test:
        //    First run full sync to create committed DuckDB + gen-1; seed gen-2 as active with
        //    truncated companyfacts.zip alongside intact gen-1; fail repair (HTTP 500) -> gen-1 survives;
        //    then succeed repair (HTTP 200) -> heals into next gen and prunes old gens cleanly.
        let tickers_bytes = minimal_valid_tickers_exchange_bytes(320193, "AAPL", "Apple Inc.");
        let mut sub_zip = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut sub_zip);
            let mut zw = ZipWriter::new(cursor);
            zw.start_file("CIK0000320193.json", SimpleFileOptions::default())
                .unwrap();
            zw.write_all(br#"{"sic":3571,"sicDescription":"ELECTRONIC COMPUTERS"}"#)
                .unwrap();
            zw.finish().unwrap();
        }
        let facts_zip =
            make_test_zip_bytes("CIK0000320193.json", &sample_companyfacts_json(320193, 900));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let fail_facts_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fail_flag = fail_facts_once.clone();
        let stop_server = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag = stop_server.clone();
        let t_serve = tickers_bytes.clone();
        let s_serve = sub_zip.clone();
        let f_serve = facts_zip.clone();
        let server = thread::spawn(move || {
            use std::io::{BufRead, Write};
            listener.set_nonblocking(true).unwrap();
            while !stop_flag.load(std::sync::atomic::Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                        let mut req_line = String::new();
                        if reader.read_line(&mut req_line).is_err() {
                            continue;
                        }
                        loop {
                            let mut line = String::new();
                            let n = reader.read_line(&mut line).unwrap_or(0);
                            if n == 0 || line == "\r\n" || line == "\n" {
                                break;
                            }
                        }
                        if req_line.contains("companyfacts.zip")
                            && fail_flag.load(std::sync::atomic::Ordering::Acquire)
                        {
                            let _ = stream.write_all(
                                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            );
                        } else {
                            let (ctype, body): (&str, &[u8]) = if req_line.contains("tickers") {
                                ("application/json", &t_serve)
                            } else if req_line.contains("submissions") {
                                ("application/zip", &s_serve)
                            } else {
                                ("application/zip", &f_serve)
                            };
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(hdr.as_bytes());
                            let _ = stream.write_all(body);
                        }
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        let feed_urls = SecFeedUrls {
            tickers_exchange_url: &format!("http://{addr}/company_tickers_exchange.json"),
            submissions_zip_url: &format!("http://{addr}/submissions.zip"),
            companyfacts_zip_url: &format!("http://{addr}/companyfacts.zip"),
        };
        let sync_opts = UniverseSyncOptions {
            full_sec: false,
            prices_top: 10,
            skip_prices: true,
        };
        // Clean pre-seeded gen-001 first so gen-1-p... creates cleanly from sync.
        let _ = fs::remove_dir_all(&gen1);
        sync_universe_with_sec_urls(&paths, &sync_opts, &feed_urls, |_| {}).unwrap();
        let first_gen_id = read_manifest_gen_id(&paths).unwrap().unwrap();
        let first_gen_dir = gens_dir.join(&first_gen_id);
        assert!(sec_gen_has_complete_assets(&first_gen_dir, "first", &first_gen_id).is_ok());

        // Create second generation gen-200 as active pointer (both DB WM_SEC_INPUT_GEN and CURRENT.json),
        // but with a TRUNCATED companyfacts.zip! Force SEC repair to fail with HTTP 500.
        let bad_gen_id = "gen-200-truncated";
        let bad_gen_dir = gens_dir.join(bad_gen_id);
        fs::create_dir_all(&bad_gen_dir).unwrap();
        fs::write(bad_gen_dir.join(TICKERS_EXCHANGE_FILE), &tickers_bytes).unwrap();
        fs::write(bad_gen_dir.join(SUBMISSIONS_ZIP_FILE), &sub_zip).unwrap();
        fs::write(
            bad_gen_dir.join(COMPANYFACTS_ZIP_FILE),
            &facts_zip[..facts_zip.len() / 2],
        )
        .unwrap();
        publish_sec_input_manifest_and_links(&paths, bad_gen_id, &bad_gen_dir, 999).unwrap();
        {
            let db = LakeDb::open(&paths).unwrap();
            db.write_watermark(&SyncWatermark {
                source_key: WM_SEC_INPUT_GEN.to_owned(),
                cursor: Some(bad_gen_id.to_owned()),
                etag: None,
                last_modified: None,
                synced_at_ms: 999,
            })
            .unwrap();
        }
        fail_facts_once.store(true, std::sync::atomic::Ordering::Release);
        let failed_repair = sync_universe_with_sec_urls(&paths, &sync_opts, &feed_urls, |_| {});
        assert!(
            failed_repair.is_err(),
            "sync must error when active generation has truncated ZIP and repair fails"
        );
        assert!(
            first_gen_dir.exists()
                && sec_gen_has_complete_assets(&first_gen_dir, "first", &first_gen_id).is_ok(),
            "complete prior generation {first_gen_id} must survive failed repair of truncated active generation"
        );

        // Now allow HTTP 200 repair: sync self-heals into a new valid generation and prunes old gens.
        fail_facts_once.store(false, std::sync::atomic::Ordering::Release);
        sync_universe_with_sec_urls(&paths, &sync_opts, &feed_urls, |_| {}).unwrap();
        let healed_gen_id = read_manifest_gen_id(&paths).unwrap().unwrap();
        assert_ne!(healed_gen_id, bad_gen_id);
        assert!(
            sec_gen_has_complete_assets(&gens_dir.join(&healed_gen_id), "healed", &healed_gen_id)
                .is_ok()
        );
        assert!(!bad_gen_dir.exists());

        stop_server.store(true, std::sync::atomic::Ordering::Release);
        server.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round16_pre_sync_migrates_legacy_duckdb_without_extractor_version_safely() {
        let dir = std::env::temp_dir().join(format!("m3-universe-r16-mig-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.lake_dir()).unwrap();
        fs::create_dir_all(paths.lake_parquet_dir()).unwrap();
        fs::create_dir_all(paths.lake_raw_sec_generations_dir()).unwrap();

        let gen1_dir = paths.lake_raw_sec_generations_dir().join("gen-001");
        let tickers_bytes = minimal_valid_tickers_exchange_bytes(320193, "AAPL", "Apple Inc.");
        let aapl_facts_json = sample_companyfacts_json(320193, 900);
        let aapl_facts_sha = edgar::hex_digest(&aapl_facts_json);
        let facts_zip = make_test_zip_bytes("CIK0000320193.json", &aapl_facts_json);
        let mut sub_zip = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut sub_zip);
            let mut zw = ZipWriter::new(cursor);
            zw.start_file("CIK0000320193.json", SimpleFileOptions::default())
                .unwrap();
            zw.write_all(br#"{"sic":3571,"sicDescription":"ELECTRONIC COMPUTERS"}"#)
                .unwrap();
            zw.finish().unwrap();
        }
        fs::create_dir_all(&gen1_dir).unwrap();
        fs::write(gen1_dir.join(TICKERS_EXCHANGE_FILE), &tickers_bytes).unwrap();
        fs::write(gen1_dir.join(SUBMISSIONS_ZIP_FILE), &sub_zip).unwrap();
        fs::write(gen1_dir.join(COMPANYFACTS_ZIP_FILE), &facts_zip).unwrap();
        publish_sec_input_manifest_and_links(&paths, "gen-001", &gen1_dir, 1).unwrap();

        // Seed a legacy DuckDB schema lacking universe_snapshot.extractor_version and other
        // newer migration columns/nullability changes.
        {
            let conn = duckdb::Connection::open(paths.lake_db_path()).unwrap();
            conn.execute_batch(
                &"CREATE TABLE sync_watermarks (
                    source_key VARCHAR PRIMARY KEY,
                    cursor VARCHAR,
                    etag VARCHAR,
                    last_modified VARCHAR,
                    synced_at_ms UBIGINT NOT NULL
                );
                CREATE TABLE companies (
                    cik VARCHAR PRIMARY KEY,
                    ticker VARCHAR NOT NULL,
                    name VARCHAR NOT NULL,
                    exchange VARCHAR NOT NULL,
                    sic INTEGER,
                    sic_description VARCHAR,
                    is_financial BOOLEAN NOT NULL,
                    is_active BOOLEAN NOT NULL
                );
                CREATE TABLE facts_annual (
                    cik VARCHAR NOT NULL, ticker VARCHAR NOT NULL, fiscal_year INTEGER NOT NULL,
                    label VARCHAR NOT NULL, period_end VARCHAR NOT NULL, filed VARCHAR,
                    revenue DECIMAL(38,6) NOT NULL, gross_profit DECIMAL(38,6) NOT NULL,
                    gross_margin_pct DECIMAL(38,10), ebit DECIMAL(38,6) NOT NULL,
                    nopat DECIMAL(38,6) NOT NULL, invested_capital DECIMAL(38,6) NOT NULL,
                    roic_pct DECIMAL(38,10), roic_note VARCHAR, roic_total_capital_pct DECIMAL(38,10),
                    ocf DECIMAL(38,6) NOT NULL, capex DECIMAL(38,6) NOT NULL, fcf DECIMAL(38,6) NOT NULL,
                    sbc DECIMAL(38,6) NOT NULL, strict_owner_earnings DECIMAL(38,6) NOT NULL,
                    sbc_to_ocf_pct DECIMAL(38,10), buybacks DECIMAL(38,6) NOT NULL,
                    cash_and_st DECIMAL(38,6) NOT NULL, total_debt DECIMAL(38,6) NOT NULL,
                    net_debt DECIMAL(38,6) NOT NULL, equity DECIMAL(38,6) NOT NULL,
                    diluted_shares DECIMAL(38,6) NOT NULL, ending_shares DECIMAL(38,6) NOT NULL,
                    raw_sha256 VARCHAR NOT NULL, provenance_json VARCHAR NOT NULL,
                    exact_decimals_json VARCHAR NOT NULL,
                    PRIMARY KEY (cik, fiscal_year, period_end)
                );
                CREATE TABLE facts_ttm (
                    cik VARCHAR PRIMARY KEY, ticker VARCHAR NOT NULL, label VARCHAR NOT NULL,
                    period_end VARCHAR NOT NULL, revenue DECIMAL(38,6) NOT NULL,
                    gross_profit DECIMAL(38,6) NOT NULL, gross_margin_pct DECIMAL(38,10),
                    ebit DECIMAL(38,6) NOT NULL, nopat DECIMAL(38,6) NOT NULL,
                    invested_capital DECIMAL(38,6) NOT NULL, roic_pct DECIMAL(38,10),
                    roic_note VARCHAR, roic_total_capital_pct DECIMAL(38,10),
                    ocf DECIMAL(38,6) NOT NULL, capex DECIMAL(38,6) NOT NULL, fcf DECIMAL(38,6) NOT NULL,
                    sbc DECIMAL(38,6) NOT NULL, strict_owner_earnings DECIMAL(38,6) NOT NULL,
                    sbc_to_ocf_pct DECIMAL(38,10), buybacks DECIMAL(38,6) NOT NULL,
                    cash_and_st DECIMAL(38,6) NOT NULL, total_debt DECIMAL(38,6) NOT NULL,
                    net_debt DECIMAL(38,6) NOT NULL, equity DECIMAL(38,6) NOT NULL,
                    diluted_shares DECIMAL(38,6) NOT NULL, ytd_period_label VARCHAR NOT NULL,
                    ytd_start VARCHAR NOT NULL, ytd_end VARCHAR NOT NULL,
                    ytd_revenue DECIMAL(38,6) NOT NULL, ytd_ocf DECIMAL(38,6) NOT NULL,
                    ytd_capex DECIMAL(38,6) NOT NULL, ytd_sbc DECIMAL(38,6) NOT NULL,
                    ytd_buybacks DECIMAL(38,6) NOT NULL, raw_sha256 VARCHAR NOT NULL,
                    provenance_json VARCHAR NOT NULL
                );
                CREATE TABLE prices_daily (
                    symbol VARCHAR NOT NULL, trade_date VARCHAR NOT NULL,
                    price DECIMAL(38,6) NOT NULL, previous_close DECIMAL(38,6),
                    currency VARCHAR NOT NULL, exchange VARCHAR NOT NULL,
                    market_time_ms UBIGINT, fetched_at_ms UBIGINT NOT NULL,
                    source_url VARCHAR NOT NULL, PRIMARY KEY (symbol, trade_date)
                );
                CREATE TABLE universe_snapshot (
                    ticker VARCHAR NOT NULL, cik VARCHAR PRIMARY KEY, name VARCHAR NOT NULL,
                    exchange VARCHAR NOT NULL, sic INTEGER, sic_description VARCHAR,
                    is_financial BOOLEAN NOT NULL, latest_filing_end VARCHAR,
                    years_of_data INTEGER NOT NULL, quality_score DECIMAL(38,6) NOT NULL,
                    in_price_pool BOOLEAN NOT NULL, in_watchlist BOOLEAN NOT NULL,
                    bucket_opt VARCHAR, latest_price DECIMAL(38,6),
                    strike_price DECIMAL(38,6), distance_to_strike_pct DECIMAL(38,10),
                    strict_fcf_yield_pct DECIMAL(38,10), roic_5y_avg_pct DECIMAL(38,10),
                    roic_5y_stddev_pct DECIMAL(38,10), gross_margin_5y_avg_pct DECIMAL(38,10),
                    gross_margin_5y_stddev_pct DECIMAL(38,10), share_change_3y_pct DECIMAL(38,10),
                    sbc_to_ocf_pct DECIMAL(38,10), net_debt_to_oe DECIMAL(38,10),
                    strict_owner_earnings DECIMAL(38,6), warnings_count INTEGER NOT NULL,
                    warnings_json VARCHAR NOT NULL, raw_sha256 VARCHAR,
                    exact_decimals_json VARCHAR NOT NULL
                );
                INSERT INTO sync_watermarks VALUES ('sec_input_generation', 'gen-001', NULL, NULL, 1);
                INSERT INTO universe_snapshot VALUES (
                    'AAPL', '0000320193', 'Apple Inc.', 'Nasdaq', 3571, 'ELECTRONIC COMPUTERS',
                    FALSE, '2024-12-31', 5, 80, FALSE, FALSE, NULL, NULL, NULL, NULL, NULL,
                    NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, '[]', '__SHA__', '{}'
                );"
                    .replace("__SHA__", &aapl_facts_sha),
            )
            .unwrap();
        }

        let pre = read_pre_sync_state(&paths).unwrap();
        assert_eq!(pre.prev_input_gen.as_deref(), Some("gen-001"));
        assert_eq!(pre.parsed_financial_companies, 1);
        assert!(pre.needs_schema_backfill);

        // Full sync succeeds end-to-end against migrated legacy DuckDB.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop_server = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag = stop_server.clone();
        let t_serve = tickers_bytes.clone();
        let s_serve = sub_zip.clone();
        let f_serve = facts_zip.clone();
        let server = thread::spawn(move || {
            use std::io::{BufRead, Write};
            listener.set_nonblocking(true).unwrap();
            while !stop_flag.load(std::sync::atomic::Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                        let mut req_line = String::new();
                        if reader.read_line(&mut req_line).is_err() {
                            continue;
                        }
                        loop {
                            let mut line = String::new();
                            let n = reader.read_line(&mut line).unwrap_or(0);
                            if n == 0 || line == "\r\n" || line == "\n" {
                                break;
                            }
                        }
                        let (ctype, body): (&str, &[u8]) = if req_line.contains("tickers") {
                            ("application/json", &t_serve)
                        } else if req_line.contains("submissions") {
                            ("application/zip", &s_serve)
                        } else {
                            ("application/zip", &f_serve)
                        };
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(body);
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        let feed_urls = SecFeedUrls {
            tickers_exchange_url: &format!("http://{addr}/company_tickers_exchange.json"),
            submissions_zip_url: &format!("http://{addr}/submissions.zip"),
            companyfacts_zip_url: &format!("http://{addr}/companyfacts.zip"),
        };
        let mut events: Vec<UniverseSyncProgress> = Vec::new();
        let rep = sync_universe_with_sec_urls_structured(
            &paths,
            &UniverseSyncOptions {
                full_sec: false,
                prices_top: 10,
                skip_prices: true,
            },
            &feed_urls,
            |prog| events.push(prog),
        )
        .unwrap();
        assert_eq!(rep.parsed_companies, 1);
        assert!(events.iter().any(|p| p.lake_ready));
        assert!(events.windows(2).all(|w| w[0].fraction <= w[1].fraction));

        stop_server.store(true, std::sync::atomic::Ordering::Release);
        server.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn structured_progress_clamps_fractions_and_parses_ratios() {
        let low = UniverseSyncProgress::new(UniverseSyncStage::Index, -0.5, "start");
        let high = UniverseSyncProgress::new(UniverseSyncStage::Exporting, 1.5, "done");
        assert_eq!(low.fraction, 0.0);
        assert_eq!(high.fraction, 1.0);
        assert!(!low.lake_ready);
        assert_eq!(
            parse_progress_ratio("流式解压并并行解析 XBRL (320/6400)..."),
            Some((320, 6400))
        );
        assert_eq!(parse_progress_ratio("no parens"), None);
    }
}
