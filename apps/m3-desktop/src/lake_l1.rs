use crate::{
    lake::{
        LakeDb, ReadSnapshotTx, UNIVERSE_SELECT_COLS, UniverseRow, escape_sql_literal,
        read_universe_row, round_for_sql,
    },
    latent::{
        self, Embedder, PreparedBm25Query, cosine_similarity, decode_embedding_json,
        extract_verbatim_snippet, hybrid_score, l2_normalize, local_embed,
    },
    model::serde_decimal_opt,
    store::WorkspacePaths,
};
use anyhow::{Context, Result};
use duckdb::{Connection, OptionalExt, ToSql, params, params_from_iter};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
    str::FromStr,
};

const WM_L1_BOOTSTRAP: &str = "l1_sec_submissions_bootstrap";
const WM_L1_CATCHUP: &str = "l1_sec_submissions_catchup";
const WM_L1_NEWS: &str = "l1_candidate_news";
const WM_L1_EVENTS_CATCHUP: &str = "l1_events_catchup";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LakeFilingRow {
    pub accession_no: String,
    pub cik: String,
    pub ticker: String,
    pub form: String,
    pub filing_date: String,
    pub report_date: Option<String>,
    pub primary_document: Option<String>,
    pub primary_doc_description: Option<String>,
    pub items_json: String,
    pub filing_url: String,
    pub ingested_at_ms: u64,
}

impl LakeFilingRow {
    #[allow(dead_code)]
    pub fn parsed_items(&self) -> Vec<String> {
        if self.items_json.trim().is_empty() {
            return Vec::new();
        }
        serde_json::from_str(&self.items_json).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LakeEventRow {
    pub event_id: String,
    pub source: String,
    pub cik: Option<String>,
    pub ticker: Option<String>,
    pub event_type: String,
    pub occurred_at_ms: u64,
    pub filing_date: Option<String>,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub metadata_json: String,
    pub processed_mask: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocChunkRow {
    pub chunk_id: String,
    pub cik: String,
    pub ticker: String,
    pub doc_type: String,
    pub source_id: String,
    pub period_end: Option<String>,
    pub filing_date: Option<String>,
    pub section: String,
    pub title: String,
    pub content: String,
    pub content_sha256: String,
    pub source_url: String,
    pub embedding_model: String,
    pub embedding_dim: usize,
    pub embedding_json: String,
}

impl DocChunkRow {
    #[allow(dead_code)]
    pub fn parsed_embedding(&self) -> Option<Vec<f32>> {
        decode_embedding_json(&self.embedding_json)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Discovery,
    Ripple,
    Strike,
    RedFlag,
}

impl SignalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Ripple => "ripple",
            Self::Strike => "strike",
            Self::RedFlag => "red_flag",
        }
    }
}

impl fmt::Display for SignalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SignalKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "discovery" => Ok(Self::Discovery),
            "ripple" => Ok(Self::Ripple),
            "strike" => Ok(Self::Strike),
            "red_flag" | "redflag" => Ok(Self::RedFlag),
            other => anyhow::bail!(
                "unknown signal kind '{other}'; expected discovery|ripple|strike|red_flag"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    #[default]
    Unread,
    Researching,
    Watch,
    TooHard,
    Archived,
}

impl SignalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Researching => "researching",
            Self::Watch => "watch",
            Self::TooHard => "too_hard",
            Self::Archived => "archived",
        }
    }
}

