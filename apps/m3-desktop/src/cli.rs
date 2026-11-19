use crate::{
    model::{Bucket, JournalAction, SyncStatusFile},
    store::{self, WorkspacePaths},
};
use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;

pub fn run(paths: &WorkspacePaths, args: &[String]) -> Result<serde_json::Value> {
    let sub = args.first().map(String::as_str).unwrap_or("status");
    match sub {
        "status" => cmd_status(paths),
        "sync" => cmd_sync(paths, &args[1..]),
        "report" => cmd_report(paths, &args[1..]),
        "dossier" => cmd_dossier(paths, &args[1..]),
        "add" => cmd_add(paths, &args[1..]),
        "journal" => cmd_journal(paths, &args[1..]),
        "start" => cmd_start(paths),
        _ => anyhow::bail!(
            "usage: m3 status | sync [--ticker SYM] [--prices-only] | report <SYM> | dossier <SYM> | add <SYM> ... | journal [list|record ...] | start"
        ),
    }
}

fn cmd_status(paths: &WorkspacePaths) -> Result<serde_json::Value> {
    let wl = store::load_watchlist(paths)?;
    let reports = store::load_or_compute_reports(paths)?;
    let journal = store::load_journal(paths)?;
    let sync_status: Option<SyncStatusFile> = store::read_json(&paths.sync_status_path()).ok();
    let benchmark = store::load_cached_price(paths, &wl.benchmark_symbol);

    let summary: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| {
            let val = r.ttm_valuation.as_ref().or(r.fy_valuation.as_ref());
            json!({
                "ticker": r.ticker,
                "name": r.name,
                "bucket": r.bucket,
                "status": r.status,
                "currency": r.currency,
                "price": r.price.as_ref().map(|p| p.price.normalize().to_string()),
                "strike_price": val.and_then(|v| v.strike_price.map(|s| s.normalize().to_string())),
                "distance_to_strike_pct": val.and_then(|v| v.distance_to_strike_pct.map(|d| format!("{:.2}", d))),
                "strict_fcf_yield_pct": val.and_then(|v| v.strict_fcf_yield_mcap_pct.map(|y| format!("{:.2}", y))),
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
    let report = store::recompute_ticker(paths, sym)?;
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
                        shares = Some(Decimal::from_str(raw)?);
                    }
                    "--falsification" => {
                        i += 1;
                        falsification = Some(args.get(i).context("--falsification requires text")?.clone());
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
        _ => anyhow::bail!("usage: m3 journal list | record --ticker SYM --action wait|out|too_hard|paper_buy|paper_sell --note ..."),
    }
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
    use crate::model::{CompanyEntry, DataSource, DecisionJournal, ValuationAssumptions, WatchlistFile};
    use std::fs;

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

        let _ = fs::remove_dir_all(&dir);
    }
}
