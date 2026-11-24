use crate::{
    calculator, edgar, market,
    model::{
        Bucket, CompanyEntry, DataSource, DecisionJournal, Dossier, HumanJudgment, JournalAction,
        JournalEntry, Latticework, MungerReport, PriceHistorySnapshot, PriceSnapshot,
        RawCompanyFactsSummary, SyncStatusFile, SyncTickerStatus, ValuationAssumptions,
        WatchlistFile,
    },
};
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn atomic_temp_path(dest: &Path) -> PathBuf {
    let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    dest.with_file_name(format!(".{name}.{pid}.{seq}.tmp"))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn civil_today_iso() -> String {
    let days = (now_ms() / 1000 + 8 * 3600) as i64 / 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let yr = y + i64::from(m <= 2);
    format!("{yr:04}-{m:02}-{d:02}")
}

pub fn resolve_repo_root() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("M3_REPO_ROOT") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            anyhow::ensure!(
                p.join("workbench/watchlist.json").exists(),
                "M3_REPO_ROOT `{trimmed}` 缺少 workbench/watchlist.json"
            );
            return p.canonicalize().context("canonicalize M3_REPO_ROOT");
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            if ancestor.join("workbench/watchlist.json").exists() {
                if let Ok(canon) = ancestor.canonicalize() {
                    return Ok(canon);
                }
                return Ok(ancestor.to_path_buf());
            }
        }
    }
    if let Some(configured) = crate::app_config::read_workspace_root() {
        return Ok(configured);
    }
    let default_ws = crate::app_config::default_workspace_dir()?;
    crate::app_config::ensure_initialized_workspace(&default_ws)
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = atomic_temp_path(path);
    let bytes = serde_json::to_vec_pretty(value)?;
    if let Err(err) = fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("读取失败 {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("解析失败 {}", path.display()))
}

#[derive(Debug, Clone)]
pub struct WorkspacePaths {
    pub repo_root: PathBuf,
    pub workbench_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl WorkspacePaths {
    pub fn from_root(repo_root: PathBuf) -> Self {
        let workbench_dir = repo_root.join("workbench");
        let data_dir = repo_root.join("data/munger");
        Self {
            repo_root,
            workbench_dir,
            data_dir,
        }
    }

    pub fn watchlist_path(&self) -> PathBuf {
        self.workbench_dir.join("watchlist.json")
    }

    pub fn journal_path(&self) -> PathBuf {
        self.workbench_dir.join("journal.json")
    }

    pub fn dossier_path(&self, ticker: &str) -> PathBuf {
        self.workbench_dir
            .join("dossiers")
            .join(format!("{}.json", market::safe_symbol_filename(ticker)))
    }

    pub fn manual_financials_path(&self, ticker: &str) -> PathBuf {
        self.workbench_dir
            .join("manual_financials")
            .join(format!("{}.json", market::safe_symbol_filename(ticker)))
    }

    pub fn sec_tickers_cache(&self) -> PathBuf {
        self.data_dir.join("raw/sec/company_tickers.json")
    }

    pub fn sec_raw_path(&self, cik: &str) -> PathBuf {
        self.data_dir.join("raw/sec").join(format!("CIK{cik}.json"))
    }

    pub fn sec_facts_summary_path(&self, ticker: &str) -> PathBuf {
        self.data_dir.join("raw/sec").join(format!(
            "{}_facts.json",
            market::safe_symbol_filename(ticker)
        ))
    }

    pub fn price_path(&self, symbol: &str) -> PathBuf {
        self.data_dir
            .join("raw/prices")
            .join(format!("{}.json", market::safe_symbol_filename(symbol)))
    }

    pub fn price_history_path(&self, symbol: &str, interval: &str) -> PathBuf {
        self.data_dir.join("raw/prices").join(format!(
            "{}_{interval}.json",
            market::safe_symbol_filename(symbol)
        ))
    }

    pub fn computed_report_path(&self, ticker: &str) -> PathBuf {
        self.data_dir
            .join("computed")
            .join(format!("{}.json", market::safe_symbol_filename(ticker)))
    }

    pub fn sync_status_path(&self) -> PathBuf {
        self.data_dir.join("sync-status.json")
    }

    pub fn lake_dir(&self) -> PathBuf {
        self.data_dir.join("lake")
    }

    pub fn lake_db_path(&self) -> PathBuf {
        self.lake_dir().join("m3.duckdb")
    }

    pub fn lake_parquet_dir(&self) -> PathBuf {
        self.lake_dir().join("parquet")
    }

    pub fn lake_raw_sec_dir(&self) -> PathBuf {
        self.lake_dir().join("raw/sec")
    }

    pub fn lake_preserved_raw_sec_dir(&self) -> PathBuf {
        self.lake_raw_sec_dir().join("preserved")
    }

    pub fn lake_raw_sec_generations_dir(&self) -> PathBuf {
        self.lake_raw_sec_dir().join("generations")
    }

    pub fn lake_raw_sec_manifest_path(&self) -> PathBuf {
        self.lake_raw_sec_dir().join("CURRENT.json")
    }

    pub(crate) fn resolve_active_sec_raw_file_checked(
        &self,
        file_name: &str,
    ) -> Result<Option<PathBuf>> {
        if let Some(db) = crate::lake::LakeDb::open_read_only_if_exists(self).context(
            "failed to open committed DuckDB lake to resolve active SEC input generation",
        )? && let Some(db_gen) = db
            .read_sec_input_generation()
            .context("failed to read committed SEC input generation watermark from DuckDB")?
        {
            let trimmed = db_gen.trim();
            anyhow::ensure!(
                crate::universe::is_valid_gen_dir_name(trimmed),
                "invalid committed SEC input generation {trimmed:?}"
            );
            let candidate = self
                .lake_raw_sec_generations_dir()
                .join(trimmed)
                .join(file_name);
            let candidate_ok =
                fs::metadata(&candidate).is_ok_and(|meta| meta.is_file() && meta.len() > 0);
            anyhow::ensure!(
                candidate_ok,
                "committed SEC input generation {trimmed:?} is missing or empty {file_name} at {}",
                candidate.display()
            );
            return Ok(Some(candidate));
        }
        #[derive(Deserialize)]
        struct SecInputManifestRef {
            #[serde(default)]
            generation_id: String,
            #[serde(default)]
            generation_dir: String,
        }
        let manifest_path = self.lake_raw_sec_manifest_path();
        match fs::symlink_metadata(&manifest_path) {
            Ok(_) => {
                let manifest: SecInputManifestRef =
                    read_json(&manifest_path).with_context(|| {
                        format!(
                            "failed to parse SEC input manifest {}",
                            manifest_path.display()
                        )
                    })?;
                let id_trimmed = manifest.generation_id.trim();
                let trimmed = manifest.generation_dir.trim();
                anyhow::ensure!(
                    crate::universe::is_valid_gen_dir_name(trimmed) && id_trimmed == trimmed,
                    "invalid or mismatched SEC input manifest generation (id={id_trimmed:?}, dir={trimmed:?})"
                );
                let candidate = self
                    .lake_raw_sec_generations_dir()
                    .join(trimmed)
                    .join(file_name);
                let candidate_ok =
                    fs::metadata(&candidate).is_ok_and(|meta| meta.is_file() && meta.len() > 0);
                anyhow::ensure!(
                    candidate_ok,
                    "committed SEC input manifest generation {trimmed:?} is missing or empty {file_name} at {}",
                    candidate.display()
                );
                return Ok(Some(candidate));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "failed to inspect SEC input manifest {}",
                        manifest_path.display()
                    )
                });
            }
        }
        let legacy = self.lake_raw_sec_dir().join(file_name);
        Ok(legacy.is_file().then_some(legacy))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn resolve_active_sec_raw_file(&self, file_name: &str) -> PathBuf {
        match self.resolve_active_sec_raw_file_checked(file_name) {
            Ok(Some(path)) => path,
            Ok(None) => self.lake_raw_sec_dir().join(file_name),
            Err(_) => self
                .lake_raw_sec_generations_dir()
                .join(".unreadable-committed-generation")
                .join(file_name),
        }
    }

    pub(crate) fn acquire_sec_generation_read_pin(&self, resolved_file: &Path) -> Option<File> {
        let gens_dir = self.lake_raw_sec_generations_dir();
        let gen_dir = resolved_file.parent()?;
        if gen_dir.parent()? != gens_dir || !gen_dir.is_dir() {
            return None;
        }
        let lock_path = gen_dir.join(".gen.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .or_else(|err| {
                if err.kind() == std::io::ErrorKind::NotFound && gen_dir.is_dir() {
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(&lock_path)
                } else {
                    Err(err)
                }
            })
            .or_else(|_| File::open(&lock_path))
            .ok()?;
        file.lock_shared().ok()?;
        if !gen_dir.is_dir() || !resolved_file.is_file() {
            return None;
        }
        Some(file)
    }

    pub fn report_cache_versions_path(&self) -> PathBuf {
        self.data_dir
            .join("computed")
            .join(".report_cache_versions.json")
    }
}

pub fn load_watchlist(paths: &WorkspacePaths) -> Result<WatchlistFile> {
    read_json(&paths.watchlist_path())
}

pub fn save_watchlist(paths: &WorkspacePaths, wl: &WatchlistFile) -> Result<()> {
    write_json_atomic(&paths.watchlist_path(), wl)
}

pub fn load_journal(paths: &WorkspacePaths) -> Result<DecisionJournal> {
    read_json(&paths.journal_path())
}

pub fn save_journal(paths: &WorkspacePaths, journal: &DecisionJournal) -> Result<()> {
    write_json_atomic(&paths.journal_path(), journal)
}

pub fn load_dossier(paths: &WorkspacePaths, ticker: &str) -> Result<Option<Dossier>> {
    let path = paths.dossier_path(ticker);
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map(Some)
}

pub fn load_cached_price(paths: &WorkspacePaths, symbol: &str) -> Option<PriceSnapshot> {
    let path = paths.price_path(symbol);
    let snap: PriceSnapshot = path.exists().then(|| read_json(&path).ok()).flatten()?;
    if !snap.symbol.eq_ignore_ascii_case(symbol.trim()) {
        return None;
    }
    market::validate_quote_session(&snap, now_ms())
        .is_ok()
        .then_some(snap)
}

pub fn load_cached_price_history(
    paths: &WorkspacePaths,
    symbol: &str,
    interval: &str,
) -> Option<PriceHistorySnapshot> {
    let path = paths.price_history_path(symbol, interval);
    path.exists().then(|| read_json(&path).ok()).flatten()
}

const REPORT_CACHE_VERSION: u32 = 14;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CachedReportStamp {
    cache_version: u32,
    facts_extractor_version: u32,
    #[serde(default)]
    facts_raw_sha256: Option<String>,
}

#[derive(Debug, Clone)]
enum LoadedFacts {
    Fresh(RawCompanyFactsSummary),
    StaleReextractFailed(RawCompanyFactsSummary),
    Missing,
}

impl LoadedFacts {
    fn summary(&self) -> Option<&RawCompanyFactsSummary> {
        match self {
            Self::Fresh(s) | Self::StaleReextractFailed(s) => Some(s),
            Self::Missing => None,
        }
    }
}

fn load_report_cache_stamps(paths: &WorkspacePaths) -> BTreeMap<String, CachedReportStamp> {
    read_json::<BTreeMap<String, CachedReportStamp>>(&paths.report_cache_versions_path())
        .unwrap_or_default()
}

fn save_report_cache_stamps(
    paths: &WorkspacePaths,
    stamps: &BTreeMap<String, CachedReportStamp>,
) -> Result<()> {
    write_json_atomic(&paths.report_cache_versions_path(), stamps)
}

fn is_usable_report(rep: &MungerReport) -> bool {
    !rep.annual_rows.is_empty() || rep.ttm_row.is_some() || rep.latest_filing_end.is_some()
}

struct CandidateRawBytes {
    bytes: Vec<u8>,
    sha_matched: bool,
    source_url: String,
    fetched_at_ms: u64,
}

fn file_mtime_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let dur = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX))
}

fn raw_sec_reextract_candidates(
    paths: &WorkspacePaths,
    norm_cik: &str,
    expected_sha256: Option<&str>,
) -> Vec<CandidateRawBytes> {
    let expected_sha = expected_sha256
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);
    let mut candidate_sources: Vec<(PathBuf, String)> = vec![(
        paths.sec_raw_path(norm_cik),
        format!("https://data.sec.gov/api/xbrl/companyfacts/CIK{norm_cik}.json"),
    )];
    let preserved_dir = paths.lake_preserved_raw_sec_dir();
    if let Some(ref sha) = expected_sha {
        let p = preserved_dir.join(format!("CIK{norm_cik}-{sha}.json"));
        let src = format!("sec-preserved://CIK{norm_cik}-{sha}.json");
        candidate_sources.push((p, src));
    }
    if let Ok(entries) = fs::read_dir(&preserved_dir) {
        let cik_dash_prefix = format!("CIK{norm_cik}-");
        let mut sha_named: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_str()?;
                if path.is_file() && name.starts_with(&cik_dash_prefix) && name.ends_with(".json") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        sha_named.sort();
        sha_named.reverse();
        for p in sha_named {
            if !candidate_sources.iter().any(|(existing, _)| existing == &p) {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("preserved.json");
                let src = format!("sec-preserved://{name}");
                candidate_sources.push((p, src));
            }
        }
    }
    let legacy_preserved = preserved_dir.join(format!("CIK{norm_cik}.json"));
    if !candidate_sources
        .iter()
        .any(|(existing, _)| existing == &legacy_preserved)
    {
        candidate_sources.push((
            legacy_preserved,
            format!("sec-preserved://CIK{norm_cik}.json"),
        ));
    }

    let mut seen_shas: HashSet<String> = HashSet::new();
    let mut matched: Vec<CandidateRawBytes> = Vec::new();
    let mut fallback: Vec<CandidateRawBytes> = Vec::new();

    let mut push_bytes = |bytes: Vec<u8>, source_url: String, fetched_at_ms: u64| {
        if bytes.is_empty() {
            return;
        }
        let sha = edgar::hex_digest(&bytes);
        if !seen_shas.insert(sha.clone()) {
            return;
        }
        let is_exact = expected_sha.as_deref() == Some(sha.as_str());
        let item = CandidateRawBytes {
            bytes,
            sha_matched: is_exact,
            source_url,
            fetched_at_ms,
        };
        if is_exact {
            matched.push(item);
        } else {
            fallback.push(item);
        }
    };

    for (path, source_url) in candidate_sources {
        if path.is_file()
            && let Ok(bytes) = fs::read(&path)
        {
            let fetched = file_mtime_ms(&path).unwrap_or_else(now_ms);
            push_bytes(bytes, source_url, fetched);
        }
    }

    for attempt in 0..5 {
        let Ok(Some(zip_path)) = paths.resolve_active_sec_raw_file_checked("companyfacts.zip")
        else {
            break;
        };
        let is_gen_file = zip_path.parent().and_then(|p| p.parent())
            == Some(paths.lake_raw_sec_generations_dir().as_path());
        let _gen_read_pin = if is_gen_file {
            match paths.acquire_sec_generation_read_pin(&zip_path) {
                Some(pin) => Some(pin),
                None => {
                    if attempt < 4 {
                        std::thread::yield_now();
                    }
                    continue;
                }
            }
        } else {
            None
        };
        let Ok(file) = fs::File::open(&zip_path) else {
            if attempt < 4 {
                std::thread::yield_now();
            }
            continue;
        };
        if let Ok(mut archive) = zip::ZipArchive::new(std::io::BufReader::new(file)) {
            let target = format!("CIK{norm_cik}.json");
            let idx_opt = (0..archive.len()).find(|&i| {
                archive
                    .name_for_index(i)
                    .and_then(|res| res.ok())
                    .is_some_and(|name| {
                        name.rsplit('/')
                            .next()
                            .is_some_and(|base| base.eq_ignore_ascii_case(&target))
                    })
            });
            if let Some(idx) = idx_opt
                && let Ok(entry) = archive.by_index(idx)
            {
                const MAX_ENTRY: u64 = 128 * 1024 * 1024;
                if entry.size() <= MAX_ENTRY {
                    let mut buf = Vec::new();
                    if entry.take(MAX_ENTRY + 1).read_to_end(&mut buf).is_ok()
                        && u64::try_from(buf.len()).unwrap_or(u64::MAX) <= MAX_ENTRY
                    {
                        let fetched = file_mtime_ms(&zip_path).unwrap_or_else(now_ms);
                        push_bytes(
                            buf,
                            format!(
                                "https://www.sec.gov/Archives/edgar/daily-index/xbrl/companyfacts.zip#{target}"
                            ),
                            fetched,
                        );
                    }
                }
            }
            break;
        }
        break;
    }

    matched.extend(fallback);
    matched
}