impl fmt::Display for SignalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SignalStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "unread" => Ok(Self::Unread),
            "researching" => Ok(Self::Researching),
            "watch" => Ok(Self::Watch),
            "too_hard" | "toohard" => Ok(Self::TooHard),
            "archived" => Ok(Self::Archived),
            other => anyhow::bail!(
                "unknown signal status '{other}'; expected unread|researching|watch|too_hard|archived"
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignalEvidence {
    pub chunk_id: Option<String>,
    pub ticker: String,
    pub doc_type: String,
    pub section: String,
    pub filing_date: Option<String>,
    pub title: String,
    pub quote: String,
    pub source_url: String,
    pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignalInboxRow {
    pub signal_id: String,
    pub kind: SignalKind,
    pub score: f64,
    pub primary_ticker: String,
    pub tickers: Vec<String>,
    pub title: String,
    pub summary: String,
    pub evidence: Vec<SignalEvidence>,
    pub status: SignalStatus,
    pub event_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarSearchFilter {
    #[serde(default, with = "serde_decimal_opt")]
    pub min_roic_5y: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub max_sbc_to_ocf: Option<Decimal>,
    pub exclude_financials: bool,
    pub positive_strict_oe_only: bool,
    pub in_price_pool_only: bool,
    pub min_score: f32,
    pub limit: usize,
}

impl Default for SimilarSearchFilter {
    fn default() -> Self {
        Self {
            min_roic_5y: None,
            max_sbc_to_ocf: None,
            exclude_financials: true,
            positive_strict_oe_only: false,
            in_price_pool_only: false,
            min_score: 0.0,
            limit: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarCompanyHit {
    pub company: UniverseRow,
    pub score: f32,
    pub semantic_score: f32,
    pub lexical_score: f32,
    pub matched_chunk_count: usize,
    pub evidence: Vec<SignalEvidence>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct L1Summary {
    pub total_filings: usize,
    pub total_events: usize,
    pub unprocessed_events: usize,
    pub doc_chunks_count: usize,
    pub indexed_companies_count: usize,
    pub total_signals: usize,
    pub unread_signals: usize,
    pub discovery_unread: usize,
    pub ripple_unread: usize,
    pub strike_or_red_flag_unread: usize,
    pub last_catchup_ms: Option<u64>,
    pub last_sec_bootstrap_ms: Option<u64>,
    pub last_news_sync_ms: Option<u64>,
}

struct WriteTx<'a> {
    conn: &'a Connection,
    committed: bool,
}

impl<'a> WriteTx<'a> {
    fn begin(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch("BEGIN TRANSACTION;")?;
        Ok(Self {
            conn,
            committed: false,
        })
    }

    fn commit(mut self) -> Result<()> {
        self.conn.execute_batch("COMMIT;")?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for WriteTx<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
    }
}

impl LakeDb {
    /// Creates additive Milestone B L1 tables and refreshes the `v_filings` view.
    pub fn ensure_l1_schema(&self, paths: &WorkspacePaths) -> Result<()> {
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS filings (
                    accession_no VARCHAR PRIMARY KEY,
                    cik VARCHAR NOT NULL,
                    ticker VARCHAR NOT NULL,
                    form VARCHAR NOT NULL,
                    filing_date VARCHAR NOT NULL,
                    report_date VARCHAR,
                    primary_document VARCHAR,
                    primary_doc_description VARCHAR,
                    items_json VARCHAR NOT NULL,
                    filing_url VARCHAR NOT NULL,
                    ingested_at_ms UBIGINT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS events (
                    event_id VARCHAR PRIMARY KEY,
                    source VARCHAR NOT NULL,
                    cik VARCHAR,
                    ticker VARCHAR,
                    event_type VARCHAR NOT NULL,
                    occurred_at_ms UBIGINT NOT NULL,
                    filing_date VARCHAR,
                    title VARCHAR NOT NULL,
                    summary VARCHAR NOT NULL,
                    url VARCHAR NOT NULL,
                    metadata_json VARCHAR NOT NULL,
                    processed_mask UBIGINT NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS doc_chunks (
                    chunk_id VARCHAR PRIMARY KEY,
                    cik VARCHAR NOT NULL,
                    ticker VARCHAR NOT NULL,
                    doc_type VARCHAR NOT NULL,
                    source_id VARCHAR NOT NULL,
                    period_end VARCHAR,
                    filing_date VARCHAR,
                    section VARCHAR NOT NULL,
                    title VARCHAR NOT NULL,
                    content VARCHAR NOT NULL,
                    content_sha256 VARCHAR NOT NULL,
                    source_url VARCHAR NOT NULL,
                    embedding_model VARCHAR NOT NULL,
                    embedding_dim INTEGER NOT NULL,
                    embedding_json VARCHAR NOT NULL
                );

                CREATE TABLE IF NOT EXISTS signal_inbox (
                    signal_id VARCHAR PRIMARY KEY,
                    kind VARCHAR NOT NULL,
                    score DOUBLE NOT NULL,
                    primary_ticker VARCHAR NOT NULL,
                    tickers_json VARCHAR NOT NULL,
                    title VARCHAR NOT NULL,
                    summary VARCHAR NOT NULL,
                    evidence_json VARCHAR NOT NULL,
                    status VARCHAR NOT NULL,
                    event_id VARCHAR,
                    created_at_ms UBIGINT NOT NULL,
                    updated_at_ms UBIGINT NOT NULL
                );
                ",
            )
            .context("failed to initialize L1 DuckDB tables")?;

        self.refresh_filings_view(paths)?;
        Ok(())
    }

    fn refresh_filings_view(&self, paths: &WorkspacePaths) -> Result<()> {
        let parquet_path = paths.lake_parquet_dir().join("filings.parquet");
        let sql = if parquet_path.exists() {
            let escaped = escape_sql_literal(&parquet_path.to_string_lossy());
            format!("CREATE OR REPLACE VIEW v_filings AS SELECT * FROM read_parquet('{escaped}');")
        } else {
            "CREATE OR REPLACE VIEW v_filings AS SELECT * FROM filings;".to_owned()
        };
        self.conn
            .execute_batch(&sql)
            .context("failed to refresh v_filings view")?;
        Ok(())
    }

    fn table_exists(&self, table_name: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
            [table_name],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn upsert_filings(&self, rows: &[LakeFilingRow]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let tx = WriteTx::begin(&self.conn)?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT OR REPLACE INTO filings (
                    accession_no, cik, ticker, form, filing_date, report_date,
                    primary_document, primary_doc_description, items_json,
                    filing_url, ingested_at_ms
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.accession_no,
                    row.cik,
                    row.ticker,
                    row.form,
                    row.filing_date,
                    row.report_date,
                    row.primary_document,
                    row.primary_doc_description,
                    row.items_json,
                    row.filing_url,
                    row.ingested_at_ms,
                ])?;
            }
        }
        tx.commit()?;
        Ok(rows.len())
    }

    #[allow(dead_code)]
    pub fn has_filing(&self, accession_no: &str) -> Result<bool> {
        if !self.table_exists("filings")? {
            return Ok(false);
        }
        let found: Option<i32> = self
            .conn
            .query_row(
                "SELECT 1 FROM filings WHERE accession_no = ? LIMIT 1",
                [accession_no],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn list_filings_for_cik(&self, cik: &str, limit: usize) -> Result<Vec<LakeFilingRow>> {
        if limit == 0 || !self.table_exists("filings")? {
            return Ok(Vec::new());
        }
        let sql_limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = self.conn.prepare(
            "SELECT accession_no, cik, ticker, form, filing_date, report_date,
                    primary_document, primary_doc_description, items_json,
                    filing_url, ingested_at_ms
             FROM filings
             WHERE cik = ?
             ORDER BY filing_date DESC, accession_no DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![cik, sql_limit], |r| {
            Ok(LakeFilingRow {
                accession_no: r.get(0)?,
                cik: r.get(1)?,
                ticker: r.get(2)?,
                form: r.get(3)?,
                filing_date: r.get(4)?,
                report_date: r.get(5)?,
                primary_document: r.get(6)?,
                primary_doc_description: r.get(7)?,
                items_json: r.get(8)?,
                filing_url: r.get(9)?,
                ingested_at_ms: r.get(10)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list filings for cik")
    }

    pub fn export_filings_parquet(&self, paths: &WorkspacePaths) -> Result<PathBuf> {
        let dest_path = paths.lake_parquet_dir().join("filings.parquet");
        let select_sql = "
            SELECT accession_no, cik, ticker, form, filing_date, report_date,
                   primary_document, primary_doc_description, items_json,
                   filing_url, ingested_at_ms
            FROM filings
            ORDER BY filing_date DESC, accession_no
        ";
        self.export_parquet_atomic(select_sql, &dest_path)?;
        self.refresh_filings_view(paths)?;
        Ok(dest_path)
    }

    pub fn upsert_events(&self, rows: &[LakeEventRow]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let tx = WriteTx::begin(&self.conn)?;
        let existing_masks = load_existing_event_masks(&self.conn, rows)?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT OR REPLACE INTO events (
                    event_id, source, cik, ticker, event_type, occurred_at_ms,
                    filing_date, title, summary, url, metadata_json, processed_mask
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for row in rows {
                let prev_mask = existing_masks.get(&row.event_id).copied().unwrap_or(0);
                let merged_mask = prev_mask | row.processed_mask;
                stmt.execute(params![
                    row.event_id,
                    row.source,
                    row.cik,
                    row.ticker,
                    row.event_type,
                    row.occurred_at_ms,
                    row.filing_date,
                    row.title,
                    row.summary,
                    row.url,
                    row.metadata_json,
                    merged_mask,
                ])?;
            }
        }
        tx.commit()?;
        Ok(rows.len())
    }

    pub fn list_events(
        &self,
        ticker: Option<&str>,
        event_type: Option<&str>,
        unprocessed_only: bool,
        limit: usize,
    ) -> Result<Vec<LakeEventRow>> {
        if limit == 0 || !self.table_exists("events")? {
            return Ok(Vec::new());
        }
        let mut clauses: Vec<&str> = Vec::new();
        let mut owned_params: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(t) = ticker.map(str::trim).filter(|s| !s.is_empty()) {
            clauses.push("UPPER(COALESCE(ticker, '')) = UPPER(?)");
            owned_params.push(Box::new(t.to_owned()));
        }
        if let Some(et) = event_type.map(str::trim).filter(|s| !s.is_empty()) {
            clauses.push("LOWER(event_type) = LOWER(?)");
            owned_params.push(Box::new(et.to_owned()));
        }
        if unprocessed_only {
            clauses.push("processed_mask = 0");
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        owned_params.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));

        let sql = format!(
            "SELECT event_id, source, cik, ticker, event_type, occurred_at_ms,
                    filing_date, title, summary, url, metadata_json, processed_mask
             FROM events
             {where_sql}
             ORDER BY occurred_at_ms DESC, event_id ASC
             LIMIT ?"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let refs = owned_params.iter().map(|b| b.as_ref());
        let rows = stmt.query_map(params_from_iter(refs), |r| {
            Ok(LakeEventRow {
                event_id: r.get(0)?,
                source: r.get(1)?,
                cik: r.get(2)?,
                ticker: r.get(3)?,
                event_type: r.get(4)?,
                occurred_at_ms: r.get(5)?,
                filing_date: r.get(6)?,
                title: r.get(7)?,
                summary: r.get(8)?,
                url: r.get(9)?,
                metadata_json: r.get(10)?,
                processed_mask: r.get(11)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list L1 events")
    }

    pub fn mark_event_processed(&self, event_id: &str, mask: u64) -> Result<bool> {
        if !self.table_exists("events")? {
            return Ok(false);
        }
        let changed = self.conn.execute(
            "UPDATE events SET processed_mask = (processed_mask | ?) WHERE event_id = ?",
            params![mask, event_id],
        )?;
        Ok(changed > 0)
    }

    pub fn existing_event_ids(&self, event_ids: &[String]) -> Result<HashSet<String>> {
        if event_ids.is_empty() || !self.table_exists("events")? {
            return Ok(HashSet::new());
        }
        let mut found = HashSet::new();
        for batch in event_ids.chunks(250) {
            let placeholders = vec!["?"; batch.len()].join(", ");
            let sql = format!("SELECT event_id FROM events WHERE event_id IN ({placeholders})");
            let mut stmt = self.conn.prepare(&sql)?;
            let refs = batch.iter().map(|s| s as &dyn ToSql);
            let rows = stmt.query_map(params_from_iter(refs), |r| r.get::<_, String>(0))?;
            for r in rows {
                found.insert(r?);
            }
        }
        Ok(found)
    }

    pub fn upsert_doc_chunks(&self, chunks: &[DocChunkRow]) -> Result<usize> {
        if chunks.is_empty() {
            return Ok(0);
        }
        let tx = WriteTx::begin(&self.conn)?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT OR REPLACE INTO doc_chunks (
                    chunk_id, cik, ticker, doc_type, source_id, period_end,
                    filing_date, section, title, content, content_sha256,
                    source_url, embedding_model, embedding_dim, embedding_json
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for chunk in chunks {
                let dim_i32 = i32::try_from(chunk.embedding_dim).unwrap_or(i32::MAX);
                stmt.execute(params![
                    chunk.chunk_id,
                    chunk.cik,
                    chunk.ticker,
                    chunk.doc_type,
                    chunk.source_id,
                    chunk.period_end,
                    chunk.filing_date,
                    chunk.section,
                    chunk.title,
                    chunk.content,
                    chunk.content_sha256,
                    chunk.source_url,
                    chunk.embedding_model,
                    dim_i32,
                    chunk.embedding_json,
                ])?;
            }
        }
        tx.commit()?;
        Ok(chunks.len())
    }

    pub fn existing_chunk_hashes(&self, cik: &str) -> Result<HashMap<String, String>> {
        if !self.table_exists("doc_chunks")? {
            return Ok(HashMap::new());
        }
        let mut out = HashMap::new();
        let trimmed = cik.trim();
        if trimmed.is_empty() {
            let mut stmt = self
                .conn
                .prepare("SELECT chunk_id, content_sha256 FROM doc_chunks")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                let (id, sha) = r?;
                out.insert(id, sha);
            }
        } else {
            let mut stmt = self
                .conn
                .prepare("SELECT chunk_id, content_sha256 FROM doc_chunks WHERE cik = ?")?;
            let rows = stmt.query_map([trimmed], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (id, sha) = r?;
                out.insert(id, sha);
            }
        }
        Ok(out)
    }

    #[allow(dead_code)]
    pub fn doc_chunk_count(&self) -> Result<usize> {
        if !self.table_exists("doc_chunks")? {
            return Ok(0);
        }
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM doc_chunks", [], |r| r.get(0))?;
        Ok(usize::try_from(count.max(0)).unwrap_or(0))
    }

    #[allow(dead_code)]
    pub fn indexed_chunk_companies_count(&self) -> Result<usize> {
        if !self.table_exists("doc_chunks")? {
            return Ok(0);
        }
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT cik) FROM doc_chunks", [], |r| {
                    r.get(0)
                })?;
        Ok(usize::try_from(count.max(0)).unwrap_or(0))
    }

    /// Upserts inbox signals while preserving user triage status on non-`unread` existing signals.
    pub fn upsert_signals(&self, signals: &[SignalInboxRow]) -> Result<usize> {
        if signals.is_empty() {
            return Ok(0);
        }
        let tx = WriteTx::begin(&self.conn)?;
        let existing_states = load_existing_signal_states(&self.conn, signals)?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT OR REPLACE INTO signal_inbox (
                    signal_id, kind, score, primary_ticker, tickers_json,
                    title, summary, evidence_json, status, event_id,
                    created_at_ms, updated_at_ms
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for sig in signals {
                let (effective_status, effective_created_ms) =
                    if let Some((prev_status, prev_created_ms)) =
                        existing_states.get(&sig.signal_id)
                    {
                        let kept_status = if *prev_status != SignalStatus::Unread {
                            *prev_status
                        } else {
                            sig.status
                        };
                        (kept_status, *prev_created_ms)
                    } else {
                        (sig.status, sig.created_at_ms)
                    };
                let tickers_json = serde_json::to_string(&sig.tickers)?;
                let evidence_json = serde_json::to_string(&sig.evidence)?;
                stmt.execute(params![
                    sig.signal_id,
                    sig.kind.as_str(),
                    sig.score,
                    sig.primary_ticker,
                    tickers_json,
                    sig.title,
                    sig.summary,
                    evidence_json,
                    effective_status.as_str(),
                    sig.event_id,
                    effective_created_ms,
                    sig.updated_at_ms,
                ])?;
            }
        }
        tx.commit()?;
        Ok(signals.len())
    }

    pub fn list_inbox_signals(
        &self,
        status: Option<SignalStatus>,
        kind: Option<SignalKind>,
        limit: usize,
    ) -> Result<Vec<SignalInboxRow>> {
        if limit == 0 || !self.table_exists("signal_inbox")? {
            return Ok(Vec::new());
        }
        let mut clauses: Vec<&str> = Vec::new();
        let mut owned_params: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(st) = status {
            clauses.push("LOWER(status) = ?");
            owned_params.push(Box::new(st.as_str().to_owned()));
        }
        if let Some(k) = kind {
            clauses.push("LOWER(kind) = ?");
            owned_params.push(Box::new(k.as_str().to_owned()));
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        owned_params.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));

        let sql = format!(
            "SELECT signal_id, kind, score, primary_ticker, tickers_json,
                    title, summary, evidence_json, status, event_id,
                    created_at_ms, updated_at_ms
             FROM signal_inbox
             {where_sql}
             ORDER BY score DESC, updated_at_ms DESC, signal_id ASC
             LIMIT ?"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let refs = owned_params.iter().map(|b| b.as_ref());
        let rows = stmt.query_map(params_from_iter(refs), read_signal_inbox_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list signal_inbox")
    }

    pub fn triage_signal(
        &self,
        signal_id: &str,
        status: SignalStatus,
        updated_at_ms: u64,
    ) -> Result<bool> {
        if !self.table_exists("signal_inbox")? {
            return Ok(false);
        }
        let changed = self.conn.execute(
            "UPDATE signal_inbox SET status = ?, updated_at_ms = ? WHERE signal_id = ?",
            params![status.as_str(), updated_at_ms, signal_id],
        )?;
        Ok(changed > 0)
    }

    pub fn l1_summary(&self) -> Result<L1Summary> {
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let mut summary = L1Summary::default();

        if self.table_exists("filings")? {
            let n: i64 = self
                .conn
                .query_row("SELECT COUNT(*) FROM filings", [], |r| r.get(0))?;
            summary.total_filings = usize::try_from(n.max(0)).unwrap_or(0);
        }
        if self.table_exists("events")? {
            let (tot, unproc): (i64, i64) = self.conn.query_row(
                "SELECT COUNT(*), SUM(CASE WHEN processed_mask = 0 THEN 1 ELSE 0 END) FROM events",
                [],
                |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
            )?;
            summary.total_events = usize::try_from(tot.max(0)).unwrap_or(0);
            summary.unprocessed_events = usize::try_from(unproc.max(0)).unwrap_or(0);
        }
        if self.table_exists("doc_chunks")? {
            let (chunks, cos): (i64, i64) = self.conn.query_row(
                "SELECT COUNT(*), COUNT(DISTINCT cik) FROM doc_chunks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            summary.doc_chunks_count = usize::try_from(chunks.max(0)).unwrap_or(0);
            summary.indexed_companies_count = usize::try_from(cos.max(0)).unwrap_or(0);
        }
        if self.table_exists("signal_inbox")? {
            let (tot, unread, disc, ripple, strike_rf): (i64, i64, i64, i64, i64) =
                self.conn.query_row(
                    "SELECT
                        COUNT(*),
                        SUM(CASE WHEN LOWER(status) = 'unread' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN LOWER(status) = 'unread' AND LOWER(kind) = 'discovery' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN LOWER(status) = 'unread' AND LOWER(kind) = 'ripple' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN LOWER(status) = 'unread' AND LOWER(kind) IN ('strike', 'red_flag') THEN 1 ELSE 0 END)
                     FROM signal_inbox",
                    [],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                            r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        ))
                    },
                )?;
            summary.total_signals = usize::try_from(tot.max(0)).unwrap_or(0);
            summary.unread_signals = usize::try_from(unread.max(0)).unwrap_or(0);
            summary.discovery_unread = usize::try_from(disc.max(0)).unwrap_or(0);
            summary.ripple_unread = usize::try_from(ripple.max(0)).unwrap_or(0);
            summary.strike_or_red_flag_unread = usize::try_from(strike_rf.max(0)).unwrap_or(0);
        }

        summary.last_sec_bootstrap_ms = self
            .read_watermark(WM_L1_BOOTSTRAP)?
            .map(|w| w.synced_at_ms);
        summary.last_news_sync_ms = self.read_watermark(WM_L1_NEWS)?.map(|w| w.synced_at_ms);
        let catchup_ms = self.read_watermark(WM_L1_CATCHUP)?.map(|w| w.synced_at_ms);
        let events_ms = self
            .read_watermark(WM_L1_EVENTS_CATCHUP)?
            .map(|w| w.synced_at_ms);
        summary.last_catchup_ms = [
            summary.last_sec_bootstrap_ms,
            catchup_ms,
            summary.last_news_sync_ms,
            events_ms,
        ]
        .into_iter()
        .flatten()
        .max();

        Ok(summary)
    }

    pub fn list_candidate_pool_companies(&self, limit: usize) -> Result<Vec<UniverseRow>> {
        if limit == 0 || !self.table_exists("universe_snapshot")? {
            return Ok(Vec::new());
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let sql_limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE in_price_pool = TRUE OR in_watchlist = TRUE
             ORDER BY in_watchlist DESC, quality_score DESC, ticker ASC
             LIMIT ?"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([sql_limit], read_universe_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list candidate pool companies")
    }

    /// Hybrid SQL financial hard filter + semantic/lexical chunk similarity search.
    pub fn search_similar(
        &self,
        query_or_ticker: &str,
        as_ticker: bool,
        filter: &SimilarSearchFilter,
    ) -> Result<Vec<SimilarCompanyHit>> {
        let trimmed = query_or_ticker.trim();
        if trimmed.is_empty()
            || filter.limit == 0
            || !self.table_exists("universe_snapshot")?
            || !self.table_exists("doc_chunks")?
        {
            return Ok(Vec::new());
        }

        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let resolved_target = if as_ticker {
            let row = self
                .load_universe_row_by_ticker(trimmed)?
                .with_context(|| format!("ticker '{trimmed}' not found in universe_snapshot"))?;
            Some(row)
        } else if !trimmed.contains(char::is_whitespace) && trimmed.len() <= 10 {
            self.load_universe_row_by_ticker(trimmed)?
        } else {
            None
        };

        let (exclude_cik, query_text, query_vec) = if let Some(target) = resolved_target {
            let (q_text, q_vec) = self.build_ticker_query_profile(&target)?;
            (Some(target.cik), q_text, q_vec)
        } else {
            let dominant_model = self.dominant_chunk_embedding_model()?;
            let q_vec = embed_query_matching_model(trimmed, dominant_model.as_deref())?;
            (None, trimmed.to_owned(), q_vec)
        };

        self.score_candidate_companies_hybrid(
            exclude_cik.as_deref(),
            &query_text,
            &query_vec,
            Some(filter),
            filter.min_score,
            filter.limit,
        )
    }

    /// Finds candidate companies whose indexed chunks resonate with a source event or text.
    pub fn find_ripple_targets(
        &self,
        source_ticker: &str,
        source_text: &str,
        source_embedding: &[f32],
        min_score: f32,
        limit: usize,
    ) -> Result<Vec<SimilarCompanyHit>> {
        if limit == 0
            || source_text.trim().is_empty()
            || !self.table_exists("universe_snapshot")?
            || !self.table_exists("doc_chunks")?
        {
            return Ok(Vec::new());
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let exclude_cik = if !source_ticker.trim().is_empty() {
            self.load_universe_row_by_ticker(source_ticker.trim())?
                .map(|r| r.cik)
        } else {
            None
        };
        let fallback_vec;
        let query_vec = if source_embedding.is_empty() {
            fallback_vec = local_embed(source_text);
            fallback_vec.as_slice()
        } else {
            source_embedding
        };

        let mut hits = self.score_candidate_companies_hybrid(
            exclude_cik.as_deref(),
            source_text,
            query_vec,
            None,
            min_score,
            limit.saturating_add(4),
        )?;
        let upper_src = source_ticker.trim().to_ascii_uppercase();
        if !upper_src.is_empty() {
            hits.retain(|h| h.company.ticker.to_ascii_uppercase() != upper_src);
        }
        hits.truncate(limit);
        Ok(hits)
    }

    fn load_universe_row_by_ticker(&self, ticker: &str) -> Result<Option<UniverseRow>> {
        let sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE UPPER(ticker) = UPPER(?)
             ORDER BY in_watchlist DESC, quality_score DESC
             LIMIT 1"
        );
        self.conn
            .query_row(&sql, [ticker.trim()], read_universe_row)
            .optional()
            .context("failed to look up ticker in universe_snapshot")
    }

    fn dominant_chunk_embedding_model(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT embedding_model
                 FROM doc_chunks
                 GROUP BY embedding_model
                 ORDER BY COUNT(*) DESC
                 LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .context("failed to inspect doc_chunks embedding_model")
    }

    fn build_ticker_query_profile(&self, target: &UniverseRow) -> Result<(String, Vec<f32>)> {
        let mut stmt = self.conn.prepare(
            "SELECT section, content, embedding_model, embedding_json
             FROM doc_chunks
             WHERE cik = ?
             ORDER BY
                CASE section
                    WHEN 'item_1' THEN 0
                    WHEN 'company_profile' THEN 1
                    WHEN 'item_7' THEN 2
                    WHEN 'item_1a' THEN 3
                    WHEN 'item_8k' THEN 4
                    ELSE 5
                END ASC,
                chunk_id ASC
             LIMIT 16",
        )?;
        let rows = stmt.query_map([target.cik.as_str()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut text_len = 0usize;
        let mut centroid: Vec<f32> = Vec::new();
        let mut target_model: Option<String> = None;

        for row in rows {
            let (section, content, model, emb_json) = row?;
            if target_model.is_none() {
                target_model = Some(model.clone());
            }
            if target_model.as_deref() == Some(model.as_str())
                && let Some(vec) = decode_embedding_json(&emb_json)
            {
                let weight = section_priority_weight(&section);
                if centroid.is_empty() {
                    centroid = vec![0.0f32; vec.len()];
                }
                if centroid.len() == vec.len() {
                    for (acc, val) in centroid.iter_mut().zip(vec.iter()) {
                        *acc += weight * val;
                    }
                }
            }
            if text_len < 4_200 && !content.trim().is_empty() {
                text_len += content.len();
                text_parts.push(content);
            }
        }

        if !centroid.is_empty() && !text_parts.is_empty() {
            l2_normalize(&mut centroid);
            return Ok((text_parts.join("\n\n"), centroid));
        }

        let synth = format!(
            "{} {} {} {}",
            target.ticker,
            target.name,
            target.sic_description.as_deref().unwrap_or(""),
            target.exchange
        );
        let dominant_model = self.dominant_chunk_embedding_model()?;
        let vec = embed_query_matching_model(&synth, dominant_model.as_deref())?;
        Ok((synth, vec))
    }

    fn score_candidate_companies_hybrid(
        &self,
        exclude_cik: Option<&str>,
        query_text: &str,
        query_vec: &[f32],
        filter: Option<&SimilarSearchFilter>,
        min_score: f32,
        limit: usize,
    ) -> Result<Vec<SimilarCompanyHit>> {
        let mut clauses: Vec<String> = vec![
            "EXISTS (SELECT 1 FROM doc_chunks dc WHERE dc.cik = universe_snapshot.cik)".to_owned(),
        ];
        let mut owned_params: Vec<Box<dyn ToSql>> = Vec::new();
        let eps = Decimal::new(1, 9);

        if let Some(ex_cik) = exclude_cik {
            clauses.push("universe_snapshot.cik != ?".to_owned());
            owned_params.push(Box::new(ex_cik.to_owned()));
        }

        if let Some(f) = filter {
            if f.exclude_financials {
                clauses.push("universe_snapshot.is_financial = FALSE".to_owned());
            }
            if f.positive_strict_oe_only {
                clauses.push(
                    "universe_snapshot.strict_owner_earnings IS NOT NULL AND universe_snapshot.strict_owner_earnings >= -0.000001"
                        .to_owned(),
                );
            }
            if f.in_price_pool_only {
                clauses.push("universe_snapshot.in_price_pool = TRUE".to_owned());
            }
            if let Some(min_roic) = f.min_roic_5y {
                clauses.push(
                    "universe_snapshot.roic_5y_avg_pct IS NOT NULL AND universe_snapshot.roic_5y_avg_pct >= ?"
                        .to_owned(),
                );
                owned_params.push(Box::new(round_for_sql(min_roic - eps, 10)));
            }
            if let Some(max_sbc) = f.max_sbc_to_ocf {
                clauses.push(
                    "universe_snapshot.sbc_to_ocf_pct IS NOT NULL AND universe_snapshot.sbc_to_ocf_pct <= ?"
                        .to_owned(),
                );
                owned_params.push(Box::new(round_for_sql(max_sbc + eps, 10)));
            }
        }

        let where_sql = clauses.join(" AND ");
        let companies_sql =
            format!("SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot WHERE {where_sql}");
        let mut companies_by_cik: HashMap<String, UniverseRow> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(&companies_sql)?;
            let refs = owned_params.iter().map(|b| b.as_ref());
            let rows = stmt.query_map(params_from_iter(refs), read_universe_row)?;
            for row in rows {
                let u = row?;
                if filter.is_none_or(|f| row_matches_similar_filter(&u, f)) {
                    companies_by_cik.insert(u.cik.clone(), u);
                }
            }
        }

        if companies_by_cik.is_empty() {
            return Ok(Vec::new());
        }

        let prepared_bm25 = PreparedBm25Query::new(query_text);
        let chunks_sql = format!(
            "SELECT dc.chunk_id, dc.cik, dc.ticker, dc.doc_type, dc.filing_date,
                    dc.section, dc.title, dc.content, dc.source_url,
                    dc.embedding_dim, dc.embedding_json
             FROM doc_chunks dc
             INNER JOIN universe_snapshot ON universe_snapshot.cik = dc.cik
             WHERE {where_sql}"
        );

        struct ScoredChunk {
            hybrid: f32,
            semantic: f32,
            lexical: f32,
            evidence: SignalEvidence,
        }

        let mut per_cik: HashMap<String, Vec<ScoredChunk>> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(&chunks_sql)?;
            let refs = owned_params.iter().map(|b| b.as_ref());
            let rows = stmt.query_map(params_from_iter(refs), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i32>(9)?,
                    r.get::<_, String>(10)?,
                ))
            })?;

            for row in rows {
                let (
                    chunk_id,
                    cik,
                    ticker,
                    doc_type,
                    filing_date,
                    section,
                    title,
                    content,
                    source_url,
                    dim_i32,
                    emb_json,
                ) = row?;
                if !companies_by_cik.contains_key(&cik) {
                    continue;
                }
                let semantic = if usize::try_from(dim_i32).ok() == Some(query_vec.len())
                    && !query_vec.is_empty()
                {
                    decode_embedding_json(&emb_json)
                        .filter(|v| v.len() == query_vec.len())
                        .map(|v| cosine_similarity(query_vec, &v).max(0.0))
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let lexical = prepared_bm25.score(&content, 180.0);
                let score = hybrid_score(semantic, lexical);
                let quote = extract_verbatim_snippet(&content, query_text, 280);
                per_cik.entry(cik).or_default().push(ScoredChunk {
                    hybrid: score,
                    semantic,
                    lexical,
                    evidence: SignalEvidence {
                        chunk_id: Some(chunk_id),
                        ticker,
                        doc_type,
                        section,
                        filing_date,
                        title,
                        quote,
                        source_url,
                        similarity: score,
                    },
                });
            }
        }

        let mut hits: Vec<SimilarCompanyHit> = Vec::with_capacity(per_cik.len());
        for (cik, mut scored_chunks) in per_cik {
            let Some(company) = companies_by_cik.remove(&cik) else {
                continue;
            };
            scored_chunks.sort_by(|a, b| {
                b.hybrid
                    .partial_cmp(&a.hybrid)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let matched_chunk_count = scored_chunks.iter().filter(|c| c.hybrid > 0.05).count();
            let top_k = scored_chunks.len().min(3);
            if top_k == 0 {
                continue;
            }
            let max_score = scored_chunks[0].hybrid;
            let top3_avg =
                scored_chunks[..top_k].iter().map(|c| c.hybrid).sum::<f32>() / (top_k as f32);
            let agg_score = (0.70 * max_score + 0.30 * top3_avg).clamp(0.0, 1.0);
            if agg_score < min_score {
                continue;
            }
            let top_sem = scored_chunks[0].semantic;
            let top_lex = scored_chunks[0].lexical;
            let evidence: Vec<SignalEvidence> = scored_chunks
                .into_iter()
                .take(3)
                .map(|c| c.evidence)
                .collect();
            hits.push(SimilarCompanyHit {
                company,
                score: agg_score,
                semantic_score: top_sem,
                lexical_score: top_lex,
                matched_chunk_count,
                evidence,
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.company.quality_score.cmp(&a.company.quality_score))
                .then_with(|| a.company.ticker.cmp(&b.company.ticker))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

fn section_priority_weight(section: &str) -> f32 {
    match section {
        "item_1" | "company_profile" => 1.5,
        "item_7" => 1.25,
        "item_1a" => 1.0,
        "item_8k" => 0.9,
        _ => 0.75,
    }
}

fn embed_query_matching_model(query: &str, dominant_model: Option<&str>) -> Result<Vec<f32>> {
    let env_embedder = Embedder::from_env();
    if let Some(model) = dominant_model
        && model == env_embedder.model_name()
    {
        return Ok(env_embedder.embed(query)?.vector);
    }
    if dominant_model.is_none() && env_embedder.is_remote() {
        return Ok(env_embedder.embed(query)?.vector);
    }
    Ok(latent::local_embed(query))
}

fn row_matches_similar_filter(row: &UniverseRow, filter: &SimilarSearchFilter) -> bool {
    if filter.exclude_financials && row.is_financial {
        return false;
    }
    if filter.in_price_pool_only && !row.in_price_pool {
        return false;
    }
    if filter.positive_strict_oe_only
        && !row
            .strict_owner_earnings
            .is_some_and(|oe| oe > Decimal::ZERO)
    {
        return false;
    }
    if let Some(min_roic) = filter.min_roic_5y
        && !row.roic_5y_avg_pct.is_some_and(|v| v >= min_roic)
    {
        return false;
    }
    if let Some(max_sbc) = filter.max_sbc_to_ocf
        && !row.sbc_to_ocf_pct.is_some_and(|v| v <= max_sbc)
    {
        return false;
    }
    true
}

fn load_existing_event_masks(
    conn: &Connection,
    rows: &[LakeEventRow],
) -> Result<HashMap<String, u64>> {
    let mut out = HashMap::new();
    for batch in rows.chunks(250) {
        let placeholders = vec!["?"; batch.len()].join(", ");
        let sql = format!(
            "SELECT event_id, processed_mask FROM events WHERE event_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let refs = batch.iter().map(|r| &r.event_id as &dyn ToSql);
        let existing = stmt.query_map(params_from_iter(refs), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        for item in existing {
            let (id, mask) = item?;
            out.insert(id, mask);
        }
    }
    Ok(out)
}

fn load_existing_signal_states(
    conn: &Connection,
    signals: &[SignalInboxRow],
) -> Result<HashMap<String, (SignalStatus, u64)>> {
    let mut out = HashMap::new();
    for batch in signals.chunks(250) {
        let placeholders = vec!["?"; batch.len()].join(", ");
        let sql = format!(
            "SELECT signal_id, status, created_at_ms FROM signal_inbox WHERE signal_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let refs = batch.iter().map(|s| &s.signal_id as &dyn ToSql);
        let existing = stmt.query_map(params_from_iter(refs), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
            ))
        })?;
        for item in existing {
            let (id, raw_status, created_at_ms) = item?;
            let status = SignalStatus::from_str(&raw_status).unwrap_or(SignalStatus::Unread);
            out.insert(id, (status, created_at_ms));
        }
    }
    Ok(out)
}

fn read_signal_inbox_row(row: &duckdb::Row<'_>) -> duckdb::Result<SignalInboxRow> {
    let signal_id: String = row.get(0)?;
    let kind_raw: String = row.get(1)?;
    let score: f64 = row.get(2)?;
    let primary_ticker: String = row.get(3)?;
    let tickers_json: String = row.get(4)?;
    let title: String = row.get(5)?;
    let summary: String = row.get(6)?;
    let evidence_json: String = row.get(7)?;
    let status_raw: String = row.get(8)?;
    let event_id: Option<String> = row.get(9)?;
    let created_at_ms: u64 = row.get(10)?;
    let updated_at_ms: u64 = row.get(11)?;

    let kind = SignalKind::from_str(&kind_raw).unwrap_or(SignalKind::Discovery);
    let status = SignalStatus::from_str(&status_raw).unwrap_or(SignalStatus::Unread);
    let tickers: Vec<String> = serde_json::from_str(&tickers_json).unwrap_or_default();
    let evidence: Vec<SignalEvidence> = serde_json::from_str(&evidence_json).unwrap_or_default();

    Ok(SignalInboxRow {
        signal_id,
        kind,
        score,
        primary_ticker,
        tickers,
        title,
        summary,
        evidence,
        status,
        event_id,
        created_at_ms,
        updated_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lake::{LakeCompanyRow, LakePriceDailyRow, SyncWatermark},
        latent::{LOCAL_EMBEDDING_DIM, LOCAL_EMBEDDING_MODEL, encode_embedding_json, sha256_hex},
        model::Bucket,
    };
    use std::fs;

    fn temp_paths(tag: &str) -> WorkspacePaths {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("m3_l1_test_{tag}_{}_{}", std::process::id(), nanos));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workbench")).unwrap();
        WorkspacePaths::from_root(root)
    }

    struct SampleSpec<'a> {
        ticker: &'a str,
        cik: &'a str,
        name: &'a str,
        roic_5y: i64,
        sbc_ocf: i64,
        quality: i64,
        in_pool: bool,
        in_watchlist: bool,
        is_financial: bool,
    }

    fn sample_universe_row(spec: SampleSpec<'_>) -> UniverseRow {
        let SampleSpec {
            ticker,
            cik,
            name,
            roic_5y,
            sbc_ocf,
            quality,
            in_pool,
            in_watchlist,
            is_financial,
        } = spec;
        UniverseRow {
            ticker: ticker.to_owned(),
            cik: cik.to_owned(),
            name: name.to_owned(),
            exchange: "NASDAQ".to_owned(),
            sic: Some(if is_financial { 6021 } else { 7372 }),
            sic_description: Some(if is_financial {
                "Commercial Banks".to_owned()
            } else {
                "Prepackaged Software".to_owned()
            }),
            is_financial,
            latest_filing_end: Some("2026-01-31".to_owned()),
            years_of_data: 8,
            quality_score: Decimal::from(quality),
            in_price_pool: in_pool,
            in_watchlist,
            bucket_opt: in_watchlist.then_some(Bucket::Watch),
            latest_price: Some(Decimal::from(120)),
            strike_price: Some(Decimal::from(150)),
            distance_to_strike_pct: Some(Decimal::from(-20)),
            strict_fcf_yield_pct: Some(Decimal::from(6)),
            roic_5y_avg_pct: Some(Decimal::from(roic_5y)),
            roic_5y_stddev_pct: Some(Decimal::from(2)),
            gross_margin_5y_avg_pct: Some(Decimal::from(78)),
            gross_margin_5y_stddev_pct: Some(Decimal::from(1)),
            share_change_3y_pct: Some(Decimal::from(-3)),
            sbc_to_ocf_pct: Some(Decimal::from(sbc_ocf)),
            net_debt_to_oe: Some(Decimal::from(0)),
            strict_owner_earnings: Some(Decimal::from(1_000_000_000i64)),
            warnings_count: 0,
            warnings_json: "[]".to_owned(),
            raw_sha256: Some("abc".to_owned()),
        }
    }

    fn seed_universe(db: &mut LakeDb, rows: Vec<UniverseRow>) {
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
                price: Decimal::from(120),
                previous_close: Some(Decimal::from(119)),
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

    fn make_chunk(
        chunk_id: &str,
        cik: &str,
        ticker: &str,
        doc_type: &str,
        section: &str,
        content: &str,
    ) -> DocChunkRow {
        let vec = local_embed(content);
        DocChunkRow {
            chunk_id: chunk_id.to_owned(),
            cik: cik.to_owned(),
            ticker: ticker.to_owned(),
            doc_type: doc_type.to_owned(),
            source_id: format!("src-{chunk_id}"),
            period_end: Some("2026-01-31".to_owned()),
            filing_date: Some("2026-03-15".to_owned()),
            section: section.to_owned(),
            title: format!("{ticker} {section}"),
            content: content.to_owned(),
            content_sha256: sha256_hex(content.as_bytes()),
            source_url: format!("https://www.sec.gov/Archives/edgar/data/{cik}/{chunk_id}.htm"),
            embedding_model: LOCAL_EMBEDDING_MODEL.to_owned(),
            embedding_dim: LOCAL_EMBEDDING_DIM,
            embedding_json: encode_embedding_json(&vec),
        }
    }

    #[test]
    fn migration_filings_parquet_events_and_chunk_hashes_are_idempotent() {
        let paths = temp_paths("migration_idempotent");
        let db = LakeDb::open(&paths).unwrap();

        assert!(db.table_exists("filings").unwrap());
        assert!(db.table_exists("events").unwrap());
        assert!(db.table_exists("doc_chunks").unwrap());
        assert!(db.table_exists("signal_inbox").unwrap());

        let filing = LakeFilingRow {
            accession_no: "0000769397-26-000010".to_owned(),
            cik: "0000769397".to_owned(),
            ticker: "ADSK".to_owned(),
            form: "10-K".to_owned(),
            filing_date: "2026-03-15".to_owned(),
            report_date: Some("2026-01-31".to_owned()),
            primary_document: Some("adsk-20260131.htm".to_owned()),
            primary_doc_description: Some("10-K".to_owned()),
            items_json: "[\"1.01\"]".to_owned(),
            filing_url: "https://www.sec.gov/Archives/edgar/data/769397/000076939726000010/adsk-20260131.htm".to_owned(),
            ingested_at_ms: 1_700_000_000_000,
        };
        assert_eq!(filing.parsed_items(), vec!["1.01".to_owned()]);
        assert!(!db.has_filing(&filing.accession_no).unwrap());
        db.upsert_filings(std::slice::from_ref(&filing)).unwrap();
        db.upsert_filings(std::slice::from_ref(&filing)).unwrap();
        assert!(db.has_filing(&filing.accession_no).unwrap());
        let listed_filings = db.list_filings_for_cik("0000769397", 10).unwrap();
        assert_eq!(listed_filings.len(), 1);

        let pq_path = db.export_filings_parquet(&paths).unwrap();
        assert!(pq_path.exists());
        let pq_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM v_filings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pq_count, 1);

        let ev = LakeEventRow {
            event_id: "sec:0000769397-26-000010".to_owned(),
            source: "sec_submissions".to_owned(),
            cik: Some("0000769397".to_owned()),
            ticker: Some("ADSK".to_owned()),
            event_type: "10-K".to_owned(),
            occurred_at_ms: 1_700_000_000_000,
            filing_date: Some("2026-03-15".to_owned()),
            title: "ADSK filed 10-K".to_owned(),
            summary: "Annual report".to_owned(),
            url: filing.filing_url.clone(),
            metadata_json: "{}".to_owned(),
            processed_mask: 0,
        };
        db.upsert_events(std::slice::from_ref(&ev)).unwrap();
        assert!(db.mark_event_processed(&ev.event_id, 1).unwrap());
        // Re-upserting with processed_mask=0 must preserve existing processed_mask=1.
        db.upsert_events(std::slice::from_ref(&ev)).unwrap();
        let events = db.list_events(Some("ADSK"), None, false, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].processed_mask, 1);
        assert!(
            db.list_events(Some("ADSK"), None, true, 10)
                .unwrap()
                .is_empty()
        );
        let existing_ids = db
            .existing_event_ids(&[ev.event_id.clone(), "missing".to_owned()])
            .unwrap();
        assert!(existing_ids.contains(&ev.event_id));
        assert!(!existing_ids.contains("missing"));

        let chunk = make_chunk(
            "c1",
            "0000769397",
            "ADSK",
            "10k",
            "item_1",
            "Autodesk provides 3D engineering CAD subscription software.",
        );
        assert!(chunk.parsed_embedding().is_some());
        db.upsert_doc_chunks(std::slice::from_ref(&chunk)).unwrap();
        assert_eq!(db.doc_chunk_count().unwrap(), 1);
        assert_eq!(db.indexed_chunk_companies_count().unwrap(), 1);
        let hashes = db.existing_chunk_hashes("0000769397").unwrap();
        assert_eq!(hashes.get("c1"), Some(&chunk.content_sha256));

        let _ = fs::remove_dir_all(paths.repo_root);
    }

    #[test]
    fn inbox_upsert_preserves_user_triage_state_and_timestamps() {
        let paths = temp_paths("inbox_triage");
        let db = LakeDb::open(&paths).unwrap();

        let s_watch = SignalInboxRow {
            signal_id: "disc:PTC".to_owned(),
            kind: SignalKind::Discovery,
            score: 82.0,
            primary_ticker: "PTC".to_owned(),
            tickers: vec!["PTC".to_owned()],
            title: "PTC quality candidate".to_owned(),
            summary: "Initial summary".to_owned(),
            evidence: vec![],
            status: SignalStatus::Unread,
            event_id: None,
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
        };
        let s_unread = SignalInboxRow {
            signal_id: "disc:ANSS".to_owned(),
            kind: SignalKind::Discovery,
            score: 79.0,
            primary_ticker: "ANSS".to_owned(),
            tickers: vec!["ANSS".to_owned()],
            title: "ANSS quality candidate".to_owned(),
            summary: "Initial ANSS".to_owned(),
            evidence: vec![],
            status: SignalStatus::Unread,
            event_id: None,
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
        };
        db.upsert_signals(&[s_watch.clone(), s_unread.clone()])
            .unwrap();
        assert!(
            db.triage_signal("disc:PTC", SignalStatus::Watch, 2_000)
                .unwrap()
        );

        let mut s_watch_updated = s_watch;
        s_watch_updated.summary = "Refreshed summary".to_owned();
        s_watch_updated.score = 90.0;
        s_watch_updated.created_at_ms = 3_000;
        s_watch_updated.updated_at_ms = 3_000;
        let mut s_unread_updated = s_unread;
        s_unread_updated.summary = "Refreshed ANSS".to_owned();
        s_unread_updated.updated_at_ms = 3_000;

        db.upsert_signals(&[s_watch_updated, s_unread_updated])
            .unwrap();

        let watch_rows = db
            .list_inbox_signals(Some(SignalStatus::Watch), None, 10)
            .unwrap();
        assert_eq!(watch_rows.len(), 1);
        assert_eq!(watch_rows[0].signal_id, "disc:PTC");
        assert_eq!(watch_rows[0].status, SignalStatus::Watch);
        assert_eq!(watch_rows[0].summary, "Refreshed summary");
        assert_eq!(watch_rows[0].created_at_ms, 1_000);

        let unread_rows = db
            .list_inbox_signals(Some(SignalStatus::Unread), None, 10)
            .unwrap();
        assert_eq!(unread_rows.len(), 1);
        assert_eq!(unread_rows[0].signal_id, "disc:ANSS");
        assert_eq!(unread_rows[0].summary, "Refreshed ANSS");

        db.write_watermark(&SyncWatermark {
            source_key: WM_L1_BOOTSTRAP.to_owned(),
            cursor: Some("done".to_owned()),
            etag: None,
            last_modified: None,
            synced_at_ms: 5_000,
        })
        .unwrap();
        let summary = db.l1_summary().unwrap();
        assert_eq!(summary.total_signals, 2);
        assert_eq!(summary.unread_signals, 1);
        assert_eq!(summary.discovery_unread, 1);
        assert_eq!(summary.last_sec_bootstrap_ms, Some(5_000));
        assert_eq!(summary.last_catchup_ms, Some(5_000));

        let _ = fs::remove_dir_all(paths.repo_root);
    }

    #[test]
    fn search_similar_and_ripple_combine_sql_filters_with_hybrid_citations() {
        let paths = temp_paths("hybrid_search");
        let mut db = LakeDb::open(&paths).unwrap();

        let adsk = sample_universe_row(SampleSpec {
            ticker: "ADSK",
            cik: "0000769397",
            name: "Autodesk Inc",
            roic_5y: 26,
            sbc_ocf: 12,
            quality: 92,
            in_pool: true,
            in_watchlist: true,
            is_financial: false,
        });
        let ptc = sample_universe_row(SampleSpec {
            ticker: "PTC",
            cik: "0000857005",
            name: "PTC Inc",
            roic_5y: 21,
            sbc_ocf: 10,
            quality: 88,
            in_pool: true,
            in_watchlist: false,
            is_financial: false,
        });
        let low_roic_sw = sample_universe_row(SampleSpec {
            ticker: "LOWR",
            cik: "0000999001",
            name: "Low ROIC CAD Co",
            roic_5y: 8,
            sbc_ocf: 10,
            quality: 60,
            in_pool: true,
            in_watchlist: false,
            is_financial: false,
        });
        let bank = sample_universe_row(SampleSpec {
            ticker: "JPM",
            cik: "0000019617",
            name: "JPMorgan Chase",
            roic_5y: 15,
            sbc_ocf: 5,
            quality: 80,
            in_pool: true,
            in_watchlist: false,
            is_financial: true,
        });
        let oil = sample_universe_row(SampleSpec {
            ticker: "XOM",
            cik: "0000034088",
            name: "Exxon Mobil",
            roic_5y: 18,
            sbc_ocf: 2,
            quality: 85,
            in_pool: true,
            in_watchlist: false,
            is_financial: false,
        });
        seed_universe(&mut db, vec![adsk, ptc, low_roic_sw, bank, oil]);

        let pool = db.list_candidate_pool_companies(10).unwrap();
        assert_eq!(pool[0].ticker, "ADSK");
        assert_eq!(pool[1].ticker, "PTC");

        let chunks = vec![
            make_chunk(
                "adsk-1",
                "0000769397",
                "ADSK",
                "10k",
                "item_1",
                "Autodesk builds enterprise engineering CAD design and manufacturing subscription software for industrial customers.",
            ),
            make_chunk(
                "ptc-1",
                "0000857005",
                "PTC",
                "10k",
                "item_1",
                "PTC provides cloud CAD, PLM product lifecycle management, and engineering design subscription software for manufacturing customers.",
            ),
            make_chunk(
                "lowr-1",
                "0000999001",
                "LOWR",
                "10k",
                "item_1",
                "Enterprise engineering CAD design and manufacturing subscription software.",
            ),
            make_chunk(
                "jpm-1",
                "0000019617",
                "JPM",
                "10k",
                "item_1",
                "Banking software treasury services commercial deposits and lending.",
            ),
            make_chunk(
                "xom-1",
                "0000034088",
                "XOM",
                "10k",
                "item_1",
                "Upstream oil and natural gas exploration refining and petrochemicals.",
            ),
        ];
        db.upsert_doc_chunks(&chunks).unwrap();

        let filter = SimilarSearchFilter {
            min_roic_5y: Some(Decimal::from(15)),
            max_sbc_to_ocf: Some(Decimal::from(20)),
            exclude_financials: true,
            positive_strict_oe_only: true,
            in_price_pool_only: true,
            min_score: 0.10,
            limit: 5,
        };

        let ticker_hits = db.search_similar("ADSK", true, &filter).unwrap();
        assert!(!ticker_hits.is_empty());
        assert_eq!(ticker_hits[0].company.ticker, "PTC");
        assert!(ticker_hits.iter().all(|h| h.company.ticker != "ADSK"));
        assert!(ticker_hits.iter().all(|h| h.company.ticker != "LOWR"));
        assert!(ticker_hits.iter().all(|h| h.company.ticker != "JPM"));
        assert!(!ticker_hits[0].evidence.is_empty());
        assert!(
            ticker_hits[0].evidence[0]
                .quote
                .contains("engineering design")
        );

        let text_hits = db
            .search_similar(
                "enterprise engineering CAD subscription software",
                false,
                &filter,
            )
            .unwrap();
        assert!(text_hits.len() >= 2);
        assert!(text_hits[0].company.ticker == "ADSK" || text_hits[0].company.ticker == "PTC");

        let ripple_text =
            "Major manufacturing customers accelerate cloud CAD and PLM subscription renewals.";
        let ripple_vec = local_embed(ripple_text);
        let ripples = db
            .find_ripple_targets("ADSK", ripple_text, &ripple_vec, 0.15, 5)
            .unwrap();
        assert!(!ripples.is_empty());
        assert_eq!(ripples[0].company.ticker, "PTC");
        assert!(ripples.iter().all(|r| r.company.ticker != "ADSK"));

        // Verify legacy L0-only DB compatibility: drop L1 tables, read-only queries return empty,
        // and LakeDb::open cleanly migrates L1 tables without touching L0 universe_snapshot.
        db.conn
            .execute_batch(
                "DROP VIEW IF EXISTS v_filings;
                 DROP TABLE IF EXISTS filings;
                 DROP TABLE IF EXISTS events;
                 DROP TABLE IF EXISTS doc_chunks;
                 DROP TABLE IF EXISTS signal_inbox;",
            )
            .unwrap();
        drop(db);

        let ro_db = LakeDb::open_read_only_if_exists(&paths).unwrap().unwrap();
        assert_eq!(ro_db.doc_chunk_count().unwrap(), 0);
        assert!(ro_db.list_inbox_signals(None, None, 10).unwrap().is_empty());
        assert!(
            ro_db
                .search_similar("software", false, &filter)
                .unwrap()
                .is_empty()
        );
        drop(ro_db);

        let migrated_db = LakeDb::open(&paths).unwrap();
        assert!(migrated_db.table_exists("filings").unwrap());
        assert!(migrated_db.table_exists("signal_inbox").unwrap());
        assert_eq!(
            migrated_db.list_candidate_pool_companies(10).unwrap().len(),
            5
        );

        let _ = fs::remove_dir_all(paths.repo_root);
    }
}
