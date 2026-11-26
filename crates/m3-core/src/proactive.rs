//! Proactive Alan core: L1.5 Circle Filing Watchdog (`signal_triage`) and
//! Calm Daily/Weekly Briefings (`circle_briefings`).
//!
//! Enforces strict human-sovereignty and cost invariants:
//! - Q2: L1.5 Watchdog strictly evaluates Circle `Bucket::Watch` + `Bucket::Core` (`WATCH + IN`) ONLY;
//!   `OUT`, `TOO HARD`, and non-Circle Discovery signals remain at 0 LLM tokens.
//! - Q3: `Noise` verdicts never delete or archive signals (`signal_inbox.status` stays untouched);
//!   `FalsificationAlert` never auto-triggers 4-lens Deep Research. Missing `falsification_metrics`
//!   falls back to `Dossier.premortem` + deterministic warnings with `fallback_no_metrics_warning = true`.
//! - Q5: Deterministic 3-part Briefing skeleton (Strike Radar, Filing & Red-Flag Delta, Watchdog Hits).
//!   Quiet periods (`is_quiet == true`) take 0 LLM tokens and surface the quiet headline.
//! - Q9: Proactive LLM runs enforce both daily quotas and monthly USD/token hard ceilings, degrading
//!   cleanly to 0-token deterministic operation when no Alan connection is configured.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use alan_os_host::embed::{
    EmbedEvent, EmbeddedAlan, LlmSource, SessionOptions, SystemPromptMode, UiInputStatus,
};
use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use duckdb::{Connection, OptionalExt, ToSql, params, params_from_iter};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::alan_connection;
use crate::alan_interrogator::{M3WorkbenchFs, boot_m3_embedded};
use crate::budget;
use crate::i18n::UiLang;
use crate::lake::LakeDb;
use crate::lake_l1::{SignalInboxRow, SignalKind};
use crate::model::{Bucket, Dossier, MungerReport, serde_decimal, serde_decimal_opt};
use crate::store::{self, WorkspacePaths};

const PROACTIVE_SESSION_TIMEOUT_SECS: u64 = 60;
const STRIKE_RADAR_MAX_DISTANCE_PCT: i64 = 15;
const DAILY_LOOKBACK_MS: u64 = 24 * 3_600_000;
const WEEKLY_LOOKBACK_MS: u64 = 7 * 86_400_000;

/// DuckDB DDL for Project M3 v0.6.0 proactive watchdog and calm briefing tables.
pub const PROACTIVE_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS signal_triage (
    signal_id TEXT PRIMARY KEY,
    ticker TEXT NOT NULL,
    verdict TEXT NOT NULL,
    headline TEXT NOT NULL,
    rationale TEXT NOT NULL,
    matched_metrics_json TEXT NOT NULL,
    suggested_metrics_json TEXT NOT NULL,
    quotes_json TEXT NOT NULL,
    fallback_no_metrics_warning BOOLEAN NOT NULL,
    model_used TEXT NOT NULL,
    tokens_used UBIGINT NOT NULL,
    cost_usd DOUBLE NOT NULL,
    created_at_ms UBIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS circle_briefings (
    briefing_id TEXT PRIMARY KEY,
    period TEXT NOT NULL,
    period_key TEXT NOT NULL,
    generated_at_ms UBIGINT NOT NULL,
    is_quiet BOOLEAN NOT NULL,
    headline TEXT NOT NULL,
    executive_summary TEXT,
    summary_tokens_used UBIGINT NOT NULL,
    summary_cost_usd DOUBLE NOT NULL,
    payload_json TEXT NOT NULL
);
";

/// L1.5 Circle Filing Watchdog triage classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageVerdict {
    Noise,
    Material,
    FalsificationAlert,
}

impl TriageVerdict {
    /// Canonical lowercase storage key.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Noise => "noise",
            Self::Material => "material",
            Self::FalsificationAlert => "falsification_alert",
        }
    }
}

impl fmt::Display for TriageVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TriageVerdict {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "noise" => Ok(Self::Noise),
            "material" => Ok(Self::Material),
            "falsification_alert" | "falsification" | "alert" => Ok(Self::FalsificationAlert),
            other => anyhow::bail!(
                "unknown triage verdict '{other}'; expected noise|material|falsification_alert"
            ),
        }
    }
}

/// Primary filing or evidence quote attached to a [`SignalTriageRecord`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageQuote {
    pub section: String,
    #[serde(default)]
    pub filing_date: Option<String>,
    pub quote: String,
    #[serde(default)]
    pub source_url: String,
}

/// Persisted L1.5 triage evaluation stored in DuckDB `signal_triage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalTriageRecord {
    pub signal_id: String,
    pub ticker: String,
    pub verdict: TriageVerdict,
    pub headline: String,
    pub rationale: String,
    #[serde(default)]
    pub matched_metrics: Vec<String>,
    #[serde(default)]
    pub suggested_metrics: Vec<String>,
    #[serde(default)]
    pub quotes: Vec<TriageQuote>,
    pub fallback_no_metrics_warning: bool,
    pub model_used: String,
    pub tokens_used: u64,
    #[serde(with = "serde_decimal")]
    pub cost_usd: Decimal,
    pub created_at_ms: u64,
}

/// Batch summary returned by [`run_l15_circle_triage`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriageRunSummary {
    pub evaluated: usize,
    pub skipped_outside_circle: usize,
    pub skipped_already_triaged: usize,
    pub skipped_budget_or_quota: usize,
    pub skipped_no_connection: usize,
    pub dry_run: bool,
    pub noise_count: usize,
    pub material_count: usize,
    pub falsification_alert_count: usize,
    pub tokens_used: u64,
    #[serde(with = "serde_decimal")]
    pub cost_usd: Decimal,
    pub budget_reason: Option<String>,
    pub records: Vec<SignalTriageRecord>,
}

/// Cadence of a generated [`CircleBriefing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefingPeriod {
    Daily,
    Weekly,
}

impl BriefingPeriod {
    /// Canonical lowercase storage key.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }
}

impl fmt::Display for BriefingPeriod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BriefingPeriod {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "daily" | "day" | "d" => Ok(Self::Daily),
            "weekly" | "week" | "w" => Ok(Self::Weekly),
            other => anyhow::bail!("unknown briefing period '{other}'; expected daily|weekly"),
        }
    }
}

/// Part 1 of a calm briefing: Circle `WATCH + IN` company in Strike Zone or <= 15% above Strike.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrikeRadarItem {
    pub ticker: String,
    pub name: String,
    pub bucket: Bucket,
    pub currency: String,
    #[serde(default, with = "serde_decimal_opt")]
    pub latest_price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strike_price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub distance_to_strike_pct: Option<Decimal>,
    pub status: crate::model::StrikeZoneStatus,
    pub in_strike_zone: bool,
}

/// Part 2 of a calm briefing: recent SEC filing or Red-Flag delta for a Circle `WATCH + IN` company.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilingDeltaItem {
    pub ticker: String,
    pub event_type: String,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub filing_date: Option<String>,
    #[serde(default)]
    pub url: String,
    pub occurred_at_ms: u64,
    pub is_red_flag: bool,
}

/// Part 3 of a calm briefing: non-noise Alan L1.5 Watchdog finding (`Material` or `FalsificationAlert`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchdogHitItem {
    pub signal_id: String,
    pub ticker: String,
    pub verdict: TriageVerdict,
    pub headline: String,
    pub rationale: String,
    pub fallback_no_metrics_warning: bool,
    #[serde(default)]
    pub quotes: Vec<TriageQuote>,
    pub created_at_ms: u64,
}

/// Calm daily/weekly briefing persisted in DuckDB `circle_briefings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CircleBriefing {
    pub briefing_id: String,
    pub period: BriefingPeriod,
    pub period_key: String,
    pub generated_at_ms: u64,
    pub lang: UiLang,
    pub is_quiet: bool,
    pub headline: String,
    pub executive_summary: Option<String>,
    pub summary_tokens_used: u64,
    #[serde(with = "serde_decimal")]
    pub summary_cost_usd: Decimal,
    pub strike_radar: Vec<StrikeRadarItem>,
    pub filing_deltas: Vec<FilingDeltaItem>,
    pub watchdog_hits: Vec<WatchdogHitItem>,
}

/// Creates additive `signal_triage` and `circle_briefings` tables idempotently.
pub fn ensure_proactive_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(PROACTIVE_SCHEMA_SQL)
        .context("failed to initialize proactive DuckDB tables")?;
    Ok(())
}

