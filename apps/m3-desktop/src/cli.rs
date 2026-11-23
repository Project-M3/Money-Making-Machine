use crate::{
    lake::{LakeDb, UniverseFilter, UniverseSortBy, UniverseSummary},
    model::{Bucket, JournalAction, SyncStatusFile, parse_decimal_token},
    store::{self, WorkspacePaths},
    universe::{self, UniverseSyncOptions},
};
use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use serde_json::json;
use std::{str::FromStr, time::Instant};

pub fn run(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sub = args.first().map(String::as_str).unwrap_or("status");
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "status" => cmd_status(paths),
        "sync" => cmd_sync(paths, rest),
        "report" => cmd_report(paths, rest),
        "dossier" => cmd_dossier(paths, rest),
        "add" => cmd_add(paths, rest),
        "journal" => cmd_journal(paths, rest),
        "universe" => cmd_universe(paths, rest),
        "start" => cmd_start(paths),
        _ => anyhow::bail!(
            "usage: m3 status | sync [--ticker SYM] [--prices-only] | report <SYM> | dossier <SYM> | add <SYM> ... | journal [list|record ...] | universe [status|sync|screen|peers ...] | start"
        ),
    }
}

fn valid_report_price(
    report: &crate::model::MungerReport,
    now_ms: u64,
) -> Option<&crate::model::PriceSnapshot> {
    report.price.as_ref().filter(|p| {
        p.symbol.eq_ignore_ascii_case(&report.ticker)
            && crate::market::validate_quote_session(p, now_ms).is_ok()
    })
}

fn derive_report_status(
    report: &crate::model::MungerReport,
    has_valid_price: bool,
) -> crate::model::StrikeZoneStatus {
    crate::calculator::derive_strike_status(
        report.bucket,
        !report.annual_rows.is_empty(),
        has_valid_price,
        &report.warnings,
        report.fy_valuation.as_ref(),
        report.ttm_valuation.as_ref(),
    )
}

fn cmd_status(paths: &WorkspacePaths) -> Result<serde_json::Value> {
    let wl = store::load_watchlist(paths)?;
    let reports = store::load_or_compute_reports(paths)?;
    let journal = store::load_journal(paths)?;
    let sync_status: Option<SyncStatusFile> = store::read_json(&paths.sync_status_path()).ok();
    let now = store::now_ms();
    let benchmark = store::load_cached_price(paths, &wl.benchmark_symbol)
        .filter(|p| crate::market::validate_quote_session(p, now).is_ok());

    let summary: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| {
            let valid_price = valid_report_price(r, now);
            let val = r.ttm_valuation.as_ref().or(r.fy_valuation.as_ref());
            let status = derive_report_status(r, valid_price.is_some());
            json!({
                "ticker": r.ticker,
                "name": r.name,
                "bucket": r.bucket,
                "status": status,
                "currency": r.currency,
                "price": valid_price.map(|p| p.price.normalize().to_string()),
                "strike_price": val.and_then(|v| v.strike_price.map(|s| s.normalize().to_string())),
                "distance_to_strike_pct": valid_price.and_then(|_| {
                    val.and_then(|v| v.distance_to_strike_pct.map(|d| format!("{:.2}", d)))
                }),
                "strict_fcf_yield_pct": valid_price.and_then(|_| {
                    val.and_then(|v| v.strict_fcf_yield_mcap_pct.map(|y| format!("{:.2}", y)))
                }),
                "roic_5y_avg_pct": r.roic_5y_avg_pct.map(|v| format!("{:.2}", v)),
                "warnings": r.warnings.len(),
            })
        })
        .collect();

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "workbench_root": paths.repo_root.display().to_string(),
            "benchmark": benchmark,
            "sync_status": sync_status,
            "companies": summary,
            "journal_entries": journal.entries.len(),
            "cash_balance": journal.cash_balance.normalize().to_string()
        }
    }))
}

fn cmd_sync(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut ticker: Option<String> = None;
    let mut prices_only = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ticker" => {
                i += 1;
                ticker = Some(args.get(i).context("--ticker requires symbol")?.clone());
            }
            "--prices-only" => prices_only = true,
            other => anyhow::bail!("unknown sync flag: {other}"),
        }
        i += 1;
    }
    let sync = store::sync_all(paths, ticker.as_deref(), prices_only)?;
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": sync
    }))
}

fn cmd_report(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sym = args.first().context("usage: m3 report <TICKER>")?;
    let mut report = store::recompute_ticker(paths, sym)?;
    let now = store::now_ms();
    if valid_report_price(&report, now).is_none() {
        report.price = None;
    }
    report.status = derive_report_status(&report, report.price.is_some());
    report.schema_version = 1;
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": report
    }))
}

fn cmd_dossier(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sym = args.first().context("usage: m3 dossier <TICKER>")?;
    let dossier = store::load_dossier(paths, &sym.to_ascii_uppercase())?
        .with_context(|| format!("未找到 {sym} 的审讯档案"))?;
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": dossier
    }))
}

