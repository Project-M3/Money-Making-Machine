use crate::{
    edgar,
    lake::{LakeDb, SyncWatermark, UniverseRow},
    lake_l1::{
        DocChunkRow, L1Summary, LakeEventRow, LakeFilingRow, SignalEvidence, SignalInboxRow,
        SignalKind, SignalStatus,
    },
    latent::{self, DEFAULT_CHUNK_CHARS, DEFAULT_CHUNK_OVERLAP, Embedder},
    market,
    model::{Bucket, WarningSeverity},
    store::{self, WorkspacePaths},
};
use anyhow::{Context, Result};
use chrono::DateTime;
use reqwest::{StatusCode, blocking::Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufReader, Read},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use zip::ZipArchive;

pub const WM_L1_BOOTSTRAP: &str = "l1_sec_submissions_bootstrap";
pub const WM_L1_CATCHUP: &str = "l1_sec_submissions_catchup";
pub const WM_L1_NEWS: &str = "l1_candidate_news";
pub const WM_L1_EVENTS_CATCHUP: &str = "l1_events_catchup";

const SEC_SUBMISSIONS_BASE_URL: &str = "https://data.sec.gov/submissions";
const SEC_ARCHIVES_DATA_BASE_URL: &str = "https://www.sec.gov/Archives/edgar/data";
const YAHOO_RSS_BASE_URL: &str = "https://feeds.finance.yahoo.com/rss/2.0/headline";

const MAX_SUBMISSION_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PRIMARY_DOC_BYTES: usize = 16 * 1024 * 1024;
const MAX_RSS_BYTES: usize = 2 * 1024 * 1024;
const MAX_CHUNKS_PER_FILING: usize = 40;
const SEC_RATE_LIMIT_MS: u64 = 185;
const NEWS_RATE_LIMIT_MS: u64 = 150;
const LOOKBACK_DAYS_10KQ: i64 = 400;
const LOOKBACK_DAYS_8K_FORM4: i64 = 90;
const MAX_ERRORS_SAMPLE: usize = 20;
const CANDIDATE_POOL_QUERY_LIMIT: usize = 1000;
const MAX_STRIKE_SIGNALS: usize = 30;
const MAX_DISCOVERY_SIGNALS: usize = 40;
const MAX_WARNING_RED_FLAGS: usize = 25;
const FORM4_CLUSTER_WINDOW_DAYS: i64 = 14;
const FORM4_CLUSTER_MIN_COUNT: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventCatchupOptions {
    pub bootstrap: bool,
    pub skip_sec: bool,
    pub skip_news: bool,
    pub skip_docs: bool,
    pub max_ciks: usize,
    pub max_docs: usize,
    pub max_news_tickers: usize,
}