fn table_exists(conn: &Connection, table_name: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
        [table_name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

/// Returns `true` when both `signal_triage` and `circle_briefings` tables exist.
pub fn has_proactive_tables(conn: &Connection) -> Result<bool> {
    Ok(table_exists(conn, "signal_triage")? && table_exists(conn, "circle_briefings")?)
}

fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

fn f64_to_decimal(value: f64) -> Decimal {
    if !value.is_finite() || value <= 0.0 {
        return Decimal::ZERO;
    }
    Decimal::from_f64_retain(value)
        .unwrap_or(Decimal::ZERO)
        .round_dp(6)
        .normalize()
}

/// Inserts or replaces a [`SignalTriageRecord`] into `signal_triage`.
pub fn upsert_signal_triage(conn: &Connection, record: &SignalTriageRecord) -> Result<()> {
    if !table_exists(conn, "signal_triage")? {
        ensure_proactive_schema(conn)?;
    }
    let matched_json = serde_json::to_string(&record.matched_metrics)?;
    let suggested_json = serde_json::to_string(&record.suggested_metrics)?;
    let quotes_json = serde_json::to_string(&record.quotes)?;
    let cost_f64 = decimal_to_f64(record.cost_usd);

    conn.execute(
        "INSERT OR REPLACE INTO signal_triage (
            signal_id, ticker, verdict, headline, rationale,
            matched_metrics_json, suggested_metrics_json, quotes_json,
            fallback_no_metrics_warning, model_used, tokens_used, cost_usd, created_at_ms
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            record.signal_id,
            record.ticker.trim().to_ascii_uppercase(),
            record.verdict.as_str(),
            record.headline,
            record.rationale,
            matched_json,
            suggested_json,
            quotes_json,
            record.fallback_no_metrics_warning,
            record.model_used,
            record.tokens_used,
            cost_f64,
            record.created_at_ms,
        ],
    )
    .with_context(|| format!("failed to upsert signal_triage {}", record.signal_id))?;
    Ok(())
}

/// Lists persisted `signal_triage` records newest-first, optionally filtered by ticker or verdict.
pub fn list_signal_triages(
    conn: &Connection,
    ticker_opt: Option<&str>,
    verdict_opt: Option<TriageVerdict>,
    limit: usize,
) -> Result<Vec<SignalTriageRecord>> {
    if limit == 0 || !table_exists(conn, "signal_triage")? {
        return Ok(Vec::new());
    }
    let mut clauses: Vec<&str> = Vec::new();
    let mut owned_params: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(t) = ticker_opt.map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push("UPPER(ticker) = UPPER(?)");
        owned_params.push(Box::new(t.to_owned()));
    }
    if let Some(v) = verdict_opt {
        clauses.push("LOWER(verdict) = ?");
        owned_params.push(Box::new(v.as_str().to_owned()));
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    owned_params.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));

    let sql = format!(
        "SELECT signal_id, ticker, verdict, headline, rationale,
                matched_metrics_json, suggested_metrics_json, quotes_json,
                fallback_no_metrics_warning, model_used, tokens_used, cost_usd, created_at_ms
         FROM signal_triage
         {where_sql}
         ORDER BY created_at_ms DESC, signal_id ASC
         LIMIT ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = owned_params.iter().map(|b| b.as_ref());
    let rows = stmt.query_map(params_from_iter(refs), read_signal_triage_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to list signal_triage records")
}

/// Looks up one persisted triage record by `signal_id`.
pub fn get_signal_triage(conn: &Connection, signal_id: &str) -> Result<Option<SignalTriageRecord>> {
    if !table_exists(conn, "signal_triage")? {
        return Ok(None);
    }
    conn.query_row(
        "SELECT signal_id, ticker, verdict, headline, rationale,
                matched_metrics_json, suggested_metrics_json, quotes_json,
                fallback_no_metrics_warning, model_used, tokens_used, cost_usd, created_at_ms
         FROM signal_triage
         WHERE signal_id = ?
         LIMIT 1",
        [signal_id],
        read_signal_triage_row,
    )
    .optional()
    .context("failed to query signal_triage by signal_id")
}

/// Counts LLM triage evaluations recorded at or after `since_ms` (excluding zero-LLM dry runs).
pub fn count_triages_since(conn: &Connection, since_ms: u64) -> Result<usize> {
    if !table_exists(conn, "signal_triage")? {
        return Ok(0);
    }
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM signal_triage
         WHERE created_at_ms >= ?
           AND LOWER(model_used) NOT IN ('dry_run', 'deterministic')",
        [since_ms],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(count.max(0)).unwrap_or(0))
}

/// Sums proactive tokens and USD cost across `signal_triage` and `circle_briefings` since `month_start_ms`.
pub fn monthly_proactive_usage(conn: &Connection, month_start_ms: u64) -> Result<(u64, Decimal)> {
    let mut total_tokens = 0u64;
    let mut total_cost_f64 = 0.0f64;

    if table_exists(conn, "signal_triage")? {
        let (tok_sum, usd_sum): (Option<i128>, Option<f64>) = conn.query_row(
            "SELECT COALESCE(SUM(CAST(tokens_used AS HUGEINT)), 0), COALESCE(SUM(cost_usd), 0.0)
             FROM signal_triage
             WHERE created_at_ms >= ?",
            [month_start_ms],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let tok_u64 = tok_sum
            .and_then(|v| u64::try_from(v.max(0)).ok())
            .unwrap_or(0);
        total_tokens = total_tokens.saturating_add(tok_u64);
        if let Some(usd) = usd_sum
            && usd.is_finite()
            && usd > 0.0
        {
            total_cost_f64 += usd;
        }
    }

    if table_exists(conn, "circle_briefings")? {
        let (tok_sum, usd_sum): (Option<i128>, Option<f64>) = conn.query_row(
            "SELECT COALESCE(SUM(CAST(summary_tokens_used AS HUGEINT)), 0),
                    COALESCE(SUM(summary_cost_usd), 0.0)
             FROM circle_briefings
             WHERE generated_at_ms >= ?",
            [month_start_ms],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let tok_u64 = tok_sum
            .and_then(|v| u64::try_from(v.max(0)).ok())
            .unwrap_or(0);
        total_tokens = total_tokens.saturating_add(tok_u64);
        if let Some(usd) = usd_sum
            && usd.is_finite()
            && usd > 0.0
        {
            total_cost_f64 += usd;
        }
    }

    Ok((total_tokens, f64_to_decimal(total_cost_f64)))
}

/// Inserts or replaces a [`CircleBriefing`] into `circle_briefings`.
pub fn upsert_circle_briefing(conn: &Connection, briefing: &CircleBriefing) -> Result<()> {
    if !table_exists(conn, "circle_briefings")? {
        ensure_proactive_schema(conn)?;
    }
    let payload_json = serde_json::to_string(briefing)?;
    let cost_f64 = decimal_to_f64(briefing.summary_cost_usd);

    conn.execute(
        "INSERT OR REPLACE INTO circle_briefings (
            briefing_id, period, period_key, generated_at_ms, is_quiet,
            headline, executive_summary, summary_tokens_used, summary_cost_usd, payload_json
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            briefing.briefing_id,
            briefing.period.as_str(),
            briefing.period_key,
            briefing.generated_at_ms,
            briefing.is_quiet,
            briefing.headline,
            briefing.executive_summary,
            briefing.summary_tokens_used,
            cost_f64,
            payload_json,
        ],
    )
    .with_context(|| format!("failed to upsert circle_briefing {}", briefing.briefing_id))?;
    Ok(())
}

/// Lists persisted [`CircleBriefing`] records newest-first, optionally filtered by period.
pub fn list_circle_briefings(
    conn: &Connection,
    period_opt: Option<BriefingPeriod>,
    limit: usize,
) -> Result<Vec<CircleBriefing>> {
    if limit == 0 || !table_exists(conn, "circle_briefings")? {
        return Ok(Vec::new());
    }
    let mut owned_params: Vec<Box<dyn ToSql>> = Vec::new();
    let where_sql = if let Some(period) = period_opt {
        owned_params.push(Box::new(period.as_str().to_owned()));
        "WHERE LOWER(period) = ?"
    } else {
        ""
    };
    owned_params.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));

    let sql = format!(
        "SELECT briefing_id, period, period_key, generated_at_ms, is_quiet,
                headline, executive_summary, summary_tokens_used, summary_cost_usd, payload_json
         FROM circle_briefings
         {where_sql}
         ORDER BY generated_at_ms DESC, briefing_id DESC
         LIMIT ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = owned_params.iter().map(|b| b.as_ref());
    let rows = stmt.query_map(params_from_iter(refs), read_circle_briefing_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to list circle_briefings")
}

/// Returns the most recent persisted [`CircleBriefing`], optionally filtered by period.
pub fn latest_circle_briefing(
    conn: &Connection,
    period_opt: Option<BriefingPeriod>,
) -> Result<Option<CircleBriefing>> {
    let mut items = list_circle_briefings(conn, period_opt, 1)?;
    Ok(items.pop())
}

/// Counts LLM-generated briefing executive summaries recorded at or after `since_ms`.
pub fn count_briefing_summaries_since(conn: &Connection, since_ms: u64) -> Result<usize> {
    if !table_exists(conn, "circle_briefings")? {
        return Ok(0);
    }
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM circle_briefings
         WHERE generated_at_ms >= ?
           AND executive_summary IS NOT NULL
           AND TRIM(executive_summary) != ''",
        [since_ms],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(count.max(0)).unwrap_or(0))
}

fn read_signal_triage_row(row: &duckdb::Row<'_>) -> duckdb::Result<SignalTriageRecord> {
    let signal_id: String = row.get(0)?;
    let ticker: String = row.get(1)?;
    let verdict_raw: String = row.get(2)?;
    let headline: String = row.get(3)?;
    let rationale: String = row.get(4)?;
    let matched_json: String = row.get(5)?;
    let suggested_json: String = row.get(6)?;
    let quotes_json: String = row.get(7)?;
    let fallback_no_metrics_warning: bool = row.get(8)?;
    let model_used: String = row.get(9)?;
    let tokens_used: u64 = row.get(10)?;
    let cost_f64: f64 = row.get(11)?;
    let created_at_ms: u64 = row.get(12)?;

    Ok(SignalTriageRecord {
        signal_id,
        ticker,
        verdict: TriageVerdict::from_str(&verdict_raw).unwrap_or(TriageVerdict::Material),
        headline,
        rationale,
        matched_metrics: serde_json::from_str(&matched_json).unwrap_or_default(),
        suggested_metrics: serde_json::from_str(&suggested_json).unwrap_or_default(),
        quotes: serde_json::from_str(&quotes_json).unwrap_or_default(),
        fallback_no_metrics_warning,
        model_used,
        tokens_used,
        cost_usd: f64_to_decimal(cost_f64),
        created_at_ms,
    })
}

fn read_circle_briefing_row(row: &duckdb::Row<'_>) -> duckdb::Result<CircleBriefing> {
    let briefing_id: String = row.get(0)?;
    let period_raw: String = row.get(1)?;
    let period_key: String = row.get(2)?;
    let generated_at_ms: u64 = row.get(3)?;
    let is_quiet: bool = row.get(4)?;
    let headline: String = row.get(5)?;
    let executive_summary: Option<String> = row.get(6)?;
    let summary_tokens_used: u64 = row.get(7)?;
    let summary_cost_f64: f64 = row.get(8)?;
    let payload_json: String = row.get(9)?;

    if let Ok(mut parsed) = serde_json::from_str::<CircleBriefing>(&payload_json) {
        parsed.briefing_id = briefing_id;
        parsed.period = BriefingPeriod::from_str(&period_raw).unwrap_or(parsed.period);
        parsed.period_key = period_key;
        parsed.generated_at_ms = generated_at_ms;
        parsed.is_quiet = is_quiet;
        parsed.headline = headline;
        parsed.executive_summary = executive_summary;
        parsed.summary_tokens_used = summary_tokens_used;
        parsed.summary_cost_usd = f64_to_decimal(summary_cost_f64);
        return Ok(parsed);
    }

    Ok(CircleBriefing {
        briefing_id,
        period: BriefingPeriod::from_str(&period_raw).unwrap_or(BriefingPeriod::Daily),
        period_key,
        generated_at_ms,
        lang: UiLang::Zh,
        is_quiet,
        headline,
        executive_summary,
        summary_tokens_used,
        summary_cost_usd: f64_to_decimal(summary_cost_f64),
        strike_radar: Vec::new(),
        filing_deltas: Vec::new(),
        watchdog_hits: Vec::new(),
    })
}