fn reextract_non_regressing(
    prev: &RawCompanyFactsSummary,
    upgraded: &RawCompanyFactsSummary,
    sha_matched: bool,
) -> bool {
    if upgraded.annuals.is_empty() {
        return false;
    }
    if sha_matched {
        return true;
    }
    let prev_ends: Vec<String> = prev.annuals.iter().map(|a| a.period_end.clone()).collect();
    let new_ends: Vec<String> = upgraded
        .annuals
        .iter()
        .map(|a| a.period_end.clone())
        .collect();
    !edgar::is_annual_period_coverage_regression(
        &prev_ends,
        prev.ttm.as_ref().map(|t| t.as_of_end.as_str()),
        &new_ends,
        upgraded.ttm.as_ref().map(|t| t.as_of_end.as_str()),
        calculator::latest_summary_filing_end(upgraded),
    )
}

const CANONICAL_VERIFIED_MIN_EXTRACTOR_VERSION: u32 = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CanonicalTickerResolution {
    Resolved(String),
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreservedCanonicalGuard {
    VerifiedCommon,
    Supplemental,
    Unresolved,
}

fn load_verified_sec_summary(path: &Path, norm_cik: &str) -> Option<RawCompanyFactsSummary> {
    let summary = read_json::<RawCompanyFactsSummary>(path).ok()?;
    (summary.cik == norm_cik
        && summary.extractor_version >= CANONICAL_VERIFIED_MIN_EXTRACTOR_VERSION
        && !calculator::is_unresolved_facts_ticker(&summary.ticker))
    .then_some(summary)
}

fn resolve_v1_facts_canonical_ticker_with_url(
    paths: &WorkspacePaths,
    client: Option<&reqwest::blocking::Client>,
    norm_cik: &str,
    tickers_v1_url_override: Option<&str>,
) -> CanonicalTickerResolution {
    if let Ok(Some(te_path)) =
        paths.resolve_active_sec_raw_file_checked("company_tickers_exchange.json")
        && let Ok(bytes) = fs::read(&te_path)
        && let Some(canonical) =
            crate::universe::canonical_ticker_for_cik_in_tickers_exchange(&bytes, norm_cik)
    {
        return CanonicalTickerResolution::Resolved(canonical);
    }

    let v1_cache = paths.sec_tickers_cache();
    if let Ok(bytes) = fs::read(&v1_cache)
        && let Some(canonical) = edgar::canonical_ticker_for_cik_from_v1_bytes(&bytes, norm_cik)
    {
        return CanonicalTickerResolution::Resolved(canonical);
    }

    if let Some(http) = client {
        let fetched = match tickers_v1_url_override {
            Some(url) => {
                edgar::ensure_v1_tickers_cache_with_url(http, &v1_cache, !v1_cache.exists(), url)
            }
            None => edgar::ensure_v1_tickers_cache(http, &v1_cache, !v1_cache.exists()),
        };
        if let Ok(bytes) = fetched
            && let Some(canonical) = edgar::canonical_ticker_for_cik_from_v1_bytes(&bytes, norm_cik)
        {
            return CanonicalTickerResolution::Resolved(canonical);
        }
    }

    if let Ok(Some(db)) = crate::lake::LakeDb::open_read_only_if_exists(paths)
        && let Ok(Some(canonical)) = db.lookup_company_ticker_by_cik(norm_cik)
    {
        return CanonicalTickerResolution::Resolved(canonical);
    }

    CanonicalTickerResolution::Unresolved
}

fn resolve_v1_facts_canonical_ticker(
    paths: &WorkspacePaths,
    client: Option<&reqwest::blocking::Client>,
    _requested_ticker: &str,
    norm_cik: &str,
) -> CanonicalTickerResolution {
    resolve_v1_facts_canonical_ticker_with_url(paths, client, norm_cik, None)
}

fn load_cached_facts_status(paths: &WorkspacePaths, company: &CompanyEntry) -> LoadedFacts {
    if company.source == DataSource::Manual {
        let manual = paths.manual_financials_path(&company.ticker);
        if manual.exists()
            && let Ok(parsed) = read_json::<RawCompanyFactsSummary>(&manual)
        {
            return LoadedFacts::Fresh(parsed);
        }
    }
    let sec = paths.sec_facts_summary_path(&company.ticker);
    if !sec.exists() {
        return LoadedFacts::Missing;
    }
    let Ok(mut summary) = read_json::<RawCompanyFactsSummary>(&sec) else {
        return LoadedFacts::Missing;
    };
    let needs_canonical_resolution = calculator::is_unresolved_facts_ticker(&summary.ticker);
    let needs_upgrade = summary.extractor_version < edgar::SEC_EXTRACTOR_VERSION
        || summary.schema_version != edgar::SEC_FACTS_SCHEMA_VERSION;
    if company.source == DataSource::SecEdgar && (needs_upgrade || needs_canonical_resolution) {
        let cik_candidate = company
            .cik
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(summary.cik.as_str());
        let Ok(norm_cik) = edgar::normalize_cik(cik_candidate) else {
            if summary.extractor_version < CANONICAL_VERIFIED_MIN_EXTRACTOR_VERSION {
                summary.ticker = edgar::UNRESOLVED_CANONICAL_TICKER.to_owned();
            }
            return LoadedFacts::StaleReextractFailed(summary);
        };
        let canonical_ticker =
            match resolve_v1_facts_canonical_ticker(paths, None, &company.ticker, &norm_cik) {
                CanonicalTickerResolution::Resolved(ticker) => ticker,
                CanonicalTickerResolution::Unresolved => {
                    if needs_canonical_resolution && !needs_upgrade {
                        return LoadedFacts::Fresh(summary);
                    }
                    if !needs_canonical_resolution
                        && summary.extractor_version >= CANONICAL_VERIFIED_MIN_EXTRACTOR_VERSION
                    {
                        return LoadedFacts::StaleReextractFailed(summary);
                    }
                    edgar::UNRESOLVED_CANONICAL_TICKER.to_owned()
                }
            };
        let candidates = raw_sec_reextract_candidates(paths, &norm_cik, Some(&summary.raw_sha256));
        for cand in candidates {
            let (source_url, fetched_at_ms) = if cand.sha_matched {
                (summary.source_url.as_str(), summary.fetched_at_ms)
            } else {
                (cand.source_url.as_str(), cand.fetched_at_ms)
            };
            if let Ok(upgraded) = edgar::parse_companyfacts_bytes(
                &cand.bytes,
                &canonical_ticker,
                &norm_cik,
                source_url,
                fetched_at_ms,
            ) && reextract_non_regressing(&summary, &upgraded, cand.sha_matched)
            {
                let _ = write_json_atomic(&sec, &upgraded);
                return LoadedFacts::Fresh(upgraded);
            }
        }
        if needs_canonical_resolution
            && !needs_upgrade
            && !calculator::is_unresolved_facts_ticker(&canonical_ticker)
        {
            summary.ticker = canonical_ticker;
            let _ = write_json_atomic(&sec, &summary);
            return LoadedFacts::Fresh(summary);
        }
        summary.ticker = canonical_ticker;
        return LoadedFacts::StaleReextractFailed(summary);
    }
    LoadedFacts::Fresh(summary)
}

pub fn load_cached_facts(
    paths: &WorkspacePaths,
    company: &CompanyEntry,
) -> Option<RawCompanyFactsSummary> {
    load_cached_facts_status(paths, company).summary().cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportResolveMode {
    LoadOrMigrate,
    RecomputeInteractive,
}

fn price_matches_observation(
    rep_price: Option<&PriceSnapshot>,
    cached_price: Option<&PriceSnapshot>,
) -> bool {
    match (rep_price, cached_price) {
        (Some(rp), Some(cp)) => {
            rp.symbol.eq_ignore_ascii_case(&cp.symbol)
                && rp.price == cp.price
                && rp.market_time_ms == cp.market_time_ms
                && rp.fetched_at_ms == cp.fetched_at_ms
        }
        (None, None) => true,
        _ => false,
    }
}

fn sanitize_absent_cash_row(row: &mut crate::model::AnnualMetricRow) {
    if row.roic_note.as_deref() == Some("cash_and_st_unavailable") && row.cash_and_st.is_some() {
        row.cash_and_st = None;
    }
}

fn resolve_preserved_canonical_guard(
    paths: &WorkspacePaths,
    company: &CompanyEntry,
    prev: &MungerReport,
    facts_summary: Option<&RawCompanyFactsSummary>,
) -> PreservedCanonicalGuard {
    if company.source != DataSource::SecEdgar {
        return PreservedCanonicalGuard::VerifiedCommon;
    }
    let cik_candidate = company
        .cik
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            facts_summary
                .map(|f| f.cik.as_str())
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| prev.cik.as_deref().filter(|s| !s.trim().is_empty()));
    let Some(norm_cik) = cik_candidate.and_then(|c| edgar::normalize_cik(c).ok()) else {
        return PreservedCanonicalGuard::Unresolved;
    };
    match resolve_v1_facts_canonical_ticker(paths, None, &company.ticker, &norm_cik) {
        CanonicalTickerResolution::Resolved(canonical) => {
            if calculator::is_supplemental_security_ticker(&company.ticker, &canonical) {
                PreservedCanonicalGuard::Supplemental
            } else {
                PreservedCanonicalGuard::VerifiedCommon
            }
        }
        CanonicalTickerResolution::Unresolved => {
            if let Some(f) = facts_summary
                && !calculator::is_unresolved_facts_ticker(&f.ticker)
                && f.extractor_version >= CANONICAL_VERIFIED_MIN_EXTRACTOR_VERSION
            {
                if calculator::is_supplemental_security_ticker(&company.ticker, &f.ticker) {
                    PreservedCanonicalGuard::Supplemental
                } else {
                    PreservedCanonicalGuard::VerifiedCommon
                }
            } else {
                PreservedCanonicalGuard::Unresolved
            }
        }
    }
}

fn align_preserved_report(
    prev: &MungerReport,
    company: &CompanyEntry,
    price: Option<&PriceSnapshot>,
    now: u64,
    sync_assumptions: bool,
    canonical_guard: PreservedCanonicalGuard,
) -> (MungerReport, bool) {
    let mut preserved = prev.clone();
    preserved.schema_version = calculator::REPORT_SCHEMA_VERSION;
    let mut modified = preserved.schema_version != prev.schema_version;

    let metadata_changed = preserved.name != company.name
        || preserved.bucket != company.bucket
        || preserved.circle != company.circle
        || preserved.too_hard_reason != company.too_hard_reason
        || (sync_assumptions && preserved.assumptions != company.assumptions);
    if metadata_changed {
        preserved.name = company.name.clone();
        preserved.bucket = company.bucket;
        preserved.circle = company.circle.clone();
        preserved.too_hard_reason = company.too_hard_reason.clone();
        if sync_assumptions {
            preserved.assumptions = company.assumptions.clone();
        }
        modified = true;
    }

    for r in &mut preserved.annual_rows {
        let before = r.cash_and_st;
        sanitize_absent_cash_row(r);
        modified |= r.cash_and_st != before;
    }
    if let Some(ref mut ttm) = preserved.ttm_row {
        let before = ttm.cash_and_st;
        sanitize_absent_cash_row(ttm);
        modified |= ttm.cash_and_st != before;
    }

    let valid_price = price.filter(|p| {
        p.symbol.eq_ignore_ascii_case(&company.ticker)
            && market::validate_quote_session(p, now).is_ok()
    });
    let prev_price_valid = preserved.price.as_ref().is_none_or(|p| {
        p.symbol.eq_ignore_ascii_case(&company.ticker)
            && market::validate_quote_session(p, now).is_ok()
    });
    let price_needs_update =
        !prev_price_valid || !price_matches_observation(preserved.price.as_ref(), valid_price);
    let ticker_non_common = calculator::is_share_basis_mismatched(&company.ticker, false);
    let non_common_mismatch =
        ticker_non_common || canonical_guard == PreservedCanonicalGuard::Supplemental;
    let unresolved_mismatch = canonical_guard == PreservedCanonicalGuard::Unresolved;
    let had_unresolved_warn = preserved.warnings.iter().any(|w| {
        w.code == "SHARE_BASIS_MISMATCH" && w.title == calculator::UNRESOLVED_SHARE_BASIS_TITLE
    });
    if had_unresolved_warn
        && !non_common_mismatch
        && canonical_guard == PreservedCanonicalGuard::VerifiedCommon
    {
        preserved.warnings.retain(|w| {
            !(w.code == "SHARE_BASIS_MISMATCH"
                && w.title == calculator::UNRESOLVED_SHARE_BASIS_TITLE)
        });
        modified = true;
    }
    let has_mismatch_warning = preserved
        .warnings
        .iter()
        .any(|w| w.code == "SHARE_BASIS_MISMATCH");
    let sec_mismatch = non_common_mismatch || unresolved_mismatch || has_mismatch_warning;
    if sec_mismatch && !has_mismatch_warning {
        preserved
            .warnings
            .push(if unresolved_mismatch && !non_common_mismatch {
                calculator::unresolved_share_basis_warning(&company.ticker)
            } else {
                calculator::share_basis_mismatch_warning(&company.ticker, non_common_mismatch)
            });
        modified = true;
    }

    if price_needs_update || sec_mismatch || modified {
        preserved.price = valid_price.cloned();
        let build_guards =
            |row: &crate::model::AnnualMetricRow,
             old_val: Option<&crate::model::ValuationSnapshot>| {
                let inferred_bad_shares = !had_unresolved_warn
                    && old_val.is_some_and(|v| {
                        row.strict_owner_earnings
                            .is_some_and(|oe| oe > Decimal::ZERO)
                            && row.diluted_shares > Decimal::ZERO
                            && v.strike_price.is_none()
                    });
                calculator::ValuationGuards {
                    strict_oe_available: row.strict_owner_earnings.is_some(),
                    share_basis_valid: !sec_mismatch && !inferred_bad_shares,
                    debt_available: row.total_debt.is_some(),
                    cash_available: row.cash_and_st.is_some(),
                }
            };
        preserved.fy_valuation = preserved.annual_rows.last().map(|row| {
            let g = build_guards(row, preserved.fy_valuation.as_ref());
            calculator::compute_valuation(
                "FY",
                row,
                preserved.price.as_ref(),
                &preserved.assumptions,
                g,
            )
        });
        preserved.ttm_valuation = preserved.ttm_row.as_ref().map(|row| {
            let g = build_guards(row, preserved.ttm_valuation.as_ref());
            calculator::compute_valuation(
                "TTM",
                row,
                preserved.price.as_ref(),
                &preserved.assumptions,
                g,
            )
        });
        modified = true;
    }

    let derived_status = calculator::derive_strike_status(
        preserved.bucket,
        !preserved.annual_rows.is_empty(),
        preserved.price.is_some(),
        &preserved.warnings,
        preserved.fy_valuation.as_ref(),
        preserved.ttm_valuation.as_ref(),
    );
    if preserved.status != derived_status {
        preserved.status = derived_status;
        modified = true;
    }

    (preserved, modified)
}

fn report_matches_current_inputs(
    rep: &MungerReport,
    company: &CompanyEntry,
    facts_summary: Option<&RawCompanyFactsSummary>,
    price: Option<&PriceSnapshot>,
    now: u64,
) -> bool {
    if rep.schema_version != calculator::REPORT_SCHEMA_VERSION
        || !rep.ticker.eq_ignore_ascii_case(&company.ticker)
        || rep.name != company.name
        || rep.source != company.source
        || rep.bucket != company.bucket
        || rep.circle != company.circle
        || rep.too_hard_reason != company.too_hard_reason
        || rep.assumptions != company.assumptions
    {
        return false;
    }
    let expected_sha = facts_summary.map(|f| f.raw_sha256.as_str());
    if rep.raw_sha256.as_deref() != expected_sha {
        return false;
    }
    let expected_latest_end = facts_summary.and_then(calculator::latest_summary_filing_end);
    if rep.latest_filing_end.as_deref() != expected_latest_end {
        return false;
    }
    let expected_cik = facts_summary
        .map(|f| f.cik.as_str())
        .or(company.cik.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|c| edgar::normalize_cik(c).ok());
    if let Some(exp_cik) = expected_cik.as_deref()
        && rep.cik.as_deref() != Some(exp_cik)
    {
        return false;
    }
    let unresolved_facts = facts_summary.is_some_and(|f| {
        calculator::is_unresolved_facts_ticker(&f.ticker)
            || f.extractor_version < CANONICAL_VERIFIED_MIN_EXTRACTOR_VERSION
    });
    let supplemental_to_facts = facts_summary.is_some_and(|f| {
        !calculator::is_unresolved_facts_ticker(&f.ticker)
            && calculator::is_supplemental_security_ticker(&company.ticker, &f.ticker)
    });
    let sec_mismatch = calculator::is_share_basis_mismatched(&company.ticker, false)
        || unresolved_facts
        || supplemental_to_facts;
    let rep_has_mismatch = rep
        .warnings
        .iter()
        .any(|w| w.code == "SHARE_BASIS_MISMATCH");
    let rep_has_unresolved_warn = rep.warnings.iter().any(|w| {
        w.code == "SHARE_BASIS_MISMATCH" && w.title == calculator::UNRESOLVED_SHARE_BASIS_TITLE
    });
    if (sec_mismatch
        && (!rep_has_mismatch
            || [rep.fy_valuation.as_ref(), rep.ttm_valuation.as_ref()]
                .into_iter()
                .flatten()
                .any(|v| v.strike_price.is_some() || v.strict_fcf_yield_mcap_pct.is_some())))
        || (!unresolved_facts && facts_summary.is_some() && rep_has_unresolved_warn)
    {
        return false;
    }
    let expected_status = calculator::derive_strike_status(
        rep.bucket,
        !rep.annual_rows.is_empty(),
        rep.price.is_some(),
        &rep.warnings,
        rep.fy_valuation.as_ref(),
        rep.ttm_valuation.as_ref(),
    );
    if rep.status != expected_status {
        return false;
    }
    price_matches_observation(rep.price.as_ref(), price)
        && rep.price.as_ref().is_none_or(|p| {
            p.symbol.eq_ignore_ascii_case(&company.ticker)
                && market::validate_quote_session(p, now).is_ok()
        })
}

fn is_report_period_regression(prev: &MungerReport, next: &MungerReport) -> bool {
    let prev_ends: Vec<String> = prev
        .annual_rows
        .iter()
        .map(|a| a.period_end.clone())
        .collect();
    let new_ends: Vec<String> = next
        .annual_rows
        .iter()
        .map(|a| a.period_end.clone())
        .collect();
    edgar::is_annual_period_coverage_regression(
        &prev_ends,
        prev.ttm_row.as_ref().map(|t| t.period_end.as_str()),
        &new_ends,
        next.ttm_row.as_ref().map(|t| t.period_end.as_str()),
        next.latest_filing_end.as_deref(),
    )
}

fn resolve_report_with_failure_safety(
    paths: &WorkspacePaths,
    company: &CompanyEntry,
    facts_status: &LoadedFacts,
    price: Option<&PriceSnapshot>,
    now: u64,
    stamps: &mut BTreeMap<String, CachedReportStamp>,
    mode: ReportResolveMode,
) -> Result<(MungerReport, bool)> {
    let allow_cache_hit = matches!(mode, ReportResolveMode::LoadOrMigrate);
    let allow_rebuild_with_stale_facts = matches!(mode, ReportResolveMode::RecomputeInteractive);
    let computed_path = paths.computed_report_path(&company.ticker);
    let existing_opt = computed_path
        .exists()
        .then(|| read_json::<MungerReport>(&computed_path).ok())
        .flatten();
    let existing_usable = existing_opt.as_ref().filter(|r| is_usable_report(r));
    let upper = company.ticker.trim().to_ascii_uppercase();
    let facts_summary = facts_status.summary();
    let facts_sha = facts_summary.map(|f| f.raw_sha256.clone());
    let facts_ext = facts_summary.map_or(0, |f| f.extractor_version);
    let facts_is_fresh =
        matches!(facts_status, LoadedFacts::Fresh(_)) || company.source == DataSource::Manual;

    let valid_price = price.filter(|p| {
        p.symbol.eq_ignore_ascii_case(&company.ticker)
            && market::validate_quote_session(p, now).is_ok()
    });

    if allow_cache_hit
        && facts_is_fresh
        && let Some(rep) = existing_opt.as_ref()
    {
        let stamp_ok = stamps.get(&upper).is_some_and(|s| {
            s.cache_version >= REPORT_CACHE_VERSION
                && s.facts_extractor_version >= facts_ext
                && s.facts_raw_sha256 == facts_sha
        });
        if stamp_ok && report_matches_current_inputs(rep, company, facts_summary, valid_price, now)
        {
            return Ok((rep.clone(), false));
        }
    }

    if let Some(prev) = existing_usable
        && company.source == DataSource::SecEdgar
    {
        match facts_status {
            LoadedFacts::Missing | LoadedFacts::StaleReextractFailed(_) => {
                let canonical_guard =
                    resolve_preserved_canonical_guard(paths, company, prev, facts_summary);
                let (preserved, modified) = align_preserved_report(
                    prev,
                    company,
                    valid_price,
                    now,
                    allow_rebuild_with_stale_facts,
                    canonical_guard,
                );
                if allow_rebuild_with_stale_facts && modified {
                    write_json_atomic(&computed_path, &preserved)?;
                }
                return Ok((preserved, false));
            }
            LoadedFacts::Fresh(_) => {}
        }
    }

    let guarded_facts_summary: Option<RawCompanyFactsSummary>;
    let facts_for_build = match facts_summary {
        Some(f)
            if company.source == DataSource::SecEdgar
                && (calculator::is_unresolved_facts_ticker(&f.ticker)
                    || f.extractor_version < CANONICAL_VERIFIED_MIN_EXTRACTOR_VERSION) =>
        {
            let mut cloned = f.clone();
            let cik_candidate = company
                .cik
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(f.cik.as_str());
            if let Ok(norm_cik) = edgar::normalize_cik(cik_candidate)
                && let CanonicalTickerResolution::Resolved(resolved) =
                    resolve_v1_facts_canonical_ticker(paths, None, &company.ticker, &norm_cik)
            {
                cloned.ticker = resolved;
            } else {
                cloned.ticker = edgar::UNRESOLVED_CANONICAL_TICKER.to_owned();
            }
            guarded_facts_summary = Some(cloned);
            guarded_facts_summary.as_ref()
        }
        other => other,
    };
    let rep = calculator::build_report(company, facts_for_build, valid_price, now);
    if let Some(prev) = existing_usable
        && company.source == DataSource::SecEdgar
        && (!is_usable_report(&rep) || is_report_period_regression(prev, &rep))
    {
        let canonical_guard =
            resolve_preserved_canonical_guard(paths, company, prev, facts_summary);
        let (preserved, _) = align_preserved_report(
            prev,
            company,
            valid_price,
            now,
            allow_rebuild_with_stale_facts,
            canonical_guard,
        );
        return Ok((preserved, false));
    }

    write_json_atomic(&computed_path, &rep)?;
    let mut stamp_dirty = false;
    if facts_is_fresh {
        stamps.insert(
            upper,
            CachedReportStamp {
                cache_version: REPORT_CACHE_VERSION,
                facts_extractor_version: facts_ext,
                facts_raw_sha256: facts_sha,
            },
        );
        stamp_dirty = true;
    }
    Ok((rep, stamp_dirty))
}

pub fn load_or_recompute_ticker(paths: &WorkspacePaths, ticker: &str) -> Result<MungerReport> {
    let wl = load_watchlist(paths)?;
    let company = wl
        .companies
        .iter()
        .find(|c| c.ticker.eq_ignore_ascii_case(ticker))
        .with_context(|| format!("观察池中不存在 {ticker}"))?;
    let facts_status = load_cached_facts_status(paths, company);
    let price = load_cached_price(paths, &company.ticker);
    let mut stamps = load_report_cache_stamps(paths);
    let (rep, dirty) = resolve_report_with_failure_safety(
        paths,
        company,
        &facts_status,
        price.as_ref(),
        now_ms(),
        &mut stamps,
        ReportResolveMode::LoadOrMigrate,
    )?;
    if dirty {
        let _ = save_report_cache_stamps(paths, &stamps);
    }
    Ok(rep)
}

pub fn load_or_compute_reports(paths: &WorkspacePaths) -> Result<Vec<MungerReport>> {
    let wl = load_watchlist(paths)?;
    let now = now_ms();
    let mut stamps = load_report_cache_stamps(paths);
    let mut stamps_dirty = false;
    let mut reports = Vec::with_capacity(wl.companies.len());
    for company in &wl.companies {
        let facts_status = load_cached_facts_status(paths, company);
        let price = load_cached_price(paths, &company.ticker);
        let (rep, dirty) = resolve_report_with_failure_safety(
            paths,
            company,
            &facts_status,
            price.as_ref(),
            now,
            &mut stamps,
            ReportResolveMode::LoadOrMigrate,
        )?;
        stamps_dirty |= dirty;
        reports.push(rep);
    }
    if stamps_dirty {
        let _ = save_report_cache_stamps(paths, &stamps);
    }
    Ok(reports)
}

pub fn recompute_ticker(paths: &WorkspacePaths, ticker: &str) -> Result<MungerReport> {
    let wl = load_watchlist(paths)?;
    let company = wl
        .companies
        .iter()
        .find(|c| c.ticker.eq_ignore_ascii_case(ticker))
        .with_context(|| format!("观察池中不存在 {ticker}"))?;
    let facts_status = load_cached_facts_status(paths, company);
    let price = load_cached_price(paths, &company.ticker);
    let mut stamps = load_report_cache_stamps(paths);
    let (rep, dirty) = resolve_report_with_failure_safety(
        paths,
        company,
        &facts_status,
        price.as_ref(),
        now_ms(),
        &mut stamps,
        ReportResolveMode::RecomputeInteractive,
    )?;
    if dirty {
        let _ = save_report_cache_stamps(paths, &stamps);
    }
    Ok(rep)
}

pub fn update_company_assumptions(
    paths: &WorkspacePaths,
    ticker: &str,
    assumptions: ValuationAssumptions,
) -> Result<MungerReport> {
    let mut wl = load_watchlist(paths)?;
    let company = wl
        .companies
        .iter_mut()
        .find(|c| c.ticker.eq_ignore_ascii_case(ticker))
        .with_context(|| format!("观察池中不存在 {ticker}"))?;
    company.assumptions = assumptions;
    save_watchlist(paths, &wl)?;
    recompute_ticker(paths, ticker)
}

pub fn update_company_bucket(
    paths: &WorkspacePaths,
    ticker: &str,
    bucket: Bucket,
) -> Result<MungerReport> {
    let mut wl = load_watchlist(paths)?;
    let company = wl
        .companies
        .iter_mut()
        .find(|c| c.ticker.eq_ignore_ascii_case(ticker))
        .with_context(|| format!("观察池中不存在 {ticker}"))?;
    company.bucket = bucket;
    save_watchlist(paths, &wl)?;
    recompute_ticker(paths, ticker)
}

#[cfg_attr(not(test), allow(dead_code))]
fn sync_v1_sec_company_with_url(
    paths: &WorkspacePaths,
    sec_client: &reqwest::blocking::Client,
    company: &mut CompanyEntry,
    facts_url_override: Option<&str>,
    now: u64,
) -> std::result::Result<(), String> {
    sync_v1_sec_company_with_urls(paths, sec_client, company, facts_url_override, None, now)
}

fn sync_v1_sec_company_with_urls(
    paths: &WorkspacePaths,
    sec_client: &reqwest::blocking::Client,
    company: &mut CompanyEntry,
    facts_url_override: Option<&str>,
    tickers_v1_url_override: Option<&str>,
    now: u64,
) -> std::result::Result<(), String> {
    let cik = match &company.cik {
        Some(cik) if !cik.trim().is_empty() => edgar::normalize_cik(cik),
        _ => {
            let found = edgar::lookup_cik_by_ticker(
                sec_client,
                &paths.sec_tickers_cache(),
                &company.ticker,
                false,
            );
            if let Ok(ref norm) = found {
                company.cik = Some(norm.clone());
            }
            found
        }
    }
    .map_err(|err| format!("CIK 解析失败: {err:#}"))?;

    let raw_path = paths.sec_raw_path(&cik);
    let summary_path = paths.sec_facts_summary_path(&company.ticker);
    let canonical_ticker = match resolve_v1_facts_canonical_ticker_with_url(
        paths,
        Some(sec_client),
        &cik,
        tickers_v1_url_override,
    ) {
        CanonicalTickerResolution::Resolved(ticker) => ticker,
        CanonicalTickerResolution::Unresolved => {
            if let Some(verified) = load_verified_sec_summary(&summary_path, &cik) {
                return Err(format!(
                    "SEC 权威普通股索引不可用，已保留 {} 的现有已验证摘要 ({})",
                    company.ticker, verified.ticker
                ));
            }
            edgar::UNRESOLVED_CANONICAL_TICKER.to_owned()
        }
    };
    let summary_res = match facts_url_override {
        Some(url) => edgar::fetch_and_extract_from_url(
            sec_client,
            &canonical_ticker,
            &cik,
            url,
            &raw_path,
            now,
        ),
        None => edgar::fetch_and_extract(sec_client, &canonical_ticker, &cik, &raw_path, now),
    };
    let summary = summary_res.map_err(|err| format!("SEC 失败: {err:#}"))?;
    write_json_atomic(&summary_path, &summary).map_err(|err| format!("SEC 写入失败: {err:#}"))?;
    Ok(())
}

pub fn sync_all(
    paths: &WorkspacePaths,
    filter_ticker: Option<&str>,
    prices_only: bool,
) -> Result<SyncStatusFile> {
    let mut wl = load_watchlist(paths)?;
    let sec_client = edgar::sec_http_client()?;
    let mkt_client = market::market_http_client()?;
    let now = now_ms();

    let mut statuses = Vec::new();
    let mut watchlist_changed = false;

    for company in &mut wl.companies {
        if let Some(target) = filter_ticker
            && !company.ticker.eq_ignore_ascii_case(target)
        {
            continue;
        }
        let mut errors = Vec::new();
        let mut fin_ok = false;

        if !prices_only {
            match company.source {
                DataSource::SecEdgar => {
                    let prev_cik = company.cik.clone();
                    match sync_v1_sec_company_with_url(paths, &sec_client, company, None, now) {
                        Ok(()) => {
                            if company.cik != prev_cik {
                                watchlist_changed = true;
                            }
                            fin_ok = true;
                        }
                        Err(msg) => {
                            if company.cik != prev_cik {
                                watchlist_changed = true;
                            }
                            errors.push(msg);
                        }
                    }
                }
                DataSource::Manual => {
                    fin_ok = paths.manual_financials_path(&company.ticker).exists();
                }
            }
        } else {
            fin_ok = load_cached_facts(paths, company).is_some();
        }

        let price_ok = match market::fetch_price_snapshot(
            &mkt_client,
            &company.ticker,
            &paths.price_path(&company.ticker),
            now,
        ) {
            Ok(_) => {
                // Non-fatal history sync for Munger 5Y weekly & 1Y daily K-line overlay
                for (interval, range) in [("1wk", "5y"), ("1d", "1y")] {
                    let _ = market::fetch_price_history(
                        &mkt_client,
                        &company.ticker,
                        interval,
                        range,
                        &paths.price_history_path(&company.ticker, interval),
                        now,
                    );
                    std::thread::sleep(std::time::Duration::from_millis(80));
                }
                true
            }
            Err(err) => {
                errors.push(format!("行情失败: {err:#}"));
                false
            }
        };

        let facts_status = load_cached_facts_status(paths, company);
        let price = load_cached_price(paths, &company.ticker);
        let mut stamps = load_report_cache_stamps(paths);
        let (_, dirty) = resolve_report_with_failure_safety(
            paths,
            company,
            &facts_status,
            price.as_ref(),
            now,
            &mut stamps,
            ReportResolveMode::RecomputeInteractive,
        )?;
        if dirty {
            save_report_cache_stamps(paths, &stamps)?;
        }

        statuses.push(SyncTickerStatus {
            ticker: company.ticker.clone(),
            financials_ok: fin_ok,
            price_ok,
            error: (!errors.is_empty()).then(|| errors.join(" | ")),
        });
    }

    if watchlist_changed {
        save_watchlist(paths, &wl)?;
    }

    let (benchmark_ok, benchmark_error) = match market::fetch_price_snapshot(
        &mkt_client,
        &wl.benchmark_symbol,
        &paths.price_path(&wl.benchmark_symbol),
        now,
    ) {
        Ok(bench) => {
            if let Ok(mut journal) = load_journal(paths)
                && journal.benchmark_start_price.is_none()
            {
                journal.benchmark_start_price = Some(bench.price);
                let _ = save_journal(paths, &journal);
            }
            (true, None)
        }
        Err(err) => (false, Some(format!("{err:#}"))),
    };

    let sync_status = SyncStatusFile {
        schema_version: 1,
        synced_at_ms: now,
        benchmark_ok,
        benchmark_error,
        tickers: statuses,
    };
    write_json_atomic(&paths.sync_status_path(), &sync_status)?;
    Ok(sync_status)
}

pub fn add_or_update_company(
    paths: &WorkspacePaths,
    ticker: &str,
    bucket: Bucket,
    name: Option<&str>,
    circle: Option<&str>,
    reason: Option<&str>,
) -> Result<CompanyEntry> {
    add_or_update_company_with_cik(paths, ticker, bucket, name, None, circle, reason)
}

pub fn add_or_update_company_with_cik(
    paths: &WorkspacePaths,
    ticker: &str,
    bucket: Bucket,
    name: Option<&str>,
    cik_override: Option<&str>,
    circle: Option<&str>,
    reason: Option<&str>,
) -> Result<CompanyEntry> {
    let mut wl = load_watchlist(paths)?;
    let upper = ticker.trim().to_ascii_uppercase();

    if let Some(existing) = wl
        .companies
        .iter_mut()
        .find(|c| c.ticker.eq_ignore_ascii_case(&upper))
    {
        existing.bucket = bucket;
        if let Some(n) = name {
            existing.name = n.to_owned();
        }
        if existing.cik.is_none()
            && let Some(raw_cik) = cik_override.map(str::trim).filter(|s| !s.is_empty())
        {
            existing.cik = Some(edgar::normalize_cik(raw_cik)?);
        }
        if let Some(c) = circle {
            existing.circle = c.to_owned();
        }
        if let Some(r) = reason {
            existing.too_hard_reason = Some(r.to_owned());
        }
        let cloned = existing.clone();
        save_watchlist(paths, &wl)?;
        let _ = recompute_ticker(paths, &cloned.ticker);
        return Ok(cloned);
    }

    let is_non_us = upper.contains('.');
    let (source, cik, currency) = if is_non_us {
        let cur = if upper.ends_with(".HK") {
            "HKD"
        } else if upper.ends_with(".PA") || upper.ends_with(".DE") {
            "EUR"
        } else if upper.ends_with(".T") {
            "JPY"
        } else {
            "USD"
        };
        (DataSource::Manual, None, cur.to_owned())
    } else if let Some(raw_cik) = cik_override.map(str::trim).filter(|s| !s.is_empty()) {
        let cik = edgar::normalize_cik(raw_cik)?;
        (DataSource::SecEdgar, Some(cik), "USD".to_owned())
    } else {
        let sec = edgar::sec_http_client()?;
        let cik = edgar::lookup_cik_by_ticker(&sec, &paths.sec_tickers_cache(), &upper, false)?;
        (DataSource::SecEdgar, Some(cik), "USD".to_owned())
    };

    let entry = CompanyEntry {
        ticker: upper.clone(),
        name: name.unwrap_or(&upper).to_owned(),
        bucket,
        source,
        cik,
        currency,
        circle: circle.unwrap_or("待补充能力圈").to_owned(),
        too_hard_reason: reason.map(str::to_owned),
        assumptions: ValuationAssumptions {
            multiple_low: Decimal::from(18),
            multiple_high: Decimal::from(22),
            target_return_pct: wl.default_target_return_pct,
            expected_growth_pct: Decimal::from(8),
            strike_price_override: None,
        },
    };

    wl.companies.push(entry.clone());
    save_watchlist(paths, &wl)?;

    if !paths.dossier_path(&upper).exists() {
        let skeleton = Dossier {
            schema_version: 1,
            ticker: upper.clone(),
            name: entry.name.clone(),
            updated_at: civil_today_iso(),
            draft_source: "agent_draft".to_owned(),
            three_sentence_business: "待运行 Alan 审讯工作流生成三句话商业逻辑。".to_owned(),
            latticework: Latticework {
                microeconomics: "待补充".to_owned(),
                ecology: "待补充".to_owned(),
                psychology_incentives: "待补充".to_owned(),
                capital_allocation: "待补充".to_owned(),
            },
            premortem: Vec::new(),
            open_questions: Vec::new(),
            human_judgment: HumanJudgment {
                pending_human_review: true,
                my_stance: None,
                weight_rationale: None,
                falsification_metrics: Vec::new(),
                feynman_checked_at: None,
            },
        };
        write_json_atomic(&paths.dossier_path(&upper), &skeleton)?;
    }

    Ok(entry)
}

pub fn record_journal_entry(
    paths: &WorkspacePaths,
    ticker: &str,
    action: JournalAction,
    note: &str,
    shares: Option<Decimal>,
    falsification: Option<&str>,
) -> Result<JournalEntry> {
    let mut journal = load_journal(paths)?;
    let wl = load_watchlist(paths)?;
    let upper = ticker.trim().to_ascii_uppercase();

    let now = now_ms();
    let bench_snap = load_cached_price(paths, &wl.benchmark_symbol)
        .filter(|b| market::validate_quote_session(b, now).is_ok());
    let in_watchlist = wl
        .companies
        .iter()
        .any(|c| c.ticker.eq_ignore_ascii_case(&upper));

    let (price, strict_yield) = if in_watchlist {
        let rep = load_or_recompute_ticker(paths, &upper)?;
        let valid_price = rep.price.as_ref().filter(|snap| {
            snap.symbol.eq_ignore_ascii_case(&upper)
                && market::validate_quote_session(snap, now).is_ok()
        });
        let share_mismatch = calculator::is_share_basis_mismatched(&upper, false)
            || rep
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH");
        let y = if valid_price.is_some()
            && !share_mismatch
            && rep.ticker.eq_ignore_ascii_case(&upper)
        {
            rep.ttm_valuation
                .as_ref()
                .or(rep.fy_valuation.as_ref())
                .and_then(|v| v.strict_fcf_yield_mcap_pct)
        } else {
            None
        };
        (valid_price.map(|snap| snap.price), y)
    } else {
        let price_snap = load_cached_price(paths, &upper).filter(|p| {
            p.symbol.eq_ignore_ascii_case(&upper) && market::validate_quote_session(p, now).is_ok()
        });
        let report = read_json::<MungerReport>(&paths.computed_report_path(&upper))
            .ok()
            .filter(|r| {
                r.schema_version >= calculator::REPORT_SCHEMA_VERSION
                    && r.ticker.eq_ignore_ascii_case(&upper)
                    && !r.warnings.iter().any(|w| w.code == "SHARE_BASIS_MISMATCH")
            });
        let aligned_yield = (!calculator::is_share_basis_mismatched(&upper, false))
            .then_some(report.as_ref())
            .flatten()
            .filter(|r| {
                price_snap.is_some()
                    && price_matches_observation(r.price.as_ref(), price_snap.as_ref())
            })
            .and_then(|r| {
                r.ttm_valuation
                    .as_ref()
                    .or(r.fy_valuation.as_ref())
                    .and_then(|v| v.strict_fcf_yield_mcap_pct)
            });
        (price_snap.as_ref().map(|p| p.price), aligned_yield)
    };

    let recorded_at = civil_today_iso();

    let entry = JournalEntry {
        id: format!("j-{}-{}", now_ms(), market::safe_symbol_filename(&upper)),
        recorded_at,
        ticker: upper,
        action,
        price,
        shares,
        strict_fcf_yield_pct: strict_yield,
        benchmark_price: bench_snap.map(|b| b.price),
        rationale: note.to_owned(),
        falsification_trigger: falsification.unwrap_or("待补充客观财务证伪条件").to_owned(),
    };
    journal.entries.push(entry.clone());
    save_journal(paths, &journal)?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_and_roundtrips_watchlist_and_journal_atomically() {
        let dir = std::env::temp_dir().join(format!("m3-store-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "TEST".into(),
                name: "Test Inc.".into(),
                bucket: Bucket::Core,
                source: DataSource::Manual,
                cik: None,
                currency: "USD".into(),
                circle: "CAD".into(),
                too_hard_reason: None,
                assumptions: ValuationAssumptions {
                    multiple_low: Decimal::from(18),
                    multiple_high: Decimal::from(22),
                    target_return_pct: Decimal::from(10),
                    expected_growth_pct: Decimal::from(8),
                    strike_price_override: None,
                },
            }],
        };
        save_watchlist(&paths, &wl).unwrap();

        let journal = DecisionJournal {
            schema_version: 1,
            account_note: "PAPER".into(),
            currency: "USD".into(),
            starting_cash: Decimal::from(100_000),
            cash_balance: Decimal::from(100_000),
            benchmark_symbol: "^GSPC".into(),
            benchmark_start_price: None,
            positions: Vec::new(),
            entries: Vec::new(),
        };
        save_journal(&paths, &journal).unwrap();

        let entry = record_journal_entry(
            &paths,
            "TEST",
            JournalAction::Wait,
            "等安全边际",
            None,
            Some("毛利率 < 60%"),
        )
        .unwrap();
        assert_eq!(entry.action, JournalAction::Wait);

        let updated = update_company_assumptions(
            &paths,
            "TEST",
            ValuationAssumptions {
                multiple_low: Decimal::from(20),
                multiple_high: Decimal::from(24),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(9),
                strike_price_override: None,
            },
        )
        .unwrap();
        assert_eq!(updated.assumptions.multiple_low, Decimal::from(20));

        let moved = update_company_bucket(&paths, "TEST", Bucket::Out).unwrap();
        assert_eq!(moved.bucket, Bucket::Out);
        assert_eq!(moved.status, crate::model::StrikeZoneStatus::Out);
        assert_eq!(
            load_watchlist(&paths).unwrap().companies[0].bucket,
            Bucket::Out
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sec_cache_versioning_reextracts_v1_facts_and_reports_and_validates_cached_quotes() {
        let dir = std::env::temp_dir().join(format!("m3-store-ver-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        let now = now_ms();

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "ADI".into(),
                name: "Analog Devices".into(),
                bucket: Bucket::Core,
                source: DataSource::SecEdgar,
                cik: Some("0000006281".into()),
                currency: "USD".into(),
                circle: "Analog".into(),
                too_hard_reason: None,
                assumptions: ValuationAssumptions {
                    multiple_low: Decimal::from(18),
                    multiple_high: Decimal::from(22),
                    target_return_pct: Decimal::from(10),
                    expected_growth_pct: Decimal::from(8),
                    strike_price_override: None,
                },
            }],
        };
        save_watchlist(&paths, &wl).unwrap();

        let raw_sec = br#"{
            "entityName": "ANALOG DEVICES INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":11000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":6800000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":3000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":4200000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":600000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-11-01","val":2000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2025-11-01","val":8145066000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "LongTermDebtCurrent": { "units": { "USD": [
                    {"end":"2025-11-01","val":0,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "CommercialPaper": { "units": { "USD": [
                    {"end":"2025-11-01","val":446639000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "ShortTermBorrowings": { "units": { "USD": [
                    {"end":"2025-11-01","val":446639000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-11-01","val":35000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2024-11-03","end":"2025-11-01","val":496000000,"form":"10-K","filed":"2025-11-25"}
                ]}}
            }}
        }"#;
        fs::create_dir_all(paths.sec_raw_path("0000006281").parent().unwrap()).unwrap();
        fs::write(paths.sec_raw_path("0000006281"), raw_sec).unwrap();

        let mut stale_facts = edgar::parse_companyfacts_bytes(
            raw_sec,
            "ADI",
            "0000006281",
            "https://sec.test",
            now - 86_400_000,
        )
        .unwrap();
        stale_facts.schema_version = 1;
        stale_facts.extractor_version = 1;
        stale_facts.annuals[0].total_debt = Decimal::from(9_038_344_000i64);
        write_json_atomic(&paths.sec_facts_summary_path("ADI"), &stale_facts).unwrap();

        let mut stale_report =
            calculator::build_report(&wl.companies[0], Some(&stale_facts), None, now - 86_400_000);
        stale_report.schema_version = 1;
        write_json_atomic(&paths.computed_report_path("ADI"), &stale_report).unwrap();

        let stale_price = PriceSnapshot {
            schema_version: 1,
            symbol: "ADI".into(),
            price: Decimal::from(200),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(now - 20 * 86_400_000),
            fetched_at_ms: now,
            source_url: "https://price.test".into(),
        };
        write_json_atomic(&paths.price_path("ADI"), &stale_price).unwrap();
        assert!(load_cached_price(&paths, "ADI").is_none());

        let fresh_price = PriceSnapshot {
            schema_version: 1,
            symbol: "ADI".into(),
            price: Decimal::from(210),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(now - 3_600_000),
            fetched_at_ms: now,
            source_url: "https://price.test".into(),
        };
        write_json_atomic(&paths.price_path("ADI"), &fresh_price).unwrap();
        assert_eq!(
            load_cached_price(&paths, "ADI").unwrap().price,
            Decimal::from(210)
        );

        let reports = load_or_compute_reports(&paths).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].schema_version, 1);
        assert_eq!(reports[0].schema_version, calculator::REPORT_SCHEMA_VERSION);
        assert_eq!(
            reports[0].annual_rows[0].total_debt,
            Some(Decimal::from(8_591_705_000i64))
        );
        let upgraded_facts = load_cached_facts(&paths, &wl.companies[0]).unwrap();
        assert_eq!(
            upgraded_facts.schema_version,
            edgar::SEC_FACTS_SCHEMA_VERSION
        );
        assert_eq!(
            upgraded_facts.extractor_version,
            edgar::SEC_EXTRACTOR_VERSION
        );

        let recomp = recompute_ticker(&paths, "ADI").unwrap();
        assert_eq!(recomp.schema_version, 1);
        assert_eq!(recomp.schema_version, calculator::REPORT_SCHEMA_VERSION);
        assert_eq!(
            recomp.annual_rows[0].total_debt,
            Some(Decimal::from(8_591_705_000i64))
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn v1_migration_matches_preserved_sha_and_never_overwrites_usable_report_on_failure() {
        let dir = std::env::temp_dir().join(format!("m3-store-v1-safe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let now = now_ms();

        let wl = WatchlistFile {
            schema_version: 1,
            default_target_return_pct: Decimal::from(10),
            benchmark_symbol: "^GSPC".into(),
            companies: vec![
                CompanyEntry {
                    ticker: "ADI".into(),
                    name: "Analog Devices".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0000006281".into()),
                    currency: "USD".into(),
                    circle: "Analog".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(20),
                        multiple_high: Decimal::from(24),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(8),
                        strike_price_override: None,
                    },
                },
                CompanyEntry {
                    ticker: "KEEPV1".into(),
                    name: "Legacy Co".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0000999888".into()),
                    currency: "USD".into(),
                    circle: "Industrial".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(18),
                        multiple_high: Decimal::from(22),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(8),
                        strike_price_override: None,
                    },
                },
            ],
        };
        save_watchlist(&paths, &wl).unwrap();

        let good_adi_raw = br#"{
            "entityName": "ANALOG DEVICES INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":10000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":6000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":3000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":4000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":500000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-11-01","val":2000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2025-11-01","val":8145066000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "LongTermDebtCurrent": { "units": { "USD": [
                    {"end":"2025-11-01","val":446639000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-11-01","val":35000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2024-11-03","end":"2025-11-01","val":496000000,"form":"10-K","filed":"2025-11-25"}
                ]}}
            }}
        }"#;
        let adi_sha = edgar::hex_digest(good_adi_raw);

        // Corrupt the primary raw/sec file while keeping the exact SHA-matched file in preserved/.
        fs::create_dir_all(paths.sec_raw_path("0000006281").parent().unwrap()).unwrap();
        fs::write(paths.sec_raw_path("0000006281"), b"{corrupt-raw-sec").unwrap();
        let preserved_sha_path = paths
            .lake_preserved_raw_sec_dir()
            .join(format!("CIK0000006281-{adi_sha}.json"));
        fs::create_dir_all(preserved_sha_path.parent().unwrap()).unwrap();
        fs::write(&preserved_sha_path, good_adi_raw).unwrap();

        let mut stale_adi_facts = edgar::parse_companyfacts_bytes(
            good_adi_raw,
            "ADI",
            "0000006281",
            "https://sec.test",
            now - 10_000,
        )
        .unwrap();
        stale_adi_facts.extractor_version = 1;
        stale_adi_facts.annuals[0].total_debt = Decimal::from(999);
        write_json_atomic(&paths.sec_facts_summary_path("ADI"), &stale_adi_facts).unwrap();
        let stale_adi_report =
            calculator::build_report(&wl.companies[0], Some(&stale_adi_facts), None, now - 10_000);
        write_json_atomic(&paths.computed_report_path("ADI"), &stale_adi_report).unwrap();

        // KEEPV1 has a usable v1 computed report and stale sec_facts, but raw bytes are corrupt/missing everywhere.
        let mut keep_facts = stale_adi_facts.clone();
        keep_facts.ticker = "KEEPV1".into();
        keep_facts.cik = "0000999888".into();
        keep_facts.raw_sha256 = "missing_sha".into();
        keep_facts.annuals[0].total_debt = Decimal::from(123_456_000i64);
        write_json_atomic(&paths.sec_facts_summary_path("KEEPV1"), &keep_facts).unwrap();
        let usable_v1_keep =
            calculator::build_report(&wl.companies[1], Some(&keep_facts), None, now - 10_000);
        write_json_atomic(&paths.computed_report_path("KEEPV1"), &usable_v1_keep).unwrap();
        fs::write(paths.sec_raw_path("0000999888"), b"corrupt-keep-raw").unwrap();

        let reports = load_or_compute_reports(&paths).unwrap();
        assert_eq!(reports.len(), 2);
        // ADI rescued cleanly via exact SHA candidate in preserved/.
        assert_eq!(reports[0].schema_version, 1);
        assert_eq!(
            reports[0].annual_rows[0].total_debt,
            Some(Decimal::from(8_591_705_000i64))
        );
        // KEEPV1 usable v1 report is preserved intact on failure (not overwritten with empty/corrupt).
        assert_eq!(reports[1].schema_version, 1);
        assert_eq!(reports[1].annual_rows.len(), 1);
        assert_eq!(
            reports[1].annual_rows[0].total_debt,
            Some(Decimal::from(123_456_000i64))
        );
        let disk_keep = read_json::<MungerReport>(&paths.computed_report_path("KEEPV1")).unwrap();
        assert_eq!(disk_keep.annual_rows.len(), 1);
        assert_eq!(
            disk_keep.annual_rows[0].total_debt,
            Some(Decimal::from(123_456_000i64))
        );

        // Even if sec_facts_summary disappears later, recompute_ticker must NOT overwrite usable v1 report with None-facts empty report.
        let _ = fs::remove_file(paths.sec_facts_summary_path("KEEPV1"));
        let preserved_recomp = recompute_ticker(&paths, "KEEPV1").unwrap();
        assert_eq!(preserved_recomp.annual_rows.len(), 1);
        assert_eq!(
            read_json::<MungerReport>(&paths.computed_report_path("KEEPV1"))
                .unwrap()
                .annual_rows
                .len(),
            1
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_metadata_separation_quote_alignment_and_journal_valuation_freeze_work() {
        let dir = std::env::temp_dir().join(format!("m3-store-item46-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        let now = now_ms();

        let assumptions = ValuationAssumptions {
            multiple_low: Decimal::from(18),
            multiple_high: Decimal::from(22),
            target_return_pct: Decimal::from(10),
            expected_growth_pct: Decimal::from(8),
            strike_price_override: None,
        };
        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![
                CompanyEntry {
                    ticker: "GEN2".into(),
                    name: "Gen2 Corp".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0000100001".into()),
                    currency: "USD".into(),
                    circle: "Tech".into(),
                    too_hard_reason: None,
                    assumptions: assumptions.clone(),
                },
                CompanyEntry {
                    ticker: "FAILMIG".into(),
                    name: "FailMig Corp".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0000100002".into()),
                    currency: "USD".into(),
                    circle: "Tech".into(),
                    too_hard_reason: None,
                    assumptions: assumptions.clone(),
                },
            ],
        };
        save_watchlist(&paths, &wl).unwrap();
        write_json_atomic(
            &paths.sec_tickers_cache(),
            &serde_json::json!({
                "0": {"cik_str": 100001, "ticker": "GEN2", "title": "Gen2 Corp"}
            }),
        )
        .unwrap();
        save_journal(
            &paths,
            &DecisionJournal {
                schema_version: 1,
                account_note: "PAPER".into(),
                currency: "USD".into(),
                starting_cash: Decimal::from(100_000),
                cash_balance: Decimal::from(100_000),
                benchmark_symbol: "^GSPC".into(),
                benchmark_start_price: None,
                positions: Vec::new(),
                entries: Vec::new(),
            },
        )
        .unwrap();

        let make_raw = |rev: i64| -> Vec<u8> {
            format!(
                r#"{{
                    "entityName": "Gen2 Corp",
                    "facts": {{
                        "us-gaap": {{
                            "Revenues": {{ "units": {{ "USD": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":{rev},"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "GrossProfit": {{ "units": {{ "USD": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":600000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "OperatingIncomeLoss": {{ "units": {{ "USD": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":200000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "IncomeTaxExpenseBenefit": {{ "units": {{ "USD": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":40000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "NetCashProvidedByUsedInOperatingActivities": {{ "units": {{ "USD": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":180000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "PaymentsToAcquirePropertyPlantAndEquipment": {{ "units": {{ "USD": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":20000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "ShareBasedCompensation": {{ "units": {{ "USD": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "CashAndCashEquivalentsAtCarryingValue": {{ "units": {{ "USD": [
                                {{"end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "LongTermDebtNoncurrent": {{ "units": {{ "USD": [
                                {{"end":"2025-12-31","val":50000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "StockholdersEquity": {{ "units": {{ "USD": [
                                {{"end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "WeightedAverageNumberOfDilutedSharesOutstanding": {{ "units": {{ "shares": [
                                {{"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}},
                            "CommonStockSharesOutstanding": {{ "units": {{ "shares": [
                                {{"end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-01"}}
                            ]}}}}
                        }}
                    }}
                }}"#
            )
            .into_bytes()
        };

        let old_raw = make_raw(1_000_000_000);
        let new_raw = make_raw(1_200_000_000);
        let mut stale_facts = edgar::parse_companyfacts_bytes(
            &old_raw,
            "GEN2",
            "0000100001",
            "https://sec.old/v1.json",
            now - 50_000,
        )
        .unwrap();
        stale_facts.extractor_version = 1;
        write_json_atomic(&paths.sec_facts_summary_path("GEN2"), &stale_facts).unwrap();

        // Put a DIFFERENT source generation (new_raw) in raw/sec while old_raw is absent.
        fs::create_dir_all(paths.sec_raw_path("0000100001").parent().unwrap()).unwrap();
        fs::write(paths.sec_raw_path("0000100001"), &new_raw).unwrap();

        // Seed FAILMIG with usable report + changed assumptions + missing/corrupt raw bytes.
        let mut fail_facts = stale_facts.clone();
        fail_facts.ticker = "FAILMIG".into();
        fail_facts.cik = "0000100002".into();
        fail_facts.extractor_version = 1;
        write_json_atomic(&paths.sec_facts_summary_path("FAILMIG"), &fail_facts).unwrap();
        let verified_fail =
            calculator::build_report(&wl.companies[1], Some(&fail_facts), None, now - 50_000);
        write_json_atomic(&paths.computed_report_path("FAILMIG"), &verified_fail).unwrap();
        let mut wl_edited = wl.clone();
        wl_edited.companies[1].assumptions.multiple_low = Decimal::from(15);
        save_watchlist(&paths, &wl_edited).unwrap();

        // Seed GEN2 price = $100 first, load reports, then update price to $200.
        let p1 = PriceSnapshot {
            schema_version: 1,
            symbol: "GEN2".into(),
            price: Decimal::from(100),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(now - 3_600_000),
            fetched_at_ms: now - 3_000,
            source_url: "https://yahoo.test/1".into(),
        };
        write_json_atomic(&paths.price_path("GEN2"), &p1).unwrap();

        let reps = load_or_compute_reports(&paths).unwrap();
        // New source generation must NOT retain old summary's source_url / fetched_at_ms.
        let migrated_facts =
            read_json::<RawCompanyFactsSummary>(&paths.sec_facts_summary_path("GEN2")).unwrap();
        assert_ne!(migrated_facts.source_url, "https://sec.old/v1.json");
        assert_ne!(migrated_facts.fetched_at_ms, now - 50_000);
        assert_eq!(reps[0].price.as_ref().unwrap().price, Decimal::from(100));
        // FAILMIG failed migration must preserve last verified report even though assumptions changed.
        assert_eq!(reps[1].assumptions.multiple_low, Decimal::from(18));
        assert_eq!(
            read_json::<MungerReport>(&paths.computed_report_path("FAILMIG"))
                .unwrap()
                .assumptions
                .multiple_low,
            Decimal::from(18)
        );

        // Update GEN2 cached price to $200 WITHOUT recomputing report manually first;
        // both load_or_recompute_ticker and record_journal_entry must reflect $200 AND matching yield ($150M / $2B = 7.5%).
        let p2 = PriceSnapshot {
            price: Decimal::from(200),
            market_time_ms: Some(now - 1_800_000),
            fetched_at_ms: now - 1_000,
            ..p1
        };
        write_json_atomic(&paths.price_path("GEN2"), &p2).unwrap();
        let entry = record_journal_entry(
            &paths,
            "GEN2",
            JournalAction::PaperBuy,
            "Aligned test",
            None,
            None,
        )
        .unwrap();
        assert_eq!(entry.price, Some(Decimal::from(200)));
        // Strict OE = OCF 180 - capex 20 - sbc 10 = 150M; market cap = 200 * 10M = 2000M -> 7.50%.
        assert_eq!(entry.strict_fcf_yield_pct, Some(Decimal::new(750, 2)));

        // Round 8 check: TTM report cache hit must NOT rewrite computed/*.json on load_or_compute_reports,
        // and watchlisted/non-watchlisted warrants suppress strict_fcf_yield_pct in journal entries.
        let mut wl2 = load_watchlist(&paths).unwrap();
        wl2.companies.push(CompanyEntry {
            ticker: "SBCWW".into(),
            name: "SBC Warrant".into(),
            bucket: Bucket::Watch,
            source: DataSource::SecEdgar,
            cik: Some("0000009992".into()),
            currency: "USD".into(),
            circle: "Tech".into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(18),
                multiple_high: Decimal::from(22),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        });
        save_watchlist(&paths, &wl2).unwrap();
        let mut sbcww_facts = migrated_facts.clone();
        sbcww_facts.ticker = "SBCWW".into();
        sbcww_facts.cik = "0000009992".into();
        sbcww_facts.raw_sha256 = "sha-w".into();
        sbcww_facts.ttm = Some(crate::model::TtmFinancials {
            as_of_end: "2026-09-30".into(),
            label: "TTM (2026-09-30)".into(),
            revenue: Decimal::from(200),
            gross_profit: Decimal::from(140),
            ebit: Decimal::from(60),
            tax_expense: Decimal::from(12),
            pretax_income: Decimal::from(60),
            ocf: Decimal::from(180),
            capex: Decimal::from(20),
            sbc: Decimal::from(10),
            buybacks: Decimal::from(5),
            cash_and_st: Some(Decimal::from(50)),
            total_debt: Decimal::from(20),
            equity: Decimal::from(100),
            diluted_shares: Decimal::from(10),
            ending_shares: Decimal::from(10),
            latest_ytd: crate::model::YtdPeriodFinancials {
                period_label: "2026 Q3 YTD".into(),
                start: "2026-01-01".into(),
                end: "2026-09-30".into(),
                filed: None,
                revenue: Some(Decimal::from(150)),
                ocf: Some(Decimal::from(130)),
                capex: Some(Decimal::from(15)),
                sbc: Some(Decimal::from(7)),
                buybacks: Some(Decimal::from(3)),
                latest_quarter_label: None,
                latest_quarter_ocf: None,
                latest_quarter_capex: None,
                latest_quarter_sbc: None,
                latest_quarter_buybacks: None,
            },
            provenance: Vec::new(),
        });
        write_json_atomic(&paths.sec_facts_summary_path("SBCWW"), &sbcww_facts).unwrap();
        write_json_atomic(
            &paths.price_path("SBCWW"),
            &PriceSnapshot {
                symbol: "SBCWW".into(),
                price: Decimal::from(5),
                ..p2
            },
        )
        .unwrap();
        let _ = load_or_compute_reports(&paths).unwrap();
        let comp_path = paths.computed_report_path("SBCWW");
        let rep_before = fs::read(&comp_path).unwrap();
        let stamp_before = fs::read(paths.report_cache_versions_path()).unwrap();
        // Make computed report and stamp read-only to assert cache hit performs zero writes on TTM!
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&comp_path).unwrap().permissions();
            perms.set_mode(0o444);
            fs::set_permissions(&comp_path, perms).unwrap();
            let mut dir_perms = fs::metadata(comp_path.parent().unwrap())
                .unwrap()
                .permissions();
            dir_perms.set_mode(0o555);
            fs::set_permissions(comp_path.parent().unwrap(), dir_perms).unwrap();
        }
        let ro_reps = load_or_compute_reports(&paths).unwrap();
        let w_rep = ro_reps.iter().find(|r| r.ticker == "SBCWW").unwrap();
        assert_eq!(w_rep.latest_filing_end.as_deref(), Some("2026-09-30"));
        assert!(
            w_rep
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(
            w_rep
                .ttm_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut dir_perms = fs::metadata(comp_path.parent().unwrap())
                .unwrap()
                .permissions();
            dir_perms.set_mode(0o755);
            fs::set_permissions(comp_path.parent().unwrap(), dir_perms).unwrap();
            let mut perms = fs::metadata(&comp_path).unwrap().permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&comp_path, perms).unwrap();
        }
        assert_eq!(fs::read(&comp_path).unwrap(), rep_before);
        assert_eq!(
            fs::read(paths.report_cache_versions_path()).unwrap(),
            stamp_before
        );

        let w_entry = record_journal_entry(
            &paths,
            "SBCWW",
            JournalAction::PaperBuy,
            "Warrant check",
            None,
            None,
        )
        .unwrap();
        assert_eq!(w_entry.price, Some(Decimal::from(5)));
        assert_eq!(w_entry.strict_fcf_yield_pct, None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round10_journal_rejects_expired_quotes_and_suppresses_class_mismatch_yield() {
        let dir =
            std::env::temp_dir().join(format!("m3-store-round10-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        let now = now_ms();
        let stale_ms = now.saturating_sub(15 * 86_400_000);
        let fresh_ms = now.saturating_sub(3_600_000);

        let make_entry = |ticker: &str, name: &str, cik: &str| CompanyEntry {
            ticker: ticker.into(),
            name: name.into(),
            bucket: Bucket::Core,
            source: DataSource::SecEdgar,
            cik: Some(cik.into()),
            currency: "USD".into(),
            circle: "Tech".into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(15),
                multiple_high: Decimal::from(20),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(10),
                strike_price_override: None,
            },
        };
        let mut wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![
                make_entry("EXPW", "Expired Watchlist Inc.", "0000009801"),
                make_entry("EXP2", "Expired Removed Inc.", "0000009802"),
                make_entry("BRK-B", "Berkshire Hathaway B", "0001067983"),
            ],
        };
        save_watchlist(&paths, &wl).unwrap();
        write_json_atomic(
            &paths.sec_tickers_cache(),
            &serde_json::json!({
                "0": {"cik_str": 100001, "ticker": "GEN2", "title": "Gen2 Corp"}
            }),
        )
        .unwrap();
        save_journal(
            &paths,
            &DecisionJournal {
                schema_version: 1,
                account_note: "PAPER".into(),
                currency: "USD".into(),
                starting_cash: Decimal::from(100_000),
                cash_balance: Decimal::from(100_000),
                benchmark_symbol: "^GSPC".into(),
                benchmark_start_price: None,
                positions: Vec::new(),
                entries: Vec::new(),
            },
        )
        .unwrap();

        let make_facts = |ticker: &str, cik: &str| RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: ticker.into(),
            entity_name: format!("{ticker} Corp"),
            cik: cik.into(),
            source_url: format!("https://sec.test/{ticker}.json"),
            raw_sha256: format!("sha-{ticker}"),
            fetched_at_ms: fresh_ms,
            annuals: (2021..=2025)
                .map(|fy| crate::model::AnnualFinancials {
                    fiscal_year: fy,
                    period_end: format!("{fy}-12-31"),
                    filed: Some(format!("{}-02-15", fy + 1)),
                    revenue: Decimal::from(500),
                    gross_profit: Decimal::from(350),
                    ebit: Decimal::from(150),
                    tax_expense: Decimal::from(30),
                    pretax_income: Decimal::from(150),
                    ocf: Decimal::from(180),
                    capex: Decimal::from(20),
                    sbc: Decimal::from(10),
                    buybacks: Decimal::ZERO,
                    cash_and_st: Some(Decimal::from(50)),
                    total_debt: Decimal::from(20),
                    equity: Decimal::from(400),
                    diluted_shares: Decimal::from(10),
                    ending_shares: Decimal::from(10),
                    provenance: Vec::new(),
                })
                .collect(),
            ttm: Some(crate::model::TtmFinancials {
                as_of_end: "2026-09-30".into(),
                label: "TTM (2026-09-30)".into(),
                revenue: Decimal::from(520),
                gross_profit: Decimal::from(360),
                ebit: Decimal::from(155),
                tax_expense: Decimal::from(31),
                pretax_income: Decimal::from(155),
                ocf: Decimal::from(180),
                capex: Decimal::from(20),
                sbc: Decimal::from(10),
                buybacks: Decimal::ZERO,
                cash_and_st: Some(Decimal::from(50)),
                total_debt: Decimal::from(20),
                equity: Decimal::from(410),
                diluted_shares: Decimal::from(10),
                ending_shares: Decimal::from(10),
                latest_ytd: crate::model::YtdPeriodFinancials {
                    period_label: "2026 Q3 YTD".into(),
                    start: "2026-01-01".into(),
                    end: "2026-09-30".into(),
                    filed: None,
                    revenue: Some(Decimal::from(390)),
                    ocf: Some(Decimal::from(130)),
                    capex: Some(Decimal::from(15)),
                    sbc: Some(Decimal::from(7)),
                    buybacks: Some(Decimal::ZERO),
                    latest_quarter_label: None,
                    latest_quarter_ocf: None,
                    latest_quarter_capex: None,
                    latest_quarter_sbc: None,
                    latest_quarter_buybacks: None,
                },
                provenance: Vec::new(),
            }),
            manual_note: None,
        };
        let make_price = |sym: &str, px: i64, mt_ms: u64| PriceSnapshot {
            schema_version: 1,
            symbol: sym.into(),
            price: Decimal::from(px),
            previous_close: Some(Decimal::from(px - 1)),
            currency: "USD".into(),
            exchange: "NASDAQ".into(),
            market_time_ms: Some(mt_ms),
            fetched_at_ms: now,
            source_url: format!("https://yahoo.test/{sym}"),
        };

        for (ticker, cik, px) in [
            ("EXPW", "0000009801", 100),
            ("EXP2", "0000009802", 100),
            ("BRK-B", "0001067983", 450),
        ] {
            write_json_atomic(
                &paths.sec_facts_summary_path(ticker),
                &make_facts(ticker, cik),
            )
            .unwrap();
            write_json_atomic(&paths.price_path(ticker), &make_price(ticker, px, fresh_ms))
                .unwrap();
        }
        let wl_reports = load_or_compute_reports(&paths).unwrap();
        assert_eq!(wl_reports.len(), 3);

        // 1. Expired disk + prev quote under StaleReextractFailed AND direct record_journal_entry:
        // both watchlisted (EXPW) and non-watchlisted (EXP2) must reject expired quotes.
        for ticker in ["EXPW", "EXP2"] {
            let mut prev: MungerReport = read_json(&paths.computed_report_path(ticker)).unwrap();
            prev.price = Some(make_price(ticker, 100, stale_ms));
            write_json_atomic(&paths.computed_report_path(ticker), &prev).unwrap();
            write_json_atomic(
                &paths.price_path(ticker),
                &make_price(ticker, 101, stale_ms),
            )
            .unwrap();
            fs::write(paths.sec_facts_summary_path(ticker), b"{corrupt").unwrap();
        }
        let expw_report = load_or_recompute_ticker(&paths, "EXPW").unwrap();
        assert!(expw_report.price.is_none());
        assert_eq!(
            expw_report
                .ttm_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );
        let exp2_report = load_or_recompute_ticker(&paths, "EXP2").unwrap();
        assert!(exp2_report.price.is_none());
        assert_eq!(
            exp2_report
                .ttm_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );

        // Remove EXP2 from watchlist to exercise the !in_watchlist branch in record_journal_entry.
        wl.companies.retain(|c| c.ticker != "EXP2");
        save_watchlist(&paths, &wl).unwrap();

        // Even if a pre-fix expired report + expired disk quote remained cached on disk,
        // record_journal_entry rejects the expired price & yield for both watchlist & non-watchlist.
        for ticker in ["EXPW", "EXP2"] {
            let mut prev: MungerReport = read_json(&paths.computed_report_path(ticker)).unwrap();
            prev.price = Some(make_price(ticker, 100, stale_ms));
            prev.ttm_valuation
                .as_mut()
                .unwrap()
                .strict_fcf_yield_mcap_pct = Some(Decimal::from(15));
            write_json_atomic(&paths.computed_report_path(ticker), &prev).unwrap();
            let entry = record_journal_entry(
                &paths,
                ticker,
                JournalAction::PaperBuy,
                "Expired check",
                None,
                None,
            )
            .unwrap();
            assert_eq!(entry.price, None);
            assert_eq!(entry.strict_fcf_yield_pct, None);
        }

        // 2. Class-mismatched BRK-B with stale unguarded cached report under StaleReextractFailed
        // AND direct !in_watchlist record_journal_entry suppresses strict_fcf_yield_pct.
        let mut brk_prev: MungerReport = read_json(&paths.computed_report_path("BRK-B")).unwrap();
        brk_prev.warnings.clear();
        brk_prev
            .ttm_valuation
            .as_mut()
            .unwrap()
            .strict_fcf_yield_mcap_pct = Some(Decimal::from(5));
        brk_prev
            .fy_valuation
            .as_mut()
            .unwrap()
            .strict_fcf_yield_mcap_pct = Some(Decimal::from(4));
        write_json_atomic(&paths.computed_report_path("BRK-B"), &brk_prev).unwrap();
        write_json_atomic(
            &paths.price_path("BRK-B"),
            &make_price("BRK-B", 451, fresh_ms),
        )
        .unwrap();
        fs::write(paths.sec_facts_summary_path("BRK-B"), b"{corrupt").unwrap();

        let brk_rep = load_or_recompute_ticker(&paths, "BRK-B").unwrap();
        assert_eq!(
            brk_rep.price.as_ref().map(|p| p.price),
            Some(Decimal::from(451))
        );
        assert_eq!(
            brk_rep
                .ttm_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );
        assert_eq!(
            brk_rep
                .fy_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );

        wl.companies.retain(|c| c.ticker == "EXPW");
        save_watchlist(&paths, &wl).unwrap();
        brk_prev.price = Some(make_price("BRK-B", 451, fresh_ms));
        brk_prev.warnings.clear();
        brk_prev
            .ttm_valuation
            .as_mut()
            .unwrap()
            .strict_fcf_yield_mcap_pct = Some(Decimal::from(5));
        write_json_atomic(&paths.computed_report_path("BRK-B"), &brk_prev).unwrap();
        let brk_entry = record_journal_entry(
            &paths,
            "BRK-B",
            JournalAction::PaperBuy,
            "Share class guard",
            None,
            None,
        )
        .unwrap();
        assert_eq!(brk_entry.price, Some(Decimal::from(451)));
        assert_eq!(brk_entry.strict_fcf_yield_pct, None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round11_report_fallback_recomputes_strike_status_and_suppressed_valuations() {
        let dir =
            std::env::temp_dir().join(format!("m3-store-round11-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        let now = now_ms();
        let fresh_ms = now.saturating_sub(3_600_000);
        let stale_ms = now.saturating_sub(15 * 86_400_000);

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![
                CompanyEntry {
                    ticker: "STZ".into(),
                    name: "Strike Co".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0000009901".into()),
                    currency: "USD".into(),
                    circle: "Tech".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(20),
                        multiple_high: Decimal::from(25),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(10),
                        strike_price_override: None,
                    },
                },
                CompanyEntry {
                    ticker: "BRK-B".into(),
                    name: "Berkshire B".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0001067983".into()),
                    currency: "USD".into(),
                    circle: "Finance".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(20),
                        multiple_high: Decimal::from(25),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(10),
                        strike_price_override: None,
                    },
                },
            ],
        };
        save_watchlist(&paths, &wl).unwrap();

        let make_facts = |ticker: &str, cik: &str| RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: ticker.into(),
            entity_name: format!("{ticker} Corp"),
            cik: cik.into(),
            source_url: format!("https://sec.test/{ticker}.json"),
            raw_sha256: format!("sha-{ticker}"),
            fetched_at_ms: fresh_ms,
            annuals: (2021..=2025)
                .map(|fy| crate::model::AnnualFinancials {
                    fiscal_year: fy,
                    period_end: format!("{fy}-12-31"),
                    filed: Some(format!("{}-02-15", fy + 1)),
                    revenue: Decimal::from(500),
                    gross_profit: Decimal::from(350),
                    ebit: Decimal::from(150),
                    tax_expense: Decimal::from(30),
                    pretax_income: Decimal::from(150),
                    ocf: Decimal::from(180),
                    capex: Decimal::from(20),
                    sbc: Decimal::from(10),
                    buybacks: Decimal::ZERO,
                    cash_and_st: Some(Decimal::from(50)),
                    total_debt: Decimal::from(20),
                    equity: Decimal::from(400),
                    diluted_shares: Decimal::from(10),
                    ending_shares: Decimal::from(10),
                    provenance: Vec::new(),
                })
                .collect(),
            ttm: None,
            manual_note: None,
        };
        let make_price = |sym: &str, px: i64, mt_ms: u64| PriceSnapshot {
            schema_version: 1,
            symbol: sym.into(),
            price: Decimal::from(px),
            previous_close: Some(Decimal::from(px - 1)),
            currency: "USD".into(),
            exchange: "NASDAQ".into(),
            market_time_ms: Some(mt_ms),
            fetched_at_ms: now,
            source_url: format!("https://yahoo.test/{sym}"),
        };

        // Initial compute puts STZ in strike zone (OE = 150, shares = 10 -> OE/sh = 15, strike @20x = 300, px = 200).
        write_json_atomic(
            &paths.sec_tickers_cache(),
            &serde_json::json!({
                "0": {"cik_str": 9901, "ticker": "STZ", "title": "Strike Co"}
            }),
        )
        .unwrap();
        write_json_atomic(
            &paths.sec_facts_summary_path("STZ"),
            &make_facts("STZ", "0000009901"),
        )
        .unwrap();
        write_json_atomic(&paths.price_path("STZ"), &make_price("STZ", 200, fresh_ms)).unwrap();
        write_json_atomic(
            &paths.sec_facts_summary_path("BRK-B"),
            &make_facts("BRK-B", "0001067983"),
        )
        .unwrap();
        write_json_atomic(
            &paths.price_path("BRK-B"),
            &make_price("BRK-B", 200, fresh_ms),
        )
        .unwrap();
        let initial = load_or_compute_reports(&paths).unwrap();
        assert_eq!(
            initial[0].status,
            crate::model::StrikeZoneStatus::InStrikeZone
        );
        assert_eq!(initial[1].status, crate::model::StrikeZoneStatus::Wait);

        // 1. Corrupt/expire STZ price & force StaleReextractFailed (unmigrated summary with mutated
        //    total_debt=999 while raw/sec is corrupt); status MUST flip InStrikeZone -> Wait AND
        //    verified fundamental rows (total_debt=20) MUST NOT be overwritten by stale_facts.
        write_json_atomic(&paths.price_path("STZ"), &make_price("STZ", 200, stale_ms)).unwrap();
        let mut stale_stz_facts = make_facts("STZ", "0000009901");
        stale_stz_facts.extractor_version = 1;
        stale_stz_facts.annuals[0].total_debt = Decimal::from(999);
        write_json_atomic(&paths.sec_facts_summary_path("STZ"), &stale_stz_facts).unwrap();
        fs::create_dir_all(paths.sec_raw_path("0000009901").parent().unwrap()).unwrap();
        fs::write(paths.sec_raw_path("0000009901"), b"{corrupt-raw").unwrap();
        // Also simulate a legacy cached BRK-B report that had stale InStrikeZone + unguarded valuation.
        let mut stale_brk: MungerReport = read_json(&paths.computed_report_path("BRK-B")).unwrap();
        stale_brk.status = crate::model::StrikeZoneStatus::InStrikeZone;
        stale_brk.warnings.clear();
        stale_brk.fy_valuation.as_mut().unwrap().strike_price = Some(Decimal::from(300));
        stale_brk
            .fy_valuation
            .as_mut()
            .unwrap()
            .distance_to_strike_pct = Some(Decimal::from(-33));
        write_json_atomic(&paths.computed_report_path("BRK-B"), &stale_brk).unwrap();

        let aligned = load_or_compute_reports(&paths).unwrap();
        assert!(aligned[0].price.is_none());
        assert_eq!(
            aligned[0].annual_rows[0].total_debt,
            Some(Decimal::from(20))
        );
        assert_eq!(
            aligned[0]
                .fy_valuation
                .as_ref()
                .and_then(|v| v.distance_to_strike_pct),
            None
        );
        assert_eq!(aligned[0].status, crate::model::StrikeZoneStatus::Wait);

        assert_eq!(aligned[1].status, crate::model::StrikeZoneStatus::Wait);
        assert!(
            aligned[1]
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(
            aligned[1]
                .fy_valuation
                .as_ref()
                .and_then(|v| v.strike_price),
            None
        );

        // 2. Interactive recompute_ticker under StaleReextractFailed with new price & assumptions
        //    MUST reprice verified fundamental rows (total_debt still 20, NOT 999 from stale summary!)
        //    and reflect new assumptions/bucket/status cleanly.
        write_json_atomic(&paths.price_path("STZ"), &make_price("STZ", 200, fresh_ms)).unwrap();
        let mut wl_edited = wl.clone();
        wl_edited.companies[0].assumptions.multiple_low = Decimal::from(30);
        save_watchlist(&paths, &wl_edited).unwrap();
        let repriced_stz = recompute_ticker(&paths, "STZ").unwrap();
        assert_eq!(
            repriced_stz.annual_rows[0].total_debt,
            Some(Decimal::from(20)),
            "verified fundamental rows must be preserved, not rebuilt from stale summary"
        );
        assert_eq!(
            repriced_stz
                .fy_valuation
                .as_ref()
                .and_then(|v| v.strike_price),
            Some(Decimal::from(450))
        );
        assert_eq!(
            repriced_stz.status,
            crate::model::StrikeZoneStatus::InStrikeZone
        );

        // 3. Bucket edit + missing/corrupt facts also aligns status & strike via recompute_ticker.
        fs::write(paths.sec_facts_summary_path("STZ"), b"{corrupt").unwrap();
        let mut wl_out = wl;
        wl_out.companies[0].bucket = Bucket::Out;
        save_watchlist(&paths, &wl_out).unwrap();
        let recomp_stz = recompute_ticker(&paths, "STZ").unwrap();
        assert_eq!(recomp_stz.bucket, Bucket::Out);
        assert_eq!(recomp_stz.status, crate::model::StrikeZoneStatus::Out);

        // 4. resolve_active_sec_raw_file prioritizes committed DuckDB WM_SEC_INPUT_GEN over stale CURRENT.json.
        let gen1 = paths.lake_raw_sec_generations_dir().join("gen-001");
        let gen2 = paths.lake_raw_sec_generations_dir().join("gen-002");
        fs::create_dir_all(&gen1).unwrap();
        fs::create_dir_all(&gen2).unwrap();
        fs::write(gen1.join("companyfacts.zip"), b"gen1-zip").unwrap();
        fs::write(gen2.join("companyfacts.zip"), b"gen2-zip").unwrap();
        write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &serde_json::json!({
                "schema_version": 1,
                "generation_id": "gen-001",
                "generation_dir": "gen-001",
                "committed_at_ms": 1
            }),
        )
        .unwrap();
        {
            let db = crate::lake::LakeDb::open(&paths).unwrap();
            db.write_watermark(&crate::lake::SyncWatermark {
                source_key: crate::lake::WM_SEC_INPUT_GEN.to_owned(),
                cursor: Some("gen-002".to_owned()),
                etag: None,
                last_modified: None,
                synced_at_ms: 2,
            })
            .unwrap();
        }
        assert_eq!(
            paths.resolve_active_sec_raw_file("companyfacts.zip"),
            gen2.join("companyfacts.zip")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round13_raw_asset_resolution_rejects_broken_pointers_and_cached_quote_checks_symbol() {
        let dir = std::env::temp_dir().join(format!("m3-store-r13-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let fresh_ms = now_ms();

        // 1. Cached quote verification rejects mismatched symbol stored under ticker path.
        let wrong_quote = PriceSnapshot {
            schema_version: 1,
            symbol: "MSFT".into(),
            price: Decimal::from(400),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NASDAQ".into(),
            market_time_ms: Some(fresh_ms),
            fetched_at_ms: fresh_ms,
            source_url: "https://yahoo.test/MSFT".into(),
        };
        write_json_atomic(&paths.price_path("AAPL"), &wrong_quote).unwrap();
        assert!(load_cached_price(&paths, "AAPL").is_none());

        // 2. True legacy workspace (no DB, no CURRENT.json) resolves flat raw/sec alias.
        let legacy_zip = paths.lake_raw_sec_dir().join("companyfacts.zip");
        fs::create_dir_all(legacy_zip.parent().unwrap()).unwrap();
        fs::write(&legacy_zip, b"legacy-zip").unwrap();
        assert_eq!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .unwrap(),
            Some(legacy_zip.clone())
        );

        // 3. Corrupt or missing CURRENT.json rejects flat raw/sec fallback loudly.
        fs::write(paths.lake_raw_sec_manifest_path(), b"{corrupt").unwrap();
        assert!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .is_err()
        );
        write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &serde_json::json!({
                "schema_version": 1,
                "generation_id": "gen-missing",
                "generation_dir": "gen-missing",
                "committed_at_ms": 1
            }),
        )
        .unwrap();
        assert!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .is_err()
        );

        // 4. Valid CURRENT.json fails loudly when DB WM_SEC_INPUT_GEN points to missing generation or DB is unreadable.
        let gen_ok = paths.lake_raw_sec_generations_dir().join("gen-001");
        fs::create_dir_all(&gen_ok).unwrap();
        fs::write(gen_ok.join("companyfacts.zip"), b"gen1-zip").unwrap();
        write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &serde_json::json!({
                "schema_version": 1,
                "generation_id": "gen-001",
                "generation_dir": "gen-001",
                "committed_at_ms": 1
            }),
        )
        .unwrap();
        assert_eq!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .unwrap(),
            Some(gen_ok.join("companyfacts.zip"))
        );
        {
            let db = crate::lake::LakeDb::open(&paths).unwrap();
            db.write_watermark(&crate::lake::SyncWatermark {
                source_key: crate::lake::WM_SEC_INPUT_GEN.to_owned(),
                cursor: Some("gen-dead".to_owned()),
                etag: None,
                last_modified: None,
                synced_at_ms: 3,
            })
            .unwrap();
        }
        assert!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .is_err()
        );
        fs::write(paths.lake_db_path(), b"corrupt-duckdb-bytes").unwrap();
        assert!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .is_err()
        );

        #[cfg(unix)]
        {
            let _ = fs::remove_file(paths.lake_db_path());
            let _ = fs::remove_file(paths.lake_raw_sec_manifest_path());
            std::os::unix::fs::symlink(
                paths
                    .lake_raw_sec_dir()
                    .join("missing-manifest-target.json"),
                paths.lake_raw_sec_manifest_path(),
            )
            .unwrap();
            assert!(
                paths
                    .resolve_active_sec_raw_file_checked("companyfacts.zip")
                    .is_err(),
                "dangling CURRENT.json symlink must propagate error instead of falling back to legacy flat path"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round15_manifest_agreement_dot_rejection_and_v1_sync_ccz_separation() {
        let dir = std::env::temp_dir().join(format!("m3-store-r15-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let legacy_zip = paths.lake_raw_sec_dir().join("companyfacts.zip");
        fs::create_dir_all(legacy_zip.parent().unwrap()).unwrap();
        fs::write(&legacy_zip, b"legacy").unwrap();

        // 1. CURRENT.json pointing to "." or mismatched generation_id != generation_dir must error.
        write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &serde_json::json!({
                "schema_version": 1,
                "generation_id": ".",
                "generation_dir": ".",
                "committed_at_ms": 1
            }),
        )
        .unwrap();
        assert!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .is_err()
        );
        let gen1 = paths.lake_raw_sec_generations_dir().join("gen-001");
        fs::create_dir_all(&gen1).unwrap();
        fs::write(gen1.join("companyfacts.zip"), b"g1").unwrap();
        write_json_atomic(
            &paths.lake_raw_sec_manifest_path(),
            &serde_json::json!({
                "schema_version": 1,
                "generation_id": "gen-002",
                "generation_dir": "gen-001",
                "committed_at_ms": 1
            }),
        )
        .unwrap();
        assert!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .is_err()
        );
        let _ = fs::remove_file(paths.lake_raw_sec_manifest_path());

        // 2. Present DuckDB WM_SEC_INPUT_GEN row with NULL/blank cursor fails loudly instead of falling back.
        {
            let db = crate::lake::LakeDb::open(&paths).unwrap();
            db.write_watermark(&crate::lake::SyncWatermark {
                source_key: crate::lake::WM_SEC_INPUT_GEN.to_owned(),
                cursor: None,
                etag: None,
                last_modified: None,
                synced_at_ms: 2,
            })
            .unwrap();
        }
        assert!(
            paths
                .resolve_active_sec_raw_file_checked("companyfacts.zip")
                .is_err()
        );

        // 3. Normal v1 sync path for watchlisted debenture CCZ resolves issuer facts ticker to CMCSA
        //    even when company_tickers.json lists CCZ ahead of CMCSA, enforcing SHARE_BASIS_MISMATCH.
        let _ = fs::remove_file(paths.lake_db_path());
        fs::create_dir_all(paths.sec_tickers_cache().parent().unwrap()).unwrap();
        let v1_tickers = serde_json::json!({
            "0": {"cik_str": 1166691, "ticker": "CCZ", "title": "COMCAST CORP"},
            "1": {"cik_str": 1166691, "ticker": "CMCSA", "title": "COMCAST CORP"}
        });
        write_json_atomic(&paths.sec_tickers_cache(), &v1_tickers).unwrap();

        let cmcsa_raw = serde_json::json!({
            "entityName": "COMCAST CORP",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":120000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "GrossProfit": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":80000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "OperatingIncomeLoss": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":23000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":28000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":12000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[{"end":"2025-12-31","val":6000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "LongTermDebtNoncurrent": {"units":{"USD":[{"end":"2025-12-31","val":90000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "StockholdersEquity": {"units":{"USD":[{"end":"2025-12-31","val":85000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[{"start":"2025-01-01","end":"2025-12-31","val":3900000000_i64,"form":"10-K","filed":"2026-02-01"}]}}
            }}
        });
        let raw_body = serde_json::to_vec(&cmcsa_raw).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).unwrap();
                if n == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                raw_body.len()
            );
            stream.write_all(hdr.as_bytes()).unwrap();
            stream.write_all(&raw_body).unwrap();
            stream.flush().unwrap();
        });

        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let mut ccz_co = CompanyEntry {
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
        };
        let facts_url = format!("http://{addr}/CIK0001166691.json");
        sync_v1_sec_company_with_url(&paths, &client, &mut ccz_co, Some(&facts_url), 1000).unwrap();
        server.join().unwrap();

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![ccz_co.clone()],
        };
        save_watchlist(&paths, &wl).unwrap();
        let snap = PriceSnapshot {
            schema_version: 1,
            symbol: "CCZ".into(),
            price: Decimal::from(25),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NYSE".into(),
            market_time_ms: Some(now_ms()),
            fetched_at_ms: now_ms(),
            source_url: "https://test".into(),
        };
        write_json_atomic(&paths.price_path("CCZ"), &snap).unwrap();

        let summary = load_cached_facts(&paths, &ccz_co).expect("cached summary for CCZ");
        assert_eq!(summary.ticker, "CMCSA");
        let report = load_or_recompute_ticker(&paths, "CCZ").unwrap();
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(report.fy_valuation.as_ref().unwrap().strike_price, None);
        assert_eq!(
            report
                .fy_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round16_v1_unavailable_index_marks_unresolved_preserves_verified_and_suppresses_valuation() {
        let dir = std::env::temp_dir().join(format!("m3-store-r16-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.sec_tickers_cache().parent().unwrap()).unwrap();

        let make_raw = |entity_name: &str| -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "entityName": entity_name,
                "facts": { "us-gaap": {
                    "Revenues": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":120000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "GrossProfit": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":80000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "OperatingIncomeLoss": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":23000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":28000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":12000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "ShareBasedCompensation": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":1000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[{"end":"2025-12-31","val":6000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "LongTermDebtNoncurrent": {"units":{"USD":[{"end":"2025-12-31","val":90000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "StockholdersEquity": {"units":{"USD":[{"end":"2025-12-31","val":85000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[{"start":"2025-01-01","end":"2025-12-31","val":3900000000_i64,"form":"10-K","filed":"2026-02-01"}]}}
                }}
            }))
            .unwrap()
        };
        let cmcsa_body = make_raw("COMCAST CORP");
        let aapl_body = make_raw("Apple Inc.");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut req_line = String::new();
                reader.read_line(&mut req_line).unwrap();
                loop {
                    let mut line = String::new();
                    let n = reader.read_line(&mut line).unwrap();
                    if n == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                }
                if req_line.contains("company_tickers.json") {
                    stream
                        .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
                } else {
                    let body = if req_line.contains("CIK0000320193") {
                        &aapl_body
                    } else {
                        &cmcsa_body
                    };
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(hdr.as_bytes()).unwrap();
                    stream.write_all(body).unwrap();
                }
                stream.flush().unwrap();
            }
        });

        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let tickers_url = format!("http://{addr}/company_tickers.json");
        let ccz_facts_url = format!("http://{addr}/CIK0001166691.json");
        let aapl_facts_url = format!("http://{addr}/CIK0000320193.json");

        assert_eq!(
            resolve_v1_facts_canonical_ticker_with_url(
                &paths,
                Some(&client),
                "0001166691",
                Some(&tickers_url),
            ),
            CanonicalTickerResolution::Unresolved
        );

        let assumptions = ValuationAssumptions {
            multiple_low: Decimal::from(15),
            multiple_high: Decimal::from(20),
            target_return_pct: Decimal::from(10),
            expected_growth_pct: Decimal::from(8),
            strike_price_override: None,
        };
        let mut ccz_co = CompanyEntry {
            ticker: "CCZ".into(),
            name: "Comcast Exchangeable Debenture".into(),
            bucket: Bucket::Core,
            source: DataSource::SecEdgar,
            cik: Some("0001166691".into()),
            currency: "USD".into(),
            circle: "Media".into(),
            too_hard_reason: None,
            assumptions: assumptions.clone(),
        };
        let mut aapl_co = CompanyEntry {
            ticker: "AAPL".into(),
            name: "Apple Inc.".into(),
            bucket: Bucket::Core,
            source: DataSource::SecEdgar,
            cik: Some("0000320193".into()),
            currency: "USD".into(),
            circle: "Tech".into(),
            too_hard_reason: None,
            assumptions,
        };
        save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![ccz_co.clone(), aapl_co.clone()],
            },
        )
        .unwrap();
        for ticker in ["CCZ", "AAPL"] {
            write_json_atomic(
                &paths.price_path(ticker),
                &PriceSnapshot {
                    schema_version: 1,
                    symbol: ticker.into(),
                    price: Decimal::from(25),
                    previous_close: None,
                    currency: "USD".into(),
                    exchange: "NYSE".into(),
                    market_time_ms: Some(now_ms()),
                    fetched_at_ms: now_ms(),
                    source_url: "https://test".into(),
                },
            )
            .unwrap();
        }

        // 1. Fresh v1 sync with unavailable indexes marks ticker UNRESOLVED (not "CCZ"!) and suppresses valuation.
        sync_v1_sec_company_with_urls(
            &paths,
            &client,
            &mut ccz_co,
            Some(&ccz_facts_url),
            Some(&tickers_url),
            1000,
        )
        .unwrap();
        let ccz_summary = load_cached_facts(&paths, &ccz_co).unwrap();
        assert_eq!(ccz_summary.ticker, edgar::UNRESOLVED_CANONICAL_TICKER);
        let ccz_rep = load_or_recompute_ticker(&paths, "CCZ").unwrap();
        assert!(ccz_rep.warnings.iter().any(|w| {
            w.code == "SHARE_BASIS_MISMATCH" && w.title == calculator::UNRESOLVED_SHARE_BASIS_TITLE
        }));
        assert_eq!(ccz_rep.fy_valuation.as_ref().unwrap().strike_price, None);
        assert_eq!(
            ccz_rep
                .fy_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );

        // Same for common equity synced while indexes are unavailable: suppressed until resolved!
        sync_v1_sec_company_with_urls(
            &paths,
            &client,
            &mut aapl_co,
            Some(&aapl_facts_url),
            Some(&tickers_url),
            1000,
        )
        .unwrap();
        let aapl_unres_rep = load_or_recompute_ticker(&paths, "AAPL").unwrap();
        assert_eq!(
            aapl_unres_rep.fy_valuation.as_ref().unwrap().strike_price,
            None
        );

        // 2. Once SEC index becomes available, CCZ resolves CMCSA (stays suppressed) and AAPL unlocks strike_price.
        write_json_atomic(
            &paths.sec_tickers_cache(),
            &serde_json::json!({
                "0": {"cik_str": 1166691, "ticker": "CCZ", "title": "COMCAST CORP"},
                "1": {"cik_str": 1166691, "ticker": "CMCSA", "title": "COMCAST CORP"},
                "2": {"cik_str": 320193, "ticker": "AAPL", "title": "Apple Inc."}
            }),
        )
        .unwrap();
        let ccz_resolved_summary = load_cached_facts(&paths, &ccz_co).unwrap();
        assert_eq!(ccz_resolved_summary.ticker, "CMCSA");
        let ccz_resolved_rep = load_or_recompute_ticker(&paths, "CCZ").unwrap();
        assert!(
            ccz_resolved_rep
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(
            ccz_resolved_rep.fy_valuation.as_ref().unwrap().strike_price,
            None
        );

        let aapl_resolved_summary = load_cached_facts(&paths, &aapl_co).unwrap();
        assert_eq!(aapl_resolved_summary.ticker, "AAPL");
        let aapl_resolved_rep = load_or_recompute_ticker(&paths, "AAPL").unwrap();
        assert!(
            !aapl_resolved_rep
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert!(
            aapl_resolved_rep
                .fy_valuation
                .as_ref()
                .unwrap()
                .strike_price
                .is_some()
        );

        // 3. When a verified summary already exists on disk, unavailable SEC indexes preserve it untouched.
        let _ = fs::remove_file(paths.sec_tickers_cache());
        let preserve_res = sync_v1_sec_company_with_urls(
            &paths,
            &client,
            &mut ccz_co,
            Some(&ccz_facts_url),
            Some(&tickers_url),
            2000,
        );
        assert!(preserve_res.is_err());
        let after_summary =
            read_json::<RawCompanyFactsSummary>(&paths.sec_facts_summary_path("CCZ")).unwrap();
        assert_eq!(after_summary.ticker, "CMCSA");
        assert_eq!(after_summary.fetched_at_ms, 1000);

        server.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round16_v13_ccz_summary_with_strike_and_unavailable_indexes_suppresses_valuation() {
        let dir = std::env::temp_dir().join(format!("m3-store-r16-v13-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.sec_tickers_cache().parent().unwrap()).unwrap();

        let cmcsa_raw = serde_json::json!({
            "entityName": "COMCAST CORP",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":120000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "GrossProfit": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":80000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "OperatingIncomeLoss": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":23000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":28000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":12000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "ShareBasedCompensation": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":1000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[{"end":"2025-12-31","val":6000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "LongTermDebtNoncurrent": {"units":{"USD":[{"end":"2025-12-31","val":90000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "StockholdersEquity": {"units":{"USD":[{"end":"2025-12-31","val":85000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[{"start":"2025-01-01","end":"2025-12-31","val":3900000000_i64,"form":"10-K","filed":"2026-02-01"}]}}
            }}
        });
        let raw_bytes = serde_json::to_vec(&cmcsa_raw).unwrap();
        let mut v13_ccz_summary = edgar::parse_companyfacts_bytes(
            &raw_bytes,
            "CCZ",
            "0001166691",
            "https://sec.test/CIK0001166691.json",
            1000,
        )
        .unwrap();
        v13_ccz_summary.extractor_version = 13;

        let mut ccz_co = CompanyEntry {
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
        };
        save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![ccz_co.clone()],
            },
        )
        .unwrap();
        let snap = PriceSnapshot {
            schema_version: 1,
            symbol: "CCZ".into(),
            price: Decimal::from(25),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NYSE".into(),
            market_time_ms: Some(now_ms()),
            fetched_at_ms: now_ms(),
            source_url: "https://test".into(),
        };
        write_json_atomic(&paths.price_path("CCZ"), &snap).unwrap();
        write_json_atomic(&paths.sec_facts_summary_path("CCZ"), &v13_ccz_summary).unwrap();

        let mut stale_v13_report =
            calculator::build_report(&ccz_co, Some(&v13_ccz_summary), Some(&snap), 1000);
        assert!(
            stale_v13_report
                .fy_valuation
                .as_ref()
                .unwrap()
                .strike_price
                .is_some(),
            "test fixture prerequisite: v13 summary labeled as CCZ produces a non-None strike"
        );
        stale_v13_report.warnings.clear();
        write_json_atomic(&paths.computed_report_path("CCZ"), &stale_v13_report).unwrap();
        let mut stamps = BTreeMap::new();
        stamps.insert(
            "CCZ".to_owned(),
            CachedReportStamp {
                cache_version: 13,
                facts_extractor_version: 13,
                facts_raw_sha256: Some(v13_ccz_summary.raw_sha256.clone()),
            },
        );
        save_report_cache_stamps(&paths, &stamps).unwrap();

        // With raw SEC bytes absent and SEC indexes unavailable, both load and interactive recompute
        // MUST refuse treating v13 CCZ summary as verified and suppress per-share valuation.
        let loaded_rep = load_or_recompute_ticker(&paths, "CCZ").unwrap();
        assert!(
            loaded_rep
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(loaded_rep.fy_valuation.as_ref().unwrap().strike_price, None);
        assert_eq!(
            loaded_rep
                .fy_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );

        let recomputed_rep = recompute_ticker(&paths, "CCZ").unwrap();
        assert!(
            recomputed_rep
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(
            recomputed_rep.fy_valuation.as_ref().unwrap().strike_price,
            None
        );
        assert_eq!(
            recomputed_rep
                .fy_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );

        // v1 sync with HTTP 500 index also refuses treating v13 CCZ summary as verified.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body_serve = raw_bytes.clone();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut req_line = String::new();
                reader.read_line(&mut req_line).unwrap();
                loop {
                    let mut line = String::new();
                    let n = reader.read_line(&mut line).unwrap();
                    if n == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                }
                if req_line.contains("company_tickers.json") {
                    stream
                        .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
                } else {
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body_serve.len()
                    );
                    stream.write_all(hdr.as_bytes()).unwrap();
                    stream.write_all(&body_serve).unwrap();
                }
                stream.flush().unwrap();
            }
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let tickers_url = format!("http://{addr}/company_tickers.json");
        let facts_url = format!("http://{addr}/CIK0001166691.json");
        sync_v1_sec_company_with_urls(
            &paths,
            &client,
            &mut ccz_co,
            Some(&facts_url),
            Some(&tickers_url),
            2000,
        )
        .unwrap();
        server.join().unwrap();
        let updated_summary =
            read_json::<RawCompanyFactsSummary>(&paths.sec_facts_summary_path("CCZ")).unwrap();
        assert_eq!(updated_summary.ticker, edgar::UNRESOLVED_CANONICAL_TICKER);
        let synced_rep = load_or_recompute_ticker(&paths, "CCZ").unwrap();
        assert!(
            synced_rep
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(synced_rep.fy_valuation.as_ref().unwrap().strike_price, None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round17_v13_ccz_missing_computed_report_and_unavailable_indexes_suppresses_valuation() {
        let dir = std::env::temp_dir().join(format!("m3-store-r17-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(paths.sec_tickers_cache().parent().unwrap()).unwrap();

        let cmcsa_raw = serde_json::json!({
            "entityName": "COMCAST CORP",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":120000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "GrossProfit": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":80000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "OperatingIncomeLoss": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":23000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":28000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":12000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "ShareBasedCompensation": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":1000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[{"end":"2025-12-31","val":6000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "LongTermDebtNoncurrent": {"units":{"USD":[{"end":"2025-12-31","val":90000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "StockholdersEquity": {"units":{"USD":[{"end":"2025-12-31","val":85000000000_i64,"form":"10-K","filed":"2026-02-01"}]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[{"start":"2025-01-01","end":"2025-12-31","val":3900000000_i64,"form":"10-K","filed":"2026-02-01"}]}}
            }}
        });
        let raw_bytes = serde_json::to_vec(&cmcsa_raw).unwrap();
        let mut v13_ccz_summary = edgar::parse_companyfacts_bytes(
            &raw_bytes,
            "CCZ",
            "0001166691",
            "https://sec.test/CIK0001166691.json",
            1000,
        )
        .unwrap();
        v13_ccz_summary.extractor_version = 13;

        let ccz_co = CompanyEntry {
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
        };
        save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![ccz_co],
            },
        )
        .unwrap();
        let snap = PriceSnapshot {
            schema_version: 1,
            symbol: "CCZ".into(),
            price: Decimal::from(25),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NYSE".into(),
            market_time_ms: Some(now_ms()),
            fetched_at_ms: now_ms(),
            source_url: "https://test".into(),
        };
        write_json_atomic(&paths.price_path("CCZ"), &snap).unwrap();
        write_json_atomic(&paths.sec_facts_summary_path("CCZ"), &v13_ccz_summary).unwrap();

        // Deliberately omit computed/CCZ.json, raw/sec/0001166691.json, SEC indexes, and DuckDB!
        assert!(!paths.computed_report_path("CCZ").exists());
        let loaded_rep = load_or_recompute_ticker(&paths, "CCZ").unwrap();
        let persisted_rep = read_json::<MungerReport>(&paths.computed_report_path("CCZ")).unwrap();
        for rep in [&loaded_rep, &persisted_rep] {
            assert_eq!(rep.annual_rows.len(), 1);
            assert_eq!(
                rep.annual_rows[0].revenue,
                Decimal::from(120_000_000_000_i64)
            );
            assert!(rep.warnings.iter().any(|w| {
                w.code == "SHARE_BASIS_MISMATCH"
                    && w.title == calculator::UNRESOLVED_SHARE_BASIS_TITLE
            }));
            assert_eq!(rep.fy_valuation.as_ref().unwrap().strike_price, None);
            assert_eq!(
                rep.fy_valuation.as_ref().unwrap().strict_fcf_yield_mcap_pct,
                None
            );
        }

        // Subsequent interactive recompute_ticker preserves fundamental rows & keeps valuation suppressed.
        let recomputed_rep = recompute_ticker(&paths, "CCZ").unwrap();
        let persisted_recomp =
            read_json::<MungerReport>(&paths.computed_report_path("CCZ")).unwrap();
        for rep in [&recomputed_rep, &persisted_recomp] {
            assert_eq!(rep.annual_rows.len(), 1);
            assert!(
                rep.warnings
                    .iter()
                    .any(|w| w.code == "SHARE_BASIS_MISMATCH")
            );
            assert_eq!(rep.fy_valuation.as_ref().unwrap().strike_price, None);
            assert_eq!(
                rep.fy_valuation.as_ref().unwrap().strict_fcf_yield_mcap_pct,
                None
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