fn cmd_add(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sym = args
        .first()
        .context("usage: m3 add <TICKER> --bucket core|in|watch|out|too_hard ...")?;
    let mut bucket = Bucket::Watch;
    let mut name: Option<String> = None;
    let mut circle: Option<String> = None;
    let mut reason: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bucket" => {
                i += 1;
                let raw = args.get(i).context("--bucket requires value")?;
                bucket = match raw.to_ascii_lowercase().as_str() {
                    "core" | "in" => Bucket::Core,
                    "watch" => Bucket::Watch,
                    "out" => Bucket::Out,
                    "too_hard" => Bucket::TooHard,
                    _ => anyhow::bail!("invalid bucket {raw}; use core|in|watch|out|too_hard"),
                };
            }
            "--name" => {
                i += 1;
                name = Some(args.get(i).context("--name requires value")?.clone());
            }
            "--circle" => {
                i += 1;
                circle = Some(args.get(i).context("--circle requires value")?.clone());
            }
            "--reason" => {
                i += 1;
                reason = Some(args.get(i).context("--reason requires value")?.clone());
            }
            other => anyhow::bail!("unknown add flag: {other}"),
        }
        i += 1;
    }

    let entry = store::add_or_update_company(
        paths,
        sym,
        bucket,
        name.as_deref(),
        circle.as_deref(),
        reason.as_deref(),
    )?;

    if paths.lake_db_path().exists() {
        let db = LakeDb::open(paths).context(
            "watchlist updated, but failed to open m3.duckdb to refresh watchlist flags",
        )?;
        db.refresh_watchlist_flags(paths)
            .context("watchlist updated, but failed to refresh m3.duckdb watchlist flags")?;
    }

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": entry
    }))
}

fn cmd_journal(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    match sub {
        "list" => {
            let journal = store::load_journal(paths)?;
            Ok(json!({
                "schema_version": 1,
                "ok": true,
                "data": journal
            }))
        }
        "record" => {
            let mut ticker: Option<String> = None;
            let mut action: Option<JournalAction> = None;
            let mut note: Option<String> = None;
            let mut shares: Option<Decimal> = None;
            let mut falsification: Option<String> = None;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--ticker" => {
                        i += 1;
                        ticker = Some(args.get(i).context("--ticker requires symbol")?.clone());
                    }
                    "--action" => {
                        i += 1;
                        let raw = args.get(i).context("--action requires value")?;
                        action = Some(match raw.to_ascii_lowercase().as_str() {
                            "wait" => JournalAction::Wait,
                            "out" => JournalAction::Out,
                            "too_hard" => JournalAction::TooHard,
                            "paper_buy" => JournalAction::PaperBuy,
                            "paper_sell" => JournalAction::PaperSell,
                            _ => anyhow::bail!("invalid action {raw}"),
                        });
                    }
                    "--note" => {
                        i += 1;
                        note = Some(args.get(i).context("--note requires text")?.clone());
                    }
                    "--shares" => {
                        i += 1;
                        let raw = args.get(i).context("--shares requires number")?;
                        let parsed = parse_decimal_token(raw)
                            .map_err(|e| anyhow::anyhow!("invalid --shares decimal value: {e}"))?;
                        shares = Some(parsed);
                    }
                    "--falsification" => {
                        i += 1;
                        falsification = Some(
                            args.get(i)
                                .context("--falsification requires text")?
                                .clone(),
                        );
                    }
                    other => anyhow::bail!("unknown journal record flag: {other}"),
                }
                i += 1;
            }

            let entry = store::record_journal_entry(
                paths,
                ticker.as_deref().context("--ticker is required")?,
                action.context("--action is required")?,
                note.as_deref().context("--note is required")?,
                shares,
                falsification.as_deref(),
            )?;

            Ok(json!({
                "schema_version": 1,
                "ok": true,
                "data": entry
            }))
        }
        _ => anyhow::bail!(
            "usage: m3 journal list | record --ticker SYM --action wait|out|too_hard|paper_buy|paper_sell --note ..."
        ),
    }
}

const MAX_PRICES_TOP: usize = 1_000;
const MAX_SCREEN_LIMIT: usize = 5_000;
const MAX_PEERS_LIMIT: usize = 200;
const MAX_MIN_YEARS: usize = 20;
const MIN_FILTER_METRIC: i64 = -10_000;
const MAX_FILTER_METRIC: i64 = 100_000;

fn parse_decimal_flag(args: &[String], index: &mut usize, flag: &str) -> Result<Decimal> {
    *index += 1;
    let raw = args
        .get(*index)
        .with_context(|| format!("{flag} requires a numeric value"))?;
    let parsed = parse_decimal_token(raw)
        .map_err(|e| anyhow::anyhow!("invalid {flag} decimal value: {e}"))?;
    let min = Decimal::from(MIN_FILTER_METRIC);
    let max = Decimal::from(MAX_FILTER_METRIC);
    ensure!(
        parsed >= min && parsed <= max,
        "{flag} must be between {min} and {max}, got {parsed}"
    );
    Ok(parsed)
}