/// Q2 invariant: L1.5 proactive Watchdog ONLY watches `Bucket::Watch` and `Bucket::Core` (`WATCH + IN`).
/// `OUT`, `TOO HARD`, and non-Circle Discovery companies stay at 0 LLM tokens.
pub fn is_proactive_triage_eligible(bucket: Option<Bucket>) -> bool {
    matches!(bucket, Some(Bucket::Watch | Bucket::Core))
}

/// Composes the L1.5 Circle Filing Watchdog triage prompt for a single signal.
///
/// Returns `(prompt, fallback_no_metrics_warning)` per Q3: when
/// `Dossier.human_judgment.falsification_metrics` is empty, falls back to `Dossier.premortem` +
/// deterministic report warnings and sets `fallback_no_metrics_warning = true`.
pub fn compose_l15_triage_prompt(
    signal: &SignalInboxRow,
    dossier: Option<&Dossier>,
    report: Option<&MungerReport>,
) -> (String, bool) {
    let ticker = signal.primary_ticker.trim().to_ascii_uppercase();
    let human_metrics: Vec<String> = dossier
        .map(|d| {
            d.human_judgment
                .falsification_metrics
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let fallback_no_metrics_warning = human_metrics.is_empty();

    let metrics_block = if !fallback_no_metrics_warning {
        let lines: Vec<String> = human_metrics
            .iter()
            .enumerate()
            .map(|(i, m)| format!("  {}. {}", i + 1, m))
            .collect();
        format!(
            "人类设定的核心证伪指标 (falsification_metrics):\n{}",
            lines.join("\n")
        )
    } else {
        let premortem_lines: Vec<String> = dossier
            .map(|d| {
                d.premortem
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .enumerate()
                    .map(|(i, item)| format!("  - [Premortem {}] {}", i + 1, item))
                    .collect()
            })
            .unwrap_or_default();
        let warning_lines: Vec<String> = report
            .map(|r| {
                r.warnings
                    .iter()
                    .map(|w| {
                        format!(
                            "  - [{:?}:{}] {} — {}",
                            w.severity, w.code, w.title, w.detail
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut parts = vec![
            "注意：该标的尚未设定人类 `falsification_metrics`，已回退至 `premortem` 与确定性财报预警作为初筛锚点，请在 `suggested_metrics` 中建议 1–3 条可量化检验的证伪指标。".to_owned(),
        ];
        if premortem_lines.is_empty() {
            parts.push("Pre-mortem 摘要: （暂无）".to_owned());
        } else {
            parts.push(format!("Pre-mortem 摘要:\n{}", premortem_lines.join("\n")));
        }
        if !warning_lines.is_empty() {
            parts.push(format!("确定性财务预警:\n{}", warning_lines.join("\n")));
        }
        parts.join("\n")
    };

    let report_summary = if let Some(rep) = report {
        let val = rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref());
        let price = rep
            .price
            .as_ref()
            .map(|p| format!("{} {}", rep.currency, p.price.round_dp(2).normalize()))
            .unwrap_or_else(|| "—".to_owned());
        let strike = val
            .and_then(|v| v.strike_price)
            .map(|s| format!("{} {}", rep.currency, s.round_dp(2).normalize()))
            .unwrap_or_else(|| "—".to_owned());
        let dist = val
            .and_then(|v| v.distance_to_strike_pct)
            .map(|d| format!("{:.2}%", d))
            .unwrap_or_else(|| "—".to_owned());
        let roic = rep
            .roic_5y_avg_pct
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());
        format!(
            "确定性底表快照: 篮子={} | 现价={price} | 击球价={strike} | 距击球={dist} | 5Y ROIC={roic} | 预警数={}",
            rep.bucket.code(),
            rep.warnings.len()
        )
    } else {
        "确定性底表快照: （暂无计算报告）".to_owned()
    };

    let evidence_block = if signal.evidence.is_empty() {
        "一手公告/新闻证据切片: （无原文切片）".to_owned()
    } else {
        let lines: Vec<String> = signal
            .evidence
            .iter()
            .take(5)
            .enumerate()
            .map(|(i, ev)| {
                format!(
                    "  [{}] section={} | date={} | url={}\n      quote: \"{}\"",
                    i + 1,
                    ev.section,
                    ev.filing_date.as_deref().unwrap_or("—"),
                    ev.source_url,
                    ev.quote.trim()
                )
            })
            .collect();
        format!("一手公告/新闻证据切片:\n{}", lines.join("\n"))
    };

    let prompt = format!(
        "你是 Project-M3 的 Alan L1.5 能力圈公告守望者 (Watchdog)。\n\
         严禁调用任何外部或湖仓工具；严禁心算或篡改财务数字；严禁给出买卖建议或删除任何信号。\n\
         请严格根据以下信号、原文切片与证伪锚点，判断该事件是否对 {ticker} 构成噪音 (`noise`)、实质基本面变化 (`material`) 或触发核心逻辑证伪警报 (`falsification_alert`)。\n\n\
         [待初筛信号]\n\
         - signal_id: {}\n\
         - kind: {}\n\
         - ticker: {ticker}\n\
         - title: {}\n\
         - summary: {}\n\n\
         {report_summary}\n\n\
         {metrics_block}\n\n\
         {evidence_block}\n\n\
         请仅输出一个合法 JSON 对象（不要加多余解释），字段格式如下：\n\
         {{\n\
           \"verdict\": \"noise | material | falsification_alert\",\n\
           \"headline\": \"一句话客观结论\",\n\
           \"rationale\": \"结合原文引文与证伪锚点的简明理由\",\n\
           \"matched_metrics\": [\"命中的证伪指标或预警条目\"],\n\
           \"suggested_metrics\": [\"建议补充到档案的可检验证伪指标\"],\n\
           \"quotes\": [{{\"section\": \"...\", \"filing_date\": \"...\", \"quote\": \"原文逐字摘录\", \"source_url\": \"...\"}}]\n\
         }}",
        signal.signal_id,
        signal.kind.as_str(),
        signal.title,
        signal.summary,
    );

    (prompt, fallback_no_metrics_warning)
}

#[derive(Debug, Deserialize)]
struct RawTriageJson {
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    headline: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    matched_metrics: Vec<String>,
    #[serde(default)]
    suggested_metrics: Vec<String>,
    #[serde(default)]
    quotes: Vec<TriageQuote>,
}

/// Parses Alan's L1.5 triage response into a [`SignalTriageRecord`], never panicking on malformed output.
pub fn parse_l15_triage_response(
    raw: &str,
    signal: &SignalInboxRow,
    fallback_no_metrics_warning: bool,
    model_used: &str,
    tokens_used: u64,
    cost_usd: Decimal,
    created_at_ms: u64,
) -> SignalTriageRecord {
    let ticker = signal.primary_ticker.trim().to_ascii_uppercase();
    let parsed_opt = extract_json_object(raw)
        .and_then(|slice| serde_json::from_str::<RawTriageJson>(slice).ok());

    let fallback_quotes: Vec<TriageQuote> = signal
        .evidence
        .iter()
        .take(3)
        .filter(|ev| !ev.quote.trim().is_empty())
        .map(|ev| TriageQuote {
            section: ev.section.clone(),
            filing_date: ev.filing_date.clone(),
            quote: ev.quote.trim().to_owned(),
            source_url: ev.source_url.clone(),
        })
        .collect();

    if let Some(parsed) = parsed_opt {
        let verdict = parsed
            .verdict
            .as_deref()
            .and_then(|v| TriageVerdict::from_str(v).ok())
            .unwrap_or_else(|| infer_verdict_from_text(raw, signal.kind));
        let headline = parsed
            .headline
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| signal.title.clone());
        let rationale = parsed
            .rationale
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| signal.summary.clone());
        let matched_metrics = clean_string_list(parsed.matched_metrics);
        let suggested_metrics = clean_string_list(parsed.suggested_metrics);
        let quotes: Vec<TriageQuote> = parsed
            .quotes
            .into_iter()
            .filter(|q| !q.quote.trim().is_empty())
            .map(|mut q| {
                q.section = q.section.trim().to_owned();
                q.quote = q.quote.trim().to_owned();
                q.source_url = q.source_url.trim().to_owned();
                q
            })
            .collect();

        return SignalTriageRecord {
            signal_id: signal.signal_id.clone(),
            ticker,
            verdict,
            headline,
            rationale,
            matched_metrics,
            suggested_metrics,
            quotes: if quotes.is_empty() {
                fallback_quotes
            } else {
                quotes
            },
            fallback_no_metrics_warning,
            model_used: model_used.to_owned(),
            tokens_used,
            cost_usd: cost_usd.normalize(),
            created_at_ms,
        };
    }

    let cleaned_raw = raw.trim();
    SignalTriageRecord {
        signal_id: signal.signal_id.clone(),
        ticker,
        verdict: infer_verdict_from_text(cleaned_raw, signal.kind),
        headline: signal.title.clone(),
        rationale: if cleaned_raw.is_empty() {
            signal.summary.clone()
        } else {
            truncate_chars(cleaned_raw, 600)
        },
        matched_metrics: Vec::new(),
        suggested_metrics: Vec::new(),
        quotes: fallback_quotes,
        fallback_no_metrics_warning,
        model_used: model_used.to_owned(),
        tokens_used,
        cost_usd: cost_usd.normalize(),
        created_at_ms,
    }
}

fn infer_verdict_from_text(raw: &str, kind: SignalKind) -> TriageVerdict {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("falsification_alert") || lower.contains("falsification") {
        TriageVerdict::FalsificationAlert
    } else if lower.contains("material") {
        TriageVerdict::Material
    } else if lower.contains("noise") {
        TriageVerdict::Noise
    } else if kind == SignalKind::RedFlag {
        TriageVerdict::Material
    } else {
        TriageVerdict::Noise
    }
}

fn extract_json_object(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let stripped = if let Some(after_fence) = trimmed.strip_prefix("```json") {
        after_fence
            .trim_start()
            .strip_suffix("```")
            .unwrap_or(after_fence)
            .trim()
    } else if let Some(after_fence) = trimmed.strip_prefix("```") {
        after_fence
            .trim_start()
            .strip_suffix("```")
            .unwrap_or(after_fence)
            .trim()
    } else {
        trimmed
    };

    let start = stripped.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (rel_idx, ch) in stripped[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(&stripped[start..=start + rel_idx]);
            }
        }
    }
    None
}

fn clean_string_list(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_owned()) {
            out.push(trimmed.to_owned());
        }
    }
    out
}

