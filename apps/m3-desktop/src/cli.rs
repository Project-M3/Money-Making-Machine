use crate::{
    app_config,
    events::{self, EventCatchupOptions},
    lake::{LakeDb, UniverseFilter, UniverseSortBy, UniverseSummary},
    lake_l1::{L1Summary, SignalKind, SignalStatus, SimilarSearchFilter},
    model::{Bucket, JournalAction, SyncStatusFile, parse_decimal_token},
    store::{self, WorkspacePaths},
    universe::{self, UniverseSyncOptions},
    update::{self, ReleaseChannel, UpdateOptions},
};
use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use serde_json::json;
use std::{path::PathBuf, str::FromStr, time::Instant};

/// Execute workspace-independent CLI subcommands (`version`, `workspace`, `update`).
/// Returns `None` if the subcommand requires an initialized M3 workspace.
pub fn run_standalone(args: &[String]) -> Option<Result<serde_json::Value>> {
    let sub = args.first().map(String::as_str)?;
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "version" => Some(cmd_version(rest)),
        "workspace" => Some(cmd_workspace(rest)),
        "update" => Some(cmd_update(rest)),
        _ => None,
    }
}

pub fn run(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    if let Some(standalone) = run_standalone(args) {
        return standalone;
    }
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
        "events" => cmd_events(paths, rest),
        "inbox" => cmd_inbox(paths, rest),
        "start" => cmd_start(paths),
        _ => anyhow::bail!(
            "usage: m3 status | sync [--ticker SYM] [--prices-only] | report <SYM> | dossier <SYM> | add <SYM> ... | journal [list|record ...] | universe [status|sync|screen|peers|similar ...] | events [catchup|list ...] | inbox [list|triage ...] | version | workspace [show|set <PATH>] | update [--check] [--channel stable|beta] [channel [stable|beta]] | start"
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
const MAX_SIMILAR_LIMIT: usize = 200;
const MAX_L1_LIST_LIMIT: usize = 5_000;
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
        "similar" => cmd_universe_similar(paths, rest),
        _ => anyhow::bail!(
            "usage: m3 universe <status|sync [--full] [--prices-top N] [--skip-prices]|screen [--min-roic N] [--max-sbc-ocf N] [--max-share-change-3y N] [--max-net-debt-to-oe N] [--min-years N] [--exclude-financials|--include-financials] [--positive-oe-only] [--price-pool-only] [--strike-only] [--query TEXT] [--sort COL] [--asc|--desc] [--limit N]|peers <TICKER> [--limit N]|similar <TICKER_OR_QUERY> [--ticker] [--min-roic N] [--max-sbc-ocf N] [--exclude-financials|--include-financials] [--limit N]>"
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

fn cmd_universe_similar(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut query_or_ticker: Option<String> = None;
    let mut as_ticker = false;
    let mut filter = SimilarSearchFilter::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ticker" => as_ticker = true,
            "--min-roic" => {
                filter.min_roic_5y = Some(parse_decimal_flag(args, &mut i, "--min-roic")?);
            }
            "--max-sbc-ocf" => {
                filter.max_sbc_to_ocf = Some(parse_decimal_flag(args, &mut i, "--max-sbc-ocf")?);
            }
            "--exclude-financials" => filter.exclude_financials = true,
            "--include-financials" => filter.exclude_financials = false,
            "--positive-oe-only" => filter.positive_strict_oe_only = true,
            "--price-pool-only" => filter.in_price_pool_only = true,
            "--limit" => {
                filter.limit = parse_usize_flag(args, &mut i, "--limit", 1, MAX_SIMILAR_LIMIT)?;
            }
            other if other.starts_with('-') => {
                anyhow::bail!("unknown universe similar flag: {other}");
            }
            text => {
                ensure!(
                    query_or_ticker.is_none(),
                    "unexpected extra argument for universe similar: {text}"
                );
                query_or_ticker = Some(text.trim().to_owned());
            }
        }
        i += 1;
    }

    let query_or_ticker = query_or_ticker
        .filter(|q| !q.is_empty())
        .context("usage: m3 universe similar <TICKER_OR_QUERY> [--ticker] [--min-roic N] [--max-sbc-ocf N] [--exclude-financials|--include-financials] [--limit N]")?;
    let db = LakeDb::open_read_only_if_exists(paths)?
        .context("universe lake has not been synced yet; run `m3 universe sync` first")?;
    let started = Instant::now();
    let hits = db.search_similar(&query_or_ticker, as_ticker, &filter)?;
    let query_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "query_ms": query_ms,
            "query": query_or_ticker,
            "as_ticker": as_ticker,
            "count": hits.len(),
            "filter": filter,
            "hits": hits,
        }
    }))
}