fn parse_usize_flag(
    args: &[String],
    index: &mut usize,
    flag: &str,
    min_value: usize,
    max_value: usize,
) -> Result<usize> {
    *index += 1;
    let raw = args
        .get(*index)
        .with_context(|| format!("{flag} requires an integer value"))?;
    let parsed = raw
        .trim()
        .parse::<usize>()
        .with_context(|| format!("invalid {flag} integer value: {raw}"))?;
    ensure!(
        parsed >= min_value && parsed <= max_value,
        "{flag} must be between {min_value} and {max_value}, got {parsed}"
    );
    Ok(parsed)
}

fn cmd_universe(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sub = args.first().map(String::as_str).unwrap_or("status");
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "status" => cmd_universe_status(paths, rest),
        "sync" => cmd_universe_sync(paths, rest),
        "screen" => cmd_universe_screen(paths, rest),
        "peers" => cmd_universe_peers(paths, rest),
        _ => anyhow::bail!(
            "usage: m3 universe <status|sync [--full] [--prices-top N] [--skip-prices]|screen [--min-roic N] [--max-sbc-ocf N] [--max-share-change-3y N] [--max-net-debt-to-oe N] [--min-years N] [--exclude-financials|--include-financials] [--positive-oe-only] [--price-pool-only] [--strike-only] [--query TEXT] [--sort COL] [--asc|--desc] [--limit N]|peers <TICKER> [--limit N]>"
        ),
    }
}

fn cmd_universe_status(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    if let Some(unexpected) = args.first() {
        anyhow::bail!("unknown universe status argument: {unexpected}");
    }
    let summary = match LakeDb::open_read_only_if_exists(paths)? {
        Some(db) => db.universe_summary()?,
        None => UniverseSummary::default(),
    };
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": summary
    }))
}

fn cmd_universe_sync(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut opts = UniverseSyncOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--full" => opts.full_sec = true,
            "--prices-top" => {
                opts.prices_top =
                    parse_usize_flag(args, &mut i, "--prices-top", 1, MAX_PRICES_TOP)?;
            }
            "--skip-prices" => opts.skip_prices = true,
            other => anyhow::bail!("unknown universe sync flag: {other}"),
        }
        i += 1;
    }

    let report = universe::sync_universe_with_progress(paths, &opts, |msg| {
        eprintln!("[m3 universe sync] {msg}");
    })?;

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": report
    }))
}

fn cmd_universe_screen(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut filter = UniverseFilter::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--min-roic" => {
                filter.min_roic_5y = Some(parse_decimal_flag(args, &mut i, "--min-roic")?);
            }
            "--max-sbc-ocf" => {
                filter.max_sbc_to_ocf = Some(parse_decimal_flag(args, &mut i, "--max-sbc-ocf")?);
            }
            "--max-share-change-3y" => {
                filter.max_share_change_3y =
                    Some(parse_decimal_flag(args, &mut i, "--max-share-change-3y")?);
            }
            "--max-net-debt-to-oe" => {
                filter.max_net_debt_to_oe =
                    Some(parse_decimal_flag(args, &mut i, "--max-net-debt-to-oe")?);
            }
            "--min-years" => {
                filter.min_years_of_data = Some(parse_usize_flag(
                    args,
                    &mut i,
                    "--min-years",
                    1,
                    MAX_MIN_YEARS,
                )?);
            }
            "--exclude-financials" => filter.exclude_financials = true,
            "--include-financials" => filter.exclude_financials = false,
            "--positive-oe-only" => filter.positive_strict_oe_only = true,
            "--price-pool-only" => filter.in_price_pool_only = true,
            "--strike-only" => filter.strike_zone_only = true,
            "--query" => {
                i += 1;
                let raw = args.get(i).context("--query requires search text")?;
                let trimmed = raw.trim();
                filter.query = (!trimmed.is_empty()).then(|| trimmed.to_owned());
            }
            "--sort" => {
                i += 1;
                let raw = args.get(i).context("--sort requires sort column")?;
                filter.sort_by = UniverseSortBy::from_str(raw)?;
            }
            "--asc" => filter.sort_asc = true,
            "--desc" => filter.sort_asc = false,
            "--limit" => {
                filter.limit = parse_usize_flag(args, &mut i, "--limit", 1, MAX_SCREEN_LIMIT)?;
            }
            other => anyhow::bail!("unknown universe screen flag: {other}"),
        }
        i += 1;
    }

    let db = LakeDb::open_read_only_if_exists(paths)?
        .context("universe lake has not been synced yet; run `m3 universe sync` first")?;
    let started = Instant::now();
    let rows = db.screen_universe(&filter)?;
    let query_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "query_ms": query_ms,
            "count": rows.len(),
            "filter": filter,
            "rows": rows,
        }
    }))
}