impl Default for EventCatchupOptions {
    fn default() -> Self {
        Self {
            bootstrap: false,
            skip_sec: false,
            skip_news: false,
            skip_docs: false,
            max_ciks: 25,
            max_docs: 12,
            max_news_tickers: 20,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventCatchupReport {
    pub schema_version: u32,
    pub synced_at_ms: u64,
    pub flagship_tokens_used: u64,
    pub bootstrapped_from_zip: bool,
    pub candidate_pool_count: usize,
    pub sec_ciks_checked: usize,
    pub sec_filings_ingested: usize,
    pub docs_attempted: usize,
    pub docs_indexed: usize,
    pub chunks_indexed: usize,
    pub news_tickers_checked: usize,
    pub news_events_ingested: usize,
    pub signals_created_or_updated: usize,
    pub ripple_signals_emitted: usize,
    pub errors_sample: Vec<String>,
    pub summary: L1Summary,
}

#[derive(Debug, Clone)]
struct CatchupEnv {
    sec_submissions_base_url: String,
    sec_archives_data_base_url: String,
    yahoo_rss_base_url: String,
    sec_rate_limit_ms: u64,
    news_rate_limit_ms: u64,
    now_ms: u64,
}

impl CatchupEnv {
    fn live(now_ms: u64) -> Self {
        Self {
            sec_submissions_base_url: SEC_SUBMISSIONS_BASE_URL.to_owned(),
            sec_archives_data_base_url: SEC_ARCHIVES_DATA_BASE_URL.to_owned(),
            yahoo_rss_base_url: YAHOO_RSS_BASE_URL.to_owned(),
            sec_rate_limit_ms: SEC_RATE_LIMIT_MS,
            news_rate_limit_ms: NEWS_RATE_LIMIT_MS,
            now_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RssNewsItem {
    pub title: String,
    pub link: String,
    pub guid: Option<String>,
    pub pub_date: Option<String>,
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct SecSubmissionDoc {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    filings: Option<SecSubmissionFilings>,
}

#[derive(Debug, Deserialize)]
struct SecSubmissionFilings {
    #[serde(default)]
    recent: Option<SecRecentFilings>,
}

#[derive(Debug, Deserialize)]
struct SecRecentFilings {
    #[serde(rename = "accessionNumber", default)]
    accession_number: Vec<Option<String>>,
    #[serde(rename = "filingDate", default)]
    filing_date: Vec<Option<String>>,
    #[serde(rename = "reportDate", default)]
    report_date: Vec<Option<String>>,
    #[serde(rename = "acceptanceDateTime", default)]
    acceptance_date_time: Vec<Option<String>>,
    #[serde(default)]
    form: Vec<Option<String>>,
    #[serde(default)]
    items: Vec<Option<ItemsField>>,
    #[serde(rename = "primaryDocument", default)]
    primary_document: Vec<Option<String>>,
    #[serde(rename = "primaryDocDescription", default)]
    primary_doc_description: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ItemsField {
    Text(String),
    List(Vec<Option<String>>),
}

impl ItemsField {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::Text(s) => s
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect(),
            Self::List(items) => items
                .into_iter()
                .flatten()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}

#[allow(dead_code)]
pub fn catchup_events(
    paths: &WorkspacePaths,
    opts: &EventCatchupOptions,
) -> Result<EventCatchupReport> {
    catchup_events_with_progress(paths, opts, |_| {})
}

pub fn catchup_events_with_progress<F: FnMut(&str)>(
    paths: &WorkspacePaths,
    opts: &EventCatchupOptions,
    on_progress: F,
) -> Result<EventCatchupReport> {
    let env = CatchupEnv::live(store::now_ms());
    catchup_events_with_env(paths, opts, &env, on_progress)
}

fn catchup_events_with_env<F: FnMut(&str)>(
    paths: &WorkspacePaths,
    opts: &EventCatchupOptions,
    env: &CatchupEnv,
    mut on_progress: F,
) -> Result<EventCatchupReport> {
    let db = LakeDb::open(paths)?;
    if paths.watchlist_path().exists() {
        let _ = db.refresh_watchlist_flags(paths);
    }

    let candidates = db.list_candidate_pool_companies(CANDIDATE_POOL_QUERY_LIMIT)?;
    let embedder = Embedder::from_env();

    let mut report = EventCatchupReport {
        schema_version: 1,
        synced_at_ms: env.now_ms,
        flagship_tokens_used: 0,
        candidate_pool_count: candidates.len(),
        ..Default::default()
    };

    let mut target_ciks: HashSet<String> = HashSet::with_capacity(candidates.len());
    let mut cik_to_ticker: HashMap<String, String> = HashMap::with_capacity(candidates.len());
    let mut candidate_by_cik: HashMap<String, &UniverseRow> =
        HashMap::with_capacity(candidates.len());
    for row in &candidates {
        if let Ok(norm_cik) = edgar::normalize_cik(&row.cik) {
            target_ciks.insert(norm_cik.clone());
            cik_to_ticker.insert(norm_cik.clone(), row.ticker.clone());
            candidate_by_cik.insert(norm_cik, row);
        }
    }

    if let Ok(wl) = store::load_watchlist(paths) {
        for co in wl.companies {
            if let Some(raw_cik) = co.cik.as_deref()
                && let Ok(norm_cik) = edgar::normalize_cik(raw_cik)
            {
                target_ciks.insert(norm_cik.clone());
                cik_to_ticker.entry(norm_cik).or_insert(co.ticker);
            }
        }
    }

    let sec_client = sec_events_http_client()?;
    let mut last_sec_req: Option<Instant> = None;

    // 1. Candidate-pool submissions.zip bootstrap
    if !opts.skip_sec && !target_ciks.is_empty() {
        let has_bootstrap_wm = db.read_watermark(WM_L1_BOOTSTRAP)?.is_some();
        if opts.bootstrap || !has_bootstrap_wm {
            on_progress("从 SEC submissions.zip 引导候选池历史公告...");
            match bootstrap_from_submissions_zip(
                paths,
                &db,
                &target_ciks,
                &cik_to_ticker,
                env,
                &mut report.errors_sample,
            ) {
                Ok(Some(ingested)) => {
                    report.bootstrapped_from_zip = true;
                    report.sec_filings_ingested += ingested;
                }
                Ok(None) => {}
                Err(err) => push_error_sample(
                    &mut report.errors_sample,
                    format!("submissions.zip bootstrap: {err:#}"),
                ),
            }
        }
    }

    // 2. Incremental SEC catch-up
    if !opts.skip_sec && opts.max_ciks > 0 && !candidates.is_empty() {
        on_progress("追平候选池最新 SEC 公告提交...");
        match incremental_sec_catchup(
            &db,
            &sec_client,
            &candidates,
            opts.max_ciks,
            env,
            &mut last_sec_req,
            &mut report.errors_sample,
        ) {
            Ok((checked, ingested)) => {
                report.sec_ciks_checked = checked;
                report.sec_filings_ingested += ingested;
                if ingested > 0
                    && let Err(err) = db.export_filings_parquet(paths)
                {
                    push_error_sample(
                        &mut report.errors_sample,
                        format!("export filings.parquet: {err:#}"),
                    );
                }
            }
            Err(err) => push_error_sample(
                &mut report.errors_sample,
                format!("incremental SEC catchup: {err:#}"),
            ),
        }
    }

    // 3. Company profile baseline + 10-K/8-K text indexing
    if !candidates.is_empty() {
        on_progress("更新候选池公司商业画像与隐空间切片...");
        match index_company_profiles(&db, &candidates, &embedder, &mut report.errors_sample) {
            Ok(indexed) => report.chunks_indexed += indexed,
            Err(err) => push_error_sample(
                &mut report.errors_sample,
                format!("company profile indexing: {err:#}"),
            ),
        }
    }

    if !opts.skip_docs && opts.max_docs > 0 && !candidates.is_empty() {
        on_progress("抓取并切分候选池最新 10-K / 8-K 原文...");
        match index_candidate_filings_text(
            paths,
            &db,
            &sec_client,
            &candidates,
            opts.max_docs,
            &embedder,
            env,
            &mut last_sec_req,
            &mut report.errors_sample,
        ) {
            Ok((attempted, indexed_docs, indexed_chunks)) => {
                report.docs_attempted = attempted;
                report.docs_indexed = indexed_docs;
                report.chunks_indexed += indexed_chunks;
            }
            Err(err) => push_error_sample(
                &mut report.errors_sample,
                format!("filing text indexing: {err:#}"),
            ),
        }
    }

    // 4. Candidate-pool News Ingestion
    if !opts.skip_news && opts.max_news_tickers > 0 && !candidates.is_empty() {
        on_progress("拉取候选池 Yahoo Finance 新闻流...");
        match ingest_candidate_news(
            &db,
            &candidates,
            opts.max_news_tickers,
            &embedder,
            env,
            &mut report.errors_sample,
        ) {
            Ok((checked, news_ingested, chunks_added)) => {
                report.news_tickers_checked = checked;
                report.news_events_ingested = news_ingested;
                report.chunks_indexed += chunks_added;
            }
            Err(err) => push_error_sample(
                &mut report.errors_sample,
                format!("candidate news ingestion: {err:#}"),
            ),
        }
    }

    // 5. Deterministic Signal Scoring & Inbox Generation
    on_progress("生成确定性收件箱信号与隐空间传导...");
    match generate_signals_and_ripples(&db, &candidates, &embedder, env, &mut report.errors_sample)
    {
        Ok((signals_count, ripples_count)) => {
            report.signals_created_or_updated = signals_count;
            report.ripple_signals_emitted = ripples_count;
        }
        Err(err) => push_error_sample(
            &mut report.errors_sample,
            format!("signal generation: {err:#}"),
        ),
    }

    db.write_watermark(&SyncWatermark {
        source_key: WM_L1_EVENTS_CATCHUP.to_owned(),
        cursor: Some(env.now_ms.to_string()),
        etag: None,
        last_modified: None,
        synced_at_ms: env.now_ms,
    })?;

    report.flagship_tokens_used = 0;
    report.summary = db.l1_summary()?;
    Ok(report)
}

fn sec_events_http_client() -> Result<Client> {
    Client::builder()
        .user_agent(edgar::sec_user_agent())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build SEC HTTP client")
}

fn push_error_sample(errors: &mut Vec<String>, msg: String) {
    if errors.len() < MAX_ERRORS_SAMPLE {
        errors.push(msg);
    }
}

fn enforce_rate_limit(last_req: &mut Option<Instant>, min_spacing_ms: u64) {
    if min_spacing_ms == 0 {
        *last_req = Some(Instant::now());
        return;
    }
    let min_dur = Duration::from_millis(min_spacing_ms);
    if let Some(prev) = *last_req {
        let elapsed = prev.elapsed();
        if elapsed < min_dur {
            thread::sleep(min_dur.saturating_sub(elapsed));
        }
    }
    *last_req = Some(Instant::now());
}

fn resolve_submissions_zip_for_bootstrap(paths: &WorkspacePaths) -> Option<PathBuf> {
    if let Ok(Some(resolved)) = paths.resolve_active_sec_raw_file_checked("submissions.zip")
        && resolved.is_file()
    {
        return Some(resolved);
    }

    #[derive(Deserialize)]
    struct ManifestRef {
        #[serde(default)]
        generation_dir: String,
    }
    let manifest_path = paths.lake_raw_sec_manifest_path();
    if manifest_path.is_file()
        && let Ok(manifest) = store::read_json::<ManifestRef>(&manifest_path)
    {
        let gen_dir = manifest.generation_dir.trim();
        if !gen_dir.is_empty() && !gen_dir.contains("..") && !gen_dir.contains('/') {
            let candidate = paths
                .lake_raw_sec_generations_dir()
                .join(gen_dir)
                .join("submissions.zip");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let top_level = paths.lake_raw_sec_dir().join("submissions.zip");
    top_level.is_file().then_some(top_level)
}

fn match_cik_json_entry(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    if base.len() != 18 {
        return None;
    }
    let prefix = base.get(..3)?;
    let digits = base.get(3..13)?;
    let suffix = base.get(13..)?;
    if prefix.eq_ignore_ascii_case("CIK")
        && digits.bytes().all(|b| b.is_ascii_digit())
        && suffix.eq_ignore_ascii_case(".json")
    {
        Some(digits.to_owned())
    } else {
        None
    }
}

fn read_bounded_reader<R: Read>(mut reader: R, max_bytes: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut limited = (&mut reader).take((max_bytes + 1) as u64);
    limited.read_to_end(&mut buf)?;
    if buf.len() > max_bytes {
        buf.truncate(max_bytes);
    }
    Ok(buf)
}

fn bootstrap_from_submissions_zip(
    paths: &WorkspacePaths,
    db: &LakeDb,
    target_ciks: &HashSet<String>,
    cik_to_ticker: &HashMap<String, String>,
    env: &CatchupEnv,
    errors: &mut Vec<String>,
) -> Result<Option<usize>> {
    let Some(zip_path) = resolve_submissions_zip_for_bootstrap(paths) else {
        push_error_sample(
            errors,
            "submissions.zip not found under data/munger/lake/raw/sec/; skipping bootstrap"
                .to_owned(),
        );
        return Ok(None);
    };
    let _read_pin = paths.acquire_sec_generation_read_pin(&zip_path);
    let file = File::open(&zip_path)
        .with_context(|| format!("open submissions.zip at {}", zip_path.display()))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("read zip archive {}", zip_path.display()))?;

    let mut matched_indices: Vec<(usize, String)> = Vec::new();
    for idx in 0..archive.len() {
        let Some(Ok(name)) = archive.name_for_index(idx) else {
            continue;
        };
        if let Some(cik) = match_cik_json_entry(&name)
            && target_ciks.contains(&cik)
        {
            matched_indices.push((idx, cik));
        }
    }

    let mut filings_batch: Vec<LakeFilingRow> = Vec::new();
    let mut events_batch: Vec<LakeEventRow> = Vec::new();

    for (idx, cik) in matched_indices {
        let ticker = cik_to_ticker
            .get(&cik)
            .cloned()
            .unwrap_or_else(|| cik.clone());
        let mut entry = match archive.by_index(idx) {
            Ok(e) => e,
            Err(err) => {
                push_error_sample(errors, format!("zip entry {cik}: {err}"));
                continue;
            }
        };
        if entry.size() > MAX_SUBMISSION_ENTRY_BYTES {
            push_error_sample(
                errors,
                format!("zip entry {cik} exceeds {MAX_SUBMISSION_ENTRY_BYTES} bytes; skipped"),
            );
            continue;
        }
        let raw_bytes = match read_bounded_reader(&mut entry, MAX_SUBMISSION_ENTRY_BYTES as usize) {
            Ok(b) => b,
            Err(err) => {
                push_error_sample(errors, format!("read zip entry {cik}: {err}"));
                continue;
            }
        };
        match parse_submissions_json_bytes(&raw_bytes, &cik, &ticker, env) {
            Ok((mut f, mut e)) => {
                filings_batch.append(&mut f);
                events_batch.append(&mut e);
            }
            Err(err) => push_error_sample(errors, format!("parse {cik} submission: {err:#}")),
        }
    }

    let ingested = filings_batch.len();
    if !filings_batch.is_empty() {
        db.upsert_filings(&filings_batch)?;
    }
    if !events_batch.is_empty() {
        db.upsert_events(&events_batch)?;
    }
    db.export_filings_parquet(paths)?;

    db.write_watermark(&SyncWatermark {
        source_key: WM_L1_BOOTSTRAP.to_owned(),
        cursor: Some(zip_path.display().to_string()),
        etag: None,
        last_modified: None,
        synced_at_ms: env.now_ms,
    })?;

    Ok(Some(ingested))
}

fn incremental_sec_catchup(
    db: &LakeDb,
    client: &Client,
    candidates: &[UniverseRow],
    max_ciks: usize,
    env: &CatchupEnv,
    last_sec_req: &mut Option<Instant>,
    errors: &mut Vec<String>,
) -> Result<(usize, usize)> {
    let wm = db.read_watermark(WM_L1_CATCHUP)?;
    let prev_cursor = wm
        .as_ref()
        .and_then(|w| w.cursor.as_deref())
        .and_then(|c| c.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let (selected, next_cursor) =
        select_watchlist_first_rotation(candidates, prev_cursor, max_ciks);
    if selected.is_empty() {
        return Ok((0, 0));
    }

    let mut checked = 0usize;
    let mut new_filings: Vec<LakeFilingRow> = Vec::new();
    let mut new_events: Vec<LakeEventRow> = Vec::new();
    let mut rate_limited = false;

    for row in selected {
        let Ok(norm_cik) = edgar::normalize_cik(&row.cik) else {
            continue;
        };
        let url = format!(
            "{}/CIK{norm_cik}.json",
            env.sec_submissions_base_url.trim_end_matches('/')
        );
        enforce_rate_limit(last_sec_req, env.sec_rate_limit_ms);

        let resp = match client.get(&url).send() {
            Ok(r) => r,
            Err(err) => {
                push_error_sample(
                    errors,
                    format!("SEC fetch {norm_cik} ({}): {err}", row.ticker),
                );
                continue;
            }
        };
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::FORBIDDEN {
            push_error_sample(
                errors,
                format!(
                    "SEC HTTP {status} for {} ({norm_cik}); halting pass",
                    row.ticker
                ),
            );
            rate_limited = true;
            break;
        }
        if !status.is_success() {
            push_error_sample(
                errors,
                format!("SEC HTTP {status} for {} ({norm_cik})", row.ticker),
            );
            continue;
        }
        let body = match resp.bytes() {
            Ok(b) => b,
            Err(err) => {
                push_error_sample(
                    errors,
                    format!("SEC body {} ({norm_cik}): {err}", row.ticker),
                );
                continue;
            }
        };
        checked += 1;

        let (parsed_filings, parsed_events) =
            match parse_submissions_json_bytes(&body, &norm_cik, &row.ticker, env) {
                Ok(v) => v,
                Err(err) => {
                    push_error_sample(
                        errors,
                        format!("parse SEC submission {} ({norm_cik}): {err:#}", row.ticker),
                    );
                    continue;
                }
            };

        let existing_accs: HashSet<String> = db
            .list_filings_for_cik(&norm_cik, 500)?
            .into_iter()
            .map(|f| f.accession_no)
            .collect();

        let mut added_accs: HashSet<String> = HashSet::new();
        for filing in parsed_filings {
            if !existing_accs.contains(&filing.accession_no) {
                added_accs.insert(filing.accession_no.clone());
                new_filings.push(filing);
            }
        }
        for ev in parsed_events {
            let acc = ev.event_id.strip_prefix("sec:").unwrap_or("");
            if added_accs.contains(acc) {
                new_events.push(ev);
            }
        }
    }

    let ingested = new_filings.len();
    if !new_filings.is_empty() {
        db.upsert_filings(&new_filings)?;
    }
    if !new_events.is_empty() {
        db.upsert_events(&new_events)?;
    }

    if checked > 0 || !rate_limited {
        db.write_watermark(&SyncWatermark {
            source_key: WM_L1_CATCHUP.to_owned(),
            cursor: Some(next_cursor.to_string()),
            etag: None,
            last_modified: None,
            synced_at_ms: env.now_ms,
        })?;
    }

    Ok((checked, ingested))
}

/// Selects watchlist entries first, then fills remaining slots with a rotating cursor
/// over non-watchlist candidates. If watchlist exceeds `limit`, rotates across all candidates.
fn select_watchlist_first_rotation(
    candidates: &[UniverseRow],
    cursor: usize,
    limit: usize,
) -> (Vec<&UniverseRow>, usize) {
    if candidates.is_empty() || limit == 0 {
        return (Vec::new(), cursor);
    }
    let watchlist: Vec<&UniverseRow> = candidates.iter().filter(|r| r.in_watchlist).collect();
    let pool: Vec<&UniverseRow> = candidates.iter().filter(|r| !r.in_watchlist).collect();

    if watchlist.len() <= limit && !pool.is_empty() {
        let mut selected = watchlist;
        let rem = limit - selected.len();
        let take_n = rem.min(pool.len());
        let start = cursor % pool.len();
        for offset in 0..take_n {
            selected.push(pool[(start + offset) % pool.len()]);
        }
        let next_cursor = (start + take_n) % pool.len();
        (selected, next_cursor)
    } else if watchlist.len() <= limit {
        (watchlist, 0)
    } else {
        let take_n = limit.min(candidates.len());
        let start = cursor % candidates.len();
        let mut selected = Vec::with_capacity(take_n);
        for offset in 0..take_n {
            selected.push(&candidates[(start + offset) % candidates.len()]);
        }
        let next_cursor = (start + take_n) % candidates.len();
        (selected, next_cursor)
    }
}

fn normalize_target_form(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "10-K" => Some("10-K"),
        "10-K/A" => Some("10-K/A"),
        "10-Q" => Some("10-Q"),
        "10-Q/A" => Some("10-Q/A"),
        "8-K" => Some("8-K"),
        "8-K/A" => Some("8-K/A"),
        "4" => Some("4"),
        "4/A" => Some("4/A"),
        _ => None,
    }
}

fn form_lookback_days(form: &str) -> i64 {
    match form {
        "10-K" | "10-K/A" | "10-Q" | "10-Q/A" => LOOKBACK_DAYS_10KQ,
        "8-K" | "8-K/A" | "4" | "4/A" => LOOKBACK_DAYS_8K_FORM4,
        _ => 0,
    }
}

fn parse_acceptance_datetime_ms(raw: Option<&str>, filing_date: &str, fallback_ms: u64) -> u64 {
    if let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty())
        && let Ok(dt) = DateTime::parse_from_rfc3339(s)
        && let Ok(ms) = u64::try_from(dt.timestamp_millis())
    {
        return ms;
    }
    if let Ok(days) = edgar::parse_date_days(filing_date)
        && let Ok(d) = u64::try_from(days)
    {
        return d.saturating_mul(86_400_000).saturating_add(16 * 3_600_000);
    }
    fallback_ms
}

fn build_sec_filing_url(
    base_archives_url: &str,
    norm_cik: &str,
    accession_no: &str,
    primary_doc: Option<&str>,
) -> String {
    let trimmed_cik = norm_cik.trim_start_matches('0');
    let cik_seg = if trimmed_cik.is_empty() {
        "0"
    } else {
        trimmed_cik
    };
    let acc_dash = accession_no.replace('-', "");
    let base = base_archives_url.trim_end_matches('/');
    match primary_doc.map(str::trim).filter(|s| !s.is_empty()) {
        Some(doc) => format!("{base}/{cik_seg}/{acc_dash}/{doc}"),
        None => format!("{base}/{cik_seg}/{acc_dash}/{accession_no}-index.htm"),
    }
}

fn parse_submissions_json_bytes(
    bytes: &[u8],
    cik: &str,
    ticker: &str,
    env: &CatchupEnv,
) -> Result<(Vec<LakeFilingRow>, Vec<LakeEventRow>)> {
    let norm_cik = edgar::normalize_cik(cik)?;
    let doc: SecSubmissionDoc =
        serde_json::from_slice(bytes).context("decode SEC submissions JSON")?;
    let entity_name = doc
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(ticker)
        .to_owned();

    let Some(recent) = doc.filings.and_then(|f| f.recent) else {
        return Ok((Vec::new(), Vec::new()));
    };

    let today_iso = market::civil_iso_from_unix_secs((env.now_ms / 1000) as i64);
    let today_days = edgar::parse_date_days(&today_iso)?;

    let count = recent.accession_number.len();
    let mut filings_out = Vec::new();
    let mut events_out = Vec::new();

    for idx in 0..count {
        let Some(acc) = recent
            .accession_number
            .get(idx)
            .and_then(|o| o.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(raw_form) = recent.form.get(idx).and_then(|o| o.as_deref()) else {
            continue;
        };
        let Some(form) = normalize_target_form(raw_form) else {
            continue;
        };
        let Some(filing_date) = recent
            .filing_date
            .get(idx)
            .and_then(|o| o.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Ok(filing_days) = edgar::parse_date_days(filing_date) else {
            continue;
        };
        let age_days = today_days - filing_days;
        let max_lookback = form_lookback_days(form);
        if age_days > max_lookback || age_days < -7 {
            continue;
        }

        let report_date = recent
            .report_date
            .get(idx)
            .and_then(|o| o.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let primary_document = recent
            .primary_document
            .get(idx)
            .and_then(|o| o.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let primary_doc_description = recent
            .primary_doc_description
            .get(idx)
            .and_then(|o| o.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let items = recent
            .items
            .get(idx)
            .and_then(|o| o.as_ref())
            .map(|f| match f {
                ItemsField::Text(s) => ItemsField::Text(s.clone()).into_vec(),
                ItemsField::List(v) => ItemsField::List(v.clone()).into_vec(),
            })
            .unwrap_or_default();
        let items_json = serde_json::to_string(&items)?;

        let filing_url = build_sec_filing_url(
            &env.sec_archives_data_base_url,
            &norm_cik,
            acc,
            primary_document.as_deref(),
        );
        let acceptance_raw = recent
            .acceptance_date_time
            .get(idx)
            .and_then(|o| o.as_deref());
        let occurred_at_ms = parse_acceptance_datetime_ms(acceptance_raw, filing_date, env.now_ms);

        let title = if items.is_empty() {
            format!("{ticker} {form} · {filing_date}")
        } else {
            format!(
                "{ticker} {form} (Items {}) · {filing_date}",
                items.join(",")
            )
        };
        let summary = match (&primary_doc_description, &report_date) {
            (Some(desc), Some(rep)) => format!("{entity_name} {form} ({desc}, period {rep})"),
            (Some(desc), None) => format!("{entity_name} {form} ({desc})"),
            (None, Some(rep)) => format!("{entity_name} {form} (period {rep})"),
            (None, None) => format!("{entity_name} SEC {form} filing"),
        };
        let metadata_json = serde_json::json!({
            "accession_no": acc,
            "form": form,
            "items": items,
            "report_date": report_date,
            "primary_document": primary_document,
            "entity_name": entity_name,
        })
        .to_string();

        filings_out.push(LakeFilingRow {
            accession_no: acc.to_owned(),
            cik: norm_cik.clone(),
            ticker: ticker.to_owned(),
            form: form.to_owned(),
            filing_date: filing_date.to_owned(),
            report_date,
            primary_document,
            primary_doc_description,
            items_json,
            filing_url: filing_url.clone(),
            ingested_at_ms: env.now_ms,
        });

        events_out.push(LakeEventRow {
            event_id: format!("sec:{acc}"),
            source: "sec_edgar".to_owned(),
            cik: Some(norm_cik.clone()),
            ticker: Some(ticker.to_owned()),
            event_type: form.to_owned(),
            occurred_at_ms,
            filing_date: Some(filing_date.to_owned()),
            title,
            summary,
            url: filing_url,
            metadata_json,
            processed_mask: 0,
        });
    }

    Ok((filings_out, events_out))
}

fn format_opt_decimal(val: Option<Decimal>, suffix: &str) -> String {
    match val {
        Some(v) => format!("{}{suffix}", v.round_dp(2).normalize()),
        None => "N/A".to_owned(),
    }
}

fn build_company_profile_text(row: &UniverseRow) -> String {
    let sic_str = match (row.sic, row.sic_description.as_deref()) {
        (Some(code), Some(desc)) => format!("SIC {code} ({desc})"),
        (Some(code), None) => format!("SIC {code}"),
        (None, Some(desc)) => desc.to_owned(),
        (None, None) => "SIC Unclassified".to_owned(),
    };
    let sector_kind = if row.is_financial {
        "Financial Sector"
    } else {
        "Non-Financial Operating Business"
    };
    format!(
        "{ticker} — {name} | Exchange: {exchange} | {sic_str} | {sector_kind}\n\
         5Y ROIC Avg: {roic} | 5Y Gross Margin Avg: {gm} | SBC/OCF: {sbc} | 3Y Share Change: {shares}\n\
         Strict Owner Earnings: {oe} | Net Debt / OE: {nd_oe} | Data Years: {years} | Latest Filing: {latest}",
        ticker = row.ticker,
        name = row.name,
        exchange = row.exchange,
        roic = format_opt_decimal(row.roic_5y_avg_pct, "%"),
        gm = format_opt_decimal(row.gross_margin_5y_avg_pct, "%"),
        sbc = format_opt_decimal(row.sbc_to_ocf_pct, "%"),
        shares = format_opt_decimal(row.share_change_3y_pct, "%"),
        oe = format_opt_decimal(row.strict_owner_earnings, ""),
        nd_oe = format_opt_decimal(row.net_debt_to_oe, "x"),
        years = row.years_of_data,
        latest = row.latest_filing_end.as_deref().unwrap_or("unknown"),
    )
}

fn index_company_profiles(
    db: &LakeDb,
    candidates: &[UniverseRow],
    embedder: &Embedder,
    errors: &mut Vec<String>,
) -> Result<usize> {
    let existing_hashes = db.existing_chunk_hashes("")?;
    let mut pending: Vec<(UniverseRow, String, String, String)> = Vec::new();

    for row in candidates {
        let Ok(norm_cik) = edgar::normalize_cik(&row.cik) else {
            continue;
        };
        let chunk_id = format!("profile:{norm_cik}");
        let content = build_company_profile_text(row);
        let content_sha = latent::sha256_hex(content.as_bytes());
        if existing_hashes.get(&chunk_id) == Some(&content_sha) {
            continue;
        }
        pending.push((row.clone(), norm_cik, chunk_id, content_sha));
    }

    if pending.is_empty() {
        return Ok(0);
    }

    let mut total_indexed = 0usize;
    for batch in pending.chunks(32) {
        let texts: Vec<String> = batch
            .iter()
            .map(|(r, _, _, _)| build_company_profile_text(r))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let embeddings = match embedder.embed_batch(&text_refs) {
            Ok(e) => e,
            Err(err) => {
                push_error_sample(errors, format!("embed company profiles: {err:#}"));
                continue;
            }
        };

        let mut chunk_rows = Vec::with_capacity(batch.len());
        for (((row, norm_cik, chunk_id, content_sha), content), emb) in
            batch.iter().zip(texts).zip(embeddings)
        {
            chunk_rows.push(DocChunkRow {
                chunk_id: chunk_id.clone(),
                cik: norm_cik.clone(),
                ticker: row.ticker.clone(),
                doc_type: "company_profile".to_owned(),
                source_id: format!("profile:{norm_cik}"),
                period_end: row.latest_filing_end.clone(),
                filing_date: row.latest_filing_end.clone(),
                section: "company_profile".to_owned(),
                title: format!("{} ({}) Company Profile", row.ticker, row.name),
                content,
                content_sha256: content_sha.clone(),
                source_url: format!(
                    "https://www.sec.gov/edgar/browse/?CIK={}",
                    norm_cik.trim_start_matches('0')
                ),
                embedding_model: emb.model,
                embedding_dim: emb.dim,
                embedding_json: latent::encode_embedding_json(&emb.vector),
            });
        }
        total_indexed += db.upsert_doc_chunks(&chunk_rows)?;
    }

    Ok(total_indexed)
}

fn filing_doc_cache_path(paths: &WorkspacePaths, cik: &str, accession_no: &str) -> PathBuf {
    let safe_acc: String = accession_no
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    paths
        .lake_dir()
        .join("raw/filings")
        .join(cik)
        .join(format!("{safe_acc}.txt"))
}

fn normalize_doc_type(form: &str) -> &'static str {
    if form.starts_with("10-K") {
        "10k"
    } else if form.starts_with("8-K") {
        "8k"
    } else {
        "sec_doc"
    }
}

#[allow(clippy::too_many_arguments)]
fn index_candidate_filings_text(
    paths: &WorkspacePaths,
    db: &LakeDb,
    client: &Client,
    candidates: &[UniverseRow],
    max_docs: usize,
    embedder: &Embedder,
    env: &CatchupEnv,
    last_sec_req: &mut Option<Instant>,
    errors: &mut Vec<String>,
) -> Result<(usize, usize, usize)> {
    let existing_hashes = db.existing_chunk_hashes("")?;
    let mut indexed_sources: HashSet<String> = HashSet::new();
    for key in existing_hashes.keys() {
        if let Some(rest) = key.strip_prefix("sec:")
            && let Some((acc, _)) = rest.split_once(':')
        {
            indexed_sources.insert(acc.to_owned());
        }
    }

    let mut work_queue: Vec<LakeFilingRow> = Vec::new();
    let mut queued_accs: HashSet<String> = HashSet::new();
    let mut overflow_queue: Vec<LakeFilingRow> = Vec::new();
    for row in candidates {
        if work_queue.len() >= max_docs {
            break;
        }
        let Ok(norm_cik) = edgar::normalize_cik(&row.cik) else {
            continue;
        };
        let filings = db.list_filings_for_cik(&norm_cik, 20)?;
        let eligible: Vec<LakeFilingRow> = filings
            .into_iter()
            .filter(|f| {
                matches!(f.form.as_str(), "10-K" | "10-K/A" | "8-K" | "8-K/A")
                    && f.primary_document
                        .as_deref()
                        .is_some_and(|d| !d.trim().is_empty())
                    && !indexed_sources.contains(&f.accession_no)
            })
            .collect();
        if let Some(ten_k) = eligible
            .iter()
            .find(|f| f.form.starts_with("10-K"))
            .cloned()
            && queued_accs.insert(ten_k.accession_no.clone())
        {
            work_queue.push(ten_k);
        }
        if work_queue.len() < max_docs
            && let Some(eight_k) = eligible.iter().find(|f| f.form.starts_with("8-K")).cloned()
            && queued_accs.insert(eight_k.accession_no.clone())
        {
            work_queue.push(eight_k);
        }
        for f in eligible {
            if !queued_accs.contains(&f.accession_no) {
                overflow_queue.push(f);
            }
        }
    }
    for f in overflow_queue {
        if work_queue.len() >= max_docs {
            break;
        }
        if queued_accs.insert(f.accession_no.clone()) {
            work_queue.push(f);
        }
    }

    let mut attempted = 0usize;
    let mut docs_indexed = 0usize;
    let mut chunks_indexed = 0usize;

    for filing in work_queue {
        attempted += 1;
        let cache_path = filing_doc_cache_path(paths, &filing.cik, &filing.accession_no);
        let raw_text = if cache_path.is_file() {
            match fs::File::open(&cache_path)
                .map_err(anyhow::Error::from)
                .and_then(|f| read_bounded_reader(f, MAX_PRIMARY_DOC_BYTES))
            {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(err) => {
                    push_error_sample(
                        errors,
                        format!("read cached filing {}: {err}", cache_path.display()),
                    );
                    continue;
                }
            }
        } else {
            let fetch_url = if filing.filing_url.starts_with("http") {
                filing.filing_url.clone()
            } else {
                build_sec_filing_url(
                    &env.sec_archives_data_base_url,
                    &filing.cik,
                    &filing.accession_no,
                    filing.primary_document.as_deref(),
                )
            };
            enforce_rate_limit(last_sec_req, env.sec_rate_limit_ms);
            let resp = match client.get(&fetch_url).send() {
                Ok(r) => r,
                Err(err) => {
                    push_error_sample(
                        errors,
                        format!("fetch doc {} {}: {err}", filing.ticker, filing.accession_no),
                    );
                    continue;
                }
            };
            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::FORBIDDEN {
                push_error_sample(
                    errors,
                    format!(
                        "SEC doc HTTP {status} for {} {}; halting doc fetches",
                        filing.ticker, filing.accession_no
                    ),
                );
                break;
            }
            if !status.is_success() {
                push_error_sample(
                    errors,
                    format!(
                        "SEC doc HTTP {status} for {} {}",
                        filing.ticker, filing.accession_no
                    ),
                );
                continue;
            }
            let bytes = match read_bounded_reader(resp, MAX_PRIMARY_DOC_BYTES) {
                Ok(b) => b,
                Err(err) => {
                    push_error_sample(
                        errors,
                        format!(
                            "read SEC doc body {} {}: {err:#}",
                            filing.ticker, filing.accession_no
                        ),
                    );
                    continue;
                }
            };
            if let Some(parent) = cache_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let tmp = store::atomic_temp_path(&cache_path);
            if fs::write(&tmp, &bytes)
                .and_then(|_| fs::rename(&tmp, &cache_path))
                .is_err()
            {
                let _ = fs::remove_file(&tmp);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        };

        let extracted = latent::extract_sec_sections(&filing.form, &raw_text);
        if extracted.sections.is_empty() {
            continue;
        }

        let doc_type = normalize_doc_type(&filing.form);
        let mut all_chunks = Vec::new();
        for sec in &extracted.sections {
            let mut sec_chunks = latent::chunk_text(
                &sec.section,
                &sec.title,
                &sec.text,
                DEFAULT_CHUNK_CHARS,
                DEFAULT_CHUNK_OVERLAP,
            );
            all_chunks.append(&mut sec_chunks);
            if all_chunks.len() >= MAX_CHUNKS_PER_FILING {
                all_chunks.truncate(MAX_CHUNKS_PER_FILING);
                break;
            }
        }

        let mut to_embed = Vec::new();
        for ch in all_chunks {
            let chunk_id = format!(
                "sec:{}:{}:{}",
                filing.accession_no, ch.section, ch.chunk_index
            );
            if existing_hashes.get(&chunk_id) == Some(&ch.content_sha256) {
                continue;
            }
            to_embed.push((chunk_id, ch));
        }

        if to_embed.is_empty() {
            continue;
        }

        let text_refs: Vec<&str> = to_embed.iter().map(|(_, c)| c.content.as_str()).collect();
        let embeddings = match embedder.embed_batch(&text_refs) {
            Ok(e) => e,
            Err(err) => {
                push_error_sample(
                    errors,
                    format!(
                        "embed filing {} {}: {err:#}",
                        filing.ticker, filing.accession_no
                    ),
                );
                continue;
            }
        };

        let mut rows = Vec::with_capacity(to_embed.len());
        for ((chunk_id, ch), emb) in to_embed.into_iter().zip(embeddings) {
            rows.push(DocChunkRow {
                chunk_id,
                cik: filing.cik.clone(),
                ticker: filing.ticker.clone(),
                doc_type: doc_type.to_owned(),
                source_id: filing.accession_no.clone(),
                period_end: filing.report_date.clone(),
                filing_date: Some(filing.filing_date.clone()),
                section: ch.section,
                title: format!("{} {} · {}", filing.ticker, filing.form, ch.title),
                content: ch.content,
                content_sha256: ch.content_sha256,
                source_url: filing.filing_url.clone(),
                embedding_model: emb.model,
                embedding_dim: emb.dim,
                embedding_json: latent::encode_embedding_json(&emb.vector),
            });
        }

        let inserted = db.upsert_doc_chunks(&rows)?;
        if inserted > 0 {
            docs_indexed += 1;
            chunks_indexed += inserted;
        }
    }

    Ok((attempted, docs_indexed, chunks_indexed))
}

fn ingest_candidate_news(
    db: &LakeDb,
    candidates: &[UniverseRow],
    max_tickers: usize,
    embedder: &Embedder,
    env: &CatchupEnv,
    errors: &mut Vec<String>,
) -> Result<(usize, usize, usize)> {
    let wm = db.read_watermark(WM_L1_NEWS)?;
    let prev_cursor = wm
        .as_ref()
        .and_then(|w| w.cursor.as_deref())
        .and_then(|c| c.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let (selected, next_cursor) =
        select_watchlist_first_rotation(candidates, prev_cursor, max_tickers);
    if selected.is_empty() {
        return Ok((0, 0, 0));
    }

    struct PendingNewsChunk {
        chunk_id: String,
        cik: String,
        ticker: String,
        event_id: String,
        filing_date: String,
        title: String,
        content: String,
        content_sha256: String,
        source_url: String,
    }

    let client = market::market_http_client()?;
    let mut last_req: Option<Instant> = None;
    let mut checked = 0usize;
    let mut new_events: Vec<LakeEventRow> = Vec::new();
    let mut pending_chunks: Vec<PendingNewsChunk> = Vec::new();
    let mut rate_limited = false;
    let today_iso = market::civil_iso_from_unix_secs((env.now_ms / 1000) as i64);

    for row in selected {
        let ticker_upper = row.ticker.trim().to_ascii_uppercase();
        let Ok(norm_cik) = edgar::normalize_cik(&row.cik) else {
            continue;
        };
        let encoded_sym = url_encode_query_value(&ticker_upper);
        let url = if env.yahoo_rss_base_url.contains('?') {
            format!("{}&s={encoded_sym}", env.yahoo_rss_base_url)
        } else {
            format!(
                "{}?s={encoded_sym}&region=US&lang=en-US",
                env.yahoo_rss_base_url.trim_end_matches('/')
            )
        };

        enforce_rate_limit(&mut last_req, env.news_rate_limit_ms);
        let resp = match client.get(&url).send() {
            Ok(r) => r,
            Err(err) => {
                push_error_sample(errors, format!("news RSS {ticker_upper}: {err}"));
                continue;
            }
        };
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::FORBIDDEN {
            push_error_sample(
                errors,
                format!("news RSS HTTP {status} for {ticker_upper}; degrading gracefully"),
            );
            rate_limited = true;
            break;
        }
        if !status.is_success() {
            push_error_sample(errors, format!("news RSS HTTP {status} for {ticker_upper}"));
            continue;
        }
        let body = match read_bounded_reader(resp, MAX_RSS_BYTES) {
            Ok(b) => b,
            Err(err) => {
                push_error_sample(errors, format!("read news RSS {ticker_upper}: {err:#}"));
                continue;
            }
        };
        checked += 1;

        let xml_str = String::from_utf8_lossy(&body);
        let items = parse_rss_items(&xml_str);
        if items.is_empty() {
            continue;
        }

        let mut candidate_ids: Vec<String> = Vec::with_capacity(items.len());
        let mut prepared = Vec::with_capacity(items.len());
        for item in items {
            let dedup_key = item
                .guid
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| Some(item.link.trim()).filter(|s| !s.is_empty()))
                .unwrap_or(item.title.trim());
            let hash = latent::sha256_hex(format!("{ticker_upper}|{dedup_key}").as_bytes());
            let event_id = format!("news:{ticker_upper}:{}", &hash[..24]);
            candidate_ids.push(event_id.clone());
            prepared.push((event_id, hash, item));
        }

        let existing = db.existing_event_ids(&candidate_ids)?;
        let existing_hashes = db.existing_chunk_hashes(&norm_cik)?;

        let mut seen_in_batch = HashSet::new();
        for (event_id, hash, item) in prepared {
            if existing.contains(&event_id) || !seen_in_batch.insert(event_id.clone()) {
                continue;
            }
            let (parsed_ms, parsed_date) = item
                .pub_date
                .as_deref()
                .map(parse_rss_pub_date)
                .unwrap_or((None, None));
            let occurred_at_ms = parsed_ms.unwrap_or(env.now_ms);
            let filing_date = parsed_date.unwrap_or_else(|| today_iso.clone());
            let summary = if item.description.is_empty() {
                item.title.clone()
            } else {
                item.description.clone()
            };
            let chunk_content = if item.description.is_empty() {
                item.title.clone()
            } else {
                format!("{}\n\n{}", item.title, item.description)
            };
            let content_sha = latent::sha256_hex(chunk_content.as_bytes());
            let chunk_id = format!("news:{norm_cik}:{}", &hash[..24]);

            new_events.push(LakeEventRow {
                event_id: event_id.clone(),
                source: "yahoo_rss".to_owned(),
                cik: Some(norm_cik.clone()),
                ticker: Some(ticker_upper.clone()),
                event_type: "news".to_owned(),
                occurred_at_ms,
                filing_date: Some(filing_date.clone()),
                title: item.title.clone(),
                summary,
                url: item.link.clone(),
                metadata_json: serde_json::json!({
                    "guid": item.guid,
                    "pub_date": item.pub_date,
                })
                .to_string(),
                processed_mask: 0,
            });

            if existing_hashes.get(&chunk_id) != Some(&content_sha) {
                pending_chunks.push(PendingNewsChunk {
                    chunk_id,
                    cik: norm_cik.clone(),
                    ticker: ticker_upper.clone(),
                    event_id,
                    filing_date,
                    title: item.title,
                    content: chunk_content,
                    content_sha256: content_sha,
                    source_url: if item.link.is_empty() {
                        url.clone()
                    } else {
                        item.link
                    },
                });
            }
        }
    }

    let news_ingested = new_events.len();
    if !new_events.is_empty() {
        db.upsert_events(&new_events)?;
    }

    let mut chunks_added = 0usize;
    if !pending_chunks.is_empty() {
        for batch in pending_chunks.chunks(32) {
            let text_refs: Vec<&str> = batch.iter().map(|c| c.content.as_str()).collect();
            let embeddings = match embedder.embed_batch(&text_refs) {
                Ok(e) => e,
                Err(err) => {
                    push_error_sample(errors, format!("embed news chunks: {err:#}"));
                    continue;
                }
            };
            let mut rows = Vec::with_capacity(batch.len());
            for (item, emb) in batch.iter().zip(embeddings) {
                rows.push(DocChunkRow {
                    chunk_id: item.chunk_id.clone(),
                    cik: item.cik.clone(),
                    ticker: item.ticker.clone(),
                    doc_type: "news".to_owned(),
                    source_id: item.event_id.clone(),
                    period_end: None,
                    filing_date: Some(item.filing_date.clone()),
                    section: "news".to_owned(),
                    title: item.title.clone(),
                    content: item.content.clone(),
                    content_sha256: item.content_sha256.clone(),
                    source_url: item.source_url.clone(),
                    embedding_model: emb.model,
                    embedding_dim: emb.dim,
                    embedding_json: latent::encode_embedding_json(&emb.vector),
                });
            }
            chunks_added += db.upsert_doc_chunks(&rows)?;
        }
    }

    if checked > 0 || !rate_limited {
        db.write_watermark(&SyncWatermark {
            source_key: WM_L1_NEWS.to_owned(),
            cursor: Some(next_cursor.to_string()),
            etag: None,
            last_modified: None,
            synced_at_ms: env.now_ms,
        })?;
    }

    Ok((checked, news_ingested, chunks_added))
}

fn url_encode_query_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            use std::fmt::Write;
            let _ = write!(&mut out, "%{b:02X}");
        }
    }
    out
}

pub(crate) fn parse_rss_items(xml: &str) -> Vec<RssNewsItem> {
    let mut items = Vec::new();
    let lower = xml.to_ascii_lowercase();
    let mut search_from = 0usize;

    while let Some(rel_start) = find_open_tag_in_lower(&lower[search_from..], "item") {
        let tag_start = search_from + rel_start;
        let Some(tag_end_rel) = lower[tag_start..].find('>') else {
            break;
        };
        let body_start = tag_start + tag_end_rel + 1;
        let Some(close_rel) = lower[body_start..].find("</item>") else {
            break;
        };
        let body_end = body_start + close_rel;
        let block = &xml[body_start..body_end];
        let block_lower = &lower[body_start..body_end];

        let title = extract_xml_tag_text(block, block_lower, "title")
            .map(clean_rss_title)
            .unwrap_or_default();
        let link = extract_xml_tag_text(block, block_lower, "link")
            .map(|s| decode_xml_entities(strip_cdata(s.trim())).trim().to_owned())
            .unwrap_or_default();
        let guid = extract_xml_tag_text(block, block_lower, "guid")
            .map(|s| decode_xml_entities(strip_cdata(s.trim())).trim().to_owned())
            .filter(|s| !s.is_empty());
        let pub_date = extract_xml_tag_text(block, block_lower, "pubdate")
            .map(|s| decode_xml_entities(strip_cdata(s.trim())).trim().to_owned())
            .filter(|s| !s.is_empty());
        let description = extract_xml_tag_text(block, block_lower, "description")
            .map(clean_rss_text)
            .unwrap_or_default();

        if !title.is_empty() || !link.is_empty() {
            items.push(RssNewsItem {
                title,
                link,
                guid,
                pub_date,
                description,
            });
        }

        search_from = body_end + "</item>".len();
    }

    items
}

fn find_open_tag_in_lower(haystack_lower: &str, tag_lower: &str) -> Option<usize> {
    let needle = format!("<{tag_lower}");
    let mut offset = 0usize;
    while let Some(pos) = haystack_lower[offset..].find(&needle) {
        let abs = offset + pos;
        let next_idx = abs + needle.len();
        let next_byte = haystack_lower.as_bytes().get(next_idx).copied();
        if matches!(
            next_byte,
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')
        ) {
            return Some(abs);
        }
        offset = abs + 1;
    }
    None
}

fn extract_xml_tag_text<'a>(block: &'a str, block_lower: &str, tag_lower: &str) -> Option<&'a str> {
    let start = find_open_tag_in_lower(block_lower, tag_lower)?;
    let open_gt = block_lower[start..].find('>')?;
    let content_start = start + open_gt + 1;
    let close_needle = format!("</{tag_lower}>");
    let close_rel = block_lower[content_start..].find(&close_needle)?;
    Some(&block[content_start..content_start + close_rel])
}

fn strip_cdata(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("<![CDATA[")
        && let Some(inner) = rest.strip_suffix("]]>")
    {
        return inner;
    }
    trimmed
}

fn clean_rss_title(raw: &str) -> String {
    let inner = strip_cdata(raw);
    decode_xml_entities(inner)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn clean_rss_text(raw: &str) -> String {
    let inner = strip_cdata(raw);
    let decoded = decode_xml_entities(inner);
    let plain = if decoded.contains('<') && decoded.contains('>') {
        latent::strip_html_to_text(&decoded)
    } else {
        decoded
    };
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_xml_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        if let Some(semi) = after[..after.len().min(12)].find(';') {
            let entity = &after[1..semi];
            let replacement = match entity {
                "amp" => Some("&".to_owned()),
                "lt" => Some("<".to_owned()),
                "gt" => Some(">".to_owned()),
                "quot" => Some("\"".to_owned()),
                "apos" | "#39" => Some("'".to_owned()),
                "nbsp" | "#160" => Some(" ".to_owned()),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    u32::from_str_radix(&entity[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(|c| c.to_string())
                }
                _ if entity.starts_with('#') => entity[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string()),
                _ => None,
            };
            if let Some(rep) = replacement {
                out.push_str(&rep);
                rest = &after[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn parse_rss_pub_date(raw: &str) -> (Option<u64>, Option<String>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
        let ms = u64::try_from(dt.timestamp_millis()).ok();
        let date = market::civil_iso_from_unix_secs(dt.timestamp());
        return (ms, Some(date));
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        let ms = u64::try_from(dt.timestamp_millis()).ok();
        let date = market::civil_iso_from_unix_secs(dt.timestamp());
        return (ms, Some(date));
    }
    (None, None)
}

fn generate_signals_and_ripples(
    db: &LakeDb,
    candidates: &[UniverseRow],
    embedder: &Embedder,
    env: &CatchupEnv,
    errors: &mut Vec<String>,
) -> Result<(usize, usize)> {
    let mut signals: Vec<SignalInboxRow> = Vec::new();
    let candidate_by_ticker: HashMap<String, &UniverseRow> = candidates
        .iter()
        .map(|r| (r.ticker.to_ascii_uppercase(), r))
        .collect();

    // 1. Strike signals (candidates in or near strike zone without Red warnings or TooHard/Out)
    let mut strike_rows: Vec<&UniverseRow> = candidates
        .iter()
        .filter(|row| {
            !row.is_financial
                && row.bucket_opt != Some(Bucket::TooHard)
                && row.bucket_opt != Some(Bucket::Out)
                && row.latest_price.is_some()
                && row.strike_price.is_some()
                && !has_red_warning(row)
                && row
                    .distance_to_strike_pct
                    .is_some_and(|d| d <= Decimal::from(5))
        })
        .collect();
    strike_rows.sort_by(|a, b| {
        b.in_watchlist
            .cmp(&a.in_watchlist)
            .then_with(|| a.distance_to_strike_pct.cmp(&b.distance_to_strike_pct))
            .then_with(|| b.quality_score.cmp(&a.quality_score))
    });
    for row in strike_rows.into_iter().take(MAX_STRIKE_SIGNALS) {
        let dist = row.distance_to_strike_pct.unwrap_or(Decimal::ZERO);
        let dist_f = decimal_to_f64(dist);
        let q_f = decimal_to_f64(row.quality_score);
        let score = if dist <= Decimal::ZERO {
            (86.0 + (-dist_f).clamp(0.0, 10.0) + (q_f / 100.0) * 4.0).clamp(60.0, 99.5)
        } else {
            (74.0 + (5.0 - dist_f).clamp(0.0, 5.0) * 2.0 + (q_f / 100.0) * 3.0).clamp(55.0, 88.0)
        };
        let evidence = best_evidence_for_ticker(db, row);
        let zone_label = if dist <= Decimal::ZERO {
            "进入击球区"
        } else {
            "接近击球区"
        };
        signals.push(SignalInboxRow {
            signal_id: format!("strike:{}", row.ticker),
            kind: SignalKind::Strike,
            score: round_score(score),
            primary_ticker: row.ticker.clone(),
            tickers: vec![row.ticker.clone()],
            title: format!(
                "{} {zone_label} · 距击球价 {}%",
                row.ticker,
                dist.round_dp(2).normalize()
            ),
            summary: format!(
                "{} ({}) 现价 {} vs 击球价 {}，5Y ROIC {}%，严苛 FCF 收益率 {}%，质量分 {}。",
                row.name,
                row.ticker,
                format_opt_decimal(row.latest_price, ""),
                format_opt_decimal(row.strike_price, ""),
                format_opt_decimal(row.roic_5y_avg_pct, ""),
                format_opt_decimal(row.strict_fcf_yield_pct, ""),
                row.quality_score.round_dp(1).normalize(),
            ),
            evidence,
            status: SignalStatus::Unread,
            event_id: None,
            created_at_ms: env.now_ms,
            updated_at_ms: env.now_ms,
        });
    }

    // 2. Discovery signals (non-watchlist quality companies)
    let mut discovery_rows: Vec<&UniverseRow> = candidates
        .iter()
        .filter(|row| {
            !row.in_watchlist
                && !row.is_financial
                && row.years_of_data >= 5
                && !has_red_warning(row)
                && row.roic_5y_avg_pct.is_some_and(|v| v >= Decimal::from(15))
                && row.strict_owner_earnings.is_some_and(|v| v > Decimal::ZERO)
                && row.sbc_to_ocf_pct.is_some_and(|v| v <= Decimal::from(25))
        })
        .collect();
    discovery_rows.sort_by(|a, b| {
        b.quality_score
            .cmp(&a.quality_score)
            .then_with(|| b.roic_5y_avg_pct.cmp(&a.roic_5y_avg_pct))
            .then_with(|| a.ticker.cmp(&b.ticker))
    });
    for row in discovery_rows.into_iter().take(MAX_DISCOVERY_SIGNALS) {
        let q_f = decimal_to_f64(row.quality_score);
        let roic_f = decimal_to_f64(row.roic_5y_avg_pct.unwrap_or(Decimal::ZERO));
        let fcf_f = decimal_to_f64(row.strict_fcf_yield_pct.unwrap_or(Decimal::ZERO));
        let score = (55.0
            + (q_f.clamp(0.0, 100.0) / 100.0) * 25.0
            + (roic_f.clamp(15.0, 60.0) - 15.0) * 0.25
            + fcf_f.clamp(0.0, 12.0) * 0.5)
            .clamp(55.0, 96.0);
        let evidence = best_evidence_for_ticker(db, row);
        signals.push(SignalInboxRow {
            signal_id: format!("discovery:{}", row.ticker),
            kind: SignalKind::Discovery,
            score: round_score(score),
            primary_ticker: row.ticker.clone(),
            tickers: vec![row.ticker.clone()],
            title: format!(
                "{} 高质量候选 · 5Y ROIC {}%",
                row.ticker,
                row.roic_5y_avg_pct
                    .unwrap_or(Decimal::ZERO)
                    .round_dp(1)
                    .normalize()
            ),
            summary: format!(
                "{} ({}) · 连续 {} 年数据，5Y ROIC {}%，SBC/OCF {}%，3Y 股本变化 {}%，严苛股东盈余 {}。",
                row.name,
                row.ticker,
                row.years_of_data,
                format_opt_decimal(row.roic_5y_avg_pct, ""),
                format_opt_decimal(row.sbc_to_ocf_pct, ""),
                format_opt_decimal(row.share_change_3y_pct, ""),
                format_opt_decimal(row.strict_owner_earnings, ""),
            ),
            evidence,
            status: SignalStatus::Unread,
            event_id: None,
            created_at_ms: env.now_ms,
            updated_at_ms: env.now_ms,
        });
    }

    // 3. Red-flag signals from high-severity financial warnings
    let mut warned_rows: Vec<&UniverseRow> = candidates
        .iter()
        .filter(|row| has_red_warning(row))
        .collect();
    warned_rows.sort_by(|a, b| {
        b.in_watchlist
            .cmp(&a.in_watchlist)
            .then_with(|| b.warnings_count.cmp(&a.warnings_count))
            .then_with(|| b.quality_score.cmp(&a.quality_score))
    });
    for row in warned_rows.into_iter().take(MAX_WARNING_RED_FLAGS) {
        let warnings = row.parsed_warnings();
        let red_titles: Vec<String> = warnings
            .iter()
            .filter(|w| w.severity == WarningSeverity::Red)
            .map(|w| format!("{} ({})", w.title, w.detail))
            .collect();
        if red_titles.is_empty() {
            continue;
        }
        let mut evidence = best_evidence_for_ticker(db, row);
        if evidence.is_empty() {
            evidence.push(SignalEvidence {
                chunk_id: None,
                ticker: row.ticker.clone(),
                doc_type: "xbrl_warning".to_owned(),
                section: "warnings".to_owned(),
                filing_date: row.latest_filing_end.clone(),
                title: format!("{} 财务排雷预警", row.ticker),
                quote: red_titles.join("；"),
                source_url: format!(
                    "https://www.sec.gov/edgar/browse/?CIK={}",
                    row.cik.trim_start_matches('0')
                ),
                similarity: 1.0,
            });
        }
        let score = if row.in_watchlist { 91.0 } else { 80.0 };
        signals.push(SignalInboxRow {
            signal_id: format!("red_flag:warn:{}", row.ticker),
            kind: SignalKind::RedFlag,
            score,
            primary_ticker: row.ticker.clone(),
            tickers: vec![row.ticker.clone()],
            title: format!("{} 财报高危预警 ({}项)", row.ticker, red_titles.len()),
            summary: red_titles.join("；"),
            evidence,
            status: SignalStatus::Unread,
            event_id: None,
            created_at_ms: env.now_ms,
            updated_at_ms: env.now_ms,
        });
    }

    // 4. Process unprocessed events -> 8-K/Form 4 red flags + cross-company Ripples
    let unprocessed = db.list_events(None, None, true, 200)?;
    let mut form4_by_ticker: HashMap<String, Vec<LakeEventRow>> = HashMap::new();
    let mut ripple_emitted = 0usize;

    for ev in &unprocessed {
        let Some(ticker) = ev
            .ticker
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            db.mark_event_processed(&ev.event_id, 1)?;
            continue;
        };
        let ticker_upper = ticker.to_ascii_uppercase();

        if matches!(ev.event_type.as_str(), "4" | "4/A") {
            form4_by_ticker
                .entry(ticker_upper.clone())
                .or_default()
                .push(ev.clone());
            if ev.event_type == "4/A"
                && candidate_by_ticker
                    .get(&ticker_upper)
                    .is_some_and(|r| r.in_watchlist)
            {
                signals.push(SignalInboxRow {
                    signal_id: format!("red_flag:form4a:{}", ev.event_id),
                    kind: SignalKind::RedFlag,
                    score: 76.0,
                    primary_ticker: ticker_upper.clone(),
                    tickers: vec![ticker_upper],
                    title: format!(
                        "{} 内部人 Form 4/A 更正申报",
                        ev.ticker.as_deref().unwrap_or("")
                    ),
                    summary: ev.summary.clone(),
                    evidence: vec![SignalEvidence {
                        chunk_id: None,
                        ticker: ev.ticker.clone().unwrap_or_default(),
                        doc_type: "form_4".to_owned(),
                        section: "insider_amendment".to_owned(),
                        filing_date: ev.filing_date.clone(),
                        title: ev.title.clone(),
                        quote: ev.summary.clone(),
                        source_url: ev.url.clone(),
                        similarity: 1.0,
                    }],
                    status: SignalStatus::Unread,
                    event_id: Some(ev.event_id.clone()),
                    created_at_ms: env.now_ms,
                    updated_at_ms: env.now_ms,
                });
            }
            db.mark_event_processed(&ev.event_id, 1)?;
            continue;
        }

        // Check 8-K red-flag items (4.01, 4.02, 5.02, 1.03)
        if matches!(ev.event_type.as_str(), "8-K" | "8-K/A") {
            let mut items = extract_items_from_event_metadata(&ev.metadata_json);
            if let Some(acc) = ev.event_id.strip_prefix("sec:") {
                let text_codes = extract_8k_item_codes_from_chunks(db, ev.cik.as_deref(), acc);
                for code in text_codes {
                    if !items.contains(&code) {
                        items.push(code);
                    }
                }
            }
            let flagged: Vec<String> = items
                .iter()
                .filter(|item| matches!(item.as_str(), "4.01" | "4.02" | "5.02" | "1.03"))
                .cloned()
                .collect();
            if !flagged.is_empty() {
                let score = if flagged
                    .iter()
                    .any(|i| matches!(i.as_str(), "4.02" | "1.03" | "4.01"))
                {
                    94.0
                } else {
                    78.0
                };
                let mut evidence = event_evidence_from_chunks(db, ev);
                if evidence.is_empty() {
                    evidence.push(SignalEvidence {
                        chunk_id: None,
                        ticker: ticker_upper.clone(),
                        doc_type: "8k".to_owned(),
                        section: "item_8k".to_owned(),
                        filing_date: ev.filing_date.clone(),
                        title: ev.title.clone(),
                        quote: ev.summary.clone(),
                        source_url: ev.url.clone(),
                        similarity: 1.0,
                    });
                }
                signals.push(SignalInboxRow {
                    signal_id: format!("red_flag:event:{}", ev.event_id),
                    kind: SignalKind::RedFlag,
                    score,
                    primary_ticker: ticker_upper.clone(),
                    tickers: vec![ticker_upper.clone()],
                    title: format!("{} 8-K 高敏感事项 ({})", ticker_upper, flagged.join(", ")),
                    summary: format!("{} · {}", ev.title, ev.summary),
                    evidence,
                    status: SignalStatus::Unread,
                    event_id: Some(ev.event_id.clone()),
                    created_at_ms: env.now_ms,
                    updated_at_ms: env.now_ms,
                });
            }
        }

        // Cross-company ripple generation for 8-K / 10-K / news
        if matches!(
            ev.event_type.as_str(),
            "8-K" | "8-K/A" | "10-K" | "10-K/A" | "news"
        ) {
            let source_text = build_ripple_source_text(db, ev);
            if !source_text.trim().is_empty() {
                let source_vec = match embedder.embed(&source_text) {
                    Ok(e) => e.vector,
                    Err(err) => {
                        push_error_sample(
                            errors,
                            format!("embed ripple source {}: {err:#}", ev.event_id),
                        );
                        latent::local_embed(&source_text)
                    }
                };
                match db.find_ripple_targets(&ticker_upper, &source_text, &source_vec, 0.15, 3) {
                    Ok(targets) => {
                        let source_citation = SignalEvidence {
                            chunk_id: None,
                            ticker: ticker_upper.clone(),
                            doc_type: ev.event_type.to_ascii_lowercase(),
                            section: "source_event".to_owned(),
                            filing_date: ev.filing_date.clone(),
                            title: ev.title.clone(),
                            quote: latent::extract_verbatim_snippet(&source_text, &ev.title, 240),
                            source_url: ev.url.clone(),
                            similarity: 1.0,
                        };
                        for hit in targets {
                            if hit.evidence.is_empty() {
                                continue;
                            }
                            let mut combined_evidence = Vec::with_capacity(1 + hit.evidence.len());
                            combined_evidence.push(source_citation.clone());
                            combined_evidence.extend(hit.evidence.clone());

                            let score = (55.0 + f64::from(hit.score) * 45.0).clamp(50.0, 99.0);
                            signals.push(SignalInboxRow {
                                signal_id: format!("ripple:{}:{}", ev.event_id, hit.company.ticker),
                                kind: SignalKind::Ripple,
                                score: round_score(score),
                                primary_ticker: hit.company.ticker.clone(),
                                tickers: vec![ticker_upper.clone(), hit.company.ticker.clone()],
                                title: format!(
                                    "{} → {} 隐空间传导 ({:.0}%)",
                                    ticker_upper,
                                    hit.company.ticker,
                                    hit.score * 100.0
                                ),
                                summary: format!(
                                    "源事件「{}」在隐空间与 {} ({}) 共振，语义 {:.2} / 词法 {:.2}。",
                                    ev.title,
                                    hit.company.name,
                                    hit.company.ticker,
                                    hit.semantic_score,
                                    hit.lexical_score,
                                ),
                                evidence: combined_evidence,
                                status: SignalStatus::Unread,
                                event_id: Some(ev.event_id.clone()),
                                created_at_ms: env.now_ms,
                                updated_at_ms: env.now_ms,
                            });
                            ripple_emitted += 1;
                        }
                    }
                    Err(err) => push_error_sample(
                        errors,
                        format!("find_ripple_targets {}: {err:#}", ev.event_id),
                    ),
                }
            }
        }

        db.mark_event_processed(&ev.event_id, 1)?;
    }

    // Check Form 4 clusters (>= 3 filings in 14 days) across unprocessed Form 4 events
    for (ticker_upper, evs) in form4_by_ticker {
        let recent_count = count_recent_cluster(&evs, env.now_ms, FORM4_CLUSTER_WINDOW_DAYS);
        if recent_count >= FORM4_CLUSTER_MIN_COUNT
            && let Some(latest) = evs.iter().max_by_key(|e| e.occurred_at_ms)
        {
            let date_tag = latest.filing_date.as_deref().unwrap_or("recent");
            signals.push(SignalInboxRow {
                signal_id: format!("red_flag:form4_cluster:{ticker_upper}:{date_tag}"),
                kind: SignalKind::RedFlag,
                score: 79.0,
                primary_ticker: ticker_upper.clone(),
                tickers: vec![ticker_upper.clone()],
                title: format!("{ticker_upper} 近期 Form 4 密集申报 ({recent_count}笔/14天)"),
                summary: format!(
                    "{ticker_upper} 近 14 天内发生 {recent_count} 次 Form 4 内部人持股变动披露，建议核对交易性质。"
                ),
                evidence: vec![SignalEvidence {
                    chunk_id: None,
                    ticker: ticker_upper,
                    doc_type: "form_4".to_owned(),
                    section: "insider_cluster".to_owned(),
                    filing_date: latest.filing_date.clone(),
                    title: latest.title.clone(),
                    quote: latest.summary.clone(),
                    source_url: latest.url.clone(),
                    similarity: 1.0,
                }],
                status: SignalStatus::Unread,
                event_id: Some(latest.event_id.clone()),
                created_at_ms: env.now_ms,
                updated_at_ms: env.now_ms,
            });
        }
    }

    let count = signals.len();
    if !signals.is_empty() {
        db.upsert_signals(&signals)?;
    }
    Ok((count, ripple_emitted))
}

fn has_red_warning(row: &UniverseRow) -> bool {
    row.parsed_warnings()
        .iter()
        .any(|w| w.severity == WarningSeverity::Red)
}

fn decimal_to_f64(d: Decimal) -> f64 {
    d.to_string().parse::<f64>().unwrap_or(0.0)
}

fn round_score(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn count_recent_cluster(evs: &[LakeEventRow], now_ms: u64, window_days: i64) -> usize {
    let window_ms = u64::try_from(window_days.max(0))
        .unwrap_or(0)
        .saturating_mul(86_400_000);
    let cutoff = now_ms.saturating_sub(window_ms);
    evs.iter().filter(|e| e.occurred_at_ms >= cutoff).count()
}

fn extract_items_from_event_metadata(metadata_json: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Meta {
        #[serde(default)]
        items: Vec<String>,
    }
    serde_json::from_str::<Meta>(metadata_json)
        .map(|m| m.items)
        .unwrap_or_default()
}

fn extract_8k_item_codes_from_chunks(
    db: &LakeDb,
    cik_opt: Option<&str>,
    accession_no: &str,
) -> Vec<String> {
    let Some(cik) = cik_opt else {
        return Vec::new();
    };
    let mut stmt = match db
        .conn
        .prepare("SELECT content FROM doc_chunks WHERE cik = ? AND source_id = ? LIMIT 4")
    {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let Ok(rows) = stmt.query_map([cik, accession_no], |r| r.get::<_, String>(0)) else {
        return Vec::new();
    };
    let mut codes = Vec::new();
    for text in rows.flatten() {
        for c in latent::extract_8k_item_codes(&text) {
            if !codes.contains(&c) {
                codes.push(c);
            }
        }
    }
    codes
}

fn event_evidence_from_chunks(db: &LakeDb, ev: &LakeEventRow) -> Vec<SignalEvidence> {
    let Some(cik) = ev.cik.as_deref() else {
        return Vec::new();
    };
    let source_id = ev.event_id.strip_prefix("sec:").unwrap_or(&ev.event_id);
    let mut stmt = match db.conn.prepare(
        "SELECT chunk_id, ticker, doc_type, section, filing_date, title, content, source_url
         FROM doc_chunks
         WHERE cik = ? AND source_id = ?
         ORDER BY chunk_id ASC
         LIMIT 2",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let Ok(rows) = stmt.query_map([cik, source_id], |r| {
        Ok(SignalEvidence {
            chunk_id: Some(r.get::<_, String>(0)?),
            ticker: r.get::<_, String>(1)?,
            doc_type: r.get::<_, String>(2)?,
            section: r.get::<_, String>(3)?,
            filing_date: r.get::<_, Option<String>>(4)?,
            title: r.get::<_, String>(5)?,
            quote: latent::extract_verbatim_snippet(&r.get::<_, String>(6)?, &ev.title, 260),
            source_url: r.get::<_, String>(7)?,
            similarity: 1.0,
        })
    }) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

fn build_ripple_source_text(db: &LakeDb, ev: &LakeEventRow) -> String {
    let mut parts = Vec::new();
    if !ev.title.trim().is_empty() {
        parts.push(ev.title.trim().to_owned());
    }
    if !ev.summary.trim().is_empty() && ev.summary.trim() != ev.title.trim() {
        parts.push(ev.summary.trim().to_owned());
    }

    if let Some(cik) = ev.cik.as_deref() {
        let source_id = ev.event_id.strip_prefix("sec:").unwrap_or(&ev.event_id);
        if let Ok(mut stmt) = db.conn.prepare(
            "SELECT content FROM doc_chunks
             WHERE cik = ? AND source_id = ?
             ORDER BY chunk_id ASC
             LIMIT 3",
        ) && let Ok(rows) = stmt.query_map([cik, source_id], |r| r.get::<_, String>(0))
        {
            for content in rows.flatten() {
                if !content.trim().is_empty() {
                    parts.push(content.chars().take(1200).collect());
                }
            }
        }
    }

    parts.join("\n\n")
}

fn best_evidence_for_ticker(db: &LakeDb, row: &UniverseRow) -> Vec<SignalEvidence> {
    let Ok(norm_cik) = edgar::normalize_cik(&row.cik) else {
        return Vec::new();
    };
    let mut stmt = match db.conn.prepare(
        "SELECT chunk_id, ticker, doc_type, section, filing_date, title, content, source_url
         FROM doc_chunks
         WHERE cik = ?
         ORDER BY
            CASE section
                WHEN 'item_1' THEN 0
                WHEN 'item_7' THEN 1
                WHEN 'item_8k' THEN 2
                WHEN 'news' THEN 3
                WHEN 'company_profile' THEN 4
                ELSE 5
            END ASC,
            chunk_id ASC
         LIMIT 2",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let Ok(rows) = stmt.query_map([norm_cik.as_str()], |r| {
        Ok(SignalEvidence {
            chunk_id: Some(r.get::<_, String>(0)?),
            ticker: r.get::<_, String>(1)?,
            doc_type: r.get::<_, String>(2)?,
            section: r.get::<_, String>(3)?,
            filing_date: r.get::<_, Option<String>>(4)?,
            title: r.get::<_, String>(5)?,
            quote: latent::extract_verbatim_snippet(&r.get::<_, String>(6)?, &row.name, 260),
            source_url: r.get::<_, String>(7)?,
            similarity: 1.0,
        })
    }) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lake::{LakeCompanyRow, LakePriceDailyRow};
    use crate::model::WarningFlag;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use zip::write::SimpleFileOptions;

    fn temp_paths(label: &str) -> WorkspacePaths {
        let root = std::env::temp_dir().join(format!(
            "m3_events_test_{label}_{}_{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        WorkspacePaths::from_root(root)
    }

    #[allow(clippy::too_many_arguments)]
    fn make_universe_row(
        ticker: &str,
        cik: &str,
        name: &str,
        sic_desc: &str,
        roic: i64,
        sbc_ocf: i64,
        in_watchlist: bool,
        in_price_pool: bool,
        distance_pct: Option<i64>,
        warnings: Vec<WarningFlag>,
    ) -> UniverseRow {
        UniverseRow {
            ticker: ticker.to_owned(),
            cik: cik.to_owned(),
            name: name.to_owned(),
            exchange: "Nasdaq".to_owned(),
            sic: Some(7372),
            sic_description: Some(sic_desc.to_owned()),
            is_financial: false,
            latest_filing_end: Some("2026-09-30".to_owned()),
            years_of_data: 6,
            quality_score: Decimal::from(88),
            in_price_pool,
            in_watchlist,
            bucket_opt: in_watchlist.then_some(Bucket::Watch),
            latest_price: Some(Decimal::from(100)),
            strike_price: Some(Decimal::from(105)),
            distance_to_strike_pct: distance_pct.map(Decimal::from),
            strict_fcf_yield_pct: Some(Decimal::from(6)),
            roic_5y_avg_pct: Some(Decimal::from(roic)),
            roic_5y_stddev_pct: Some(Decimal::from(2)),
            gross_margin_5y_avg_pct: Some(Decimal::from(72)),
            gross_margin_5y_stddev_pct: Some(Decimal::from(1)),
            share_change_3y_pct: Some(Decimal::from(-2)),
            sbc_to_ocf_pct: Some(Decimal::from(sbc_ocf)),
            net_debt_to_oe: Some(Decimal::from(0)),
            strict_owner_earnings: Some(Decimal::from(500_000_000i64)),
            warnings_count: warnings.len(),
            warnings_json: serde_json::to_string(&warnings).unwrap(),
            raw_sha256: Some("sha123".to_owned()),
        }
    }

    fn seed_universe(paths: &WorkspacePaths, rows: Vec<UniverseRow>) {
        let mut db = LakeDb::open(paths).unwrap();
        let companies: Vec<LakeCompanyRow> = rows
            .iter()
            .map(|r| LakeCompanyRow {
                cik: r.cik.clone(),
                ticker: r.ticker.clone(),
                name: r.name.clone(),
                exchange: r.exchange.clone(),
                sic: r.sic,
                sic_description: r.sic_description.clone(),
                is_financial: r.is_financial,
                is_active: true,
            })
            .collect();
        db.replace_sec_lake_data(&companies, &[], &[], &rows)
            .unwrap();
        let prices: Vec<LakePriceDailyRow> = rows
            .iter()
            .map(|r| LakePriceDailyRow {
                symbol: r.ticker.clone(),
                trade_date: "2026-11-23".to_owned(),
                price: Decimal::from(100),
                previous_close: Some(Decimal::from(99)),
                currency: "USD".to_owned(),
                exchange: r.exchange.clone(),
                market_time_ms: Some(1_763_900_000_000),
                fetched_at_ms: 1_763_900_000_100,
                source_url: "https://example.com".to_owned(),
            })
            .collect();
        db.upsert_prices_and_update_snapshot(&prices, &[], &[])
            .unwrap();
    }

    fn build_submissions_zip(path: &Path, entries: &[(&str, &str)]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        for (name, body) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn spawn_http_mock<F>(handler: F) -> (String, std::thread::JoinHandle<()>)
    where
        F: Fn(&str) -> (u16, &'static str, String) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let path = first_line.split_whitespace().nth(1).unwrap_or("/");
                if path == "/__shutdown" {
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                    break;
                }
                let (code, content_type, body) = handler(path);
                let reason = match code {
                    200 => "OK",
                    404 => "Not Found",
                    429 => "Too Many Requests",
                    _ => "Error",
                };
                let resp = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn rss_xml_parser_handles_cdata_entities_and_dates() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Yahoo Finance</title>
    <item>
      <title>Autodesk &amp; PTC expand CAD &lt;Cloud&gt; alliance</title>
      <link>https://finance.yahoo.com/news/adsk-ptc.html</link>
      <guid isPermaLink="false">guid-001</guid>
      <pubDate>Mon, 23 Nov 2026 14:30:00 +0000</pubDate>
      <description><![CDATA[<p>Strong Q3 &quot;subscription&quot; demand across engineering &#38; manufacturing.</p>]]></description>
    </item>
    <item>
      <title>Second Headline</title>
      <link>https://finance.yahoo.com/news/second.html</link>
      <pubDate>Tue, 24 Nov 2026 09:00:00 GMT</pubDate>
      <description>Plain summary</description>
    </item>
  </channel>
</rss>"#;
        let items = parse_rss_items(xml);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Autodesk & PTC expand CAD <Cloud> alliance");
        assert_eq!(
            items[0].link,
            "https://finance.yahoo.com/news/adsk-ptc.html"
        );
        assert_eq!(items[0].guid.as_deref(), Some("guid-001"));
        assert!(items[0].description.contains("Strong Q3 \"subscription\""));
        assert!(items[0].description.contains("engineering & manufacturing"));

        let (ms, date) = parse_rss_pub_date(items[0].pub_date.as_deref().unwrap());
        assert!(ms.is_some());
        assert_eq!(date.as_deref(), Some("2026-11-23"));
        let (ms2, date2) = parse_rss_pub_date(items[1].pub_date.as_deref().unwrap());
        assert!(ms2.is_some());
        assert_eq!(date2.as_deref(), Some("2026-11-24"));
    }

    #[test]
    fn submissions_zip_bootstrap_is_idempotent_and_streams_only_candidates() {
        let paths = temp_paths("bootstrap_idempotent");
        let adsk = make_universe_row(
            "ADSK",
            "0000769397",
            "Autodesk Inc",
            "CAD engineering software",
            28,
            12,
            true,
            true,
            Some(8),
            vec![],
        );
        let ptc = make_universe_row(
            "PTC",
            "0000857005",
            "PTC Inc",
            "Enterprise CAD engineering and manufacturing design subscription software",
            22,
            10,
            false,
            true,
            Some(-4),
            vec![],
        );
        seed_universe(&paths, vec![adsk, ptc]);

        // Place submissions.zip under a CURRENT.json generation
        let gen_id = "gen-00000000000000000001-1795000000000-1";
        let gen_zip = paths
            .lake_raw_sec_generations_dir()
            .join(gen_id)
            .join("submissions.zip");
        let adsk_json = serde_json::json!({
            "name": "AUTODESK, INC.",
            "filings": {
                "recent": {
                    "accessionNumber": ["0000769397-26-000100", "0000769397-26-000101", "0000769397-20-000001"],
                    "filingDate": ["2026-11-10", "2026-11-18", "2020-03-01"],
                    "reportDate": ["2026-10-31", "2026-11-18", "2020-01-31"],
                    "acceptanceDateTime": ["2026-11-10T21:00:00.000Z", "2026-11-18T18:00:00.000Z", "2020-03-01T12:00:00.000Z"],
                    "form": ["10-K", "8-K", "10-K"],
                    "items": ["", "4.02,9.01", ""],
                    "primaryDocument": ["adsk-10k.htm", "adsk-8k.htm", "old.htm"],
                    "primaryDocDescription": ["10-K", "8-K", "10-K"]
                }
            }
        })
        .to_string();
        let non_candidate_corrupt = "{broken json that should never be parsed";
        build_submissions_zip(
            &gen_zip,
            &[
                ("CIK0000769397.json", &adsk_json),
                ("CIK9999999999.json", non_candidate_corrupt),
            ],
        );
        let manifest = serde_json::json!({
            "schema_version": 1,
            "generation_id": gen_id,
            "generation_dir": gen_id,
            "committed_at_ms": 1795000000000u64,
        });
        store::write_json_atomic(&paths.lake_raw_sec_manifest_path(), &manifest).unwrap();

        let (base_url, handle) = spawn_http_mock(|path| {
            if path.contains("adsk-10k.htm") {
                (
                    200,
                    "text/html",
                    "<html><body><h1>Item 1. Business</h1><p>Autodesk develops enterprise CAD engineering and manufacturing design subscription software.</p></body></html>".to_owned(),
                )
            } else if path.contains("adsk-8k.htm") {
                (
                    200,
                    "text/html",
                    "<html><body><p>Item 4.02 Non-Reliance on Previously Issued Financial Statements for engineering subscription contracts.</p></body></html>".to_owned(),
                )
            } else {
                (404, "text/plain", "not found".to_owned())
            }
        });

        let now_ms = 1_795_455_000_000u64; // ~2026-11-24
        let env = CatchupEnv {
            sec_submissions_base_url: format!("{base_url}/submissions"),
            sec_archives_data_base_url: format!("{base_url}/Archives/edgar/data"),
            yahoo_rss_base_url: format!("{base_url}/rss"),
            sec_rate_limit_ms: 0,
            news_rate_limit_ms: 0,
            now_ms,
        };
        let opts = EventCatchupOptions {
            bootstrap: false,
            skip_sec: false,
            skip_news: true,
            skip_docs: false,
            max_ciks: 5,
            max_docs: 4,
            max_news_tickers: 0,
        };

        let rep1 = catchup_events_with_env(&paths, &opts, &env, |_| {}).unwrap();
        assert_eq!(rep1.flagship_tokens_used, 0);
        assert!(rep1.bootstrapped_from_zip);
        assert_eq!(rep1.sec_filings_ingested, 2); // old 2020 10-K filtered by 400-day lookback
        assert_eq!(rep1.docs_indexed, 2);
        assert!(paths.lake_parquet_dir().join("filings.parquet").exists());

        let db = LakeDb::open(&paths).unwrap();
        let inbox = db.list_inbox_signals(None, None, 50).unwrap();
        assert!(
            inbox
                .iter()
                .any(|s| s.kind == SignalKind::Discovery && s.primary_ticker == "PTC")
        );
        assert!(
            inbox
                .iter()
                .any(|s| s.kind == SignalKind::Strike && s.primary_ticker == "PTC")
        );
        assert!(
            inbox
                .iter()
                .any(|s| s.kind == SignalKind::RedFlag && s.primary_ticker == "ADSK")
        );
        assert!(
            inbox
                .iter()
                .any(|s| s.kind == SignalKind::Ripple && !s.evidence.is_empty())
        );

        // Triage one signal and verify offline restart never re-bootstraps, duplicates, or overwrites triage
        db.triage_signal("discovery:PTC", SignalStatus::Watch, now_ms + 10)
            .unwrap();
        drop(db);

        // Remove submissions.zip to prove second run does not touch it
        fs::remove_file(&gen_zip).unwrap();
        let env2 = CatchupEnv {
            now_ms: now_ms + 60_000,
            ..env
        };
        let rep2 = catchup_events_with_env(&paths, &opts, &env2, |_| {}).unwrap();
        assert!(!rep2.bootstrapped_from_zip);
        assert_eq!(rep2.sec_filings_ingested, 0);
        assert_eq!(rep2.flagship_tokens_used, 0);

        let db2 = LakeDb::open(&paths).unwrap();
        let ptc_watch = db2
            .list_inbox_signals(Some(SignalStatus::Watch), Some(SignalKind::Discovery), 10)
            .unwrap();
        assert_eq!(ptc_watch.len(), 1);
        assert_eq!(ptc_watch[0].signal_id, "discovery:PTC");

        let _ = reqwest::blocking::get(format!("{base_url}/__shutdown"));
        let _ = handle.join();
        let _ = fs::remove_dir_all(paths.repo_root);
    }

    #[test]
    fn incremental_sec_and_news_rotate_cursors_and_degrade_on_429() {
        let paths = temp_paths("incremental_rotate_429");
        let c1 = make_universe_row(
            "AAA",
            "0000000001",
            "AAA Co",
            "Software",
            20,
            8,
            true,
            true,
            None,
            vec![],
        );
        let c2 = make_universe_row(
            "BBB",
            "0000000002",
            "BBB Co",
            "Software",
            20,
            8,
            false,
            true,
            None,
            vec![],
        );
        let c3 = make_universe_row(
            "CCC",
            "0000000003",
            "CCC Co",
            "Software",
            20,
            8,
            false,
            true,
            None,
            vec![],
        );
        seed_universe(&paths, vec![c1, c2, c3]);

        let db = LakeDb::open(&paths).unwrap();
        db.write_watermark(&SyncWatermark {
            source_key: WM_L1_BOOTSTRAP.to_owned(),
            cursor: Some("done".to_owned()),
            etag: None,
            last_modified: None,
            synced_at_ms: 1_795_400_000_000,
        })
        .unwrap();
        drop(db);

        let news_calls = Arc::new(AtomicUsize::new(0));
        let news_calls_ref = Arc::clone(&news_calls);
        let (base_url, handle) = spawn_http_mock(move |path| {
            if path.contains("CIK0000000001.json") {
                (
                    200,
                    "application/json",
                    serde_json::json!({
                        "name": "AAA Co",
                        "filings": {
                            "recent": {
                                "accessionNumber": ["0000000001-26-000010"],
                                "filingDate": ["2026-11-22"],
                                "reportDate": ["2026-11-22"],
                                "acceptanceDateTime": ["2026-11-22T20:00:00.000Z"],
                                "form": ["8-K"],
                                "items": ["5.02"],
                                "primaryDocument": ["aaa-8k.htm"],
                                "primaryDocDescription": ["8-K"]
                            }
                        }
                    })
                    .to_string(),
                )
            } else if path.contains("CIK0000000002.json") || path.contains("CIK0000000003.json") {
                (
                    200,
                    "application/json",
                    serde_json::json!({ "name": "Other Co", "filings": { "recent": { "accessionNumber": [] } } }).to_string(),
                )
            } else if path.contains("rss") {
                let n = news_calls_ref.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    (
                        200,
                        "application/xml",
                        r#"<rss version="2.0"><channel><item><title>AAA wins contract</title><link>https://example.com/1</link><pubDate>Mon, 23 Nov 2026 12:00:00 +0000</pubDate><description>Enterprise software win</description></item></channel></rss>"#.to_owned(),
                    )
                } else {
                    (429, "text/plain", "rate limited".to_owned())
                }
            } else {
                (404, "text/plain", "not found".to_owned())
            }
        });

        let env = CatchupEnv {
            sec_submissions_base_url: format!("{base_url}/submissions"),
            sec_archives_data_base_url: format!("{base_url}/Archives/edgar/data"),
            yahoo_rss_base_url: format!("{base_url}/rss"),
            sec_rate_limit_ms: 0,
            news_rate_limit_ms: 0,
            now_ms: 1_795_455_000_000,
        };
        let opts = EventCatchupOptions {
            bootstrap: false,
            skip_sec: false,
            skip_news: false,
            skip_docs: true,
            max_ciks: 2, // watchlist AAA + 1 rotating pool item (BBB)
            max_docs: 0,
            max_news_tickers: 2,
        };

        let rep1 = catchup_events_with_env(&paths, &opts, &env, |_| {}).unwrap();
        assert_eq!(rep1.sec_ciks_checked, 2);
        assert_eq!(rep1.sec_filings_ingested, 1);
        assert_eq!(rep1.news_tickers_checked, 1);
        assert_eq!(rep1.news_events_ingested, 1);
        assert!(rep1.errors_sample.iter().any(|e| e.contains("429")));

        // Run again with news skipped: rotating cursor must now visit AAA + CCC, and zero duplicate filings!
        let opts2 = EventCatchupOptions {
            skip_news: true,
            ..opts
        };
        let rep2 = catchup_events_with_env(&paths, &opts2, &env, |_| {}).unwrap();
        assert_eq!(rep2.sec_ciks_checked, 2);
        assert_eq!(rep2.sec_filings_ingested, 0);

        let _ = reqwest::blocking::get(format!("{base_url}/__shutdown"));
        let _ = handle.join();
        let _ = fs::remove_dir_all(paths.repo_root);
    }

    #[test]
    fn red_flags_cover_high_severity_warnings_and_form4_clusters() {
        let paths = temp_paths("red_flag_cluster");
        let red_warn = WarningFlag {
            code: "NEGATIVE_STRICT_OE".to_owned(),
            severity: WarningSeverity::Red,
            title: "Negative Strict Owner Earnings".to_owned(),
            detail: "OCF - CapEx - SBC < 0".to_owned(),
        };
        let flagged = make_universe_row(
            "FLG",
            "0000000010",
            "Flagged Corp",
            "Software",
            18,
            30,
            true,
            true,
            Some(-10),
            vec![red_warn],
        );
        seed_universe(&paths, vec![flagged]);

        let now_ms = 1_795_455_000_000u64;
        let db = LakeDb::open(&paths).unwrap();
        let form4_events: Vec<LakeEventRow> = (1..=3)
            .map(|i| LakeEventRow {
                event_id: format!("sec:0000000010-26-00000{i}"),
                source: "sec_edgar".to_owned(),
                cik: Some("0000000010".to_owned()),
                ticker: Some("FLG".to_owned()),
                event_type: "4".to_owned(),
                occurred_at_ms: now_ms - i * 86_400_000,
                filing_date: Some(format!("2026-11-2{i}")),
                title: format!("FLG Form 4 #{i}"),
                summary: "Insider Form 4 disclosure".to_owned(),
                url: format!("https://www.sec.gov/Archives/edgar/data/10/f{i}.xml"),
                metadata_json: "{}".to_owned(),
                processed_mask: 0,
            })
            .collect();
        db.upsert_events(&form4_events).unwrap();
        drop(db);

        let env = CatchupEnv {
            sec_rate_limit_ms: 0,
            news_rate_limit_ms: 0,
            now_ms,
            ..CatchupEnv::live(now_ms)
        };
        let opts = EventCatchupOptions {
            bootstrap: false,
            skip_sec: true,
            skip_news: true,
            skip_docs: true,
            max_ciks: 0,
            max_docs: 0,
            max_news_tickers: 0,
        };
        let rep = catchup_events_with_env(&paths, &opts, &env, |_| {}).unwrap();
        assert_eq!(rep.flagship_tokens_used, 0);

        let db2 = LakeDb::open(&paths).unwrap();
        let red_flags = db2
            .list_inbox_signals(Some(SignalStatus::Unread), Some(SignalKind::RedFlag), 20)
            .unwrap();
        assert!(red_flags.iter().any(|s| s.signal_id == "red_flag:warn:FLG"));
        assert!(
            red_flags
                .iter()
                .any(|s| s.signal_id.starts_with("red_flag:form4_cluster:FLG:"))
        );
        // Red-flagged company must not also emit a strike signal.
        let strikes = db2
            .list_inbox_signals(None, Some(SignalKind::Strike), 20)
            .unwrap();
        assert!(strikes.is_empty());

        let _ = fs::remove_dir_all(paths.repo_root);
    }

    #[test]
    fn empty_lake_catchup_succeeds_gracefully() {
        let paths = temp_paths("empty_lake");
        let opts = EventCatchupOptions {
            bootstrap: false,
            skip_sec: true,
            skip_news: true,
            skip_docs: true,
            max_ciks: 5,
            max_docs: 5,
            max_news_tickers: 5,
        };
        let rep = catchup_events_with_progress(&paths, &opts, |_| {}).unwrap();
        assert_eq!(rep.flagship_tokens_used, 0);
        assert_eq!(rep.candidate_pool_count, 0);
        assert_eq!(rep.signals_created_or_updated, 0);
        let _ = fs::remove_dir_all(paths.repo_root);
    }
}