fn cmd_events(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "catchup" => cmd_events_catchup(paths, rest),
        "list" => cmd_events_list(paths, rest),
        _ => anyhow::bail!(
            "usage: m3 events <catchup [--bootstrap] [--skip-sec] [--skip-news] [--skip-docs] [--max-ciks N] [--max-docs N] [--max-news-tickers N]|list [--ticker SYM] [--type TYPE] [--limit N]>"
        ),
    }
}

fn cmd_events_catchup(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut opts = EventCatchupOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bootstrap" => opts.bootstrap = true,
            "--skip-sec" => opts.skip_sec = true,
            "--skip-news" => opts.skip_news = true,
            "--skip-docs" => opts.skip_docs = true,
            "--max-ciks" => {
                opts.max_ciks = parse_usize_flag(args, &mut i, "--max-ciks", 1, 5_000)?;
            }
            "--max-docs" => {
                opts.max_docs = parse_usize_flag(args, &mut i, "--max-docs", 1, 1_000)?;
            }
            "--max-news-tickers" => {
                opts.max_news_tickers =
                    parse_usize_flag(args, &mut i, "--max-news-tickers", 1, 1_000)?;
            }
            other => anyhow::bail!("unknown events catchup flag: {other}"),
        }
        i += 1;
    }

    let report = events::catchup_events_with_progress(paths, &opts, |msg| {
        eprintln!("[m3 events catchup] {msg}");
    })?;
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": report
    }))
}

fn cmd_events_list(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut ticker: Option<String> = None;
    let mut event_type: Option<String> = None;
    let mut unprocessed_only = false;
    let mut limit: usize = 50;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ticker" => {
                i += 1;
                let raw = args.get(i).context("--ticker requires symbol")?.trim();
                ensure!(!raw.is_empty(), "--ticker cannot be empty");
                ticker = Some(raw.to_ascii_uppercase());
            }
            "--type" => {
                i += 1;
                let raw = args.get(i).context("--type requires event type")?.trim();
                ensure!(!raw.is_empty(), "--type cannot be empty");
                event_type = Some(raw.to_owned());
            }
            "--unprocessed-only" => unprocessed_only = true,
            "--limit" => {
                limit = parse_usize_flag(args, &mut i, "--limit", 1, MAX_L1_LIST_LIMIT)?;
            }
            other => anyhow::bail!("unknown events list flag: {other}"),
        }
        i += 1;
    }

    let events = match LakeDb::open_read_only_if_exists(paths)? {
        Some(db) => db.list_events(
            ticker.as_deref(),
            event_type.as_deref(),
            unprocessed_only,
            limit,
        )?,
        None => Vec::new(),
    };

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "count": events.len(),
            "events": events,
        }
    }))
}

fn cmd_inbox(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "list" => cmd_inbox_list(paths, rest),
        "triage" => cmd_inbox_triage(paths, rest),
        _ => anyhow::bail!(
            "usage: m3 inbox <list [--status unread|all|watch|too_hard|researching|archived] [--kind discovery|ripple|strike|red_flag] [--limit N]|triage <SIGNAL_ID> --status unread|researching|watch|too_hard|archived [--promote-watch]>"
        ),
    }
}

