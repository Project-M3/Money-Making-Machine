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
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

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
        let p = PathBuf::from(explicit);
        if p.join("workbench/watchlist.json").exists() {
            return p.canonicalize().context("canonicalize M3_REPO_ROOT");
        }
    }
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    manifest_root
        .canonicalize()
        .context("找不到仓库根目录")
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
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
        self.data_dir
            .join("raw/sec")
            .join(format!("{}_facts.json", market::safe_symbol_filename(ticker)))
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
    path.exists().then(|| read_json(&path).ok()).flatten()
}

pub fn load_cached_price_history(
    paths: &WorkspacePaths,
    symbol: &str,
    interval: &str,
) -> Option<PriceHistorySnapshot> {
    let path = paths.price_history_path(symbol, interval);
    path.exists().then(|| read_json(&path).ok()).flatten()
}

pub fn load_cached_facts(paths: &WorkspacePaths, company: &CompanyEntry) -> Option<RawCompanyFactsSummary> {
    if company.source == DataSource::Manual {
        let manual = paths.manual_financials_path(&company.ticker);
        if manual.exists()
            && let Ok(parsed) = read_json::<RawCompanyFactsSummary>(&manual)
        {
            return Some(parsed);
        }
    }
    let sec = paths.sec_facts_summary_path(&company.ticker);
    if sec.exists() {
        read_json(&sec).ok()
    } else {
        None
    }
}

pub fn load_or_compute_reports(paths: &WorkspacePaths) -> Result<Vec<MungerReport>> {
    let wl = load_watchlist(paths)?;
    let now = now_ms();
    let mut reports = Vec::with_capacity(wl.companies.len());
    for company in &wl.companies {
        let computed_path = paths.computed_report_path(&company.ticker);
        if computed_path.exists()
            && let Ok(rep) = read_json::<MungerReport>(&computed_path)
        {
            reports.push(rep);
            continue;
        }
        let facts = load_cached_facts(paths, company);
        let price = load_cached_price(paths, &company.ticker);
        let rep = calculator::build_report(company, facts.as_ref(), price.as_ref(), now);
        let _ = write_json_atomic(&computed_path, &rep);
        reports.push(rep);
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
    let facts = load_cached_facts(paths, company);
    let price = load_cached_price(paths, &company.ticker);
    let rep = calculator::build_report(company, facts.as_ref(), price.as_ref(), now_ms());
    write_json_atomic(&paths.computed_report_path(&company.ticker), &rep)?;
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
                    let cik_res = match &company.cik {
                        Some(cik) if !cik.trim().is_empty() => edgar::normalize_cik(cik),
                        _ => {
                            let found = edgar::lookup_cik_by_ticker(
                                &sec_client,
                                &paths.sec_tickers_cache(),
                                &company.ticker,
                                false,
                            );
                            if let Ok(ref norm) = found {
                                company.cik = Some(norm.clone());
                                watchlist_changed = true;
                            }
                            found
                        }
                    };
                    match cik_res {
                        Ok(cik) => {
                            let raw_path = paths.sec_raw_path(&cik);
                            match edgar::fetch_and_extract(
                                &sec_client,
                                &company.ticker,
                                &cik,
                                &raw_path,
                                now,
                            ) {
                                Ok(summary) => {
                                    write_json_atomic(
                                        &paths.sec_facts_summary_path(&company.ticker),
                                        &summary,
                                    )?;
                                    fin_ok = true;
                                }
                                Err(err) => errors.push(format!("SEC 失败: {err:#}")),
                            }
                        }
                        Err(err) => errors.push(format!("CIK 解析失败: {err:#}")),
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

        let facts = load_cached_facts(paths, company);
        let price = load_cached_price(paths, &company.ticker);
        let report = calculator::build_report(company, facts.as_ref(), price.as_ref(), now);
        write_json_atomic(&paths.computed_report_path(&company.ticker), &report)?;

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

    let price_snap = load_cached_price(paths, &upper);
    let bench_snap = load_cached_price(paths, &wl.benchmark_symbol);
    let report = read_json::<MungerReport>(&paths.computed_report_path(&upper)).ok();

    let strict_yield = report.as_ref().and_then(|r| {
        r.ttm_valuation
            .as_ref()
            .or(r.fy_valuation.as_ref())
            .and_then(|v| v.strict_fcf_yield_mcap_pct)
    });

    let recorded_at = civil_today_iso();
    let price = price_snap.as_ref().map(|p| p.price);

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
        falsification_trigger: falsification
            .unwrap_or("待补充客观财务证伪条件")
            .to_owned(),
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

        let entry =
            record_journal_entry(&paths, "TEST", JournalAction::Wait, "等安全边际", None, Some("毛利率 < 60%"))
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
        assert_eq!(load_watchlist(&paths).unwrap().companies[0].bucket, Bucket::Out);

        let _ = fs::remove_dir_all(&dir);
    }
}