/// Synchronous wrapper for [`run_l15_circle_triage`] for CLI and blocking callers.
pub fn run_l15_circle_triage_blocking(
    paths: &WorkspacePaths,
    lake: &LakeDb,
    limit: usize,
    dry_run: bool,
) -> Result<TriageRunSummary> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for L1.5 circle triage")?;
    rt.block_on(run_l15_circle_triage(paths, lake, limit, dry_run))
}

/// Runs L1.5 Circle Filing Watchdog triage across eligible `WATCH + IN` signals.
///
/// Enforces Q2 (`WATCH + IN` only), Q3 (non-destructive on `signal_inbox` and never auto-triggers
/// Deep Research), and Q9 (daily triage quota + monthly USD/token hard ceilings + clean 0-token
/// degradation when Alan is unconfigured).
pub async fn run_l15_circle_triage(
    paths: &WorkspacePaths,
    lake: &LakeDb,
    limit: usize,
    dry_run: bool,
) -> Result<TriageRunSummary> {
    run_l15_circle_triage_with_llm(paths, lake, limit, dry_run, None).await
}

/// Testable L1.5 Circle Filing Watchdog entrypoint accepting an explicit `LlmSource`.
pub async fn run_l15_circle_triage_with_llm(
    paths: &WorkspacePaths,
    lake: &LakeDb,
    limit: usize,
    dry_run: bool,
    llm_override: Option<(LlmSource, String)>,
) -> Result<TriageRunSummary> {
    ensure_proactive_schema(&lake.conn)?;

    let mut summary = TriageRunSummary {
        evaluated: 0,
        skipped_outside_circle: 0,
        skipped_already_triaged: 0,
        skipped_budget_or_quota: 0,
        skipped_no_connection: 0,
        dry_run,
        noise_count: 0,
        material_count: 0,
        falsification_alert_count: 0,
        tokens_used: 0,
        cost_usd: Decimal::ZERO,
        budget_reason: None,
        records: Vec::new(),
    };

    if limit == 0 {
        return Ok(summary);
    }

    let bucket_by_ticker = load_watchlist_bucket_map(paths)?;
    if bucket_by_ticker.is_empty() {
        return Ok(summary);
    }

    let signals = lake.list_inbox_signals(None, None, 500)?;
    let mut pending_signals: Vec<SignalInboxRow> = Vec::new();

    for sig in signals {
        let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
        let bucket = bucket_by_ticker.get(&ticker).copied();
        if !is_proactive_triage_eligible(bucket) {
            summary.skipped_outside_circle += 1;
            continue;
        }
        if get_signal_triage(&lake.conn, &sig.signal_id)?.is_some() {
            summary.skipped_already_triaged += 1;
            continue;
        }
        pending_signals.push(sig);
    }

    if pending_signals.is_empty() {
        return Ok(summary);
    }

    let budget_cfg = budget::load_budget_config(paths)?;

    if dry_run {
        for sig in pending_signals.into_iter().take(limit) {
            let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
            let dossier = store::load_dossier(paths, &ticker).ok().flatten();
            let report = store::load_or_recompute_ticker(paths, &ticker).ok();
            let (prompt, fallback_warning) =
                compose_l15_triage_prompt(&sig, dossier.as_ref(), report.as_ref());
            let record = SignalTriageRecord {
                signal_id: sig.signal_id,
                ticker,
                verdict: if sig.kind == SignalKind::RedFlag {
                    TriageVerdict::Material
                } else {
                    TriageVerdict::Noise
                },
                headline: format!("[dry-run] {}", sig.title),
                rationale: truncate_chars(&prompt, 240),
                matched_metrics: Vec::new(),
                suggested_metrics: Vec::new(),
                quotes: sig
                    .evidence
                    .iter()
                    .take(2)
                    .map(|ev| TriageQuote {
                        section: ev.section.clone(),
                        filing_date: ev.filing_date.clone(),
                        quote: ev.quote.clone(),
                        source_url: ev.source_url.clone(),
                    })
                    .collect(),
                fallback_no_metrics_warning: fallback_warning,
                model_used: "dry_run".to_owned(),
                tokens_used: 0,
                cost_usd: Decimal::ZERO,
                created_at_ms: store::now_ms(),
            };
            tally_record(&mut summary, &record);
        }
        return Ok(summary);
    }

    let (llm_source, model_used) = match llm_override {
        Some(pair) => pair,
        None => match resolve_ready_alan_model()? {
            Some(model_name) => (LlmSource::SharedProductCredentials, model_name),
            None => {
                summary.skipped_no_connection = pending_signals.len().min(limit);
                return Ok(summary);
            }
        },
    };

    let mut llm_source_opt = Some(llm_source);
    let mut alan_app: Option<EmbeddedAlan> = None;

    for (idx, sig) in pending_signals.iter().enumerate() {
        if summary.evaluated >= limit {
            break;
        }

        let now_ms = store::now_ms();
        let (monthly_tokens, monthly_usd) = combined_monthly_usage(lake, now_ms)?;
        let triages_today = count_triages_since(&lake.conn, utc_day_start_ms(now_ms))?;
        let gate = budget::evaluate_proactive_triage_gate(
            &budget_cfg,
            monthly_tokens,
            monthly_usd,
            triages_today,
        );
        if !gate.allowed {
            let remaining = pending_signals
                .len()
                .saturating_sub(idx)
                .min(limit - summary.evaluated);
            summary.skipped_budget_or_quota += remaining;
            summary.budget_reason = gate.reason;
            break;
        }

        if alan_app.is_none() {
            let Some(src) = llm_source_opt.take() else {
                break;
            };
            let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
            alan_app = Some(boot_m3_embedded(paths.clone(), fs, src).await?);
        }

        let Some(app) = alan_app.as_ref() else {
            break;
        };

        let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
        let dossier = store::load_dossier(paths, &ticker).ok().flatten();
        let report = store::load_or_recompute_ticker(paths, &ticker).ok();
        let (prompt, fallback_warning) =
            compose_l15_triage_prompt(sig, dossier.as_ref(), report.as_ref());

        let sys_prompt = "你是 Project-M3 的 Alan L1.5 能力圈公告守望者。\
                          严禁调用任何工具，严禁心算或篡改财务数字，仅输出一段严格合法的 JSON 对象。";
        let (raw_text, in_tokens, out_tokens) =
            run_single_alan_turn(app, sys_prompt, prompt).await?;
        let total_tokens = in_tokens.saturating_add(out_tokens);
        let cost_usd = budget::estimate_cost_usd(&budget_cfg, in_tokens, out_tokens);
        let record = parse_l15_triage_response(
            &raw_text,
            sig,
            fallback_warning,
            &model_used,
            total_tokens,
            cost_usd,
            store::now_ms(),
        );
        upsert_signal_triage(&lake.conn, &record)?;
        tally_record(&mut summary, &record);
    }

    if let Some(app) = alan_app
        && let Err(err) = app.shutdown().await
    {
        log::warn!("Alan shutdown warning after L1.5 triage: {err:#}");
    }

    Ok(summary)
}

fn tally_record(summary: &mut TriageRunSummary, record: &SignalTriageRecord) {
    summary.evaluated += 1;
    summary.tokens_used = summary.tokens_used.saturating_add(record.tokens_used);
    summary.cost_usd = (summary.cost_usd + record.cost_usd).normalize();
    match record.verdict {
        TriageVerdict::Noise => summary.noise_count += 1,
        TriageVerdict::Material => summary.material_count += 1,
        TriageVerdict::FalsificationAlert => summary.falsification_alert_count += 1,
    }
    summary.records.push(record.clone());
}