fn cmd_inbox_list(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut status_raw = "all".to_owned();
    let mut status: Option<SignalStatus> = None;
    let mut kind: Option<SignalKind> = None;
    let mut limit: usize = 50;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--status" => {
                i += 1;
                let raw = args
                    .get(i)
                    .context("--status requires unread|all|watch|too_hard|researching|archived")?;
                let lower = raw.trim().to_ascii_lowercase();
                if lower == "all" {
                    status = None;
                    status_raw = "all".to_owned();
                } else {
                    let parsed = SignalStatus::from_str(&lower)?;
                    status = Some(parsed);
                    status_raw = parsed.as_str().to_owned();
                }
            }
            "--kind" => {
                i += 1;
                let raw = args
                    .get(i)
                    .context("--kind requires discovery|ripple|strike|red_flag")?;
                kind = Some(SignalKind::from_str(raw)?);
            }
            "--limit" => {
                limit = parse_usize_flag(args, &mut i, "--limit", 1, MAX_L1_LIST_LIMIT)?;
            }
            other => anyhow::bail!("unknown inbox list flag: {other}"),
        }
        i += 1;
    }

    let (summary, signals) = match LakeDb::open_read_only_if_exists(paths)? {
        Some(db) => (
            db.l1_summary()?,
            db.list_inbox_signals(status, kind, limit)?,
        ),
        None => (L1Summary::default(), Vec::new()),
    };

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "status_filter": status_raw,
            "kind_filter": kind.map(|k| k.as_str()),
            "summary": summary,
            "count": signals.len(),
            "signals": signals,
        }
    }))
}

fn cmd_inbox_triage(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let mut signal_id: Option<String> = None;
    let mut status: Option<SignalStatus> = None;
    let mut promote_watch = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--status" => {
                i += 1;
                let raw = args
                    .get(i)
                    .context("--status requires unread|researching|watch|too_hard|archived")?;
                status = Some(SignalStatus::from_str(raw)?);
            }
            "--promote-watch" => promote_watch = true,
            other if other.starts_with('-') => {
                anyhow::bail!("unknown inbox triage flag: {other}");
            }
            sid => {
                ensure!(
                    signal_id.is_none(),
                    "unexpected extra argument for inbox triage: {sid}"
                );
                signal_id = Some(sid.trim().to_owned());
            }
        }
        i += 1;
    }

    let signal_id = signal_id
        .filter(|s| !s.is_empty())
        .context("usage: m3 inbox triage <SIGNAL_ID> --status unread|researching|watch|too_hard|archived [--promote-watch]")?;
    let target_status = status.context("--status is required for inbox triage")?;

    ensure!(
        LakeDb::open_read_only_if_exists(paths)?.is_some(),
        "universe lake has not been synced yet; run `m3 universe sync` first"
    );
    let db = LakeDb::open(paths)?;
    let now_ms = store::now_ms();
    let updated = db.triage_signal(&signal_id, target_status, now_ms)?;
    ensure!(updated, "signal '{signal_id}' not found in signal_inbox");

    let signal = db
        .list_inbox_signals(None, None, 10_000)?
        .into_iter()
        .find(|s| s.signal_id == signal_id)
        .context("triaged signal disappeared from signal_inbox")?;

    let promoted = if promote_watch {
        let ticker = signal.primary_ticker.trim().to_ascii_uppercase();
        let u_row = db
            .screen_universe(&UniverseFilter {
                exclude_financials: false,
                query: Some(ticker.clone()),
                limit: 20,
                ..Default::default()
            })
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .find(|r| r.ticker.eq_ignore_ascii_case(&ticker))
            });
        let (name_opt, cik_opt, circle_opt) = if let Some(row) = &u_row {
            let circle = match (row.sic, row.sic_description.as_deref()) {
                (Some(sic), Some(desc)) if !desc.trim().is_empty() => {
                    Some(format!("{} / SIC {sic}", desc.trim()))
                }
                (None, Some(desc)) if !desc.trim().is_empty() => Some(desc.trim().to_owned()),
                (Some(sic), _) => Some(format!("SIC {sic}")),
                _ => None,
            };
            let cik = (!row.cik.trim().is_empty()).then_some(row.cik.as_str());
            (Some(row.name.as_str()), cik, circle)
        } else {
            (None, None, None)
        };
        let entry = store::add_or_update_company_with_cik(
            paths,
            &ticker,
            Bucket::Watch,
            name_opt,
            cik_opt,
            circle_opt.as_deref(),
            None,
        )?;
        db.refresh_watchlist_flags(paths)?;
        Some(entry)
    } else {
        None
    };

    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "signal": signal,
            "promoted_to_watch": promoted,
        }
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

fn cmd_version(args: &[String]) -> Result<serde_json::Value> {
    if let Some(unexpected) = args.first().filter(|a| a.as_str() != "--json") {
        anyhow::bail!("unknown version argument: {unexpected}");
    }
    let (effective_channel, channel_source) = update::effective_channel();
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": {
            "version": update::current_binary_version(),
            "long_version": update::current_long_version(),
            "channel": update::current_binary_channel().as_str(),
            "effective_channel": effective_channel.as_str(),
            "channel_source": channel_source.as_str(),
            "commit": update::current_build_commit(),
            "target": update::semver::current_target_triple().ok(),
            "exe_path": std::env::current_exe().ok().map(|p| p.display().to_string()),
            "workspace_root": store::resolve_repo_root().ok().map(|p| p.display().to_string()),
        }
    }))
}

