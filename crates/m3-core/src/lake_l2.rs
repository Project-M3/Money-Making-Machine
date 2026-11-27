//! L2 Lakehouse schema (`research_runs`), Deep Research audit lifecycle,
//! and monthly/weekly budget aggregation.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use duckdb::{OptionalExt, ToSql, params, params_from_iter};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget::{self, BudgetConfig, ThemesFile};
use crate::lake::{LakeDb, ReadSnapshotTx};
use crate::model::serde_decimal;
use crate::store::{self, WorkspacePaths};

/// Execution status of a persisted L2 Deep Research run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchRunStatus {
    Running,
    Completed,
    BlockedBudget,
    Failed,
}

/// Default age threshold (15 minutes) after which an unfinished `running` Deep Research row is treated as stale.
pub const DEFAULT_STALE_RESEARCH_RUN_MS: u64 = 15 * 60 * 1000;

impl ResearchRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::BlockedBudget => "blocked_budget",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for ResearchRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ResearchRunStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "blocked_budget" => Ok(Self::BlockedBudget),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!(
                "unknown research run status '{other}'; expected running|completed|blocked_budget|failed"
            ),
        }
    }
}

/// Audit row stored in `m3.duckdb`'s `research_runs` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchRunRow {
    pub run_id: String,
    pub ticker: String,
    pub trigger: String,
    pub focus: Option<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(with = "serde_decimal")]
    pub cost_usd: Decimal,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub status: ResearchRunStatus,
    pub error: Option<String>,
    pub memo_path: Option<String>,
    pub perspectives_json: String,
    pub citations_json: String,
}

/// Aggregated AI token and USD usage for the current UTC calendar month
/// (plus rolling 7-day auto-triggered run count).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyBudgetUsage {
    pub month_utc: String,
    pub total_runs: usize,
    pub completed_runs: usize,
    pub blocked_runs: usize,
    pub failed_runs: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    #[serde(with = "serde_decimal")]
    pub total_cost_usd: Decimal,
    pub weekly_auto_runs: usize,
}

/// Combined budget configuration, monthly usage, and remaining headroom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetStatusReport {
    pub config: BudgetConfig,
    pub usage: MonthlyBudgetUsage,
    #[serde(with = "serde_decimal")]
    pub remaining_monthly_usd: Decimal,
    pub remaining_monthly_tokens: u64,
    pub auto_trigger_remaining_weekly: usize,
}