/// Builds the deterministic 3-part Calm Briefing skeleton (0 LLM tokens).
///
/// 1. Strike Radar: Circle `WATCH + IN` companies in Strike Zone or `<= 15%` above Strike.
/// 2. Filing & Red-Flag Delta: recent events & Red-Flag signals within the period window for `WATCH + IN`.
/// 3. Alan L1.5 Watchdog Hits: non-noise `Material` / `FalsificationAlert` triage results in the window.
pub fn build_deterministic_briefing(
    paths: &WorkspacePaths,
    lake: &LakeDb,
    period: BriefingPeriod,
    lang: UiLang,
) -> Result<CircleBriefing> {
    let now_ms = store::now_ms();
    let window_ms = match period {
        BriefingPeriod::Daily => DAILY_LOOKBACK_MS,
        BriefingPeriod::Weekly => WEEKLY_LOOKBACK_MS,
    };
    let window_start_ms = now_ms.saturating_sub(window_ms);
    let period_key = period_key_for(period, now_ms);
    let briefing_id = format!("briefing-{}-{period_key}", period.as_str());

    let bucket_by_ticker = load_watchlist_bucket_map(paths)?;

    // Part 1: Strike Radar (`WATCH + IN`, Strike Zone or <= 15% above Strike).
    let mut strike_radar: Vec<StrikeRadarItem> = Vec::new();
    if paths.watchlist_path().exists() {
        let reports = store::load_or_compute_reports(paths)?;
        let max_dist = Decimal::from(STRIKE_RADAR_MAX_DISTANCE_PCT);
        for rep in reports {
            if !is_proactive_triage_eligible(Some(rep.bucket)) {
                continue;
            }
            let val = rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref());
            let Some(val) = val else {
                continue;
            };
            if val.strike_price.is_none() {
                continue;
            }
            let Some(dist) = val.distance_to_strike_pct else {
                continue;
            };
            if dist <= max_dist {
                strike_radar.push(StrikeRadarItem {
                    ticker: rep.ticker,
                    name: rep.name,
                    bucket: rep.bucket,
                    currency: rep.currency,
                    latest_price: rep.price.as_ref().map(|p| p.price),
                    strike_price: val.strike_price,
                    distance_to_strike_pct: Some(dist),
                    status: rep.status,
                    in_strike_zone: rep.bucket == Bucket::Core && dist <= Decimal::ZERO,
                });
            }
        }
    }
    strike_radar.sort_by(|a, b| {
        a.distance_to_strike_pct
            .cmp(&b.distance_to_strike_pct)
            .then_with(|| a.ticker.cmp(&b.ticker))
    });

    // Part 2: Filing & Red-Flag Delta (`WATCH + IN` within lookback window).
    let mut filing_deltas: Vec<FilingDeltaItem> = Vec::new();
    let mut seen_delta_keys: HashSet<String> = HashSet::new();

    for ev in lake.list_events(None, None, false, 200)? {
        if ev.occurred_at_ms < window_start_ms {
            continue;
        }
        let Some(raw_ticker) = ev.ticker.as_deref() else {
            continue;
        };
        let ticker = raw_ticker.trim().to_ascii_uppercase();
        if !is_proactive_triage_eligible(bucket_by_ticker.get(&ticker).copied()) {
            continue;
        }
        let is_red_flag = ev.event_type.to_ascii_lowercase().contains("red_flag");
        let dedup_key = format!("ev:{}", ev.event_id);
        if seen_delta_keys.insert(dedup_key) {
            filing_deltas.push(FilingDeltaItem {
                ticker,
                event_type: ev.event_type,
                title: ev.title,
                summary: ev.summary,
                filing_date: ev.filing_date,
                url: ev.url,
                occurred_at_ms: ev.occurred_at_ms,
                is_red_flag,
            });
        }
    }

    for sig in lake.list_inbox_signals(None, Some(SignalKind::RedFlag), 100)? {
        if sig.updated_at_ms < window_start_ms {
            continue;
        }
        let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
        if !is_proactive_triage_eligible(bucket_by_ticker.get(&ticker).copied()) {
            continue;
        }
        let dedup_key = format!("rf:{}", sig.signal_id);
        if seen_delta_keys.insert(dedup_key) {
            let url = sig
                .evidence
                .first()
                .map(|ev| ev.source_url.clone())
                .unwrap_or_default();
            let filing_date = sig.evidence.first().and_then(|ev| ev.filing_date.clone());
            filing_deltas.push(FilingDeltaItem {
                ticker,
                event_type: "red_flag".to_owned(),
                title: sig.title,
                summary: sig.summary,
                filing_date,
                url,
                occurred_at_ms: sig.updated_at_ms,
                is_red_flag: true,
            });
        }
    }
    filing_deltas.sort_by(|a, b| {
        b.is_red_flag
            .cmp(&a.is_red_flag)
            .then_with(|| b.occurred_at_ms.cmp(&a.occurred_at_ms))
            .then_with(|| a.ticker.cmp(&b.ticker))
    });

    // Part 3: Alan L1.5 Watchdog Hits (`Material` and `FalsificationAlert` only; `Noise` excluded).
    let mut watchdog_hits: Vec<WatchdogHitItem> = list_signal_triages(&lake.conn, None, None, 200)?
        .into_iter()
        .filter(|rec| {
            rec.created_at_ms >= window_start_ms
                && rec.verdict != TriageVerdict::Noise
                && is_proactive_triage_eligible(
                    bucket_by_ticker
                        .get(&rec.ticker.trim().to_ascii_uppercase())
                        .copied(),
                )
        })
        .map(|rec| WatchdogHitItem {
            signal_id: rec.signal_id,
            ticker: rec.ticker,
            verdict: rec.verdict,
            headline: rec.headline,
            rationale: rec.rationale,
            fallback_no_metrics_warning: rec.fallback_no_metrics_warning,
            quotes: rec.quotes,
            created_at_ms: rec.created_at_ms,
        })
        .collect();
    watchdog_hits.sort_by(|a, b| {
        verdict_priority(b.verdict)
            .cmp(&verdict_priority(a.verdict))
            .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
            .then_with(|| a.ticker.cmp(&b.ticker))
    });

    let is_quiet = strike_radar.is_empty() && filing_deltas.is_empty() && watchdog_hits.is_empty();
    let headline = if is_quiet {
        lang.t("无球可挥 · 耐心等待", "No Fat Pitches · Stay Patient")
            .to_owned()
    } else {
        match lang {
            UiLang::Zh => format!(
                "能力圈{} · 击球雷达 {} · 公告与预警异动 {} · Watchdog 命中 {}",
                match period {
                    BriefingPeriod::Daily => "每日简报",
                    BriefingPeriod::Weekly => "每周简报",
                },
                strike_radar.len(),
                filing_deltas.len(),
                watchdog_hits.len()
            ),
            UiLang::En => format!(
                "Circle {} Briefing · {} Strike Radar · {} Filing Deltas · {} Watchdog Hits",
                match period {
                    BriefingPeriod::Daily => "Daily",
                    BriefingPeriod::Weekly => "Weekly",
                },
                strike_radar.len(),
                filing_deltas.len(),
                watchdog_hits.len()
            ),
        }
    };

    Ok(CircleBriefing {
        briefing_id,
        period,
        period_key,
        generated_at_ms: now_ms,
        lang,
        is_quiet,
        headline,
        executive_summary: None,
        summary_tokens_used: 0,
        summary_cost_usd: Decimal::ZERO,
        strike_radar,
        filing_deltas,
        watchdog_hits,
    })
}

fn verdict_priority(verdict: TriageVerdict) -> u8 {
    match verdict {
        TriageVerdict::FalsificationAlert => 2,
        TriageVerdict::Material => 1,
        TriageVerdict::Noise => 0,
    }
}

/// Synchronous wrapper for [`generate_and_save_briefing`] for CLI and blocking callers.
pub fn generate_and_save_briefing_blocking(
    paths: &WorkspacePaths,
    lake: &LakeDb,
    period: BriefingPeriod,
    lang: UiLang,
    allow_llm_summary: bool,
) -> Result<CircleBriefing> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for circle briefing")?;
    rt.block_on(generate_and_save_briefing(
        paths,
        lake,
        period,
        lang,
        allow_llm_summary,
    ))
}

/// Generates a deterministic 3-part briefing, optionally adds at most 1 lightweight Alan
/// Munger-style executive summary when `!is_quiet`, and persists the result to `circle_briefings`.
pub async fn generate_and_save_briefing(
    paths: &WorkspacePaths,
    lake: &LakeDb,
    period: BriefingPeriod,
    lang: UiLang,
    allow_llm_summary: bool,
) -> Result<CircleBriefing> {
    generate_and_save_briefing_with_llm(paths, lake, period, lang, allow_llm_summary, None).await
}

/// Testable briefing generator accepting an explicit `LlmSource`.
pub async fn generate_and_save_briefing_with_llm(
    paths: &WorkspacePaths,
    lake: &LakeDb,
    period: BriefingPeriod,
    lang: UiLang,
    allow_llm_summary: bool,
    llm_override: Option<LlmSource>,
) -> Result<CircleBriefing> {
    ensure_proactive_schema(&lake.conn)?;
    let mut briefing = build_deterministic_briefing(paths, lake, period, lang)?;

    if !briefing.is_quiet && allow_llm_summary {
        let llm_opt = match llm_override {
            Some(llm) => Some(llm),
            None => resolve_ready_alan_model()?.map(|_| LlmSource::SharedProductCredentials),
        };

        if let Some(llm_source) = llm_opt {
            let now_ms = briefing.generated_at_ms;
            let budget_cfg = budget::load_budget_config(paths)?;
            let (monthly_tokens, monthly_usd) = combined_monthly_usage(lake, now_ms)?;
            let summaries_today =
                count_briefing_summaries_since(&lake.conn, utc_day_start_ms(now_ms))?;
            let gate = budget::evaluate_briefing_summary_gate(
                &budget_cfg,
                monthly_tokens,
                monthly_usd,
                summaries_today,
            );

            if gate.allowed {
                let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
                let app = boot_m3_embedded(paths.clone(), fs, llm_source).await?;
                let sys_prompt = match lang {
                    UiLang::Zh => {
                        "你是 Project-M3 的 Alan（查理·芒格派研究幕僚）。严禁调用工具，严禁自行编造或改写任何数字；请用不超过 160 字给出冷静、克制的执行摘要。"
                    }
                    UiLang::En => {
                        "You are Alan, Project-M3's Charlie Munger-style research aide. Never call tools and never invent or alter numbers; write a calm, concise executive summary under 140 words."
                    }
                };
                let user_prompt = compose_briefing_summary_prompt(&briefing, lang);
                let turn_res = run_single_alan_turn(&app, sys_prompt, user_prompt).await;
                if let Err(err) = app.shutdown().await {
                    log::warn!("Alan shutdown warning after briefing summary: {err:#}");
                }
                if let Ok((raw_summary, in_tok, out_tok)) = turn_res {
                    let trimmed = raw_summary.trim();
                    if !trimmed.is_empty() {
                        briefing.executive_summary = Some(trimmed.to_owned());
                        briefing.summary_tokens_used = in_tok.saturating_add(out_tok);
                        briefing.summary_cost_usd =
                            budget::estimate_cost_usd(&budget_cfg, in_tok, out_tok);
                    }
                }
            }
        }
    }

    upsert_circle_briefing(&lake.conn, &briefing)?;
    Ok(briefing)
}