fn cmd_workspace(args: &[String]) -> Result<serde_json::Value> {
    let sub = args.first().map(String::as_str).unwrap_or("show");
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "show" => {
            if let Some(unexpected) = rest.first().filter(|a| a.as_str() != "--json") {
                anyhow::bail!("unknown workspace show argument: {unexpected}");
            }
            let cfg_path = app_config::config_path()?;
            let configured = app_config::read_app_config()?.and_then(|c| c.workspace_root);
            let resolved = store::resolve_repo_root()
                .ok()
                .map(|p| p.display().to_string());
            Ok(json!({
                "schema_version": 1,
                "ok": true,
                "data": {
                    "config_path": cfg_path.display().to_string(),
                    "configured_workspace_root": configured,
                    "resolved_workspace_root": resolved,
                }
            }))
        }
        "set" => {
            let target = rest
                .first()
                .filter(|s| !s.starts_with('-'))
                .context("usage: m3 workspace set <PATH>")?;
            if let Some(extra) = rest.get(1).filter(|a| a.as_str() != "--json") {
                anyhow::bail!("unexpected extra argument for workspace set: {extra}");
            }
            let saved = app_config::write_workspace_root(std::path::Path::new(target))?;
            Ok(json!({
                "schema_version": 1,
                "ok": true,
                "data": {
                    "config_path": app_config::config_path()?.display().to_string(),
                    "workspace_root": saved.display().to_string(),
                }
            }))
        }
        _ => anyhow::bail!("usage: m3 workspace [show | set <PATH>]"),
    }
}

fn cmd_update(args: &[String]) -> Result<serde_json::Value> {
    if args.first().map(String::as_str) == Some("channel") {
        let rest = args.get(1..).unwrap_or(&[]);
        return cmd_update_channel(rest);
    }

    let mut opts = UpdateOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => opts.check_only = true,
            "--force" => opts.force = true,
            "--no-cli-link" => opts.no_cli_link = true,
            "--json" => {}
            "--channel" => {
                i += 1;
                let raw = args
                    .get(i)
                    .context("--channel requires `stable` or `beta`")?;
                opts.channel = Some(ReleaseChannel::parse(raw)?);
            }
            "--version" => {
                i += 1;
                let raw = args.get(i).context("--version requires a version string")?;
                opts.version = Some(raw.clone());
            }
            "--app-dir" => {
                i += 1;
                let raw = args.get(i).context("--app-dir requires a directory path")?;
                opts.app_dir = Some(PathBuf::from(raw));
            }
            "--bin-dir" => {
                i += 1;
                let raw = args.get(i).context("--bin-dir requires a directory path")?;
                opts.bin_dir = Some(PathBuf::from(raw));
            }
            "--manifest-url" => {
                i += 1;
                let raw = args.get(i).context("--manifest-url requires a URL")?;
                opts.manifest_url = Some(raw.clone());
            }
            other => anyhow::bail!(
                "unknown update argument `{other}`; usage: m3 update [--check] [--channel stable|beta] [--version VER] [--force] [--app-dir DIR] [--bin-dir DIR] [--no-cli-link] [--manifest-url URL] [--json] | channel [stable|beta]"
            ),
        }
        i += 1;
    }

    let report = update::run_update(&opts)?;
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": report
    }))
}