impl LakeDb {
    /// Creates additive Milestone C L2 tables (`research_runs`).
    pub fn ensure_l2_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS research_runs (
                    run_id TEXT PRIMARY KEY,
                    ticker TEXT NOT NULL,
                    trigger TEXT NOT NULL,
                    focus TEXT,
                    model TEXT NOT NULL,
                    input_tokens BIGINT NOT NULL,
                    output_tokens BIGINT NOT NULL,
                    cost_usd TEXT NOT NULL,
                    started_at_ms BIGINT NOT NULL,
                    finished_at_ms BIGINT,
                    status TEXT NOT NULL,
                    error TEXT,
                    memo_path TEXT,
                    perspectives_json TEXT NOT NULL,
                    citations_json TEXT NOT NULL
                );
                ",
            )
            .context("failed to initialize L2 DuckDB research_runs table")?;
        Ok(())
    }

    fn l2_table_exists(&self, table_name: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
            [table_name],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Inserts a new `research_runs` audit row.
    pub fn insert_research_run(&self, row: &ResearchRunRow) -> Result<()> {
        let input_i64 = i64::try_from(row.input_tokens).unwrap_or(i64::MAX);
        let output_i64 = i64::try_from(row.output_tokens).unwrap_or(i64::MAX);
        let started_i64 = i64::try_from(row.started_at_ms).unwrap_or(i64::MAX);
        let finished_i64 = row
            .finished_at_ms
            .map(|v| i64::try_from(v).unwrap_or(i64::MAX));
        let cost_str = row.cost_usd.normalize().to_string();

        self.conn
            .execute(
                "INSERT OR REPLACE INTO research_runs (
                    run_id, ticker, trigger, focus, model, input_tokens, output_tokens,
                    cost_usd, started_at_ms, finished_at_ms, status, error, memo_path,
                    perspectives_json, citations_json
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    row.run_id,
                    row.ticker.trim().to_ascii_uppercase(),
                    row.trigger,
                    row.focus,
                    row.model,
                    input_i64,
                    output_i64,
                    cost_str,
                    started_i64,
                    finished_i64,
                    row.status.as_str(),
                    row.error,
                    row.memo_path,
                    row.perspectives_json,
                    row.citations_json,
                ],
            )
            .with_context(|| format!("failed to insert research_run {}", row.run_id))?;
        Ok(())
    }

    /// Updates a `research_runs` row with final execution metrics and outcome.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_research_run(
        &self,
        run_id: &str,
        status: ResearchRunStatus,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Decimal,
        finished_at_ms: u64,
        error: Option<&str>,
        memo_path: Option<&str>,
        perspectives_json: &str,
        citations_json: &str,
    ) -> Result<bool> {
        if !self.l2_table_exists("research_runs")? {
            return Ok(false);
        }
        let input_i64 = i64::try_from(input_tokens).unwrap_or(i64::MAX);
        let output_i64 = i64::try_from(output_tokens).unwrap_or(i64::MAX);
        let finished_i64 = i64::try_from(finished_at_ms).unwrap_or(i64::MAX);
        let cost_str = cost_usd.normalize().to_string();

        let changed = self
            .conn
            .execute(
                "UPDATE research_runs
                 SET status = ?,
                     input_tokens = ?,
                     output_tokens = ?,
                     cost_usd = ?,
                     finished_at_ms = ?,
                     error = ?,
                     memo_path = ?,
                     perspectives_json = ?,
                     citations_json = ?
                 WHERE run_id = ?",
                params![
                    status.as_str(),
                    input_i64,
                    output_i64,
                    cost_str,
                    finished_i64,
                    error,
                    memo_path,
                    perspectives_json,
                    citations_json,
                    run_id,
                ],
            )
            .with_context(|| format!("failed to finish research_run {run_id}"))?;
        Ok(changed > 0)
    }

    /// Marks stale `running` rows in `research_runs` (where
    /// `now_ms.saturating_sub(started_at_ms) >= stale_after_ms`) as `failed` with a clear
    /// interrupted note and `finished_at_ms = now_ms`.
    pub fn reclaim_stale_running_runs(&self, now_ms: u64, stale_after_ms: u64) -> Result<usize> {
        if !self.l2_table_exists("research_runs")? {
            return Ok(0);
        }
        if now_ms < stale_after_ms {
            return Ok(0);
        }
        let now_i64 = i64::try_from(now_ms).unwrap_or(i64::MAX);
        let note = "Interrupted before completion (stale running run reclaimed).";
        let updated = if stale_after_ms == 0 {
            self.conn
                .execute(
                    "UPDATE research_runs
                     SET status = 'failed',
                         finished_at_ms = ?,
                         error = COALESCE(NULLIF(TRIM(error), ''), ?)
                     WHERE LOWER(status) = 'running'",
                    params![now_i64, note],
                )
                .context("failed to reclaim stale running research_runs")?
        } else {
            let cutoff_ms = now_ms.saturating_sub(stale_after_ms);
            let cutoff_i64 = i64::try_from(cutoff_ms).unwrap_or(i64::MAX);
            self.conn
                .execute(
                    "UPDATE research_runs
                     SET status = 'failed',
                         finished_at_ms = ?,
                         error = COALESCE(NULLIF(TRIM(error), ''), ?)
                     WHERE LOWER(status) = 'running' AND started_at_ms <= ?",
                    params![now_i64, note, cutoff_i64],
                )
                .context("failed to reclaim stale running research_runs")?
        };
        Ok(updated)
    }

    /// Lists recent `research_runs` ordered newest first, optionally filtered by ticker.
    pub fn list_research_runs(
        &self,
        ticker: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResearchRunRow>> {
        if limit == 0 || !self.l2_table_exists("research_runs")? {
            return Ok(Vec::new());
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let mut owned_params: Vec<Box<dyn ToSql>> = Vec::new();
        let where_sql = if let Some(sym) = ticker.map(str::trim).filter(|s| !s.is_empty()) {
            owned_params.push(Box::new(sym.to_ascii_uppercase()));
            "WHERE UPPER(ticker) = ?"
        } else {
            ""
        };
        owned_params.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));

        let sql = format!(
            "SELECT run_id, ticker, trigger, focus, model, input_tokens, output_tokens,
                    cost_usd, started_at_ms, finished_at_ms, status, error, memo_path,
                    perspectives_json, citations_json
             FROM research_runs
             {where_sql}
             ORDER BY started_at_ms DESC, run_id DESC
             LIMIT ?"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let refs = owned_params.iter().map(|b| b.as_ref());
        let rows = stmt.query_map(params_from_iter(refs), read_research_run_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list research_runs")
    }

    /// Aggregates monthly UTC token/cost consumption and rolling 7-day auto-triggered run count.
    pub fn monthly_budget_usage(&self, as_of_ms: u64) -> Result<MonthlyBudgetUsage> {
        let (month_utc, month_start_ms, next_month_start_ms) = utc_month_bounds_ms(as_of_ms);
        let mut usage = MonthlyBudgetUsage {
            month_utc,
            total_runs: 0,
            completed_runs: 0,
            blocked_runs: 0,
            failed_runs: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: Decimal::ZERO,
            weekly_auto_runs: 0,
        };

        if !self.l2_table_exists("research_runs")? {
            return Ok(usage);
        }

        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let start_i64 = i64::try_from(month_start_ms).unwrap_or(i64::MAX);
        let end_i64 = i64::try_from(next_month_start_ms).unwrap_or(i64::MAX);

        let mut stmt = self.conn.prepare(
            "SELECT status, input_tokens, output_tokens, cost_usd
             FROM research_runs
             WHERE started_at_ms >= ? AND started_at_ms < ?",
        )?;
        let rows = stmt.query_map(params![start_i64, end_i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;

        for item in rows {
            let (raw_status, in_tok, out_tok, cost_str) = item?;
            usage.total_runs += 1;
            match ResearchRunStatus::from_str(&raw_status) {
                Ok(ResearchRunStatus::Completed) => usage.completed_runs += 1,
                Ok(ResearchRunStatus::BlockedBudget) => usage.blocked_runs += 1,
                Ok(ResearchRunStatus::Failed) => usage.failed_runs += 1,
                Ok(ResearchRunStatus::Running) | Err(_) => {}
            }
            usage.total_input_tokens = usage
                .total_input_tokens
                .saturating_add(u64::try_from(in_tok.max(0)).unwrap_or(0));
            usage.total_output_tokens = usage
                .total_output_tokens
                .saturating_add(u64::try_from(out_tok.max(0)).unwrap_or(0));
            if let Ok(cost) = Decimal::from_str(cost_str.trim()) {
                usage.total_cost_usd += cost.max(Decimal::ZERO);
            }
        }
        usage.total_cost_usd = usage.total_cost_usd.normalize();

        let week_start_ms = as_of_ms.saturating_sub(7 * 86_400_000);
        let week_start_i64 = i64::try_from(week_start_ms).unwrap_or(i64::MAX);
        let as_of_i64 = i64::try_from(as_of_ms).unwrap_or(i64::MAX);
        let auto_count: Option<i64> = self
            .conn
            .query_row(
                "SELECT COUNT(*)
                 FROM research_runs
                 WHERE started_at_ms >= ?
                   AND started_at_ms <= ?
                   AND LOWER(trigger) = 'auto'
                   AND LOWER(status) != 'blocked_budget'",
                params![week_start_i64, as_of_i64],
                |r| r.get(0),
            )
            .optional()?;
        usage.weekly_auto_runs = usize::try_from(auto_count.unwrap_or(0).max(0)).unwrap_or(0);

        Ok(usage)
    }

    /// Loads `workbench/budget.json` and computes current monthly usage & remaining headroom.
    pub fn budget_status_report(&self, paths: &WorkspacePaths) -> Result<BudgetStatusReport> {
        let config = budget::load_budget_config(paths)?;
        let usage = self.monthly_budget_usage(store::now_ms())?;
        let used_tokens = usage
            .total_input_tokens
            .saturating_add(usage.total_output_tokens);
        let remaining_monthly_usd = (config.monthly_budget_usd - usage.total_cost_usd)
            .max(Decimal::ZERO)
            .normalize();
        let remaining_monthly_tokens = config.monthly_token_limit.saturating_sub(used_tokens);
        let auto_trigger_remaining_weekly = if config.auto_trigger_enabled {
            config
                .weekly_auto_quota
                .saturating_sub(usage.weekly_auto_runs)
        } else {
            0
        };

        Ok(BudgetStatusReport {
            config,
            usage,
            remaining_monthly_usd,
            remaining_monthly_tokens,
            auto_trigger_remaining_weekly,
        })
    }

    /// Rebuilds and persists `workbench/themes.json` from the lakehouse while preserving `user_notes`.
    #[allow(dead_code)]
    pub fn sync_and_load_themes(&self, paths: &WorkspacePaths) -> Result<ThemesFile> {
        budget::sync_themes_from_lake(paths, self)
    }
}

/// Build a zero-usage [`BudgetStatusReport`] from `workbench/budget.json` when `m3.duckdb` does
/// not yet exist.
pub fn fallback_budget_status_report(paths: &WorkspacePaths) -> Result<BudgetStatusReport> {
    let config = budget::load_budget_config(paths)?;
    let month_utc = chrono::Utc::now().format("%Y-%m").to_string();
    let remaining_monthly_usd = config.monthly_budget_usd.max(Decimal::ZERO).normalize();
    let remaining_monthly_tokens = config.monthly_token_limit;
    let auto_trigger_remaining_weekly = if config.auto_trigger_enabled {
        config.weekly_auto_quota
    } else {
        0
    };
    Ok(BudgetStatusReport {
        config,
        usage: MonthlyBudgetUsage {
            month_utc,
            total_runs: 0,
            completed_runs: 0,
            blocked_runs: 0,
            failed_runs: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: Decimal::ZERO,
            weekly_auto_runs: 0,
        },
        remaining_monthly_usd,
        remaining_monthly_tokens,
        auto_trigger_remaining_weekly,
    })
}

/// Load [`BudgetStatusReport`] from `m3.duckdb` if present, falling back to config-only defaults.
pub fn load_budget_status_report(paths: &WorkspacePaths) -> Result<BudgetStatusReport> {
    match LakeDb::open_read_only_if_exists(paths)? {
        Some(db) => db.budget_status_report(paths),
        None => fallback_budget_status_report(paths),
    }
}

fn read_research_run_row(row: &duckdb::Row<'_>) -> duckdb::Result<ResearchRunRow> {
    let run_id: String = row.get(0)?;
    let ticker: String = row.get(1)?;
    let trigger: String = row.get(2)?;
    let focus: Option<String> = row.get(3)?;
    let model: String = row.get(4)?;
    let input_i64: i64 = row.get(5)?;
    let output_i64: i64 = row.get(6)?;
    let cost_raw: String = row.get(7)?;
    let started_i64: i64 = row.get(8)?;
    let finished_i64: Option<i64> = row.get(9)?;
    let status_raw: String = row.get(10)?;
    let error: Option<String> = row.get(11)?;
    let memo_path: Option<String> = row.get(12)?;
    let perspectives_json: String = row.get(13)?;
    let citations_json: String = row.get(14)?;

    let cost_usd = Decimal::from_str(cost_raw.trim())
        .unwrap_or(Decimal::ZERO)
        .normalize();
    let status = ResearchRunStatus::from_str(&status_raw).unwrap_or(ResearchRunStatus::Failed);

    Ok(ResearchRunRow {
        run_id,
        ticker,
        trigger,
        focus,
        model,
        input_tokens: u64::try_from(input_i64.max(0)).unwrap_or(0),
        output_tokens: u64::try_from(output_i64.max(0)).unwrap_or(0),
        cost_usd,
        started_at_ms: u64::try_from(started_i64.max(0)).unwrap_or(0),
        finished_at_ms: finished_i64.map(|v| u64::try_from(v.max(0)).unwrap_or(0)),
        status,
        error,
        memo_path,
        perspectives_json,
        citations_json,
    })
}

fn utc_month_bounds_ms(as_of_ms: u64) -> (String, u64, u64) {
    let clamped = i64::try_from(as_of_ms).unwrap_or(i64::MAX);
    let Some(dt) = Utc.timestamp_millis_opt(clamped).single() else {
        return ("1970-01".to_owned(), 0, 2_678_400_000);
    };
    let year = dt.year();
    let month = dt.month();
    let month_utc = format!("{year:04}-{month:02}");

    let start_date = NaiveDate::from_ymd_opt(year, month, 1);
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next_date = NaiveDate::from_ymd_opt(next_year, next_month, 1);

    let start_ms = start_date
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| Utc.from_utc_datetime(&ndt).timestamp_millis().max(0) as u64)
        .unwrap_or(0);
    let next_ms = next_date
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| Utc.from_utc_datetime(&ndt).timestamp_millis().max(0) as u64)
        .unwrap_or_else(|| start_ms.saturating_add(31 * 86_400_000));

    (month_utc, start_ms, next_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_paths(tag: &str) -> WorkspacePaths {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("m3_l2_test_{tag}_{}_{}", std::process::id(), nanos));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workbench")).unwrap();
        WorkspacePaths::from_root(root)
    }

    #[test]
    fn ensure_l2_schema_is_idempotent_and_migrates_existing_db() {
        let paths = temp_paths("l2_migration");
        let db = LakeDb::open(&paths).unwrap();
        db.ensure_l2_schema().unwrap();
        assert!(db.l2_table_exists("research_runs").unwrap());
        drop(db);

        let ro = LakeDb::open_read_only_if_exists(&paths).unwrap().unwrap();
        assert!(ro.list_research_runs(None, 10).unwrap().is_empty());
        let report = ro.budget_status_report(&paths).unwrap();
        assert_eq!(report.usage.total_runs, 0);
        assert_eq!(report.remaining_monthly_usd, Decimal::new(1000, 2));
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn research_run_lifecycle_and_monthly_usage_aggregation_work() {
        let paths = temp_paths("l2_lifecycle");
        let db = LakeDb::open(&paths).unwrap();

        // 2026-11-15T12:00:00Z
        let nov_mid = 1_794_744_000_000u64;
        // 2026-10-20T12:00:00Z (previous month, must be excluded from Nov usage)
        let oct_mid = 1_792_497_600_000u64;

        db.insert_research_run(&ResearchRunRow {
            run_id: "run-oct".to_owned(),
            ticker: "AAPL".to_owned(),
            trigger: "manual".to_owned(),
            focus: None,
            model: "test-model".to_owned(),
            input_tokens: 50_000,
            output_tokens: 10_000,
            cost_usd: Decimal::new(99, 2),
            started_at_ms: oct_mid,
            finished_at_ms: Some(oct_mid + 5_000),
            status: ResearchRunStatus::Completed,
            error: None,
            memo_path: Some("workbench/memos/AAPL.md".to_owned()),
            perspectives_json: "[]".to_owned(),
            citations_json: "[]".to_owned(),
        })
        .unwrap();

        db.insert_research_run(&ResearchRunRow {
            run_id: "run-1".to_owned(),
            ticker: "adsk".to_owned(),
            trigger: "manual".to_owned(),
            focus: Some("Moat".to_owned()),
            model: "claude-sonnet".to_owned(),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Decimal::ZERO,
            started_at_ms: nov_mid,
            finished_at_ms: None,
            status: ResearchRunStatus::Running,
            error: None,
            memo_path: None,
            perspectives_json: "[]".to_owned(),
            citations_json: "[]".to_owned(),
        })
        .unwrap();

        let updated = db
            .finish_research_run(
                "run-1",
                ResearchRunStatus::Completed,
                40_000,
                8_000,
                Decimal::new(24, 2),
                nov_mid + 20_000,
                None,
                Some("workbench/memos/ADSK.md"),
                "[{\"perspective\":\"moat\"}]",
                "[]",
            )
            .unwrap();
        assert!(updated);

        db.insert_research_run(&ResearchRunRow {
            run_id: "run-2".to_owned(),
            ticker: "PTC".to_owned(),
            trigger: "auto".to_owned(),
            focus: None,
            model: "claude-sonnet".to_owned(),
            input_tokens: 10_000,
            output_tokens: 2_000,
            cost_usd: Decimal::new(6, 2),
            started_at_ms: nov_mid + 1_000,
            finished_at_ms: Some(nov_mid + 4_000),
            status: ResearchRunStatus::Failed,
            error: Some("provider timeout".to_owned()),
            memo_path: None,
            perspectives_json: "[]".to_owned(),
            citations_json: "[]".to_owned(),
        })
        .unwrap();

        db.insert_research_run(&ResearchRunRow {
            run_id: "run-3".to_owned(),
            ticker: "COST".to_owned(),
            trigger: "auto".to_owned(),
            focus: None,
            model: "gate".to_owned(),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Decimal::ZERO,
            started_at_ms: nov_mid + 2_000,
            finished_at_ms: Some(nov_mid + 2_001),
            status: ResearchRunStatus::BlockedBudget,
            error: Some("quota reached".to_owned()),
            memo_path: None,
            perspectives_json: "[]".to_owned(),
            citations_json: "[]".to_owned(),
        })
        .unwrap();

        let adsk_runs = db.list_research_runs(Some("ADSK"), 10).unwrap();
        assert_eq!(adsk_runs.len(), 1);
        assert_eq!(adsk_runs[0].status, ResearchRunStatus::Completed);
        assert_eq!(adsk_runs[0].cost_usd, Decimal::new(24, 2));

        let usage = db.monthly_budget_usage(nov_mid + 10_000).unwrap();
        assert_eq!(usage.month_utc, "2026-11");
        assert_eq!(usage.total_runs, 3);
        assert_eq!(usage.completed_runs, 1);
        assert_eq!(usage.failed_runs, 1);
        assert_eq!(usage.blocked_runs, 1);
        assert_eq!(usage.total_input_tokens, 50_000);
        assert_eq!(usage.total_output_tokens, 10_000);
        assert_eq!(usage.total_cost_usd, Decimal::new(30, 2));
        // Only run-2 counts toward weekly_auto_runs; run-3 was blocked_budget.
        assert_eq!(usage.weekly_auto_runs, 1);

        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn reclaim_stale_running_runs_marks_only_expired_running_rows_failed() {
        let paths = temp_paths("reclaim_stale");
        let db = LakeDb::open(&paths).unwrap();

        let now_ms = 2_000_000_000u64;
        let stale_started = now_ms - DEFAULT_STALE_RESEARCH_RUN_MS;
        let very_stale_started = now_ms - DEFAULT_STALE_RESEARCH_RUN_MS - 60_000;
        let fresh_started = now_ms - (DEFAULT_STALE_RESEARCH_RUN_MS - 1_000);

        let mk_run =
            |id: &str, ticker: &str, started: u64, status: ResearchRunStatus| ResearchRunRow {
                run_id: id.to_owned(),
                ticker: ticker.to_owned(),
                trigger: "manual".to_owned(),
                focus: None,
                model: "test".to_owned(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: Decimal::ZERO,
                started_at_ms: started,
                finished_at_ms: (status != ResearchRunStatus::Running).then_some(started + 500),
                status,
                error: None,
                memo_path: None,
                perspectives_json: "[]".to_owned(),
                citations_json: "[]".to_owned(),
            };

        db.insert_research_run(&mk_run(
            "stale-exact",
            "AAPL",
            stale_started,
            ResearchRunStatus::Running,
        ))
        .unwrap();
        db.insert_research_run(&mk_run(
            "stale-old",
            "COST",
            very_stale_started,
            ResearchRunStatus::Running,
        ))
        .unwrap();
        db.insert_research_run(&mk_run(
            "fresh-run",
            "MSFT",
            fresh_started,
            ResearchRunStatus::Running,
        ))
        .unwrap();
        db.insert_research_run(&mk_run(
            "already-done",
            "ADSK",
            very_stale_started,
            ResearchRunStatus::Completed,
        ))
        .unwrap();

        let reclaimed = db
            .reclaim_stale_running_runs(now_ms, DEFAULT_STALE_RESEARCH_RUN_MS)
            .unwrap();
        assert_eq!(reclaimed, 2);

        let all = db.list_research_runs(None, 10).unwrap();
        let by_id = |id: &str| all.iter().find(|r| r.run_id == id).unwrap().clone();

        let r1 = by_id("stale-exact");
        assert_eq!(r1.status, ResearchRunStatus::Failed);
        assert_eq!(r1.finished_at_ms, Some(now_ms));
        assert!(r1.error.as_deref().unwrap_or("").contains("stale"));

        let r2 = by_id("stale-old");
        assert_eq!(r2.status, ResearchRunStatus::Failed);
        assert_eq!(r2.finished_at_ms, Some(now_ms));

        let r3 = by_id("fresh-run");
        assert_eq!(r3.status, ResearchRunStatus::Running);
        assert_eq!(r3.finished_at_ms, None);

        let r4 = by_id("already-done");
        assert_eq!(r4.status, ResearchRunStatus::Completed);

        // Second call is idempotent.
        assert_eq!(
            db.reclaim_stale_running_runs(now_ms, DEFAULT_STALE_RESEARCH_RUN_MS)
                .unwrap(),
            0
        );

        let _ = fs::remove_dir_all(&paths.repo_root);
    }
}