fn cmd_universe_peers(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut ticker: Option<String> = None;
    let mut limit: usize = 15;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" => {
                limit = parse_usize_flag(args, &mut i, "--limit", 1, MAX_PEERS_LIMIT)?;
            }
            other if other.starts_with('-') => {
                anyhow::bail!("unknown universe peers flag: {other}");
            }
            sym => {
                ensure!(
                    ticker.is_none(),
                    "unexpected extra argument for universe peers: {sym}"
                );
                ticker = Some(sym.trim().to_ascii_uppercase());
            }
        }
        i += 1;
    }

    let ticker = ticker
        .filter(|t| !t.is_empty())
        .context("usage: m3 universe peers <TICKER> [--limit N]")?;
    let db = LakeDb::open_read_only_if_exists(paths)?
        .context("universe lake has not been synced yet; run `m3 universe sync` first")?;
    let peers = db.query_peers(&ticker, limit)?;

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": peers
    }))
}

fn cmd_start(paths: &WorkspacePaths) -> Result<serde_json::Value> {
    let exe = std::env::current_exe()?;
    let app = exe
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .context("CLI must run from M3.app bundle or open M3.app directly")?;
    ensure!(
        app.extension().is_some_and(|s| s == "app"),
        "start expects M3.app bundle"
    );
    ensure!(
        std::process::Command::new("/usr/bin/open")
            .arg(app)
            .status()?
            .success(),
        "failed to launch M3.app"
    );
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "launched": app.display().to_string(),
            "workbench_root": paths.repo_root.display().to_string()
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lake::{LakePriceDailyRow, QuoteSnapshotUpdate, SyncWatermark, UniverseRow},
        model::{
            CompanyEntry, DataSource, DecisionJournal, ValuationAssumptions, WarningFlag,
            WarningSeverity, WatchlistFile,
        },
    };
    use std::fs;

    struct SyntheticRowSpec<'a> {
        ticker: &'a str,
        cik: &'a str,
        sic: Option<u32>,
        sic_desc: Option<&'a str>,
        is_financial: bool,
        roic_5y: Option<i64>,
        gm_5y: Option<i64>,
        sbc_ocf: Option<i64>,
        share_3y: Option<i64>,
        dist: Option<i64>,
        fcf_yield: Option<i64>,
        quality: i64,
    }

    fn synthetic_universe_row(spec: SyntheticRowSpec<'_>) -> UniverseRow {
        UniverseRow {
            ticker: spec.ticker.to_owned(),
            cik: spec.cik.to_owned(),
            name: format!("{} Corp.", spec.ticker),
            exchange: "Nasdaq".to_owned(),
            sic: spec.sic,
            sic_description: spec.sic_desc.map(str::to_owned),
            is_financial: spec.is_financial,
            latest_filing_end: Some("2025-12-31".to_owned()),
            years_of_data: 6,
            quality_score: Decimal::from(spec.quality),
            in_price_pool: true,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: Some(Decimal::from(100)),
            strike_price: Some(Decimal::from(110)),
            distance_to_strike_pct: spec.dist.map(Decimal::from),
            strict_fcf_yield_pct: spec.fcf_yield.map(Decimal::from),
            roic_5y_avg_pct: spec.roic_5y.map(Decimal::from),
            roic_5y_stddev_pct: Some(Decimal::new(15, 1)),
            gross_margin_5y_avg_pct: spec.gm_5y.map(Decimal::from),
            gross_margin_5y_stddev_pct: Some(Decimal::new(9, 1)),
            share_change_3y_pct: spec.share_3y.map(Decimal::from),
            sbc_to_ocf_pct: spec.sbc_ocf.map(Decimal::from),
            net_debt_to_oe: Some(Decimal::new(-4, 1)),
            strict_owner_earnings: Some(Decimal::from(1_000_000_000)),
            warnings_count: 1,
            warnings_json: serde_json::to_string(&vec![WarningFlag {
                code: "TEST_WARN".to_owned(),
                severity: WarningSeverity::Info,
                title: "Info".to_owned(),
                detail: "Test".to_owned(),
            }])
            .unwrap(),
            raw_sha256: Some("abc123".to_owned()),
        }
    }

    #[test]
    fn cli_status_and_journal_list_emit_valid_schema() {
        let dir = std::env::temp_dir().join(format!("m3-cli-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());

        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![CompanyEntry {
                    ticker: "AAPL".into(),
                    name: "Apple Inc.".into(),
                    bucket: Bucket::Core,
                    source: DataSource::Manual,
                    cik: None,
                    currency: "USD".into(),
                    circle: "生态".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(18),
                        multiple_high: Decimal::from(22),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(7),
                        strike_price_override: None,
                    },
                }],
            },
        )
        .unwrap();
        store::save_journal(
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

        let status = run(&paths, &["status".into()]).unwrap();
        assert_eq!(status["schema_version"], 1);
        assert_eq!(status["ok"], true);
        assert_eq!(status["data"]["companies"][0]["ticker"], "AAPL");

        let report = run(&paths, &["report".into(), "AAPL".into()]).unwrap();
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["ok"], true);
        assert_eq!(report["data"]["schema_version"], 1);
        assert_eq!(report["data"]["ticker"], "AAPL");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_universe_unsynced_and_flag_validation() {
        let dir =
            std::env::temp_dir().join(format!("m3-cli-universe-unsynced-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());

        let u_status = run(&paths, &["universe".into(), "status".into()]).unwrap();
        assert_eq!(u_status["schema_version"], 1);
        assert_eq!(u_status["ok"], true);
        assert_eq!(u_status["data"]["total_companies"], 0);

        let u_default = run(&paths, &["universe".into()]).unwrap();
        assert_eq!(u_default["ok"], true);
        assert_eq!(u_default["data"]["total_companies"], 0);

        let screen_err = run(&paths, &["universe".into(), "screen".into()]).unwrap_err();
        assert!(screen_err.to_string().contains("m3 universe sync"));

        let peers_err =
            run(&paths, &["universe".into(), "peers".into(), "ADSK".into()]).unwrap_err();
        assert!(peers_err.to_string().contains("m3 universe sync"));

        let limit_zero = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--limit".into(),
                "0".into(),
            ],
        )
        .unwrap_err();
        assert!(limit_zero.to_string().contains("between 1 and"));

        let limit_huge = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--limit".into(),
                "5001".into(),
            ],
        )
        .unwrap_err();
        assert!(limit_huge.to_string().contains("between 1 and 5000"));

        let peers_limit_huge = run(
            &paths,
            &[
                "universe".into(),
                "peers".into(),
                "ADSK".into(),
                "--limit".into(),
                "201".into(),
            ],
        )
        .unwrap_err();
        assert!(peers_limit_huge.to_string().contains("between 1 and 200"));

        let prices_top_zero = run(
            &paths,
            &[
                "universe".into(),
                "sync".into(),
                "--prices-top".into(),
                "0".into(),
            ],
        )
        .unwrap_err();
        assert!(prices_top_zero.to_string().contains("between 1 and 1000"));

        let prices_top_huge = run(
            &paths,
            &[
                "universe".into(),
                "sync".into(),
                "--prices-top".into(),
                "1001".into(),
            ],
        )
        .unwrap_err();
        assert!(prices_top_huge.to_string().contains("between 1 and 1000"));

        let roic_oob = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--min-roic".into(),
                "99999999".into(),
            ],
        )
        .unwrap_err();
        assert!(roic_oob.to_string().contains("must be between"));

        let min_years_oob = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--min-years".into(),
                "21".into(),
            ],
        )
        .unwrap_err();
        assert!(min_years_oob.to_string().contains("between 1 and 20"));

        let bad_sort = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--sort".into(),
                "bogus".into(),
            ],
        )
        .unwrap_err();
        assert!(bad_sort.to_string().contains("unknown sort column"));

        let roic_underflow = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--min-roic".into(),
                "0.00000000000000000000000000001e0".into(),
            ],
        )
        .unwrap_err();
        assert!(
            roic_underflow
                .to_string()
                .contains("exceeds maximum 28-decimal-place exact precision")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_scientific_decimal_flags_reject_silent_zero_and_preserve_exact_scale() {
        let dir = std::env::temp_dir().join(format!(
            "m3-cli-sci-decimal-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());

        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: Vec::new(),
            },
        )
        .unwrap();

        let snapshot = vec![
            synthetic_universe_row(SyntheticRowSpec {
                ticker: "ZERO",
                cik: "0000000001",
                sic: Some(7372),
                sic_desc: Some("Prepackaged Software"),
                is_financial: false,
                roic_5y: Some(0),
                gm_5y: Some(50),
                sbc_ocf: Some(10),
                share_3y: Some(0),
                dist: Some(-5),
                fcf_yield: Some(5),
                quality: 50,
            }),
            synthetic_universe_row(SyntheticRowSpec {
                ticker: "HIGH",
                cik: "0000000002",
                sic: Some(7372),
                sic_desc: Some("Prepackaged Software"),
                is_financial: false,
                roic_5y: Some(15),
                gm_5y: Some(80),
                sbc_ocf: Some(10),
                share_3y: Some(0),
                dist: Some(-5),
                fcf_yield: Some(5),
                quality: 80,
            }),
        ];

        {
            let mut db = LakeDb::open(&paths).unwrap();
            db.replace_sec_lake_data(&[], &[], &[], &snapshot).unwrap();
        }

        // Exact 1e-28 (represented as 29dp * e1) must parse as 1e-28, NOT round to 0,
        // and therefore must exclude the ROIC=0 row.
        let tiny_positive = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--min-roic".into(),
                "0.00000000000000000000000000001e1".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            tiny_positive["data"]["filter"]["min_roic_5y"],
            "0.0000000000000000000000000001"
        );
        assert_eq!(tiny_positive["data"]["count"], 1);
        assert_eq!(tiny_positive["data"]["rows"][0]["ticker"], "HIGH");

        // Standard scientific notation (1.5e1 == 15) works identically to fixed decimal.
        let sci_fifteen = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--min-roic".into(),
                "1.5e1".into(),
            ],
        )
        .unwrap();
        assert_eq!(sci_fifteen["data"]["filter"]["min_roic_5y"], "15");
        assert_eq!(sci_fifteen["data"]["count"], 1);
        assert_eq!(sci_fifteen["data"]["rows"][0]["ticker"], "HIGH");

        let shares_underflow = run(
            &paths,
            &[
                "journal".into(),
                "record".into(),
                "--ticker".into(),
                "HIGH".into(),
                "--action".into(),
                "wait".into(),
                "--note".into(),
                "precision check".into(),
                "--shares".into(),
                "0.00000000000000000000000000001e0".into(),
            ],
        )
        .unwrap_err();
        assert!(
            shares_underflow
                .to_string()
                .contains("exceeds maximum 28-decimal-place exact precision")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_universe_status_screen_and_peers_on_synthetic_lake() {
        let dir = std::env::temp_dir().join(format!(
            "m3-cli-universe-lake-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());

        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: Vec::new(),
            },
        )
        .unwrap();

        let snapshot = vec![
            synthetic_universe_row(SyntheticRowSpec {
                ticker: "ADSK",
                cik: "0000769397",
                sic: Some(7372),
                sic_desc: Some("Prepackaged Software"),
                is_financial: false,
                roic_5y: Some(28),
                gm_5y: Some(91),
                sbc_ocf: Some(42),
                share_3y: Some(-2),
                dist: Some(12),
                fcf_yield: Some(4),
                quality: 82,
            }),
            synthetic_universe_row(SyntheticRowSpec {
                ticker: "PTC",
                cik: "0000857005",
                sic: Some(7372),
                sic_desc: Some("Prepackaged Software"),
                is_financial: false,
                roic_5y: Some(22),
                gm_5y: Some(80),
                sbc_ocf: Some(18),
                share_3y: Some(1),
                dist: Some(-5),
                fcf_yield: Some(5),
                quality: 86,
            }),
            synthetic_universe_row(SyntheticRowSpec {
                ticker: "SNPS",
                cik: "0000883241",
                sic: Some(7372),
                sic_desc: Some("Prepackaged Software"),
                is_financial: false,
                roic_5y: Some(25),
                gm_5y: Some(79),
                sbc_ocf: Some(24),
                share_3y: Some(0),
                dist: Some(20),
                fcf_yield: Some(3),
                quality: 84,
            }),
            synthetic_universe_row(SyntheticRowSpec {
                ticker: "ADBE",
                cik: "0000796343",
                sic: Some(7372),
                sic_desc: Some("Prepackaged Software"),
                is_financial: false,
                roic_5y: Some(35),
                gm_5y: Some(88),
                sbc_ocf: Some(19),
                share_3y: Some(-5),
                dist: Some(-8),
                fcf_yield: Some(5),
                quality: 93,
            }),
            synthetic_universe_row(SyntheticRowSpec {
                ticker: "BAC",
                cik: "0000070858",
                sic: Some(6021),
                sic_desc: Some("National Commercial Banks"),
                is_financial: true,
                roic_5y: Some(40),
                gm_5y: Some(95),
                sbc_ocf: Some(5),
                share_3y: Some(-3),
                dist: Some(-10),
                fcf_yield: Some(8),
                quality: 0,
            }),
        ];

        {
            let now_ms = store::now_ms();
            let trade_date = crate::market::civil_iso_from_unix_secs((now_ms / 1000) as i64);
            let mut db = LakeDb::open(&paths).unwrap();
            db.replace_sec_lake_data(&[], &[], &[], &snapshot).unwrap();
            let prices: Vec<LakePriceDailyRow> = snapshot
                .iter()
                .map(|row| LakePriceDailyRow {
                    symbol: row.ticker.clone(),
                    trade_date: trade_date.clone(),
                    price: Decimal::from(100),
                    previous_close: Some(Decimal::from(99)),
                    currency: "USD".into(),
                    exchange: row.exchange.clone(),
                    market_time_ms: Some(now_ms),
                    fetched_at_ms: now_ms,
                    source_url: "https://example.test/quote".into(),
                })
                .collect();
            let quote_updates: Vec<QuoteSnapshotUpdate> = snapshot
                .iter()
                .map(|row| QuoteSnapshotUpdate {
                    cik: row.cik.clone(),
                    expected_ticker: row.ticker.clone(),
                    expected_raw_sha256: row.raw_sha256.clone(),
                    expected_latest_filing_end: row.latest_filing_end.clone(),
                    expected_basis_period_end: None,
                    quote_market_time_ms: Some(now_ms),
                    quote_fetched_at_ms: now_ms,
                    latest_price: row.latest_price,
                    strike_price: row.strike_price,
                    distance_to_strike_pct: row.distance_to_strike_pct,
                    strict_fcf_yield_pct: row.strict_fcf_yield_pct,
                })
                .collect();
            db.upsert_prices_and_update_snapshot(&prices, &quote_updates, &[])
                .unwrap();
            db.write_watermark(&SyncWatermark {
                source_key: "sec_companyfacts_zip".into(),
                cursor: Some(trade_date),
                etag: None,
                last_modified: None,
                synced_at_ms: 1_795_400_000_000,
            })
            .unwrap();
            db.write_watermark(&SyncWatermark {
                source_key: "yahoo_prices".into(),
                cursor: None,
                etag: None,
                last_modified: None,
                synced_at_ms: 1_795_400_001_000,
            })
            .unwrap();
        }

        let u_status = run(&paths, &["universe".into(), "status".into()]).unwrap();
        assert_eq!(u_status["schema_version"], 1);
        assert_eq!(u_status["ok"], true);
        assert_eq!(u_status["data"]["total_companies"], 5);
        assert_eq!(u_status["data"]["parsed_financial_companies"], 5);
        assert_eq!(u_status["data"]["non_financial_companies"], 4);
        assert_eq!(u_status["data"]["quality_pool_count"], 5);
        assert_eq!(u_status["data"]["priced_count"], 5);
        assert_eq!(u_status["data"]["strike_zone_count"], 3);
        assert_eq!(u_status["data"]["last_sec_sync_ms"], 1_795_400_000_000u64);
        assert_eq!(u_status["data"]["last_price_sync_ms"], 1_795_400_001_000u64);

        let screened = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--min-roic".into(),
                "20".into(),
                "--max-sbc-ocf".into(),
                "25".into(),
                "--max-share-change-3y".into(),
                "2".into(),
                "--min-years".into(),
                "5".into(),
                "--exclude-financials".into(),
                "--strike-only".into(),
                "--query".into(),
                "Prepackaged".into(),
                "--sort".into(),
                "roic".into(),
                "--limit".into(),
                "1".into(),
            ],
        )
        .unwrap();
        assert_eq!(screened["schema_version"], 1);
        assert_eq!(screened["ok"], true);
        assert_eq!(screened["data"]["count"], 1);
        assert_eq!(screened["data"]["rows"][0]["ticker"], "ADBE");

        let with_fin = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--include-financials".into(),
                "--sort".into(),
                "distance".into(),
                "--asc".into(),
                "--limit".into(),
                "10".into(),
            ],
        )
        .unwrap();
        assert_eq!(with_fin["data"]["count"], 5);
        assert_eq!(with_fin["data"]["rows"][0]["ticker"], "BAC");

        let peers = run(
            &paths,
            &[
                "universe".into(),
                "peers".into(),
                "adsk".into(),
                "--limit".into(),
                "2".into(),
            ],
        )
        .unwrap();
        assert_eq!(peers["schema_version"], 1);
        assert_eq!(peers["ok"], true);
        assert_eq!(peers["data"]["target"]["ticker"], "ADSK");
        assert_eq!(peers["data"]["peer_count"], 3);
        assert!(
            peers["data"]["sic_group"]
                .as_str()
                .unwrap()
                .contains("7372")
        );
        assert_eq!(peers["data"]["roic_percentile"], "66.67");
        assert_eq!(peers["data"]["top_peers"].as_array().unwrap().len(), 2);

        let added = run(
            &paths,
            &[
                "add".into(),
                "PTC".into(),
                "--bucket".into(),
                "core".into(),
                "--name".into(),
                "PTC Inc.".into(),
                "--circle".into(),
                "CAD/PLM".into(),
            ],
        )
        .unwrap();
        assert_eq!(added["ok"], true);
        let ptc_screen = run(
            &paths,
            &[
                "universe".into(),
                "screen".into(),
                "--query".into(),
                "PTC".into(),
            ],
        )
        .unwrap();
        assert_eq!(ptc_screen["data"]["rows"][0]["in_watchlist"], true);
        assert_eq!(ptc_screen["data"]["rows"][0]["bucket_opt"], "core");

        fs::write(paths.lake_db_path(), b"corrupt-duckdb").unwrap();
        let add_db_err = run(
            &paths,
            &[
                "add".into(),
                "NVDA".into(),
                "--bucket".into(),
                "watch".into(),
            ],
        )
        .unwrap_err();
        assert!(
            add_db_err.to_string().contains("m3.duckdb"),
            "unexpected add error: {add_db_err:#}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_status_and_report_suppress_expired_quote_and_warrant_strike_status() {
        let dir = std::env::temp_dir().join(format!(
            "m3-cli-status-consistency-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        let paths = WorkspacePaths::from_root(dir.clone());
        let now_ms = store::now_ms();
        let stale_ms = now_ms.saturating_sub(10 * 24 * 60 * 60 * 1000);

        let acme_raw = br#"{
            "entityName": "ACME CORP",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":1000000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":600000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "IncomeTaxExpenseBenefit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":60000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":260000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":20000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "ShareBasedCompensation": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-12-31","val":200000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":50000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-12-31","val":800000000,"form":"10-K","filed":"2026-02-10"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-01-01","end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-10"}
                ]}}
            }}
        }"#;

        let assumptions = ValuationAssumptions {
            multiple_low: Decimal::from(18),
            multiple_high: Decimal::from(22),
            target_return_pct: Decimal::from(10),
            expected_growth_pct: Decimal::from(7),
            strike_price_override: None,
        };
        let acme_entry = CompanyEntry {
            ticker: "ACME".into(),
            name: "Acme Corp.".into(),
            bucket: Bucket::Core,
            source: DataSource::SecEdgar,
            cik: Some("0000000100".into()),
            currency: "USD".into(),
            circle: "Software".into(),
            too_hard_reason: None,
            assumptions: assumptions.clone(),
        };
        let warrant_entry = CompanyEntry {
            ticker: "SBCWW".into(),
            name: "SBC Warrant".into(),
            bucket: Bucket::Core,
            source: DataSource::SecEdgar,
            cik: Some("0001930313".into()),
            currency: "USD".into(),
            circle: "Healthcare".into(),
            too_hard_reason: None,
            assumptions,
        };
        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![acme_entry.clone(), warrant_entry],
            },
        )
        .unwrap();
        store::save_journal(
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

        let acme_facts = crate::edgar::parse_companyfacts_bytes(
            acme_raw,
            "ACME",
            "0000000100",
            "https://sec.test",
            now_ms,
        )
        .unwrap();
        let sbcww_facts = crate::edgar::parse_companyfacts_bytes(
            acme_raw,
            "SBCWW",
            "0001930313",
            "https://sec.test",
            now_ms,
        )
        .unwrap();
        store::write_json_atomic(&paths.sec_facts_summary_path("ACME"), &acme_facts).unwrap();
        store::write_json_atomic(&paths.sec_facts_summary_path("SBCWW"), &sbcww_facts).unwrap();

        let stale_price = crate::model::PriceSnapshot {
            schema_version: 1,
            symbol: "ACME".into(),
            price: Decimal::from(10),
            previous_close: Some(Decimal::from(10)),
            currency: "USD".into(),
            exchange: "Nasdaq".into(),
            market_time_ms: Some(stale_ms),
            fetched_at_ms: stale_ms,
            source_url: "https://example.test/acme".into(),
        };
        let fresh_warrant_price = crate::model::PriceSnapshot {
            schema_version: 1,
            symbol: "SBCWW".into(),
            price: Decimal::from(5),
            previous_close: Some(Decimal::from(5)),
            currency: "USD".into(),
            exchange: "Nasdaq".into(),
            market_time_ms: Some(now_ms),
            fetched_at_ms: now_ms,
            source_url: "https://example.test/sbcww".into(),
        };
        let expired_benchmark = crate::model::PriceSnapshot {
            schema_version: 1,
            symbol: "^GSPC".into(),
            price: Decimal::from(5000),
            previous_close: Some(Decimal::from(5000)),
            currency: "USD".into(),
            exchange: "SNP".into(),
            market_time_ms: Some(stale_ms),
            fetched_at_ms: stale_ms,
            source_url: "https://example.test/gspc".into(),
        };
        store::write_json_atomic(&paths.price_path("ACME"), &stale_price).unwrap();
        store::write_json_atomic(&paths.price_path("SBCWW"), &fresh_warrant_price).unwrap();
        store::write_json_atomic(&paths.price_path("^GSPC"), &expired_benchmark).unwrap();

        // Seed a stale cached report claiming InStrikeZone from an old quote.
        let mut stale_cached_report = crate::calculator::build_report(
            &acme_entry,
            Some(&acme_facts),
            Some(&stale_price),
            stale_ms,
        );
        stale_cached_report.status = crate::model::StrikeZoneStatus::InStrikeZone;
        store::write_json_atomic(&paths.computed_report_path("ACME"), &stale_cached_report)
            .unwrap();

        let status = run(&paths, &["status".into()]).unwrap();
        assert_eq!(status["data"]["benchmark"], serde_json::Value::Null);
        let acme_row = &status["data"]["companies"][0];
        assert_eq!(acme_row["ticker"], "ACME");
        assert_eq!(acme_row["status"], "wait");
        assert_eq!(acme_row["price"], serde_json::Value::Null);
        assert_eq!(acme_row["distance_to_strike_pct"], serde_json::Value::Null);
        assert_eq!(acme_row["strict_fcf_yield_pct"], serde_json::Value::Null);

        let sbcww_row = &status["data"]["companies"][1];
        assert_eq!(sbcww_row["ticker"], "SBCWW");
        assert_ne!(sbcww_row["status"], "in_strike_zone");
        assert_eq!(sbcww_row["strike_price"], serde_json::Value::Null);
        assert_eq!(sbcww_row["distance_to_strike_pct"], serde_json::Value::Null);
        assert_eq!(sbcww_row["strict_fcf_yield_pct"], serde_json::Value::Null);

        let acme_rep = run(&paths, &["report".into(), "ACME".into()]).unwrap();
        assert_eq!(acme_rep["data"]["status"], "wait");
        assert_eq!(acme_rep["data"]["price"], serde_json::Value::Null);

        let sbcww_rep = run(&paths, &["report".into(), "SBCWW".into()]).unwrap();
        assert_ne!(sbcww_rep["data"]["status"], "in_strike_zone");
        assert_eq!(
            sbcww_rep["data"]["fy_valuation"]["strike_price"],
            serde_json::Value::Null
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