fn compose_briefing_summary_prompt(briefing: &CircleBriefing, lang: UiLang) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Period: {} ({})",
        briefing.period.as_str(),
        briefing.period_key
    ));
    if !briefing.strike_radar.is_empty() {
        lines.push("Strike Radar:".to_owned());
        for item in briefing.strike_radar.iter().take(10) {
            let price = item
                .latest_price
                .map(|p| format!("{} {}", item.currency, p.round_dp(2).normalize()))
                .unwrap_or_else(|| "—".to_owned());
            let strike = item
                .strike_price
                .map(|s| format!("{} {}", item.currency, s.round_dp(2).normalize()))
                .unwrap_or_else(|| "—".to_owned());
            let dist = item
                .distance_to_strike_pct
                .map(|d| format!("{:.2}%", d))
                .unwrap_or_else(|| "—".to_owned());
            lines.push(format!(
                "- {} [{}]: price={price}, strike={strike}, distance={dist}, in_zone={}",
                item.ticker,
                item.bucket.code(),
                item.in_strike_zone
            ));
        }
    }
    if !briefing.filing_deltas.is_empty() {
        lines.push("Filing & Red-Flag Delta:".to_owned());
        for item in briefing.filing_deltas.iter().take(10) {
            lines.push(format!(
                "- {} [{}]: {} — {}",
                item.ticker,
                item.event_type,
                item.title,
                truncate_chars(&item.summary, 140)
            ));
        }
    }
    if !briefing.watchdog_hits.is_empty() {
        lines.push("Alan L1.5 Watchdog Hits:".to_owned());
        for hit in briefing.watchdog_hits.iter().take(10) {
            lines.push(format!(
                "- {} [{}]: {} — {}",
                hit.ticker,
                hit.verdict.as_str(),
                hit.headline,
                truncate_chars(&hit.rationale, 160)
            ));
        }
    }

    let instruction = match lang {
        UiLang::Zh => {
            "请基于以上确定性简报骨架，输出一段查理·芒格风格的冷静执行摘要（先关注证伪风险，再看击球价边界，不要给出买卖指令）。"
        }
        UiLang::En => {
            "Based strictly on the deterministic briefing facts above, write a calm Charlie Munger-style executive summary highlighting falsification risks first, then strike-price discipline, with no buy/sell advice."
        }
    };

    format!("{instruction}\n\n{}", lines.join("\n"))
}

async fn run_single_alan_turn(
    app: &EmbeddedAlan,
    system_prompt: &str,
    user_prompt: String,
) -> Result<(String, u64, u64)> {
    let opts = SessionOptions::default().with_system_prompt_mode(SystemPromptMode::Replace {
        prompt: system_prompt.to_owned(),
    });
    let session = app
        .spawn_child_session(app.root_pid(), opts)
        .await
        .context("failed to spawn Alan child session")?;

    let client = session.client();
    let res = drive_single_turn_session(client, user_prompt).await;
    if let Err(err) = session.close().await {
        log::warn!("Alan child session close warning: {err:#}");
    }
    res
}

async fn drive_single_turn_session(
    client: &alan_os_host::embed::RootAgentClient,
    user_prompt: String,
) -> Result<(String, u64, u64)> {
    let mut stream = client
        .event_stream()
        .await
        .context("failed to open Alan event stream")?;
    let submission_id = client
        .submit_text(user_prompt)
        .await
        .context("failed to submit proactive Alan prompt")?;

    let mut content = String::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;

    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(PROACTIVE_SESSION_TIMEOUT_SECS);
    loop {
        let ev = tokio::time::timeout_at(deadline, stream.next_event())
            .await
            .context("Alan proactive turn timed out")??;
        match ev {
            EmbedEvent::TextDelta { chunk, .. } => {
                if !chunk.is_empty() {
                    content.push_str(&chunk);
                }
            }
            EmbedEvent::Usage { usage, .. } => {
                input_tokens = input_tokens.max(usage.session_input_tokens.max(usage.input_tokens));
                output_tokens =
                    output_tokens.max(usage.session_output_tokens.max(usage.output_tokens));
            }
            EmbedEvent::PendingRequest { request_id, .. } => {
                let _ = client
                    .respond_request(&request_id, "denied: non-interactive proactive run")
                    .await;
            }
            EmbedEvent::Error {
                message,
                recoverable,
            } => {
                if !recoverable {
                    anyhow::bail!("{message}");
                }
            }
            EmbedEvent::TurnFinished {
                submission_id: finished_id,
                status,
                error,
            } if finished_id == submission_id => {
                if status != UiInputStatus::Completed {
                    let msg = error.unwrap_or_else(|| format!("status {status:?}"));
                    anyhow::bail!("{msg}");
                }
                break;
            }
            EmbedEvent::ThinkingDelta { .. }
            | EmbedEvent::ToolStarted { .. }
            | EmbedEvent::ToolProgress { .. }
            | EmbedEvent::ToolFinished { .. }
            | EmbedEvent::TurnFinished { .. } => {}
        }
    }

    Ok((content, input_tokens, output_tokens))
}

fn resolve_ready_alan_model() -> Result<Option<String>> {
    let status = match alan_connection::list_alan_connection_status() {
        Ok(st) => st,
        Err(_) => return Ok(None),
    };
    let Some(active) = status.active_profile() else {
        return Ok(None);
    };
    if !active.has_secret {
        return Ok(None);
    }
    let model = active
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(&active.provider)
        .to_owned();
    Ok(Some(model))
}

fn load_watchlist_bucket_map(paths: &WorkspacePaths) -> Result<HashMap<String, Bucket>> {
    if !paths.watchlist_path().exists() {
        return Ok(HashMap::new());
    }
    let wl = store::load_watchlist(paths)?;
    let mut map = HashMap::with_capacity(wl.companies.len());
    for company in wl.companies {
        let ticker = company.ticker.trim().to_ascii_uppercase();
        if !ticker.is_empty() {
            map.insert(ticker, company.bucket);
        }
    }
    Ok(map)
}

fn combined_monthly_usage(lake: &LakeDb, now_ms: u64) -> Result<(u64, Decimal)> {
    let research = lake.monthly_budget_usage(now_ms)?;
    let (proactive_tokens, proactive_usd) =
        monthly_proactive_usage(&lake.conn, utc_month_start_ms(now_ms))?;
    let total_tokens = research
        .total_input_tokens
        .saturating_add(research.total_output_tokens)
        .saturating_add(proactive_tokens);
    let total_usd = (research.total_cost_usd + proactive_usd).normalize();
    Ok((total_tokens, total_usd))
}