fn cmd_update_channel(args: &[String]) -> Result<serde_json::Value> {
    let mut target: Option<ReleaseChannel> = None;
    for arg in args {
        if arg == "--json" {
            continue;
        }
        if arg.starts_with('-') {
            anyhow::bail!("unknown update channel flag: {arg}");
        }
        ensure!(
            target.is_none(),
            "unexpected extra argument `{arg}` for `m3 update channel [stable|beta]`"
        );
        target = Some(ReleaseChannel::parse(arg)?);
    }

    if let Some(ch) = target {
        update::channel::write_configured_channel(ch)?;
    }
    let status = update::channel::channel_status_report();
    Ok(json!({
        "schema_version": 1,
        "ok": true,
        "data": status
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

    #[test]
    fn cli_l1_events_inbox_and_universe_similar_validation_and_queries() {
        use crate::{
            lake_l1::{DocChunkRow, LakeEventRow, SignalEvidence, SignalInboxRow},
            latent::{
                LOCAL_EMBEDDING_DIM, LOCAL_EMBEDDING_MODEL, encode_embedding_json, local_embed,
                sha256_hex,
            },
        };

        let dir = std::env::temp_dir().join(format!(
            "m3-cli-l1-suite-{}-{}",
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

        // Empty-lake defaults for events list & inbox list; unsynced error for similar & triage.
        let empty_ev = run(&paths, &["events".into(), "list".into()]).unwrap();
        assert_eq!(empty_ev["ok"], true);
        assert_eq!(empty_ev["data"]["count"], 0);

        let empty_inbox = run(&paths, &["inbox".into(), "list".into()]).unwrap();
        assert_eq!(empty_inbox["ok"], true);
        assert_eq!(empty_inbox["data"]["count"], 0);

        let sim_unsynced = run(
            &paths,
            &["universe".into(), "similar".into(), "ADSK".into()],
        )
        .unwrap_err();
        assert!(sim_unsynced.to_string().contains("m3 universe sync"));

        let triage_unsynced = run(
            &paths,
            &[
                "inbox".into(),
                "triage".into(),
                "sig-1".into(),
                "--status".into(),
                "watch".into(),
            ],
        )
        .unwrap_err();
        assert!(triage_unsynced.to_string().contains("m3 universe sync"));

        // Flag validation checks.
        assert!(
            run(
                &paths,
                &[
                    "events".into(),
                    "catchup".into(),
                    "--max-ciks".into(),
                    "0".into()
                ]
            )
            .is_err()
        );
        assert!(
            run(
                &paths,
                &[
                    "inbox".into(),
                    "list".into(),
                    "--status".into(),
                    "bogus".into()
                ]
            )
            .is_err()
        );
        assert!(
            run(
                &paths,
                &[
                    "inbox".into(),
                    "list".into(),
                    "--kind".into(),
                    "bogus".into()
                ]
            )
            .is_err()
        );
        assert!(run(&paths, &["inbox".into(), "triage".into(), "sig-1".into()]).is_err());
        assert!(
            run(
                &paths,
                &[
                    "universe".into(),
                    "similar".into(),
                    "--limit".into(),
                    "201".into(),
                    "ADSK".into()
                ]
            )
            .is_err()
        );

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
            let db = LakeDb::open(&paths).unwrap();
            let mut db_mut = db;
            db_mut
                .replace_sec_lake_data(&[], &[], &[], &snapshot)
                .unwrap();
            db_mut.ensure_l1_schema(&paths).unwrap();

            let adsk_text = "Autodesk builds enterprise engineering CAD design and subscription construction software.";
            let ptc_text = "PTC provides industrial engineering CAD and PLM subscription software for manufacturing.";
            let bac_text =
                "Bank of America provides commercial banking deposits and consumer credit loans.";
            let chunks = vec![
                DocChunkRow {
                    chunk_id: "chunk-adsk-1".into(),
                    cik: "0000769397".into(),
                    ticker: "ADSK".into(),
                    doc_type: "10-K".into(),
                    source_id: "acc-adsk-1".into(),
                    period_end: Some("2026-01-31".into()),
                    filing_date: Some("2026-03-10".into()),
                    section: "item_1".into(),
                    title: "Business".into(),
                    content: adsk_text.into(),
                    content_sha256: sha256_hex(adsk_text.as_bytes()),
                    source_url: "https://www.sec.gov/Archives/edgar/data/769397/adsk.htm".into(),
                    embedding_model: LOCAL_EMBEDDING_MODEL.into(),
                    embedding_dim: LOCAL_EMBEDDING_DIM,
                    embedding_json: encode_embedding_json(&local_embed(adsk_text)),
                },
                DocChunkRow {
                    chunk_id: "chunk-ptc-1".into(),
                    cik: "0000857005".into(),
                    ticker: "PTC".into(),
                    doc_type: "10-K".into(),
                    source_id: "acc-ptc-1".into(),
                    period_end: Some("2025-09-30".into()),
                    filing_date: Some("2025-11-15".into()),
                    section: "item_1".into(),
                    title: "Business".into(),
                    content: ptc_text.into(),
                    content_sha256: sha256_hex(ptc_text.as_bytes()),
                    source_url: "https://www.sec.gov/Archives/edgar/data/857005/ptc.htm".into(),
                    embedding_model: LOCAL_EMBEDDING_MODEL.into(),
                    embedding_dim: LOCAL_EMBEDDING_DIM,
                    embedding_json: encode_embedding_json(&local_embed(ptc_text)),
                },
                DocChunkRow {
                    chunk_id: "chunk-bac-1".into(),
                    cik: "0000070858".into(),
                    ticker: "BAC".into(),
                    doc_type: "10-K".into(),
                    source_id: "acc-bac-1".into(),
                    period_end: Some("2025-12-31".into()),
                    filing_date: Some("2026-02-20".into()),
                    section: "item_1".into(),
                    title: "Business".into(),
                    content: bac_text.into(),
                    content_sha256: sha256_hex(bac_text.as_bytes()),
                    source_url: "https://www.sec.gov/Archives/edgar/data/70858/bac.htm".into(),
                    embedding_model: LOCAL_EMBEDDING_MODEL.into(),
                    embedding_dim: LOCAL_EMBEDDING_DIM,
                    embedding_json: encode_embedding_json(&local_embed(bac_text)),
                },
            ];
            db_mut.upsert_doc_chunks(&chunks).unwrap();

            db_mut
                .upsert_events(&[LakeEventRow {
                    event_id: "ev-ptc-8k".into(),
                    source: "sec_edgar".into(),
                    cik: Some("0000857005".into()),
                    ticker: Some("PTC".into()),
                    event_type: "sec_8k".into(),
                    occurred_at_ms: 1_795_450_000_000,
                    filing_date: Some("2026-11-20".into()),
                    title: "PTC Form 8-K Item 2.02".into(),
                    summary: "Strong recurring ARR growth".into(),
                    url: "https://www.sec.gov/Archives/edgar/data/857005/8k.htm".into(),
                    metadata_json: "{}".into(),
                    processed_mask: 0,
                }])
                .unwrap();

            db_mut
                .upsert_signals(&[
                    SignalInboxRow {
                        signal_id: "sig-ptc-disc".into(),
                        kind: SignalKind::Discovery,
                        score: 0.91,
                        primary_ticker: "PTC".into(),
                        tickers: vec!["PTC".into()],
                        title: "PTC 新浮现高质量击球候选".into(),
                        summary: "5Y ROIC 22%, 位于击球区".into(),
                        evidence: vec![SignalEvidence {
                            chunk_id: Some("chunk-ptc-1".into()),
                            ticker: "PTC".into(),
                            doc_type: "10-K".into(),
                            section: "item_1".into(),
                            filing_date: Some("2025-11-15".into()),
                            title: "Business".into(),
                            quote: ptc_text.into(),
                            source_url: "https://www.sec.gov/Archives/edgar/data/857005/ptc.htm"
                                .into(),
                            similarity: 0.91,
                        }],
                        status: SignalStatus::Unread,
                        event_id: Some("ev-ptc-8k".into()),
                        created_at_ms: 1_795_450_000_000,
                        updated_at_ms: 1_795_450_000_000,
                    },
                    SignalInboxRow {
                        signal_id: "sig-adsk-ripple".into(),
                        kind: SignalKind::Ripple,
                        score: 0.77,
                        primary_ticker: "ADSK".into(),
                        tickers: vec!["ADSK".into(), "PTC".into()],
                        title: "PTC 8-K 隐空间传导至 ADSK".into(),
                        summary: "工业 CAD 订阅景气度共振".into(),
                        evidence: Vec::new(),
                        status: SignalStatus::Unread,
                        event_id: Some("ev-ptc-8k".into()),
                        created_at_ms: 1_795_450_000_000,
                        updated_at_ms: 1_795_450_000_000,
                    },
                ])
                .unwrap();
        }

        // `m3 events catchup --skip-sec --skip-news --skip-docs` offline check.
        let catchup = run(
            &paths,
            &[
                "events".into(),
                "catchup".into(),
                "--skip-sec".into(),
                "--skip-news".into(),
                "--skip-docs".into(),
                "--max-ciks".into(),
                "5".into(),
                "--max-docs".into(),
                "2".into(),
                "--max-news-tickers".into(),
                "2".into(),
            ],
        )
        .unwrap();
        assert_eq!(catchup["ok"], true);
        assert_eq!(catchup["data"]["flagship_tokens_used"], 0);

        // `m3 events list`
        let ev_list = run(
            &paths,
            &[
                "events".into(),
                "list".into(),
                "--ticker".into(),
                "ptc".into(),
                "--type".into(),
                "sec_8k".into(),
                "--limit".into(),
                "10".into(),
            ],
        )
        .unwrap();
        assert_eq!(ev_list["ok"], true);
        assert_eq!(ev_list["data"]["count"], 1);
        assert_eq!(ev_list["data"]["events"][0]["event_id"], "ev-ptc-8k");

        // `m3 inbox list`
        let inbox_disc = run(
            &paths,
            &[
                "inbox".into(),
                "list".into(),
                "--status".into(),
                "unread".into(),
                "--kind".into(),
                "discovery".into(),
                "--limit".into(),
                "10".into(),
            ],
        )
        .unwrap();
        assert_eq!(inbox_disc["ok"], true);
        let disc_signals = inbox_disc["data"]["signals"].as_array().unwrap();
        let ptc_disc = disc_signals
            .iter()
            .find(|s| s["signal_id"] == "sig-ptc-disc")
            .expect("sig-ptc-disc should appear in unread discovery signals");
        assert_eq!(ptc_disc["evidence"][0]["doc_type"], "10-K");

        // `m3 inbox triage ... --promote-watch`
        let triaged = run(
            &paths,
            &[
                "inbox".into(),
                "triage".into(),
                "sig-ptc-disc".into(),
                "--status".into(),
                "watch".into(),
                "--promote-watch".into(),
            ],
        )
        .unwrap();
        assert_eq!(triaged["ok"], true);
        assert_eq!(triaged["data"]["signal"]["status"], "watch");
        assert_eq!(triaged["data"]["promoted_to_watch"]["ticker"], "PTC");
        assert_eq!(triaged["data"]["promoted_to_watch"]["bucket"], "watch");

        let unread_after = run(
            &paths,
            &[
                "inbox".into(),
                "list".into(),
                "--status".into(),
                "unread".into(),
            ],
        )
        .unwrap();
        let unread_signals = unread_after["data"]["signals"].as_array().unwrap();
        assert!(
            unread_signals
                .iter()
                .all(|s| s["signal_id"] != "sig-ptc-disc")
        );
        assert!(
            unread_signals
                .iter()
                .any(|s| s["signal_id"] == "sig-adsk-ripple")
        );

        // `m3 universe similar` (by ticker and by text + financial hard filter)
        let sim_ticker = run(
            &paths,
            &[
                "universe".into(),
                "similar".into(),
                "ADSK".into(),
                "--ticker".into(),
                "--limit".into(),
                "5".into(),
            ],
        )
        .unwrap();
        assert_eq!(sim_ticker["ok"], true);
        assert_eq!(sim_ticker["data"]["as_ticker"], true);
        assert_eq!(sim_ticker["data"]["count"], 1);
        assert_eq!(sim_ticker["data"]["hits"][0]["company"]["ticker"], "PTC");
        assert_eq!(
            sim_ticker["data"]["hits"][0]["company"]["in_watchlist"],
            true
        );

        let sim_query = run(
            &paths,
            &[
                "universe".into(),
                "similar".into(),
                "enterprise engineering CAD subscription software".into(),
                "--min-roic".into(),
                "20".into(),
                "--max-sbc-ocf".into(),
                "25".into(),
                "--exclude-financials".into(),
                "--limit".into(),
                "5".into(),
            ],
        )
        .unwrap();
        assert_eq!(sim_query["ok"], true);
        assert_eq!(sim_query["data"]["count"], 1);
        assert_eq!(sim_query["data"]["hits"][0]["company"]["ticker"], "PTC");

        let sim_with_fin = run(
            &paths,
            &[
                "universe".into(),
                "similar".into(),
                "banking deposits credit".into(),
                "--include-financials".into(),
                "--limit".into(),
                "5".into(),
            ],
        )
        .unwrap();
        assert_eq!(sim_with_fin["ok"], true);
        assert_eq!(sim_with_fin["data"]["hits"][0]["company"]["ticker"], "BAC");

        let _ = fs::remove_dir_all(&dir);
    }
}