/// Returns the UTC start-of-day timestamp in milliseconds for `now_ms`.
pub fn utc_day_start_ms(now_ms: u64) -> u64 {
    let clamped = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let Some(dt) = Utc.timestamp_millis_opt(clamped).single() else {
        return 0;
    };
    NaiveDate::from_ymd_opt(dt.year(), dt.month(), dt.day())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| Utc.from_utc_datetime(&ndt).timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

/// Returns the UTC start-of-month timestamp in milliseconds for `now_ms`.
pub fn utc_month_start_ms(now_ms: u64) -> u64 {
    let clamped = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let Some(dt) = Utc.timestamp_millis_opt(clamped).single() else {
        return 0;
    };
    NaiveDate::from_ymd_opt(dt.year(), dt.month(), 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| Utc.from_utc_datetime(&ndt).timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

fn period_key_for(period: BriefingPeriod, now_ms: u64) -> String {
    let clamped = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let Some(dt) = Utc.timestamp_millis_opt(clamped).single() else {
        return store::civil_today_iso();
    };
    match period {
        BriefingPeriod::Daily => dt.format("%Y-%m-%d").to_string(),
        BriefingPeriod::Weekly => {
            let iso = dt.iso_week();
            format!("{:04}-W{:02}", iso.year(), iso.week())
        }
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let prefix: String = trimmed.chars().take(max_chars).collect();
    format!("{prefix}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::BudgetConfig;
    use crate::lake_l1::{LakeEventRow, SignalEvidence, SignalStatus};
    use crate::model::{
        AnnualFinancials, CompanyEntry, DataSource, DecisionJournal, FieldProvenance,
        HumanJudgment, Latticework, PriceSnapshot, RawCompanyFactsSummary, ValuationAssumptions,
        WatchlistFile,
    };
    use alan_llm::{GenerationResponse, MockLlmProvider, TokenUsage};
    use alan_os_host::embed::LlmClient;
    use std::fs;
    use std::path::PathBuf;

    struct TempWorkspace {
        root: PathBuf,
        paths: WorkspacePaths,
    }

    impl TempWorkspace {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let root = std::env::temp_dir().join(format!(
                "m3_proactive_test_{tag}_{}_{}",
                std::process::id(),
                nanos
            ));
            let _ = fs::remove_dir_all(&root);
            let paths = WorkspacePaths::from_root(root.clone());
            fs::create_dir_all(&paths.workbench_dir).unwrap();
            fs::create_dir_all(paths.workbench_dir.join("dossiers")).unwrap();
            fs::create_dir_all(paths.data_dir.join("raw/sec")).unwrap();
            fs::create_dir_all(paths.data_dir.join("raw/prices")).unwrap();
            fs::create_dir_all(paths.data_dir.join("computed")).unwrap();
            Self { root, paths }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn sample_company(ticker: &str, bucket: Bucket, cik: &str) -> CompanyEntry {
        CompanyEntry {
            ticker: ticker.to_owned(),
            name: format!("{ticker} Corp."),
            bucket,
            source: DataSource::SecEdgar,
            cik: Some(cik.to_owned()),
            currency: "USD".to_owned(),
            circle: "Software".to_owned(),
            too_hard_reason: (bucket == Bucket::TooHard).then(|| "Unclear".to_owned()),
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(15),
                multiple_high: Decimal::from(20),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        }
    }

    fn seed_financials_and_price(paths: &WorkspacePaths, ticker: &str, cik: &str, price_val: i64) {
        let facts = RawCompanyFactsSummary {
            schema_version: crate::edgar::SEC_FACTS_SCHEMA_VERSION,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: ticker.to_owned(),
            entity_name: format!("{ticker} Corp."),
            cik: cik.to_owned(),
            source_url: format!("https://data.sec.gov/api/xbrl/companyfacts/CIK{cik}.json"),
            raw_sha256: format!("sha-{ticker}"),
            fetched_at_ms: 1_700_000_000_000,
            annuals: vec![AnnualFinancials {
                fiscal_year: 2025,
                period_end: "2025-12-31".to_owned(),
                filed: Some("2026-02-15".to_owned()),
                revenue: Decimal::from(1_000_000_000i64),
                gross_profit: Decimal::from(750_000_000i64),
                ebit: Decimal::from(300_000_000i64),
                tax_expense: Decimal::from(60_000_000i64),
                pretax_income: Decimal::from(300_000_000i64),
                ocf: Decimal::from(320_000_000i64),
                capex: Decimal::from(20_000_000i64),
                sbc: Decimal::from(20_000_000i64),
                buybacks: Decimal::from(50_000_000i64),
                cash_and_st: Some(Decimal::from(200_000_000i64)),
                total_debt: Decimal::from(50_000_000i64),
                equity: Decimal::from(800_000_000i64),
                diluted_shares: Decimal::from(100_000_000i64),
                ending_shares: Decimal::from(100_000_000i64),
                provenance: vec![FieldProvenance {
                    field: "ocf".to_owned(),
                    source_tag: "us-gaap:NetCashProvidedByUsedInOperatingActivities".to_owned(),
                    filed: Some("2026-02-15".to_owned()),
                    period_end: "2025-12-31".to_owned(),
                    note: None,
                }],
            }],
            ttm: None,
            manual_note: None,
        };
        store::write_json_atomic(&paths.sec_facts_summary_path(ticker), &facts).unwrap();
        let price = PriceSnapshot {
            schema_version: 1,
            symbol: ticker.to_owned(),
            price: Decimal::from(price_val),
            previous_close: Some(Decimal::from(price_val)),
            currency: "USD".to_owned(),
            exchange: "NASDAQ".to_owned(),
            market_time_ms: Some(store::now_ms()),
            fetched_at_ms: store::now_ms(),
            source_url: "https://example.com/quote".to_owned(),
        };
        store::write_json_atomic(&paths.price_path(ticker), &price).unwrap();
    }

    fn seed_watchlist_and_journal(paths: &WorkspacePaths, companies: Vec<CompanyEntry>) {
        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".to_owned(),
            default_target_return_pct: Decimal::from(10),
            companies,
        };
        store::save_watchlist(paths, &wl).unwrap();
        let journal = DecisionJournal {
            schema_version: 1,
            account_note: "test".to_owned(),
            currency: "USD".to_owned(),
            starting_cash: Decimal::from(100_000),
            cash_balance: Decimal::from(100_000),
            benchmark_symbol: "^GSPC".to_owned(),
            benchmark_start_price: None,
            positions: Vec::new(),
            entries: Vec::new(),
        };
        store::save_journal(paths, &journal).unwrap();
    }

    fn text_response(text: &str) -> GenerationResponse {
        GenerationResponse {
            content: text.to_owned(),
            thinking: None,
            thinking_signature: None,
            redacted_thinking: Vec::new(),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                prompt_tokens: 100,
                cached_prompt_tokens: None,
                completion_tokens: 40,
                total_tokens: 140,
                reasoning_tokens: None,
                ..Default::default()
            }),
            finish_reason: Some("stop".to_owned()),
            warnings: Vec::new(),
            provider_response_id: None,
            provider_response_status: None,
        }
    }

    fn sample_signal(signal_id: &str, ticker: &str, kind: SignalKind) -> SignalInboxRow {
        let now = store::now_ms();
        SignalInboxRow {
            signal_id: signal_id.to_owned(),
            kind,
            score: 85.0,
            primary_ticker: ticker.to_owned(),
            tickers: vec![ticker.to_owned()],
            title: format!("{ticker} filed 8-K Item 5.02"),
            summary: "Executive transition and margin guidance update.".to_owned(),
            evidence: vec![SignalEvidence {
                chunk_id: Some("chk-1".to_owned()),
                ticker: ticker.to_owned(),
                doc_type: "8-K".to_owned(),
                section: "item_5_02".to_owned(),
                filing_date: Some("2026-11-25".to_owned()),
                title: "Item 5.02".to_owned(),
                quote: "CFO resigned effective immediately; gross margin declined 450 bps."
                    .to_owned(),
                source_url: "https://www.sec.gov/Archives/edgar/data/1/8k.htm".to_owned(),
                similarity: 0.92,
            }],
            status: SignalStatus::Unread,
            event_id: None,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    #[test]
    fn schema_and_crud_roundtrip_on_in_memory_duckdb() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(!has_proactive_tables(&conn).unwrap());
        ensure_proactive_schema(&conn).unwrap();
        ensure_proactive_schema(&conn).unwrap();
        assert!(has_proactive_tables(&conn).unwrap());

        let now = 1_795_000_000_000u64;
        let rec = SignalTriageRecord {
            signal_id: "sig-1".to_owned(),
            ticker: "aapl".to_owned(),
            verdict: TriageVerdict::FalsificationAlert,
            headline: "Services margin hit".to_owned(),
            rationale: "Matched metric".to_owned(),
            matched_metrics: vec!["Services gross margin < 68%".to_owned()],
            suggested_metrics: vec![],
            quotes: vec![TriageQuote {
                section: "item_7".to_owned(),
                filing_date: Some("2026-11-01".to_owned()),
                quote: "Services gross margin fell to 66.5%.".to_owned(),
                source_url: "https://sec.gov/aapl".to_owned(),
            }],
            fallback_no_metrics_warning: false,
            model_used: "gpt-5.4".to_owned(),
            tokens_used: 1_200,
            cost_usd: Decimal::new(12, 3),
            created_at_ms: now,
        };
        upsert_signal_triage(&conn, &rec).unwrap();

        let fetched = get_signal_triage(&conn, "sig-1").unwrap().unwrap();
        assert_eq!(fetched.ticker, "AAPL");
        assert_eq!(fetched.verdict, TriageVerdict::FalsificationAlert);
        assert_eq!(fetched.cost_usd, Decimal::new(12, 3));
        assert_eq!(
            list_signal_triages(
                &conn,
                Some("AAPL"),
                Some(TriageVerdict::FalsificationAlert),
                10
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(count_triages_since(&conn, now - 1000).unwrap(), 1);

        let briefing = CircleBriefing {
            briefing_id: "briefing-daily-2026-11-26".to_owned(),
            period: BriefingPeriod::Daily,
            period_key: "2026-11-26".to_owned(),
            generated_at_ms: now,
            lang: UiLang::En,
            is_quiet: false,
            headline: "Circle Daily Briefing".to_owned(),
            executive_summary: Some("Watch AAPL margin trend.".to_owned()),
            summary_tokens_used: 300,
            summary_cost_usd: Decimal::new(3, 3),
            strike_radar: vec![],
            filing_deltas: vec![],
            watchdog_hits: vec![],
        };
        upsert_circle_briefing(&conn, &briefing).unwrap();
        let latest = latest_circle_briefing(&conn, Some(BriefingPeriod::Daily))
            .unwrap()
            .unwrap();
        assert_eq!(latest.briefing_id, briefing.briefing_id);
        assert_eq!(
            count_briefing_summaries_since(&conn, now - 1000).unwrap(),
            1
        );

        let (tok, usd) = monthly_proactive_usage(&conn, now - 10_000).unwrap();
        assert_eq!(tok, 1_500);
        assert_eq!(usd, Decimal::new(15, 3));
    }

    #[test]
    fn eligibility_and_prompt_fallback_respect_q2_and_q3() {
        assert!(is_proactive_triage_eligible(Some(Bucket::Watch)));
        assert!(is_proactive_triage_eligible(Some(Bucket::Core)));
        assert!(!is_proactive_triage_eligible(Some(Bucket::Out)));
        assert!(!is_proactive_triage_eligible(Some(Bucket::TooHard)));
        assert!(!is_proactive_triage_eligible(None));

        let sig = sample_signal("sig-q3", "VRSN", SignalKind::RedFlag);
        let mut dossier = Dossier {
            schema_version: 1,
            ticker: "VRSN".to_owned(),
            name: "VeriSign".to_owned(),
            updated_at: "2026-11-20".to_owned(),
            draft_source: "manual".to_owned(),
            three_sentence_business: "Registry".to_owned(),
            latticework: Latticework {
                microeconomics: "Monopoly".to_owned(),
                ecology: "ICANN".to_owned(),
                psychology_incentives: "Buybacks".to_owned(),
                capital_allocation: "Clean".to_owned(),
            },
            premortem: vec!["ICANN removes .com price cap renewal".to_owned()],
            open_questions: vec![],
            human_judgment: HumanJudgment {
                pending_human_review: true,
                my_stance: None,
                weight_rationale: None,
                falsification_metrics: vec![],
                feynman_checked_at: None,
            },
        };

        let (prompt_fb, warn_fb) = compose_l15_triage_prompt(&sig, Some(&dossier), None);
        assert!(warn_fb);
        assert!(prompt_fb.contains("ICANN removes .com price cap renewal"));

        dossier.human_judgment.falsification_metrics =
            vec![".com domain base shrinks > 2% YoY".to_owned()];
        let (prompt_ok, warn_ok) = compose_l15_triage_prompt(&sig, Some(&dossier), None);
        assert!(!warn_ok);
        assert!(prompt_ok.contains(".com domain base shrinks > 2% YoY"));
    }

    #[test]
    fn parse_l15_triage_response_handles_fenced_json_and_malformed_text() {
        let sig = sample_signal("sig-parse", "MCO", SignalKind::Strike);
        let fenced = "```json\n{\n  \"verdict\": \"noise\",\n  \"headline\": \"Routine 8-K\",\n  \"rationale\": \"Director re-election only.\",\n  \"matched_metrics\": [],\n  \"suggested_metrics\": [\"Issuance volume < -15%\"],\n  \"quotes\": []\n}\n```";
        let rec = parse_l15_triage_response(
            fenced,
            &sig,
            true,
            "gpt-5.4",
            140,
            Decimal::new(2, 3),
            1_795_000_000_000,
        );
        assert_eq!(rec.verdict, TriageVerdict::Noise);
        assert_eq!(rec.headline, "Routine 8-K");
        assert!(rec.fallback_no_metrics_warning);
        // Falls back to signal evidence quote when model omitted quotes.
        assert_eq!(rec.quotes.len(), 1);

        let broken = "Potential falsification_alert: gross margin fell sharply.";
        let rec_broken = parse_l15_triage_response(
            broken,
            &sig,
            false,
            "gpt-5.4",
            100,
            Decimal::ZERO,
            1_795_000_000_000,
        );
        assert_eq!(rec_broken.verdict, TriageVerdict::FalsificationAlert);
        assert_eq!(rec_broken.quotes.len(), 1);
    }

    #[tokio::test]
    async fn run_l15_circle_triage_enforces_q2_q3_and_q9_quotas() {
        let ws = TempWorkspace::new("triage_e2e");
        seed_watchlist_and_journal(
            &ws.paths,
            vec![
                sample_company("COST", Bucket::Core, "0000909832"),
                sample_company("VRSN", Bucket::Watch, "0001014473"),
                sample_company("INTC", Bucket::TooHard, "0000050863"),
            ],
        );

        let lake = LakeDb::open(&ws.paths).unwrap();
        lake.ensure_l1_schema(&ws.paths).unwrap();
        lake.ensure_l2_schema().unwrap();
        ensure_proactive_schema(&lake.conn).unwrap();

        let signals = vec![
            sample_signal("sig-cost", "COST", SignalKind::RedFlag),
            sample_signal("sig-vrsn", "VRSN", SignalKind::Strike),
            sample_signal("sig-intc", "INTC", SignalKind::RedFlag),
            sample_signal("sig-nvda", "NVDA", SignalKind::Discovery),
        ];
        lake.upsert_signals(&signals).unwrap();

        let provider = MockLlmProvider::new().with_responses(vec![
            text_response(
                r#"{"verdict":"noise","headline":"Routine filing","rationale":"No thesis impact","matched_metrics":[],"suggested_metrics":["Renewal rate < 90%"],"quotes":[]}"#,
            ),
            text_response(
                r#"{"verdict":"falsification_alert","headline":"Margin thesis alert","rationale":"Gross margin dropped 450 bps","matched_metrics":["Premortem"],"suggested_metrics":[],"quotes":[]}"#,
            ),
        ]);

        let summary = run_l15_circle_triage_with_llm(
            &ws.paths,
            &lake,
            10,
            false,
            Some((
                LlmSource::Client(LlmClient::new(provider)),
                "mock-model".to_owned(),
            )),
        )
        .await
        .unwrap();

        // Only COST (Core/IN) and VRSN (Watch) are evaluated; INTC (TooHard) and NVDA (non-Circle) stay at 0 LLM tokens.
        assert_eq!(summary.evaluated, 2);
        assert_eq!(summary.skipped_outside_circle, 2);
        assert_eq!(summary.noise_count, 1);
        assert_eq!(summary.falsification_alert_count, 1);
        assert!(summary.tokens_used > 0);

        // Q3: Noise verdict NEVER deletes/archives signals, and FalsificationAlert NEVER triggers Deep Research.
        let inbox_after = lake.list_inbox_signals(None, None, 20).unwrap();
        assert_eq!(inbox_after.len(), 4);
        assert!(
            inbox_after
                .iter()
                .all(|row| row.status == SignalStatus::Unread)
        );
        assert!(lake.list_research_runs(None, 10).unwrap().is_empty());

        // Re-running skips already-triaged signals.
        let rerun = run_l15_circle_triage_with_llm(
            &ws.paths,
            &lake,
            10,
            false,
            Some((
                LlmSource::Client(LlmClient::new(MockLlmProvider::new())),
                "mock-model".to_owned(),
            )),
        )
        .await
        .unwrap();
        assert_eq!(rerun.evaluated, 0);
        assert_eq!(rerun.skipped_already_triaged, 2);
        assert_eq!(rerun.tokens_used, 0);

        // Daily quota blocking enforces 0 additional LLM tokens.
        lake.upsert_signals(&[sample_signal("sig-cost-2", "COST", SignalKind::RedFlag)])
            .unwrap();
        let tight_cfg = BudgetConfig {
            daily_triage_quota: 2,
            ..BudgetConfig::default()
        };
        budget::save_budget_config(&ws.paths, &tight_cfg).unwrap();
        let blocked = run_l15_circle_triage_with_llm(
            &ws.paths,
            &lake,
            10,
            false,
            Some((
                LlmSource::Client(LlmClient::new(MockLlmProvider::new())),
                "mock-model".to_owned(),
            )),
        )
        .await
        .unwrap();
        assert_eq!(blocked.evaluated, 0);
        assert_eq!(blocked.skipped_budget_or_quota, 1);
        assert!(blocked.budget_reason.is_some());
    }

    #[tokio::test]
    async fn calm_briefing_quiet_uses_zero_tokens_and_active_summarizes_once() {
        let ws = TempWorkspace::new("briefing_e2e");
        // Strike price for 300M OE / 100M shares = $3.00/sh; at 10% target return, 10% growth?
        // With price = $500, distance_to_strike_pct >> 15% so Strike Radar is empty.
        seed_watchlist_and_journal(
            &ws.paths,
            vec![sample_company("COST", Bucket::Core, "0000909832")],
        );
        seed_financials_and_price(&ws.paths, "COST", "0000909832", 500);

        let lake = LakeDb::open(&ws.paths).unwrap();
        lake.ensure_l1_schema(&ws.paths).unwrap();
        ensure_proactive_schema(&lake.conn).unwrap();

        let quiet_briefing = generate_and_save_briefing_with_llm(
            &ws.paths,
            &lake,
            BriefingPeriod::Daily,
            UiLang::En,
            true,
            Some(LlmSource::Client(LlmClient::new(MockLlmProvider::new()))),
        )
        .await
        .unwrap();
        assert!(quiet_briefing.is_quiet);
        assert_eq!(quiet_briefing.headline, "No Fat Pitches · Stay Patient");
        assert!(quiet_briefing.executive_summary.is_none());
        assert_eq!(quiet_briefing.summary_tokens_used, 0);

        // Now move COST into Strike Zone ($15/share) + add recent filing event + Watchdog Material hit.
        seed_financials_and_price(&ws.paths, "COST", "0000909832", 15);
        let now = store::now_ms();
        lake.upsert_events(&[LakeEventRow {
            event_id: "ev-cost-8k".to_owned(),
            source: "sec".to_owned(),
            cik: Some("0000909832".to_owned()),
            ticker: Some("COST".to_owned()),
            event_type: "8-K".to_owned(),
            occurred_at_ms: now,
            filing_date: Some("2026-11-26".to_owned()),
            title: "COST 8-K".to_owned(),
            summary: "Membership fee adjustment disclosed.".to_owned(),
            url: "https://sec.gov/cost-8k".to_owned(),
            metadata_json: "{}".to_owned(),
            processed_mask: 0,
        }])
        .unwrap();
        upsert_signal_triage(
            &lake.conn,
            &SignalTriageRecord {
                signal_id: "sig-cost-mat".to_owned(),
                ticker: "COST".to_owned(),
                verdict: TriageVerdict::Material,
                headline: "Membership pricing update".to_owned(),
                rationale: "Positive pricing power signal".to_owned(),
                matched_metrics: vec![],
                suggested_metrics: vec![],
                quotes: vec![],
                fallback_no_metrics_warning: false,
                model_used: "mock".to_owned(),
                tokens_used: 50,
                cost_usd: Decimal::ZERO,
                created_at_ms: now,
            },
        )
        .unwrap();

        let active_provider = MockLlmProvider::new().with_response(text_response(
            "COST entered the strike zone with an 8-K fee disclosure; verify renewal elasticity before acting.",
        ));
        let active_briefing = generate_and_save_briefing_with_llm(
            &ws.paths,
            &lake,
            BriefingPeriod::Daily,
            UiLang::En,
            true,
            Some(LlmSource::Client(LlmClient::new(active_provider))),
        )
        .await
        .unwrap();
        assert!(!active_briefing.is_quiet);
        assert_eq!(active_briefing.strike_radar.len(), 1);
        assert!(active_briefing.strike_radar[0].in_strike_zone);
        assert_eq!(active_briefing.filing_deltas.len(), 1);
        assert_eq!(active_briefing.watchdog_hits.len(), 1);
        assert!(active_briefing.executive_summary.is_some());
        assert!(active_briefing.summary_tokens_used > 0);
        assert_eq!(
            count_briefing_summaries_since(&lake.conn, utc_day_start_ms(now)).unwrap(),
            1
        );

        // Second run same day hits daily_briefing_summary_quota = 1 and uses 0 extra LLM tokens.
        let quota_limited = generate_and_save_briefing_with_llm(
            &ws.paths,
            &lake,
            BriefingPeriod::Weekly,
            UiLang::Zh,
            true,
            Some(LlmSource::Client(LlmClient::new(MockLlmProvider::new()))),
        )
        .await
        .unwrap();
        assert!(!quota_limited.is_quiet);
        assert!(quota_limited.executive_summary.is_none());
        assert_eq!(quota_limited.summary_tokens_used, 0);

        // Moving COST to WATCH at the same discount keeps it in strike_radar as a Watchlist
        // Discount, while `in_strike_zone` MUST become false (only IN / Bucket::Core qualifies).
        seed_watchlist_and_journal(
            &ws.paths,
            vec![sample_company("COST", Bucket::Watch, "0000909832")],
        );
        let watch_briefing = generate_and_save_briefing_with_llm(
            &ws.paths,
            &lake,
            BriefingPeriod::Daily,
            UiLang::En,
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(watch_briefing.strike_radar.len(), 1);
        assert_eq!(watch_briefing.strike_radar[0].bucket, Bucket::Watch);
        assert!(!watch_briefing.strike_radar[0].in_strike_zone);
    }
}
