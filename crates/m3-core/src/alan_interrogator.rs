//! In-process Alan V2 interrogator for Project-M3.
//!
//! - Mounts read-only `/mnt/m3` exposing `watchlist.json`, `journal.json`,
//!   `computed/<TICKER>.json`, `dossiers/<TICKER>.json`, `industries/index.json`,
//!   `industries/<slug>.json`, `memos/<TICKER>.md`, `memos/<TICKER>-feynman.md`,
//!   `briefings/latest.json`, and `events/recent.json`.
//! - Registers deterministic M3 tools (`m3_status`, `m3_report`, `m3_dossier`,
//!   `m3_update_draft`, `m3_universe_query`, `m3_peer_percentiles`,
//!   `m3_semantic_neighbors`, `m3_events_for_ticker`, `m3_industry_matrix`,
//!   `m3_web_search`) backed directly by `store.rs`, `calculator.rs`,
//!   `industry.rs`, `lake.rs`, and `lake_l1.rs`.
//! - Enforces [`M3ToolPolicyHook`] unconditionally denying any tool call
//!   touching `human_judgment`, `computed/`, `raw/`, `data/munger/`, `m3.duckdb`,
//!   `.parquet`, `lake/`, `journal.json`, `watchlist.json`, `budget.json`,
//!   `themes.json`, `memos/`, `industries/`, `briefings/`, or `events/`.
//! - Emits live streamed prose, `alanworks.app`-style Turing Tape rows,
//!   clickable Acme evidence tokens (`[report:TICKER]`, `[xbrl:TICKER]`,
//!   `[dossier:TICKER]`), and deterministic [`InlineEvidenceCard`] summaries.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::str::FromStr;
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::Duration;

use alan_agent_engine::TrustPosture;
use alan_agent_engine::tools::{Tool, ToolCapability, ToolContext, ToolRegistry, ToolResult};
use alan_ap::{ErrorCode, Fid, FileKind, FileServer, Offset, OpenMode, Qid, Stat, VersionTable};
use alan_os_host::embed::{
    Access, AgentProcessConfig, EmbedEvent, EmbeddedAlan, EmbeddedConfig, HookDecision, LlmSource,
    RootAgentClient, SessionOptions, SystemPromptMode, ToolPolicyHook, UiInputStatus,
};
use alan_shell::Shell;
use anyhow::{Context, Result};
use duckdb::params;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, mpsc as tokio_mpsc};

use crate::i18n::UiLang;
use crate::industry::{self, IndustryProfile};
use crate::lake::{LakeDb, UniverseFilter, UniverseRow, UniverseSortBy};
use crate::lake_l1::SimilarSearchFilter;
use crate::model::{
    Bucket, Dossier, HumanJudgment, Latticework, MungerReport, PriceSnapshot, serde_decimal_opt,
};
use crate::store::{self, WorkspacePaths};

const M3_INTERROGATOR_SYSTEM_PROMPT: &str = "\
你是 Project-M3 的查理·芒格跨学科研究幕僚 Alan（费曼审讯台）。请严格遵循以下纪律：
1. 严禁自行心算、编造或篡改任何财务数字、估值倍数、ROIC 或击球价；需要数据时必须调用 `m3_status`、`m3_report`、`m3_dossier` 或只读湖仓工具（`m3_universe_query`、`m3_peer_percentiles`、`m3_semantic_neighbors`、`m3_events_for_ticker`），或读取只读 `/mnt/m3` 挂载树。
2. 允许通过 `m3_update_draft` 更新公司档案草稿区（`three_sentence_business`、`latticework`、`premortem`、`open_questions`），但严禁修改或代填 `human_judgment`、`computed/`、`raw/`、`data/munger/`、`m3.duckdb`、`.parquet`、`watchlist.json`、`journal.json`、`budget.json`、`themes.json` 与 `memos/`（系统会在工具策略层硬拦截）。
3. 回答中引用确定性数据与档案时，附上可点击证据标记 `[report:TICKER]`、`[xbrl:TICKER]`、`[dossier:TICKER]`。
4. 先核对财务与护城河事实，再提出锋利的 5 年逆向事前验尸问题与费曼追问；严禁直接给出买卖指令。";

/// One entry in the `alanworks.app`-style Turing Tape displayed in `2 Alan 审讯台`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuringTapeKind {
    Input,
    Tool,
    Out,
    Policy,
    Error,
}

impl TuringTapeKind {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Input => "in",
            Self::Tool => "tool",
            Self::Out => "out",
            Self::Policy => "policy",
            Self::Error => "err",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuringTapeEntry {
    pub seq: usize,
    pub call_id: Option<String>,
    pub kind: TuringTapeKind,
    pub summary: String,
    pub detail: Option<String>,
    pub status: String,
}

/// Clickable Acme evidence token resolved from Alan prose & tool executions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeEvidenceKind {
    Report,
    Xbrl,
    Dossier,
}

impl AcmeEvidenceKind {
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Xbrl => "xbrl",
            Self::Dossier => "dossier",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcmeEvidenceToken {
    pub id: String,
    pub kind: AcmeEvidenceKind,
    pub ticker: String,
    pub label: String,
    pub summary: String,
}

/// Deterministic inline evidence card resolved without LLM calls from local
/// computed reports, watchlist entries, dossiers, and DuckDB universe rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineEvidenceCard {
    pub ticker: String,
    pub name: String,
    pub bucket: Option<Bucket>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default, with = "serde_decimal_opt")]
    pub latest_price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strike_price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strike_gap_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_5y: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub fcf_conversion_or_yield: Option<Decimal>,
    pub warning_count: usize,
    pub source_kind: AcmeEvidenceKind,
    #[serde(default)]
    pub has_dossier: bool,
}

/// Unified interrogation scope across Company, Industry, Briefing, and Portfolio views.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlanScope {
    Company(String),
    Industry(String),
    Briefing(String),
    Portfolio,
}

fn sanitize_scope_slug(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return fallback.to_owned();
    }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    while out.contains("..") {
        out = out.replace("..", "_");
    }
    let cleaned = out.trim_matches(|c| c == '-' || c == '.');
    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned.to_owned()
    }
}

impl AlanScope {
    /// Returns a canonically normalized copy of this scope.
    pub fn normalized(&self) -> Self {
        match self {
            Self::Company(ticker) => Self::Company(ticker.trim().to_ascii_uppercase()),
            Self::Industry(slug) => Self::Industry(slug.trim().to_owned()),
            Self::Briefing(id) => {
                let trimmed = id.trim();
                Self::Briefing(if trimmed.is_empty() {
                    "latest".to_owned()
                } else {
                    trimmed.to_owned()
                })
            }
            Self::Portfolio => Self::Portfolio,
        }
    }

    /// Safe filename slug for session persistence and routing
    /// (e.g. `company-COST`, `industry-retail`, `briefing-latest`, `portfolio`).
    pub fn scope_key(&self) -> String {
        match self {
            Self::Company(ticker) => {
                let norm = ticker.trim().to_ascii_uppercase();
                format!("company-{}", sanitize_scope_slug(&norm, "unknown"))
            }
            Self::Industry(slug) => {
                format!("industry-{}", sanitize_scope_slug(slug, "unknown"))
            }
            Self::Briefing(id) => {
                let trimmed = id.trim();
                let key = if trimmed.is_empty() {
                    "latest"
                } else {
                    trimmed
                };
                format!("briefing-{}", sanitize_scope_slug(key, "latest"))
            }
            Self::Portfolio => "portfolio".to_owned(),
        }
    }

    /// Localized scope badge label shown in the Alan Canvas header.
    pub fn label_for(&self, lang: UiLang) -> String {
        match self {
            Self::Company(ticker) => {
                let upper = ticker.trim().to_ascii_uppercase();
                if upper.is_empty() {
                    lang.t("公司", "Company").to_owned()
                } else {
                    upper
                }
            }
            Self::Industry(slug) => {
                let trimmed = slug.trim();
                if trimmed.is_empty() {
                    lang.t("行业", "Industry").to_owned()
                } else {
                    trimmed.to_owned()
                }
            }
            Self::Briefing(id) => {
                let trimmed = id.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("latest") {
                    lang.t("简报", "Briefing").to_owned()
                } else {
                    format!("{} · {trimmed}", lang.t("简报", "Briefing"))
                }
            }
            Self::Portfolio => lang.t("全局能力圈", "Portfolio").to_owned(),
        }
    }

    /// Builds a grounded, bounded deterministic context preamble for this scope
    /// from local workspace files and read-only DuckDB tables.
    pub fn context_preamble(&self, paths: &WorkspacePaths, lang: UiLang) -> String {
        match self {
            Self::Company(ticker) => build_company_context_preamble(paths, ticker, lang),
            Self::Industry(slug) => build_industry_context_preamble(paths, slug, lang),
            Self::Briefing(id) => build_briefing_context_preamble(paths, id, lang),
            Self::Portfolio => build_portfolio_context_preamble(paths, lang),
        }
    }

    /// Composes a full interrogation turn combining [`AlanScope::context_preamble`]
    /// with the user's question.
    pub fn compose_interrogation_turn(
        &self,
        paths: &WorkspacePaths,
        lang: UiLang,
        question: &str,
    ) -> String {
        let preamble = self.context_preamble(paths, lang);
        format!("{preamble}\n\n{}", question.trim())
    }
}

/// Events streamed into the M3 UI and CLI consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterrogatorEvent {
    Ready,
    UserSubmitted {
        submission_id: String,
        prompt: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
        title: String,
    },
    ToolProgress {
        call_id: String,
        preview: String,
    },
    ToolFinished {
        call_id: String,
        name: String,
        status: String,
        preview: String,
    },
    TextDelta {
        submission_id: Option<String>,
        chunk: String,
        is_final: bool,
    },
    TurnDone {
        submission_id: String,
        completed: bool,
        error: Option<String>,
    },
    ResetDone,
    Error {
        message: String,
    },
}

#[derive(Clone)]
struct TreeSnapshot {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeMap<String, BTreeSet<String>>,
    path_ids: HashMap<String, u64>,
    versions: VersionTable,
}

const RECENT_EVENTS_MOUNT_CAP: usize = 40;

impl TreeSnapshot {
    fn from_paths(paths: &WorkspacePaths) -> Self {
        let mut files = BTreeMap::new();

        if let Ok(bytes) = std::fs::read(paths.watchlist_path()) {
            files.insert("watchlist.json".to_string(), bytes);
        }
        if let Ok(bytes) = std::fs::read(paths.journal_path()) {
            files.insert("journal.json".to_string(), bytes);
        }

        // Project deterministic computed reports without overwriting existing computed caches.
        if let Ok(reports) = store::load_or_compute_reports(paths) {
            for rep in reports {
                let name = format!(
                    "computed/{}.json",
                    crate::market::safe_symbol_filename(&rep.ticker)
                );
                if let Ok(bytes) = serde_json::to_vec_pretty(&rep) {
                    files.insert(name, bytes);
                }
            }
        }
        let computed_dir = paths.data_dir.join("computed");
        if let Ok(entries) = std::fs::read_dir(&computed_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json")
                    && let Some(fname) = p.file_name().and_then(|n| n.to_str())
                    && let Ok(bytes) = std::fs::read(&p)
                {
                    files.entry(format!("computed/{fname}")).or_insert(bytes);
                }
            }
        }

        let dossiers_dir = paths.dossiers_dir();
        if let Ok(entries) = std::fs::read_dir(&dossiers_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json")
                    && let Some(fname) = p.file_name().and_then(|n| n.to_str())
                    && let Ok(bytes) = std::fs::read(&p)
                {
                    files.insert(format!("dossiers/{fname}"), bytes);
                }
            }
        }

        // Read-only industries/*.json + generated industries/index.json.
        let industries_dir = paths.industries_dir();
        if let Ok(entries) = std::fs::read_dir(&industries_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json")
                    && let Some(fname) = p.file_name().and_then(|n| n.to_str())
                    && fname != "index.json"
                    && let Ok(bytes) = std::fs::read(&p)
                {
                    files.insert(format!("industries/{fname}"), bytes);
                }
            }
        }
        let industry_profiles = industry::list_industries(paths).unwrap_or_default();
        let industry_index_rows: Vec<Value> = industry_profiles
            .iter()
            .map(|p| {
                let safe = crate::market::safe_symbol_filename(&p.slug);
                if !files.contains_key(&format!("industries/{safe}.json"))
                    && let Ok(bytes) = serde_json::to_vec_pretty(p)
                {
                    files.insert(format!("industries/{safe}.json"), bytes);
                }
                json!({
                    "slug": p.slug,
                    "title": p.title,
                    "status": p.status,
                    "sic_codes": p.sic_codes,
                    "keywords": p.keywords,
                    "bound_tickers": p.bound_tickers,
                    "multiple_low": p.multiple_low.map(|d| d.normalize().to_string()),
                    "multiple_high": p.multiple_high.map(|d| d.normalize().to_string()),
                    "updated_at_ms": p.updated_at_ms,
                    "has_primer": paths.industry_primer_path(&p.slug).is_file(),
                })
            })
            .collect();
        if let Ok(bytes) = serde_json::to_vec_pretty(&json!({
            "count": industry_index_rows.len(),
            "industries": industry_index_rows,
        })) {
            files.insert("industries/index.json".to_string(), bytes);
        }

        // Read-only memos/<TICKER>.md and memos/<TICKER>-feynman.md.
        let memos_dir = paths.workbench_dir.join("memos");
        if let Ok(entries) = std::fs::read_dir(&memos_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("md")
                    && let Some(fname) = p.file_name().and_then(|n| n.to_str())
                    && let Ok(bytes) = std::fs::read(&p)
                {
                    files.insert(format!("memos/{fname}"), bytes);
                }
            }
        }

        // Read-only briefings/latest.json and events/recent.json from DuckDB with graceful fallback.
        let (briefing_payload, events_payload) = match LakeDb::open_read_only_if_exists(paths) {
            Ok(Some(db)) => {
                let briefing_json = match db.latest_circle_briefing(None) {
                    Ok(Some(b)) => json!({
                        "available": true,
                        "briefing": b,
                    }),
                    Ok(None) => json!({
                        "available": false,
                        "briefing": Value::Null,
                        "note": "No persisted circle briefing found yet.",
                    }),
                    Err(err) => json!({
                        "available": false,
                        "briefing": Value::Null,
                        "note": format!("Failed to read latest briefing: {err}"),
                    }),
                };
                let signals = db
                    .list_inbox_signals(None, None, RECENT_EVENTS_MOUNT_CAP)
                    .unwrap_or_default();
                let triages = db
                    .list_signal_triages(None, None, RECENT_EVENTS_MOUNT_CAP)
                    .unwrap_or_default();
                let events_json = json!({
                    "available": true,
                    "cap": RECENT_EVENTS_MOUNT_CAP,
                    "count": signals.len(),
                    "signals": signals,
                    "triage_count": triages.len(),
                    "triages": triages,
                });
                (briefing_json, events_json)
            }
            Ok(None) => (
                json!({
                    "available": false,
                    "briefing": Value::Null,
                    "note": "Lake database not initialized yet.",
                }),
                json!({
                    "available": false,
                    "cap": RECENT_EVENTS_MOUNT_CAP,
                    "count": 0,
                    "signals": [],
                    "triage_count": 0,
                    "triages": [],
                }),
            ),
            Err(err) => (
                json!({
                    "available": false,
                    "briefing": Value::Null,
                    "note": format!("Lake database unavailable: {err}"),
                }),
                json!({
                    "available": false,
                    "cap": RECENT_EVENTS_MOUNT_CAP,
                    "count": 0,
                    "signals": [],
                    "triage_count": 0,
                    "triages": [],
                    "note": format!("Lake database unavailable: {err}"),
                }),
            ),
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&briefing_payload) {
            files.insert("briefings/latest.json".to_string(), bytes);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&events_payload) {
            files.insert("events/recent.json".to_string(), bytes);
        }

        const TOP_DIRS: [&str; 6] = [
            "computed",
            "dossiers",
            "industries",
            "memos",
            "briefings",
            "events",
        ];
        let mut dirs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        {
            let root = dirs.entry(String::new()).or_default();
            for dir in TOP_DIRS {
                root.insert(dir.to_string());
            }
        }
        let mut path_ids = HashMap::new();
        path_ids.insert(String::new(), 0);
        let mut next_id = 1u64;
        for dir in TOP_DIRS {
            dirs.entry(dir.to_string()).or_default();
            path_ids.insert(dir.to_string(), next_id);
            next_id += 1;
        }

        for rel in files.keys() {
            let parts: Vec<&str> = rel.split('/').collect();
            let mut cur_dir = String::new();
            for dir_part in &parts[..parts.len() - 1] {
                dirs.entry(cur_dir.clone())
                    .or_default()
                    .insert((*dir_part).to_string());
                cur_dir = if cur_dir.is_empty() {
                    (*dir_part).to_string()
                } else {
                    format!("{cur_dir}/{dir_part}")
                };
                dirs.entry(cur_dir.clone()).or_default();
                path_ids.entry(cur_dir.clone()).or_insert_with(|| {
                    let id = next_id;
                    next_id += 1;
                    id
                });
            }
            let file_name = parts[parts.len() - 1];
            dirs.entry(cur_dir)
                .or_default()
                .insert(file_name.to_string());
            path_ids.entry(rel.clone()).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            });
        }

        Self {
            files,
            dirs,
            path_ids,
            versions: VersionTable::new(),
        }
    }

    fn qid_for(&self, rel: &str, kind: FileKind) -> Qid {
        let path_id = *self.path_ids.get(rel).unwrap_or(&0);
        Qid {
            kind,
            version: self.versions.get(path_id),
            path: path_id,
        }
    }
}

enum FidNode {
    Dir {
        rel_path: String,
        listing: Vec<u8>,
        qid: Qid,
    },
    File {
        rel_path: String,
        bytes: Vec<u8>,
        qid: Qid,
    },
}

struct FidState {
    node: FidNode,
    mode: Option<OpenMode>,
}

/// Read-only `FileServer` projected at `/mnt/m3`.
pub struct M3WorkbenchFs {
    paths: WorkspacePaths,
    tree: RwLock<TreeSnapshot>,
    fids: AsyncMutex<HashMap<Fid, FidState>>,
}

impl M3WorkbenchFs {
    pub fn new(paths: WorkspacePaths) -> Self {
        let tree = TreeSnapshot::from_paths(&paths);
        Self {
            paths,
            tree: RwLock::new(tree),
            fids: AsyncMutex::new(HashMap::new()),
        }
    }

    pub fn refresh(&self) {
        let mut updated = TreeSnapshot::from_paths(&self.paths);
        if let Ok(mut guard) = self.tree.write() {
            for path_id in updated.path_ids.values() {
                updated.versions.bump(*path_id);
            }
            *guard = updated;
        }
    }
}

#[async_trait::async_trait]
impl FileServer for M3WorkbenchFs {
    async fn walk(&self, fid: Fid, newfid: Fid, names: &[String]) -> Result<Qid, ErrorCode> {
        let mut fids = self.fids.lock().await;
        if newfid == Fid::ROOT || fids.contains_key(&newfid) {
            return Err(ErrorCode::BadRequest);
        }

        let start_path = if fid == Fid::ROOT {
            String::new()
        } else {
            match &fids.get(&fid).ok_or(ErrorCode::NotFound)?.node {
                FidNode::Dir { rel_path, .. } => rel_path.clone(),
                FidNode::File { .. } => {
                    if !names.is_empty() {
                        return Err(ErrorCode::NotDirectory);
                    }
                    let existing = &fids.get(&fid).ok_or(ErrorCode::NotFound)?.node;
                    let FidNode::File {
                        rel_path,
                        bytes,
                        qid,
                    } = existing
                    else {
                        unreachable!();
                    };
                    let rel_copy = rel_path.clone();
                    let bytes_copy = bytes.clone();
                    let qid_copy = *qid;
                    fids.insert(
                        newfid,
                        FidState {
                            node: FidNode::File {
                                rel_path: rel_copy,
                                bytes: bytes_copy,
                                qid: qid_copy,
                            },
                            mode: None,
                        },
                    );
                    return Ok(qid_copy);
                }
            }
        };

        let tree = self.tree.read().map_err(|_| ErrorCode::Io)?;
        let mut current = start_path;
        for name in names {
            if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                return Err(ErrorCode::BadRequest);
            }
            let children = tree.dirs.get(&current).ok_or(ErrorCode::NotDirectory)?;
            if !children.contains(name) {
                return Err(ErrorCode::NotFound);
            }
            current = if current.is_empty() {
                name.clone()
            } else {
                format!("{current}/{name}")
            };
        }

        let node = if let Some(entries) = tree.dirs.get(&current) {
            let listing = entries
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes();
            let qid = tree.qid_for(&current, FileKind::Dir);
            FidNode::Dir {
                rel_path: current,
                listing,
                qid,
            }
        } else if let Some(bytes) = tree.files.get(&current) {
            let qid = tree.qid_for(&current, FileKind::File);
            FidNode::File {
                rel_path: current,
                bytes: bytes.clone(),
                qid,
            }
        } else {
            return Err(ErrorCode::NotFound);
        };

        let qid = match &node {
            FidNode::Dir { qid, .. } | FidNode::File { qid, .. } => *qid,
        };
        fids.insert(newfid, FidState { node, mode: None });
        Ok(qid)
    }

    async fn open(&self, fid: Fid, mode: OpenMode) -> Result<Qid, ErrorCode> {
        if !matches!(mode, OpenMode::Read) {
            return Err(ErrorCode::NoAccess);
        }
        if fid == Fid::ROOT {
            return Ok(Qid {
                kind: FileKind::Dir,
                version: 0,
                path: 0,
            });
        }
        let mut fids = self.fids.lock().await;
        let state = fids.get_mut(&fid).ok_or(ErrorCode::NotFound)?;
        if state.mode.is_some() {
            return Err(ErrorCode::BadRequest);
        }
        state.mode = Some(mode);
        let qid = match &state.node {
            FidNode::Dir { qid, .. } | FidNode::File { qid, .. } => *qid,
        };
        Ok(qid)
    }

    async fn read(&self, fid: Fid, offset: Offset, count: u32) -> Result<Vec<u8>, ErrorCode> {
        let fids = self.fids.lock().await;
        let state = fids.get(&fid).ok_or(ErrorCode::NotFound)?;
        if !matches!(state.mode, Some(OpenMode::Read)) {
            return Err(ErrorCode::NoAccess);
        }
        let data = match &state.node {
            FidNode::Dir { listing, .. } => listing,
            FidNode::File { bytes, .. } => bytes,
        };
        let start = (offset as usize).min(data.len());
        let end = data.len().min(start + count as usize);
        Ok(data[start..end].to_vec())
    }

    async fn write(&self, _fid: Fid, _offset: Offset, _data: &[u8]) -> Result<u32, ErrorCode> {
        Err(ErrorCode::NoAccess)
    }

    async fn stat(&self, fid: Fid) -> Result<Stat, ErrorCode> {
        if fid == Fid::ROOT {
            return Ok(Stat {
                name: String::new(),
                qid: Qid {
                    kind: FileKind::Dir,
                    version: 0,
                    path: 0,
                },
                length: 0,
                executable: false,
                writable: false,
            });
        }
        let fids = self.fids.lock().await;
        let state = fids.get(&fid).ok_or(ErrorCode::NotFound)?;
        let (name, qid, length) = match &state.node {
            FidNode::Dir { rel_path, qid, .. } => (rel_path.clone(), *qid, 0),
            FidNode::File {
                rel_path,
                bytes,
                qid,
            } => (rel_path.clone(), *qid, bytes.len() as u64),
        };
        Ok(Stat {
            name,
            qid,
            length,
            executable: false,
            writable: false,
        })
    }

    async fn create(
        &self,
        _fid: Fid,
        _newfid: Fid,
        _name: &str,
        _kind: FileKind,
    ) -> Result<Qid, ErrorCode> {
        Err(ErrorCode::NoAccess)
    }

    async fn remove(&self, _fid: Fid) -> Result<(), ErrorCode> {
        Err(ErrorCode::NoAccess)
    }

    async fn clunk(&self, fid: Fid) -> Result<(), ErrorCode> {
        if fid == Fid::ROOT {
            return Ok(());
        }
        let mut fids = self.fids.lock().await;
        fids.remove(&fid).ok_or(ErrorCode::NotFound)?;
        Ok(())
    }
}

/// Synchronous policy hook enforcing Project-M3's ironclad red lines:
/// unconditionally denies any tool invocation touching `human_judgment`,
/// `computed/`, `raw/`, `data/munger/`, `m3.duckdb`, `.duckdb`, `.parquet`,
/// `parquet/`, `lake/`, `journal.json`, `watchlist.json`, `budget.json`,
/// `themes.json`, or `memos/`, as well as any arbitrary write tool outside
/// `m3_update_draft`'s draft-only schema and any Network/Unknown tool.
#[derive(Debug, Default)]
pub struct M3ToolPolicyHook;

impl M3ToolPolicyHook {
    fn inspect_value_for_redline(value: &Value) -> Option<&'static str> {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    let lk = k.to_ascii_lowercase();
                    if lk == "human_judgment"
                        || lk.contains("human_judgment")
                        || matches!(
                            lk.as_str(),
                            "my_stance"
                                | "weight_rationale"
                                | "falsification_metrics"
                                | "feynman_checked_at"
                                | "pending_human_review"
                                | "industry_status"
                                | "multiple_low"
                                | "multiple_high"
                                | "human_notes"
                                | "feynman_checked"
                        )
                    {
                        return Some("human_judgment");
                    }
                    if matches!(
                        lk.as_str(),
                        "industries" | "industries_dir" | "industry_profile"
                    ) {
                        return Some("industries/");
                    }
                    if matches!(
                        lk.as_str(),
                        "computed" | "computed_path" | "computed_report"
                    ) {
                        return Some("computed/");
                    }
                    if matches!(lk.as_str(), "raw" | "raw_path" | "sec_raw") {
                        return Some("raw/");
                    }
                    if matches!(lk.as_str(), "data_munger" | "munger_data") {
                        return Some("data/munger/");
                    }
                    if matches!(lk.as_str(), "duckdb" | "duckdb_path" | "lake_db") {
                        return Some("m3.duckdb");
                    }
                    if matches!(lk.as_str(), "parquet" | "parquet_path") {
                        return Some("parquet/");
                    }
                    if matches!(lk.as_str(), "lake" | "lake_path" | "lake_dir") {
                        return Some("lake/");
                    }
                    if matches!(lk.as_str(), "journal" | "journal_path") {
                        return Some("journal.json");
                    }
                    if matches!(lk.as_str(), "watchlist" | "watchlist_path") {
                        return Some("watchlist.json");
                    }
                    if matches!(lk.as_str(), "budget" | "budget_path" | "budget_config") {
                        return Some("budget.json");
                    }
                    if matches!(lk.as_str(), "themes" | "themes_path") {
                        return Some("themes.json");
                    }
                    if matches!(lk.as_str(), "memo_path" | "memos_dir") {
                        return Some("memos/");
                    }
                    if let Some(hit) = Self::inspect_value_for_redline(v) {
                        return Some(hit);
                    }
                }
                None
            }
            Value::Array(items) => items.iter().find_map(Self::inspect_value_for_redline),
            Value::String(s) => Self::inspect_str_for_redline(s),
            _ => None,
        }
    }

    fn matches_path_segment(lower: &str, seg: &str) -> bool {
        lower == seg
            || lower.starts_with(&format!("{seg}/"))
            || lower.contains(&format!("/{seg}/"))
            || lower.ends_with(&format!("/{seg}"))
    }

    fn inspect_str_for_redline(s: &str) -> Option<&'static str> {
        let lower = s.trim().to_ascii_lowercase().replace('\\', "/");
        if lower.contains("human_judgment")
            || lower.contains("pending_human_review")
            || lower.contains("my_stance")
            || lower.contains("weight_rationale")
            || lower.contains("falsification_metrics")
            || lower.contains("feynman_checked_at")
        {
            return Some("human_judgment");
        }
        if lower.contains("m3.duckdb") || lower.contains(".duckdb") {
            return Some("m3.duckdb");
        }
        if lower.contains(".parquet") || Self::matches_path_segment(&lower, "parquet") {
            return Some("parquet/");
        }
        if Self::matches_path_segment(&lower, "computed") || lower.contains("data/munger/computed")
        {
            return Some("computed/");
        }
        if Self::matches_path_segment(&lower, "raw") || lower.contains("data/munger/raw") {
            return Some("raw/");
        }
        if Self::matches_path_segment(&lower, "lake") {
            return Some("lake/");
        }
        if lower.contains("data/munger") {
            return Some("data/munger/");
        }
        if lower.contains("journal.json") || lower == "journal" || lower.ends_with("/journal") {
            return Some("journal.json");
        }
        if lower.contains("watchlist.json") || lower == "watchlist" || lower.ends_with("/watchlist")
        {
            return Some("watchlist.json");
        }
        if lower.contains("budget.json") || lower == "budget" || lower.ends_with("/budget") {
            return Some("budget.json");
        }
        if lower.contains("themes.json") || lower == "themes" || lower.ends_with("/themes") {
            return Some("themes.json");
        }
        if Self::matches_path_segment(&lower, "memos") || lower.contains("workbench/memos") {
            return Some("memos/");
        }
        if Self::matches_path_segment(&lower, "industries")
            || lower.contains("workbench/industries")
        {
            return Some("industries/");
        }
        None
    }

    fn inspect_write_readonly_mount(value: &Value) -> Option<&'static str> {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    let lk = k.to_ascii_lowercase();
                    if matches!(lk.as_str(), "briefings" | "briefings_dir" | "briefing_path") {
                        return Some("briefings/");
                    }
                    if matches!(lk.as_str(), "events_path" | "recent_events") {
                        return Some("events/");
                    }
                    if let Some(hit) = Self::inspect_write_readonly_mount(v) {
                        return Some(hit);
                    }
                }
                None
            }
            Value::Array(items) => items.iter().find_map(Self::inspect_write_readonly_mount),
            Value::String(s) => {
                let lower = s.trim().to_ascii_lowercase().replace('\\', "/");
                if Self::matches_path_segment(&lower, "briefings") {
                    return Some("briefings/");
                }
                if Self::matches_path_segment(&lower, "events")
                    || lower.contains("events/recent.json")
                {
                    return Some("events/");
                }
                if lower == "/mnt/m3" || lower.starts_with("/mnt/m3/") {
                    return Some("/mnt/m3");
                }
                None
            }
            _ => None,
        }
    }
}

impl ToolPolicyHook for M3ToolPolicyHook {
    fn evaluate(&self, tool_name: &str, args: &Value, capability: ToolCapability) -> HookDecision {
        if tool_name == "m3_web_search" && capability == ToolCapability::Network {
            let Some(obj) = args.as_object() else {
                return HookDecision::Deny(
                    "红线拦截：m3_web_search 参数必须是包含 query 的 JSON 对象。".into(),
                );
            };
            for key in obj.keys() {
                if !matches!(key.as_str(), "query" | "limit") {
                    return HookDecision::Deny(format!(
                        "红线拦截：m3_web_search 不允许参数 `{key}`。"
                    ));
                }
            }
            let valid_query = obj
                .get("query")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty());
            if !valid_query {
                return HookDecision::Deny(
                    "红线拦截：m3_web_search 缺少非空 query 字符串。".into(),
                );
            }
            if let Some(lim) = obj.get("limit") {
                let valid_lim = lim.as_u64().is_some_and(|v| (1..=8).contains(&v));
                if !valid_lim {
                    return HookDecision::Deny(
                        "红线拦截：m3_web_search limit 必须是 1..=8 之间的整数。".into(),
                    );
                }
            }
            return HookDecision::Allow;
        }

        if let Some(target) = Self::inspect_value_for_redline(args) {
            return HookDecision::Deny(format!(
                "红线拦截：{target} 属于不可篡改事实源或人类判断主权区，严禁 Alan 写入或覆盖。"
            ));
        }

        match capability {
            ToolCapability::Read => HookDecision::Allow,
            ToolCapability::Write => {
                if let Some(target) = Self::inspect_write_readonly_mount(args) {
                    return HookDecision::Deny(format!(
                        "红线拦截：{target} 属于只读事实挂载区，严禁 Alan 写入或覆盖。"
                    ));
                }
                if tool_name != "m3_update_draft" {
                    return HookDecision::Deny(format!(
                        "红线拦截：仅允许通过 m3_update_draft 更新 AI 草稿区，拒绝执行 {tool_name}。"
                    ));
                }
                const ALLOWED_DRAFT_KEYS: &[&str] = &[
                    "ticker",
                    "three_sentence_business",
                    "latticework",
                    "premortem",
                    "open_questions",
                    "draft_source",
                ];
                if let Some(obj) = args.as_object() {
                    for key in obj.keys() {
                        if !ALLOWED_DRAFT_KEYS.contains(&key.as_str()) {
                            return HookDecision::Deny(format!(
                                "红线拦截：m3_update_draft 不允许修改字段 `{key}`。"
                            ));
                        }
                    }
                    HookDecision::Allow
                } else {
                    HookDecision::Deny("红线拦截：m3_update_draft 参数必须是 JSON 对象。".into())
                }
            }
            ToolCapability::Network | ToolCapability::Unknown => HookDecision::Deny(format!(
                "红线拦截：嵌入式 Alan 禁止执行网络或未知高危工具 {tool_name}。"
            )),
        }
    }
}

struct M3StatusTool {
    paths: WorkspacePaths,
}

impl Tool for M3StatusTool {
    fn name(&self) -> &str {
        "m3_status"
    }

    fn description(&self) -> &str {
        "读取 Project-M3 观察池总览、击球价状态、ROIC 均值、预警数量及决策账本概况。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, _arguments: Value, ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        let ns = ctx.namespace().cloned();
        Box::pin(async move {
            let ns_watchlist = if let Some(ns) = ns.as_ref() {
                Shell::new(ns.clone())
                    .cat("/mnt/m3/watchlist.json")
                    .await
                    .ok()
                    .map(|b| b.len())
            } else {
                None
            };
            let wl = store::load_watchlist(&paths)?;
            let reports = store::load_or_compute_reports(&paths)?;
            let journal = store::load_journal(&paths)?;
            let companies: Vec<Value> = reports
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
                "ok": true,
                "benchmark_symbol": wl.benchmark_symbol,
                "namespace_watchlist_bytes": ns_watchlist,
                "companies": companies,
                "journal_entries": journal.entries.len(),
                "cash_balance": journal.cash_balance.normalize().to_string(),
            }))
        })
    }
}

struct M3ReportTool {
    paths: WorkspacePaths,
}

impl Tool for M3ReportTool {
    fn name(&self) -> &str {
        "m3_report"
    }

    fn description(&self) -> &str {
        "获取指定公司由 store.rs / calculator.rs 生成的确定性查理计算器报告（含 ROIC、严格股东盈余、击球价、反推增速及 XBRL 溯源）。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ticker": {
                    "type": "string",
                    "description": "股票代码，例如 AAPL"
                },
                "recompute": {
                    "type": "boolean",
                    "description": "是否强制用 calculator.rs 重算；默认 false 优先读取一致缓存或按需计算"
                }
            },
            "required": ["ticker"],
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        let ns = ctx.namespace().cloned();
        Box::pin(async move {
            let ticker = arguments
                .get("ticker")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("m3_report 缺少 ticker 参数")?
                .to_ascii_uppercase();
            let force_recompute = arguments
                .get("recompute")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let report: MungerReport = if force_recompute {
                store::recompute_ticker(&paths, &ticker)?
            } else {
                store::load_or_recompute_ticker(&paths, &ticker)?
            };

            let ns_path = format!(
                "/mnt/m3/computed/{}.json",
                crate::market::safe_symbol_filename(&report.ticker)
            );
            let namespace_verified = if let Some(ns) = ns.as_ref() {
                Shell::new(ns.clone()).cat(&ns_path).await.is_ok()
            } else {
                false
            };

            let mut value = serde_json::to_value(&report)?;
            if let Some(obj) = value.as_object_mut() {
                obj.insert("namespace_path".to_string(), Value::String(ns_path));
                obj.insert(
                    "namespace_verified".to_string(),
                    Value::Bool(namespace_verified),
                );
            }
            Ok(value)
        })
    }
}

struct M3DossierTool {
    paths: WorkspacePaths,
}

impl Tool for M3DossierTool {
    fn name(&self) -> &str {
        "m3_dossier"
    }

    fn description(&self) -> &str {
        "读取指定公司的人机审讯档案 workbench/dossiers/<TICKER>.json（含三句话过滤、跨学科格栅、事前验尸及只读 human_judgment 状态）。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ticker": {
                    "type": "string",
                    "description": "股票代码，例如 AAPL"
                }
            },
            "required": ["ticker"],
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        let ns = ctx.namespace().cloned();
        Box::pin(async move {
            let ticker = arguments
                .get("ticker")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("m3_dossier 缺少 ticker 参数")?
                .to_ascii_uppercase();
            let ns_path = format!(
                "/mnt/m3/dossiers/{}.json",
                crate::market::safe_symbol_filename(&ticker)
            );
            let namespace_hit = if let Some(ns) = ns.as_ref() {
                Shell::new(ns.clone())
                    .cat(&ns_path)
                    .await
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Dossier>(&bytes).ok())
            } else {
                None
            };
            let dossier = store::load_dossier(&paths, &ticker)?.or(namespace_hit);
            Ok(json!({
                "ticker": ticker,
                "exists": dossier.is_some(),
                "namespace_path": ns_path,
                "dossier": dossier,
            }))
        })
    }
}

struct M3UpdateDraftTool {
    paths: WorkspacePaths,
    fs: Arc<M3WorkbenchFs>,
}

impl Tool for M3UpdateDraftTool {
    fn name(&self) -> &str {
        "m3_update_draft"
    }

    fn description(&self) -> &str {
        "仅更新指定公司 dossier 中的 AI 幕僚草稿字段（three_sentence_business / latticework / premortem / open_questions），严禁触及 human_judgment。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ticker": { "type": "string" },
                "three_sentence_business": { "type": "string" },
                "latticework": {
                    "type": "object",
                    "properties": {
                        "microeconomics": { "type": "string" },
                        "ecology": { "type": "string" },
                        "psychology_incentives": { "type": "string" },
                        "capital_allocation": { "type": "string" }
                    },
                    "additionalProperties": false
                },
                "premortem": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "open_questions": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "draft_source": { "type": "string" }
            },
            "required": ["ticker"],
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Write
    }

    fn execute(&self, arguments: Value, _ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        let fs = Arc::clone(&self.fs);
        Box::pin(async move {
            // Defense-in-depth inside execute even though M3ToolPolicyHook runs first.
            if let Some(hit) = M3ToolPolicyHook::inspect_value_for_redline(&arguments) {
                anyhow::bail!("红线拦截：禁止写入 {hit}");
            }

            let ticker = arguments
                .get("ticker")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("m3_update_draft 缺少 ticker 参数")?
                .to_ascii_uppercase();

            let wl = store::load_watchlist(&paths)?;
            let company = wl
                .companies
                .iter()
                .find(|c| c.ticker.eq_ignore_ascii_case(&ticker))
                .with_context(|| format!("观察池中不存在 {ticker}"))?;

            let existing = store::load_dossier(&paths, &company.ticker)?;
            let preserved_judgment = existing
                .as_ref()
                .map(|d| d.human_judgment.clone())
                .unwrap_or(HumanJudgment {
                    pending_human_review: true,
                    my_stance: None,
                    weight_rationale: None,
                    falsification_metrics: Vec::new(),
                    feynman_checked_at: None,
                });

            let mut dossier = existing.unwrap_or_else(|| Dossier {
                schema_version: 1,
                ticker: company.ticker.clone(),
                name: company.name.clone(),
                updated_at: store::civil_today_iso(),
                draft_source: "alan_embed_v2".to_string(),
                three_sentence_business: String::new(),
                latticework: Latticework {
                    microeconomics: String::new(),
                    ecology: String::new(),
                    psychology_incentives: String::new(),
                    capital_allocation: String::new(),
                },
                premortem: Vec::new(),
                open_questions: Vec::new(),
                human_judgment: preserved_judgment.clone(),
            });

            if let Some(s) = arguments
                .get("three_sentence_business")
                .and_then(Value::as_str)
            {
                dossier.three_sentence_business = s.to_string();
            }
            if let Some(lat) = arguments.get("latticework").and_then(Value::as_object) {
                if let Some(v) = lat.get("microeconomics").and_then(Value::as_str) {
                    dossier.latticework.microeconomics = v.to_string();
                }
                if let Some(v) = lat.get("ecology").and_then(Value::as_str) {
                    dossier.latticework.ecology = v.to_string();
                }
                if let Some(v) = lat.get("psychology_incentives").and_then(Value::as_str) {
                    dossier.latticework.psychology_incentives = v.to_string();
                }
                if let Some(v) = lat.get("capital_allocation").and_then(Value::as_str) {
                    dossier.latticework.capital_allocation = v.to_string();
                }
            }
            if let Some(arr) = arguments.get("premortem").and_then(Value::as_array) {
                dossier.premortem = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
            }
            if let Some(arr) = arguments.get("open_questions").and_then(Value::as_array) {
                dossier.open_questions = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
            }
            if let Some(src) = arguments.get("draft_source").and_then(Value::as_str) {
                dossier.draft_source = src.to_string();
            } else {
                dossier.draft_source = "alan_embed_v2".to_string();
            }
            dossier.updated_at = store::civil_today_iso();
            dossier.human_judgment = preserved_judgment;

            store::write_json_atomic(&paths.dossier_path(&company.ticker), &dossier)?;
            fs.refresh();

            Ok(json!({
                "ok": true,
                "ticker": dossier.ticker,
                "updated_at": dossier.updated_at,
                "human_judgment_preserved": true,
                "pending_human_review": dossier.human_judgment.pending_human_review,
            }))
        })
    }
}

fn parse_opt_decimal_arg(val: Option<&Value>, field: &str) -> Result<Option<Decimal>> {
    let Some(v) = val else {
        return Ok(None);
    };
    match v {
        Value::Null => Ok(None),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Decimal::from_str(trimmed)
                    .map(Some)
                    .with_context(|| format!("{field} 不是有效 Decimal: {trimmed}"))
            }
        }
        Value::Number(n) => Decimal::from_str(&n.to_string())
            .map(Some)
            .with_context(|| format!("{field} 不是有效数值: {n}")),
        other => anyhow::bail!("{field} 需要数字或字符串，实际为 {other}"),
    }
}

fn lake_table_exists(db: &LakeDb, table_name: &str) -> Result<bool> {
    let count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
        [table_name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let prefix: String = text.chars().take(max_chars).collect();
    format!("{prefix}…")
}

struct M3UniverseQueryTool {
    paths: WorkspacePaths,
}

impl Tool for M3UniverseQueryTool {
    fn name(&self) -> &str {
        "m3_universe_query"
    }

    fn description(&self) -> &str {
        "只读查询全美股 DuckDB 确定性宽表 `universe_snapshot`（按 ROIC、SBC/OCF、股本稀释、击球距、行业过滤并排序，最多返回 50 条）。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "关键词（代码/名称/SIC 描述）" },
                "min_roic_5y": { "type": ["number", "string"], "description": "5年 ROIC 均值下限（%）" },
                "max_sbc_to_ocf": { "type": ["number", "string"], "description": "SBC/OCF 上限（%）" },
                "max_share_change_3y": { "type": ["number", "string"], "description": "3年股本变化上限（%）" },
                "max_net_debt_to_oe": { "type": ["number", "string"], "description": "净负债/严格股东盈余上限" },
                "min_years_of_data": { "type": "integer", "minimum": 0 },
                "exclude_financials": { "type": "boolean", "default": true },
                "positive_strict_oe_only": { "type": "boolean", "default": false },
                "in_price_pool_only": { "type": "boolean", "default": false },
                "strike_zone_only": { "type": "boolean", "default": false },
                "sort_by": { "type": "string", "description": "quality|roic|fcf_yield|distance|share_change|sbc_ocf|price|strike|warnings|ticker" },
                "sort_asc": { "type": "boolean", "default": false },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 }
            },
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, _ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        Box::pin(async move {
            let Some(db) = LakeDb::open_read_only_if_exists(&paths)? else {
                return Ok(json!({
                    "ok": true,
                    "lake_available": false,
                    "count": 0,
                    "rows": [],
                }));
            };

            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(20usize, |v| usize::try_from(v).unwrap_or(50))
                .clamp(1, 50);

            let sort_by = match arguments.get("sort_by").and_then(Value::as_str) {
                Some(raw) if !raw.trim().is_empty() => UniverseSortBy::from_str(raw)?,
                _ => UniverseSortBy::QualityScore,
            };

            let filter = UniverseFilter {
                min_roic_5y: parse_opt_decimal_arg(arguments.get("min_roic_5y"), "min_roic_5y")?,
                max_sbc_to_ocf: parse_opt_decimal_arg(
                    arguments.get("max_sbc_to_ocf"),
                    "max_sbc_to_ocf",
                )?,
                max_share_change_3y: parse_opt_decimal_arg(
                    arguments.get("max_share_change_3y"),
                    "max_share_change_3y",
                )?,
                max_net_debt_to_oe: parse_opt_decimal_arg(
                    arguments.get("max_net_debt_to_oe"),
                    "max_net_debt_to_oe",
                )?,
                min_years_of_data: arguments
                    .get("min_years_of_data")
                    .and_then(Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok()),
                exclude_financials: arguments
                    .get("exclude_financials")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                positive_strict_oe_only: arguments
                    .get("positive_strict_oe_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                in_price_pool_only: arguments
                    .get("in_price_pool_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                strike_zone_only: arguments
                    .get("strike_zone_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                query: arguments
                    .get("query")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty()),
                sort_by,
                sort_asc: arguments
                    .get("sort_asc")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                limit,
            };

            let rows = db.screen_universe(&filter)?;
            Ok(json!({
                "ok": true,
                "lake_available": true,
                "count": rows.len(),
                "rows": rows,
            }))
        })
    }
}

struct M3PeerPercentilesTool {
    paths: WorkspacePaths,
}

impl Tool for M3PeerPercentilesTool {
    fn name(&self) -> &str {
        "m3_peer_percentiles"
    }

    fn description(&self) -> &str {
        "只读查询指定公司在全美股同 SIC 行业组内的 ROIC、毛利率与严格 FCF 收益率分位数及头部同行（最多 20 家）。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ticker": { "type": "string", "description": "股票代码，例如 ADSK" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10 }
            },
            "required": ["ticker"],
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, _ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        Box::pin(async move {
            let ticker = arguments
                .get("ticker")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("m3_peer_percentiles 缺少 ticker 参数")?
                .to_ascii_uppercase();
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(10usize, |v| usize::try_from(v).unwrap_or(20))
                .clamp(1, 20);

            let Some(db) = LakeDb::open_read_only_if_exists(&paths)? else {
                return Ok(json!({
                    "ok": true,
                    "lake_available": false,
                    "ticker": ticker,
                    "comparison": Value::Null,
                }));
            };

            match db.query_peers(&ticker, limit) {
                Ok(comparison) => Ok(json!({
                    "ok": true,
                    "lake_available": true,
                    "ticker": ticker,
                    "comparison": comparison,
                })),
                Err(err) => Ok(json!({
                    "ok": false,
                    "lake_available": true,
                    "ticker": ticker,
                    "error": format!("{err:#}"),
                })),
            }
        })
    }
}

struct M3SemanticNeighborsTool {
    paths: WorkspacePaths,
}

impl Tool for M3SemanticNeighborsTool {
    fn name(&self) -> &str {
        "m3_semantic_neighbors"
    }

    fn description(&self) -> &str {
        "只读混合检索（SQL 财务硬过滤 × 10-K/8-K/新闻向量+BM25 相似度），返回相似商业原型公司及一手原文 SignalEvidence 引用。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ticker": { "type": "string", "description": "以目标公司业务轮廓找相似公司（与 query 二选一）" },
                "query": { "type": "string", "description": "商业模式或关键词描述（与 ticker 二选一）" },
                "as_ticker": { "type": "boolean" },
                "min_roic_5y": { "type": ["number", "string"] },
                "max_sbc_to_ocf": { "type": ["number", "string"] },
                "exclude_financials": { "type": "boolean", "default": true },
                "positive_strict_oe_only": { "type": "boolean", "default": false },
                "in_price_pool_only": { "type": "boolean", "default": false },
                "min_score": { "type": "number", "default": 0.0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10 }
            },
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, _ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        Box::pin(async move {
            let ticker_arg = arguments
                .get("ticker")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_ascii_uppercase);
            let query_arg = arguments
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);

            let (needle, as_ticker) = match (query_arg, ticker_arg) {
                (Some(q), _) => {
                    let flag = arguments
                        .get("as_ticker")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    (q, flag)
                }
                (None, Some(t)) => {
                    let flag = arguments
                        .get("as_ticker")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    (t, flag)
                }
                (None, None) => anyhow::bail!("m3_semantic_neighbors 需要 query 或 ticker 参数"),
            };

            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(10usize, |v| usize::try_from(v).unwrap_or(20))
                .clamp(1, 20);

            let filter = SimilarSearchFilter {
                min_roic_5y: parse_opt_decimal_arg(arguments.get("min_roic_5y"), "min_roic_5y")?,
                max_sbc_to_ocf: parse_opt_decimal_arg(
                    arguments.get("max_sbc_to_ocf"),
                    "max_sbc_to_ocf",
                )?,
                exclude_financials: arguments
                    .get("exclude_financials")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                positive_strict_oe_only: arguments
                    .get("positive_strict_oe_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                in_price_pool_only: arguments
                    .get("in_price_pool_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                min_score: arguments
                    .get("min_score")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32,
                limit,
            };

            let Some(db) = LakeDb::open_read_only_if_exists(&paths)? else {
                return Ok(json!({
                    "ok": true,
                    "lake_available": false,
                    "count": 0,
                    "hits": [],
                }));
            };

            match db.search_similar(&needle, as_ticker, &filter) {
                Ok(hits) => Ok(json!({
                    "ok": true,
                    "lake_available": true,
                    "query": needle,
                    "as_ticker": as_ticker,
                    "count": hits.len(),
                    "hits": hits,
                })),
                Err(err) => Ok(json!({
                    "ok": false,
                    "lake_available": true,
                    "query": needle,
                    "error": format!("{err:#}"),
                })),
            }
        })
    }
}

struct M3EventsForTickerTool {
    paths: WorkspacePaths,
}

impl Tool for M3EventsForTickerTool {
    fn name(&self) -> &str {
        "m3_events_for_ticker"
    }

    fn description(&self) -> &str {
        "只读查询指定公司最近的 SEC filings（10-K/10-Q/8-K/Form 4）、L1 事件流与有界一手财报文本切片摘要。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ticker": { "type": "string", "description": "股票代码，例如 AAPL" },
                "filings_limit": { "type": "integer", "minimum": 0, "maximum": 20, "default": 8 },
                "events_limit": { "type": "integer", "minimum": 0, "maximum": 20, "default": 10 },
                "chunks_limit": { "type": "integer", "minimum": 0, "maximum": 15, "default": 6 },
                "max_chunk_chars": { "type": "integer", "minimum": 100, "maximum": 1500, "default": 600 }
            },
            "required": ["ticker"],
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, _ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        Box::pin(async move {
            let ticker = arguments
                .get("ticker")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("m3_events_for_ticker 缺少 ticker 参数")?
                .to_ascii_uppercase();

            let filings_limit = arguments
                .get("filings_limit")
                .and_then(Value::as_u64)
                .map_or(8usize, |v| usize::try_from(v).unwrap_or(20))
                .min(20);
            let events_limit = arguments
                .get("events_limit")
                .and_then(Value::as_u64)
                .map_or(10usize, |v| usize::try_from(v).unwrap_or(20))
                .min(20);
            let chunks_limit = arguments
                .get("chunks_limit")
                .and_then(Value::as_u64)
                .map_or(6usize, |v| usize::try_from(v).unwrap_or(15))
                .min(15);
            let max_chunk_chars = arguments
                .get("max_chunk_chars")
                .and_then(Value::as_u64)
                .map_or(600usize, |v| usize::try_from(v).unwrap_or(1500))
                .clamp(100, 1500);

            let Some(db) = LakeDb::open_read_only_if_exists(&paths)? else {
                return Ok(json!({
                    "ok": true,
                    "lake_available": false,
                    "ticker": ticker,
                    "filings": [],
                    "events": [],
                    "doc_chunks": [],
                }));
            };

            let mut filings = Vec::new();
            if filings_limit > 0 && lake_table_exists(&db, "filings")? {
                let cik_opt: Option<String> = if lake_table_exists(&db, "universe_snapshot")? {
                    db.conn
                        .query_row(
                            "SELECT cik FROM universe_snapshot WHERE UPPER(ticker) = ? LIMIT 1",
                            [&ticker],
                            |r| r.get(0),
                        )
                        .ok()
                } else {
                    None
                };

                if let Some(cik) = cik_opt {
                    filings = db.list_filings_for_cik(&cik, filings_limit)?;
                } else {
                    let sql_limit = i64::try_from(filings_limit).unwrap_or(20);
                    let mut stmt = db.conn.prepare(
                        "SELECT accession_no, cik, ticker, form, filing_date, report_date,
                                primary_document, primary_doc_description, items_json,
                                filing_url, ingested_at_ms
                         FROM filings
                         WHERE UPPER(ticker) = UPPER(?)
                         ORDER BY filing_date DESC, accession_no DESC
                         LIMIT ?",
                    )?;
                    let rows = stmt.query_map(params![ticker.as_str(), sql_limit], |r| {
                        Ok(crate::lake_l1::LakeFilingRow {
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
                    for r in rows {
                        filings.push(r?);
                    }
                }
            }

            let events = db.list_events(Some(&ticker), None, false, events_limit)?;

            let mut chunk_excerpts: Vec<Value> = Vec::new();
            if chunks_limit > 0 && lake_table_exists(&db, "doc_chunks")? {
                let sql_limit = i64::try_from(chunks_limit).unwrap_or(15);
                let mut stmt = db.conn.prepare(
                    "SELECT chunk_id, doc_type, source_id, period_end, filing_date,
                            section, title, content, source_url
                     FROM doc_chunks
                     WHERE UPPER(ticker) = UPPER(?)
                     ORDER BY
                        CASE section
                            WHEN 'item_1' THEN 0
                            WHEN 'company_profile' THEN 1
                            WHEN 'item_1a' THEN 2
                            WHEN 'item_7' THEN 3
                            WHEN 'item_8k' THEN 4
                            ELSE 5
                        END ASC,
                        COALESCE(filing_date, '') DESC,
                        chunk_id ASC
                     LIMIT ?",
                )?;
                let rows = stmt.query_map(params![ticker.as_str(), sql_limit], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                    ))
                })?;
                for r in rows {
                    let (
                        chunk_id,
                        doc_type,
                        source_id,
                        period_end,
                        filing_date,
                        section,
                        title,
                        content,
                        source_url,
                    ) = r?;
                    chunk_excerpts.push(json!({
                        "chunk_id": chunk_id,
                        "doc_type": doc_type,
                        "source_id": source_id,
                        "period_end": period_end,
                        "filing_date": filing_date,
                        "section": section,
                        "title": title,
                        "excerpt": truncate_chars(&content, max_chunk_chars),
                        "source_url": source_url,
                    }));
                }
            }

            Ok(json!({
                "ok": true,
                "lake_available": true,
                "ticker": ticker,
                "filings": filings,
                "events": events,
                "doc_chunks": chunk_excerpts,
            }))
        })
    }
}

struct M3IndustryMatrixTool {
    paths: WorkspacePaths,
}

impl Tool for M3IndustryMatrixTool {
    fn name(&self) -> &str {
        "m3_industry_matrix"
    }

    fn description(&self) -> &str {
        "读取某个行业研究项目或一组代表公司池的 DuckDB L0 确定性 5 年财务矩阵与 10-K 原文片段数。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "industry_slug": { "type": "string", "description": "workbench/industries/<slug>.json 的 slug" },
                "tickers": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "可选：直接指定代表公司代码列表（最多 15 家）"
                }
            },
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Read
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, _ctx: &ToolContext) -> ToolResult {
        let paths = self.paths.clone();
        Box::pin(async move {
            let mut tickers: Vec<String> = Vec::new();
            let mut industry_meta: Option<Value> = None;

            if let Some(slug) = arguments
                .get("industry_slug")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                && let Ok(profile) = industry::load_industry(&paths, slug)
            {
                tickers.extend(profile.bound_tickers.iter().cloned());
                industry_meta = Some(json!({
                    "slug": profile.slug,
                    "title": profile.title,
                    "status": profile.status,
                    "sic_codes": profile.sic_codes,
                    "multiple_low": profile.multiple_low.map(|d| d.normalize().to_string()),
                    "multiple_high": profile.multiple_high.map(|d| d.normalize().to_string()),
                }));
            }

            if let Some(arr) = arguments.get("tickers").and_then(Value::as_array) {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        let up = s.trim().to_ascii_uppercase();
                        if !up.is_empty() && !tickers.iter().any(|t| t.eq_ignore_ascii_case(&up)) {
                            tickers.push(up);
                        }
                    }
                }
            }
            tickers.truncate(15);

            if tickers.is_empty() {
                return Ok(json!({
                    "ok": true,
                    "lake_available": false,
                    "industry": industry_meta,
                    "rows": [],
                }));
            }

            let Some(db) = LakeDb::open_read_only_if_exists(&paths)? else {
                return Ok(json!({
                    "ok": true,
                    "lake_available": false,
                    "industry": industry_meta,
                    "tickers": tickers,
                    "rows": [],
                }));
            };

            let rows = db.load_universe_rows_by_tickers(&tickers)?;
            let sec_counts = db.count_sec_10k_chunks_by_ticker(&tickers)?;

            let matrix: Vec<Value> = rows
                .into_iter()
                .map(|r| {
                    let up = r.ticker.to_ascii_uppercase();
                    json!({
                        "ticker": r.ticker,
                        "name": r.name,
                        "sic": r.sic,
                        "sic_description": r.sic_description,
                        "years_of_data": r.years_of_data,
                        "roic_5y_avg_pct": r.roic_5y_avg_pct.map(|v| v.round_dp(2).normalize().to_string()),
                        "gross_margin_5y_avg_pct": r.gross_margin_5y_avg_pct.map(|v| v.round_dp(2).normalize().to_string()),
                        "strict_owner_earnings": r.strict_owner_earnings.map(|v| v.round_dp(0).normalize().to_string()),
                        "strict_fcf_yield_pct": r.strict_fcf_yield_pct.map(|v| v.round_dp(2).normalize().to_string()),
                        "sbc_to_ocf_pct": r.sbc_to_ocf_pct.map(|v| v.round_dp(2).normalize().to_string()),
                        "share_change_3y_pct": r.share_change_3y_pct.map(|v| v.round_dp(2).normalize().to_string()),
                        "net_debt_to_oe": r.net_debt_to_oe.map(|v| v.round_dp(2).normalize().to_string()),
                        "quality_score": r.quality_score.round_dp(1).normalize().to_string(),
                        "latest_price": r.latest_price.map(|v| v.round_dp(2).normalize().to_string()),
                        "strike_price_18x": r.strike_price.map(|v| v.round_dp(2).normalize().to_string()),
                        "sec_10k_chunks": sec_counts.get(&up).copied().unwrap_or(0),
                        "warnings": r.parsed_warnings().into_iter().map(|w| w.title).collect::<Vec<_>>(),
                    })
                })
                .collect();

            Ok(json!({
                "ok": true,
                "lake_available": true,
                "industry": industry_meta,
                "count": matrix.len(),
                "rows": matrix,
            }))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

struct M3WebSearchTool;

impl Tool for M3WebSearchTool {
    fn name(&self) -> &str {
        "m3_web_search"
    }

    fn description(&self) -> &str {
        "搜索公开网络行业资料、竞争格局演变、历史周期与产业链信息（结果标记为 source_type=web，严禁替代 SEC 10-K 确定性财报数据）。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "公开网页检索关键词（建议中/英文行业词，如 EDA duopoly Synopsys Cadence profit pool history）" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 8, "default": 5 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn capability(&self, _arguments: &Value) -> ToolCapability {
        ToolCapability::Network
    }

    fn presentation_is_read_only(&self) -> bool {
        true
    }

    fn execute(&self, arguments: Value, _ctx: &ToolContext) -> ToolResult {
        Box::pin(async move {
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("m3_web_search 缺少 query 参数")?
                .to_owned();
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(5usize, |v| usize::try_from(v).unwrap_or(5))
                .clamp(1, 8);

            let query_clone = query.clone();
            let fetch_res =
                tokio::task::spawn_blocking(move || run_public_web_search(&query_clone, limit))
                    .await;

            match fetch_res {
                Ok(Ok(hits)) => Ok(json!({
                    "ok": true,
                    "source_type": "web",
                    "citation_rule": "请在正文中与 SEC 10-K 一手引用严格区分并标注 [Web: 标题 (URL)]",
                    "query": query,
                    "count": hits.len(),
                    "results": hits.into_iter().map(|h| json!({
                        "title": h.title,
                        "url": h.url,
                        "snippet": h.snippet,
                        "source_type": "web",
                    })).collect::<Vec<_>>(),
                })),
                Ok(Err(err)) => Ok(json!({
                    "ok": false,
                    "source_type": "web",
                    "query": query,
                    "results": [],
                    "warning": format!("公开网页搜索暂不可用（已安全降级为本地 SEC/DuckDB 检索）：{err:#}"),
                })),
                Err(err) => Ok(json!({
                    "ok": false,
                    "source_type": "web",
                    "query": query,
                    "results": [],
                    "warning": format!("网页搜索任务中止：{err}"),
                })),
            }
        })
    }
}

fn run_public_web_search(query: &str, limit: usize) -> Result<Vec<WebSearchHit>> {
    const MAX_SEARCH_BYTES: usize = 256 * 1024;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Project-M3/0.3")
        .build()
        .context("初始化网页搜索客户端失败")?;

    let mut url = reqwest::Url::parse("https://html.duckduckgo.com/html/")?;
    url.query_pairs_mut().append_pair("q", query);

    let resp = client.get(url).send().context("请求公开网页搜索引擎失败")?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}");
    }
    let mut buf = Vec::with_capacity(32 * 1024);
    resp.take(MAX_SEARCH_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .context("读取搜索结果失败")?;
    if buf.len() > MAX_SEARCH_BYTES {
        buf.truncate(MAX_SEARCH_BYTES);
    }
    let html = String::from_utf8_lossy(&buf);
    Ok(parse_ddg_html_results(&html, limit))
}

pub fn parse_ddg_html_results(html: &str, limit: usize) -> Vec<WebSearchHit> {
    let mut hits = Vec::new();
    let mut search_pos = 0usize;
    let marker = "class=\"result__a\"";

    while hits.len() < limit {
        let Some(rel_idx) = html[search_pos..].find(marker) else {
            break;
        };
        let tag_start = search_pos + rel_idx;
        let after_marker = tag_start + marker.len();
        let Some(close_gt) = html[after_marker..].find('>') else {
            break;
        };
        let open_lt = html[..tag_start].rfind('<').unwrap_or(tag_start);
        let anchor_tag = &html[open_lt..=after_marker + close_gt];
        let raw_href = extract_attr(anchor_tag, "href").unwrap_or_default();
        let resolved_url = normalize_ddg_href(&raw_href);

        let title_start = after_marker + close_gt + 1;
        let Some(end_a) = html[title_start..].find("</a>") else {
            break;
        };
        let raw_title = &html[title_start..title_start + end_a];
        let title = clean_html_text(raw_title);

        let rest_start = title_start + end_a + 4;
        let snippet_window_end = (rest_start + 2400).min(html.len());
        let window = &html[rest_start..snippet_window_end];
        let snippet = extract_ddg_snippet(window);

        if !title.is_empty() && resolved_url.starts_with("http") {
            hits.push(WebSearchHit {
                title,
                url: resolved_url,
                snippet,
            });
        }
        search_pos = rest_start;
    }
    hits
}

fn extract_attr(fragment: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let pos = fragment.rfind(&needle)?;
    let val_start = pos + needle.len();
    let val_end = fragment[val_start..].find('"')?;
    Some(decode_html_entities(
        &fragment[val_start..val_start + val_end],
    ))
}

fn extract_ddg_snippet(window: &str) -> String {
    if let Some(pos) = window.find("class=\"result__snippet\"") {
        let after = &window[pos..];
        if let Some(gt) = after.find('>') {
            let body = &after[gt + 1..];
            if let Some(end) = body.find("</a>").or_else(|| body.find("</div>")) {
                return clean_html_text(&body[..end]);
            }
        }
    }
    String::new()
}

fn normalize_ddg_href(raw: &str) -> String {
    let trimmed = raw.trim();
    let normalized = if trimmed.starts_with("//") {
        format!("https:{trimmed}")
    } else if trimmed.starts_with('/') {
        format!("https://html.duckduckgo.com{trimmed}")
    } else {
        trimmed.to_owned()
    };
    if let Ok(parsed) = reqwest::Url::parse(&normalized) {
        if parsed.path().starts_with("/l/") {
            for (k, v) in parsed.query_pairs() {
                if k == "uddg" && !v.trim().is_empty() {
                    return v.into_owned();
                }
            }
        }
        return parsed.to_string();
    }
    normalized
}

fn clean_html_text(raw: &str) -> String {
    let mut no_tags = String::with_capacity(raw.len());
    let mut in_tag = false;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                let next_is_punct = matches!(
                    chars.peek().copied(),
                    Some('.' | ',' | ';' | ':' | '!' | '?' | ')' | ']')
                );
                if !next_is_punct && !no_tags.ends_with(' ') {
                    no_tags.push(' ');
                }
            }
            _ if !in_tag => no_tags.push(ch),
            _ => {}
        }
    }
    let decoded = decode_html_entities(&no_tags);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

enum InterrogatorCommand {
    Submit {
        ticker: String,
        name: String,
        bucket: Bucket,
        question: String,
    },
    SubmitScoped {
        scope: AlanScope,
        lang: UiLang,
        question: String,
    },
    SubmitPrompt {
        display_prompt: String,
        full_prompt: String,
    },
    Interrupt,
    Reset,
    RefreshFs,
    Shutdown,
}

/// Asynchronous controller hosting Alan V2 across Company, Industry, Briefing, and Portfolio scopes.
#[derive(Clone)]
pub struct M3InterrogatorHandle {
    paths: WorkspacePaths,
    fs: Arc<M3WorkbenchFs>,
    cmd_tx: tokio_mpsc::UnboundedSender<InterrogatorCommand>,
    events: Arc<StdMutex<Vec<InterrogatorEvent>>>,
    #[allow(dead_code)]
    usage: Arc<StdMutex<(u64, u64)>>,
}

/// Canonical alias for [`M3InterrogatorHandle`].
pub type M3AlanInterrogator = M3InterrogatorHandle;

impl M3InterrogatorHandle {
    pub fn start_shared_credentials(paths: WorkspacePaths) -> Self {
        Self::start_with_llm(paths, LlmSource::SharedProductCredentials)
    }

    /// Starts Alan with an async wakeup notifier fired whenever an event is pushed.
    pub fn start_shared_credentials_notified(
        paths: WorkspacePaths,
        notify: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self::start_with_llm_inner(paths, LlmSource::SharedProductCredentials, Some(notify))
    }

    pub fn start_with_llm(paths: WorkspacePaths, llm: LlmSource) -> Self {
        Self::start_with_llm_inner(paths, llm, None)
    }

    fn start_with_llm_inner(
        paths: WorkspacePaths,
        llm: LlmSource,
        notify: Option<Arc<tokio::sync::Notify>>,
    ) -> Self {
        let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
        let (cmd_tx, mut cmd_rx) = tokio_mpsc::unbounded_channel::<InterrogatorCommand>();
        let events = Arc::new(StdMutex::new(Vec::new()));
        let usage = Arc::new(StdMutex::new((0u64, 0u64)));
        let fs_worker = Arc::clone(&fs);
        let worker_paths = paths.clone();
        let events_worker = EventSink {
            events: Arc::clone(&events),
            notify: notify.clone(),
        };
        let events_spawn_err = EventSink {
            events: Arc::clone(&events),
            notify,
        };
        let usage_worker = Arc::clone(&usage);

        if let Err(err) = std::thread::Builder::new()
            .name("m3-alan-interrogator".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(err) => {
                        push_event(
                            &events_worker,
                            InterrogatorEvent::Error {
                                message: format!("无法启动 Alan 异步运行时: {err}"),
                            },
                        );
                        return;
                    }
                };

                rt.block_on(async move {
                    let app = match boot_m3_embedded(
                        worker_paths.clone(),
                        Arc::clone(&fs_worker),
                        llm,
                    )
                    .await
                    {
                        Ok(app) => app,
                        Err(err) => {
                            push_event(
                                &events_worker,
                                InterrogatorEvent::Error {
                                    message: friendly_m3_credentials_error(&format!("{err:#}")),
                                },
                            );
                            return;
                        }
                    };

                    let mut client: RootAgentClient = app.root_agent();
                    let mut stream = match client.event_stream().await {
                        Ok(s) => s,
                        Err(err) => {
                            push_event(
                                &events_worker,
                                InterrogatorEvent::Error {
                                    message: format!("打开 Alan 事件流失败: {err:#}"),
                                },
                            );
                            let _ = app.shutdown().await;
                            return;
                        }
                    };
                    push_event(&events_worker, InterrogatorEvent::Ready);

                    let mut active_submission: Option<String> = None;

                    loop {
                        tokio::select! {
                            maybe_cmd = cmd_rx.recv() => {
                                let Some(cmd) = maybe_cmd else {
                                    break;
                                };
                                match cmd {
                                    InterrogatorCommand::Submit { ticker, name, bucket, question } => {
                                        fs_worker.refresh();
                                        let prompt = compose_interrogation_turn(&ticker, &name, bucket, &question);
                                        match client.submit_text(prompt.clone()).await {
                                            Ok(submission_id) => {
                                                active_submission = Some(submission_id.clone());
                                                push_event(&events_worker, InterrogatorEvent::UserSubmitted {
                                                    submission_id,
                                                    prompt: question,
                                                });
                                            }
                                            Err(err) => {
                                                push_event(&events_worker, InterrogatorEvent::Error {
                                                    message: friendly_m3_credentials_error(&format!("{err:#}")),
                                                });
                                            }
                                        }
                                    }
                                    InterrogatorCommand::SubmitScoped { scope, lang, question } => {
                                        fs_worker.refresh();
                                        let prompt = scope.compose_interrogation_turn(&worker_paths, lang, &question);
                                        match client.submit_text(prompt).await {
                                            Ok(submission_id) => {
                                                active_submission = Some(submission_id.clone());
                                                push_event(&events_worker, InterrogatorEvent::UserSubmitted {
                                                    submission_id,
                                                    prompt: question,
                                                });
                                            }
                                            Err(err) => {
                                                push_event(&events_worker, InterrogatorEvent::Error {
                                                    message: friendly_m3_credentials_error(&format!("{err:#}")),
                                                });
                                            }
                                        }
                                    }
                                    InterrogatorCommand::SubmitPrompt { display_prompt, full_prompt } => {
                                        fs_worker.refresh();
                                        match client.submit_text(full_prompt).await {
                                            Ok(submission_id) => {
                                                active_submission = Some(submission_id.clone());
                                                push_event(&events_worker, InterrogatorEvent::UserSubmitted {
                                                    submission_id,
                                                    prompt: display_prompt,
                                                });
                                            }
                                            Err(err) => {
                                                push_event(&events_worker, InterrogatorEvent::Error {
                                                    message: friendly_m3_credentials_error(&format!("{err:#}")),
                                                });
                                            }
                                        }
                                    }
                                    InterrogatorCommand::Interrupt => {
                                        let _ = client.interrupt(active_submission.as_deref()).await;
                                    }
                                    InterrogatorCommand::Reset => {
                                        active_submission = None;
                                        let opts = SessionOptions::default().with_system_prompt_mode(
                                            SystemPromptMode::Replace {
                                                prompt: M3_INTERROGATOR_SYSTEM_PROMPT.to_string(),
                                            },
                                        );
                                        match app.reset_session(opts).await {
                                            Ok(_) => {
                                                client = app.root_agent();
                                                match client.event_stream().await {
                                                    Ok(next_stream) => {
                                                        stream = next_stream;
                                                        push_event(&events_worker, InterrogatorEvent::ResetDone);
                                                    }
                                                    Err(err) => {
                                                        push_event(&events_worker, InterrogatorEvent::Error {
                                                            message: format!("重置事件流失败: {err:#}"),
                                                        });
                                                    }
                                                }
                                            }
                                            Err(err) => {
                                                push_event(&events_worker, InterrogatorEvent::Error {
                                                    message: format!("重置审讯会话失败: {err:#}"),
                                                });
                                            }
                                        }
                                    }
                                    InterrogatorCommand::RefreshFs => {
                                        fs_worker.refresh();
                                    }
                                    InterrogatorCommand::Shutdown => break,
                                }
                            }
                            ev = stream.next_event() => {
                                match ev {
                                    Ok(event) => match event {
                                        EmbedEvent::TextDelta { submission_id, chunk, is_final } => {
                                            push_event(&events_worker, InterrogatorEvent::TextDelta {
                                                submission_id,
                                                chunk,
                                                is_final,
                                            });
                                        }
                                        EmbedEvent::ToolStarted { call_id, name, title } => {
                                            push_event(&events_worker, InterrogatorEvent::ToolStarted {
                                                call_id,
                                                name,
                                                title,
                                            });
                                        }
                                        EmbedEvent::ToolProgress { call_id, preview } => {
                                            push_event(&events_worker, InterrogatorEvent::ToolProgress {
                                                call_id,
                                                preview,
                                            });
                                        }
                                        EmbedEvent::ToolFinished { call_id, name, status, preview } => {
                                            push_event(&events_worker, InterrogatorEvent::ToolFinished {
                                                call_id,
                                                name,
                                                status,
                                                preview: preview.unwrap_or_default(),
                                            });
                                        }
                                        EmbedEvent::Error { message, .. } => {
                                            push_event(&events_worker, InterrogatorEvent::Error {
                                                message: friendly_m3_credentials_error(&message),
                                            });
                                        }
                                        EmbedEvent::TurnFinished { submission_id, status, error } => {
                                            active_submission = None;
                                            push_event(&events_worker, InterrogatorEvent::TurnDone {
                                                submission_id,
                                                completed: status == UiInputStatus::Completed,
                                                error: error.map(|e| friendly_m3_credentials_error(&e)),
                                            });
                                        }
                                        EmbedEvent::Usage { usage: snap, .. } => {
                                            if let Ok(mut guard) = usage_worker.lock() {
                                                guard.0 = guard.0.saturating_add(snap.input_tokens);
                                                guard.1 = guard.1.saturating_add(snap.output_tokens);
                                            }
                                        }
                                        EmbedEvent::ThinkingDelta { .. }
                                        | EmbedEvent::PendingRequest { .. } => {}
                                    }
                                    Err(err) => {
                                        push_event(&events_worker, InterrogatorEvent::Error {
                                            message: format!("Alan 事件流中断: {err:#}"),
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    let _ = app.shutdown().await;
                });
            })
        {
            push_event(
                &events_spawn_err,
                InterrogatorEvent::Error {
                    message: format!("无法启动 Alan 审讯线程: {err}"),
                },
            );
        }

        Self {
            paths,
            fs,
            cmd_tx,
            events,
            usage,
        }
    }

    pub fn paths(&self) -> &WorkspacePaths {
        &self.paths
    }

    #[allow(dead_code)]
    pub fn session_usage(&self) -> (u64, u64) {
        self.usage.lock().map(|g| *g).unwrap_or((0, 0))
    }

    pub fn refresh_fs(&self) {
        self.fs.refresh();
        let _ = self.cmd_tx.send(InterrogatorCommand::RefreshFs);
    }

    pub fn submit(
        &self,
        ticker: impl Into<String>,
        name: impl Into<String>,
        bucket: Bucket,
        question: impl Into<String>,
    ) {
        let q = question.into();
        if q.trim().is_empty() {
            return;
        }
        let _ = self.cmd_tx.send(InterrogatorCommand::Submit {
            ticker: ticker.into(),
            name: name.into(),
            bucket,
            question: q,
        });
    }

    /// Submits a scoped interrogation turn across any [`AlanScope`] variant using
    /// the shared `/mnt/m3` mount, deterministic tools, and [`M3ToolPolicyHook`].
    pub fn submit_scoped(&self, scope: &AlanScope, lang: UiLang, question: impl Into<String>) {
        let q = question.into();
        if q.trim().is_empty() {
            return;
        }
        let _ = self.cmd_tx.send(InterrogatorCommand::SubmitScoped {
            scope: scope.normalized(),
            lang,
            question: q,
        });
    }

    pub fn submit_prompt(&self, display_prompt: impl Into<String>, full_prompt: impl Into<String>) {
        let display = display_prompt.into();
        let full = full_prompt.into();
        if display.trim().is_empty() || full.trim().is_empty() {
            return;
        }
        let _ = self.cmd_tx.send(InterrogatorCommand::SubmitPrompt {
            display_prompt: display,
            full_prompt: full,
        });
    }

    pub fn interrupt(&self) {
        let _ = self.cmd_tx.send(InterrogatorCommand::Interrupt);
    }

    pub fn reset(&self) {
        let _ = self.cmd_tx.send(InterrogatorCommand::Reset);
    }

    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(InterrogatorCommand::Shutdown);
    }

    pub fn drain_events(&self) -> Vec<InterrogatorEvent> {
        self.events
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }
}

struct EventSink {
    events: Arc<StdMutex<Vec<InterrogatorEvent>>>,
    notify: Option<Arc<tokio::sync::Notify>>,
}

fn push_event(sink: &EventSink, event: InterrogatorEvent) {
    if let Ok(mut guard) = sink.events.lock() {
        guard.push(event);
    }
    if let Some(notify) = &sink.notify {
        notify.notify_one();
    }
}

pub fn compose_interrogation_turn(
    ticker: &str,
    name: &str,
    bucket: Bucket,
    question: &str,
) -> String {
    format!(
        "[当前审讯标的: {ticker} · {name} · 分桶 {}]\n\
         请先借助 `m3_report` / `m3_dossier` 核对确定性数据，再用 `[report:{ticker}]`、`[xbrl:{ticker}]`、`[dossier:{ticker}]` 标注核心依据：\n\n\
         {question}",
        bucket.code()
    )
}

pub fn compose_industry_tutor_turn(
    profile: &IndustryProfile,
    primer_excerpt: &str,
    question: &str,
) -> String {
    let tickers = if profile.bound_tickers.is_empty() {
        "暂无绑定标的".to_owned()
    } else {
        profile.bound_tickers.join(", ")
    };
    let anchor = profile
        .valuation_anchor_label()
        .unwrap_or_else(|| "尚未由人类设定".to_owned());
    let excerpt = if primer_excerpt.trim().is_empty() {
        "（尚未生成行业活手册正文，请直接调用工具检索代表公司 10-K、DuckDB 财务矩阵与公开网页资料）"
            .to_owned()
    } else {
        truncate_chars(primer_excerpt, 2000)
    };
    format!(
        "[当前行业学习台：{} (slug: `{}`) | 代表公司池：{tickers} | 人类估值锚：{anchor}]\n\
         请务必调用 `m3_industry_matrix`、`m3_events_for_ticker`（按需）或 `m3_web_search` 核对事实；\n\
         并在引用出处时严格区分 `[SEC 10-K: TICKER]`（一手官方年报/财务）与 `[Web: 标题 (URL)]`（公开网页资料）；严禁给出买卖建议或代改人类判断。\n\n\
         当前行业活手册摘要：\n{excerpt}\n\n\
         人类提问：{question}",
        profile.title, profile.slug
    )
}

pub fn compose_feynman_quiz_turn(profile: &IndustryProfile, primer_excerpt: &str) -> String {
    let tickers = if profile.bound_tickers.is_empty() {
        "暂无绑定标的".to_owned()
    } else {
        profile.bound_tickers.join(", ")
    };
    let excerpt = truncate_chars(primer_excerpt, 2000);
    format!(
        "[费曼通关自测：{} (slug: `{}`) | 代表公司池：{tickers}]\n\
         请先用 `m3_industry_matrix` 查看代表公司 5Y ROIC、毛利与资本开支差异，然后严格模仿查理·芒格，向我提出 **4 道极具穿透力的费曼通关拷问**（编号 1–4）：\n\
         1. 利润池与单位经济（为什么不是最赚钱的环节在承担周期风险？）\n\
         2. 护城河与替代品边界（客户为什么不能自己做，或者几年后会被谁颠覆？）\n\
         3. 历史死法与杠杆反噬（这个行业过去 20 年破产或长期毁灭股东价值的公司通常犯了什么错？）\n\
         4. 估值锚验证（我能否用一句话说清它合理的股东盈余倍数区间，以及什么指标变化说明我判断错了？）\n\
         不要直接帮我作答！列出这 4 个问题后等待我用大白话回答，再由你指出逻辑漏洞。\n\n\
         行业手册参考：\n{excerpt}",
        profile.title, profile.slug
    )
}

fn fmt_opt_dec(v: Option<Decimal>, dp: u32) -> String {
    v.map(|d| d.round_dp(dp).normalize().to_string())
        .unwrap_or_else(|| "—".to_owned())
}

fn build_company_context_preamble(paths: &WorkspacePaths, ticker: &str, lang: UiLang) -> String {
    let upper = ticker.trim().to_ascii_uppercase();
    let safe = crate::market::safe_symbol_filename(&upper);
    let rep_opt = store::read_json::<MungerReport>(&paths.computed_report_path(&upper))
        .ok()
        .or_else(|| store::load_or_recompute_ticker(paths, &upper).ok());
    let dossier_opt = store::load_dossier(paths, &upper).ok().flatten();
    let has_memo = paths
        .workbench_dir
        .join("memos")
        .join(format!("{safe}.md"))
        .is_file();
    let has_feynman = paths
        .workbench_dir
        .join("memos")
        .join(format!("{safe}-feynman.md"))
        .is_file();

    let mut lines = Vec::new();
    match rep_opt {
        Some(rep) => {
            let val = rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref());
            let price = fmt_opt_dec(rep.price.as_ref().map(|p| p.price), 2);
            let strike = fmt_opt_dec(val.and_then(|v| v.strike_price), 2);
            let gap = fmt_opt_dec(val.and_then(|v| v.distance_to_strike_pct), 2);
            let roic = fmt_opt_dec(rep.roic_5y_avg_pct, 2);
            lines.push(match lang {
                UiLang::Zh => format!(
                    "[当前研究范围：公司 {upper} · {} | 分桶 {} | 状态 {} | 现价 {price} {} | 击球价 {strike} ({gap}%) | 5Y ROIC {roic}% | 预警 {} 项]",
                    rep.name,
                    rep.bucket.code(),
                    rep.status.label_for(lang),
                    rep.currency,
                    rep.warnings.len(),
                ),
                UiLang::En => format!(
                    "[Active Scope: Company {upper} · {} | Bucket {} | Status {} | Price {price} {} | Strike {strike} ({gap}%) | 5Y ROIC {roic}% | Warnings {}]",
                    rep.name,
                    rep.bucket.code(),
                    rep.status.label_for(lang),
                    rep.currency,
                    rep.warnings.len(),
                ),
            });
        }
        None => {
            lines.push(match lang {
                UiLang::Zh => format!("[当前研究范围：公司 {upper}（观察池/计算报告尚未生成）]"),
                UiLang::En => {
                    format!("[Active Scope: Company {upper} (no saved computed report yet)]")
                }
            });
        }
    }

    if let Some(d) = dossier_opt {
        let stance = d.human_judgment.my_stance.as_deref().unwrap_or("UNDECIDED");
        let excerpt = truncate_chars(d.three_sentence_business.trim(), 240);
        lines.push(match lang {
            UiLang::Zh => format!(
                "现有档案摘要（人类立场只读 {stance} | 待人类答辩 {}）：{excerpt}",
                d.human_judgment.pending_human_review
            ),
            UiLang::En => format!(
                "Existing dossier (read-only human stance {stance} | pending_review={}): {excerpt}",
                d.human_judgment.pending_human_review
            ),
        });
    }

    lines.push(match lang {
        UiLang::Zh => format!(
            "请优先调用 `m3_report`、`m3_dossier`、`m3_events_for_ticker` 或读取 `/mnt/m3/computed/{safe}.json`、`/mnt/m3/dossiers/{safe}.json`{}{}; 引用依据时请标注 `[report:{upper}]`、`[xbrl:{upper}]`、`[dossier:{upper}]`，严禁修改 `human_judgment` 或直接给出买卖指令。",
            if has_memo { format!("、`/mnt/m3/memos/{safe}.md`") } else { String::new() },
            if has_feynman { format!("、`/mnt/m3/memos/{safe}-feynman.md`") } else { String::new() },
        ),
        UiLang::En => format!(
            "Verify facts via `m3_report`, `m3_dossier`, `m3_events_for_ticker`, or read-only `/mnt/m3/computed/{safe}.json` and `/mnt/m3/dossiers/{safe}.json`{}{}; cite `[report:{upper}]`, `[xbrl:{upper}]`, `[dossier:{upper}]`. Never modify `human_judgment` or issue buy/sell commands.",
            if has_memo { format!(", `/mnt/m3/memos/{safe}.md`") } else { String::new() },
            if has_feynman { format!(", `/mnt/m3/memos/{safe}-feynman.md`") } else { String::new() },
        ),
    });

    lines.join("\n")
}

fn build_industry_context_preamble(paths: &WorkspacePaths, slug: &str, lang: UiLang) -> String {
    let trimmed = slug.trim();
    if let Ok(profile) = industry::load_industry(paths, trimmed) {
        let primer = industry::load_industry_primer_md(paths, trimmed).unwrap_or_default();
        let tickers = if profile.bound_tickers.is_empty() {
            lang.t("暂无绑定标的", "none bound").to_owned()
        } else {
            profile.bound_tickers.join(", ")
        };
        let anchor = profile
            .valuation_anchor_label()
            .unwrap_or_else(|| lang.t("尚未设定", "not set").to_owned());
        let excerpt = if primer.trim().is_empty() {
            lang.t("（暂无行业活手册正文）", "(No industry living primer yet)")
                .to_owned()
        } else {
            truncate_chars(&primer, 1600)
        };
        return match lang {
            UiLang::Zh => format!(
                "[当前研究范围：行业 {} (slug: `{}`) | 状态 {} | 代表公司池：{tickers} | 人类估值锚：{anchor}]\n\
                 请使用 `m3_industry_matrix`、`m3_events_for_ticker`、`m3_web_search` 及只读 `/mnt/m3/industries/{}.json` 核对事实；严格区分 `[SEC 10-K: TICKER]` 与 `[Web: 标题 (URL)]`，禁止修改人类判断。\n\
                 行业活手册摘要：\n{excerpt}",
                profile.title,
                profile.slug,
                profile.status.label_for(lang),
                profile.slug
            ),
            UiLang::En => format!(
                "[Active Scope: Industry {} (slug: `{}`) | Status {} | Bound Tickers: {tickers} | Human Valuation Anchor: {anchor}]\n\
                 Verify facts via `m3_industry_matrix`, `m3_events_for_ticker`, `m3_web_search`, and read-only `/mnt/m3/industries/{}.json`; distinguish `[SEC 10-K: TICKER]` from `[Web: Title (URL)]`. Never alter human judgment.\n\
                 Primer Excerpt:\n{excerpt}",
                profile.title,
                profile.slug,
                profile.status.label_for(lang),
                profile.slug
            ),
        };
    }

    match lang {
        UiLang::Zh => format!(
            "[当前研究范围：行业 `{trimmed}`]\n\
             未找到该行业档案，可先读取只读 `/mnt/m3/industries/index.json` 或调用 `m3_universe_query` 核对。"
        ),
        UiLang::En => format!(
            "[Active Scope: Industry `{trimmed}`]\n\
             Industry profile not found on disk; inspect read-only `/mnt/m3/industries/index.json` or call `m3_universe_query`."
        ),
    }
}

fn build_briefing_context_preamble(
    paths: &WorkspacePaths,
    briefing_id: &str,
    lang: UiLang,
) -> String {
    let trimmed = briefing_id.trim();
    let target_id = if trimmed.is_empty() {
        "latest"
    } else {
        trimmed
    };

    let mut briefing_opt = None;
    let mut recent_signals_count = 0usize;
    let mut recent_triages_count = 0usize;
    if let Ok(Some(db)) = LakeDb::open_read_only_if_exists(paths) {
        if target_id.eq_ignore_ascii_case("latest") {
            briefing_opt = db.latest_circle_briefing(None).ok().flatten();
        } else if let Ok(list) = db.list_circle_briefings(None, 25) {
            briefing_opt = list
                .into_iter()
                .find(|b| b.briefing_id == target_id || b.period_key == target_id);
        }
        recent_signals_count = db
            .list_inbox_signals(None, None, RECENT_EVENTS_MOUNT_CAP)
            .map(|v| v.len())
            .unwrap_or(0);
        recent_triages_count = db
            .list_signal_triages(None, None, RECENT_EVENTS_MOUNT_CAP)
            .map(|v| v.len())
            .unwrap_or(0);
    }

    match briefing_opt {
        Some(b) => {
            let radar_tickers: Vec<&str> = b
                .strike_radar
                .iter()
                .take(8)
                .map(|r| r.ticker.as_str())
                .collect();
            let radar_label = if radar_tickers.is_empty() {
                lang.t("无", "none").to_owned()
            } else {
                radar_tickers.join(", ")
            };
            let exec_summary = b
                .executive_summary
                .as_deref()
                .map(|s| truncate_chars(s.trim(), 500))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    lang.t(
                        "（静默期或未生成 LLM 摘要）",
                        "(Quiet period or deterministic preview)",
                    )
                    .to_owned()
                });
            match lang {
                UiLang::Zh => format!(
                    "[当前研究范围：能力圈简报 {} ({}) | 击球雷达 {} 项: {radar_label} | 财报异动 {} 项 | L1.5 哨兵命中 {} 项 | 近 40 条信号 {recent_signals_count} / 研判 {recent_triages_count}]\n\
                     简报标题：{}\n\
                     摘要：{exec_summary}\n\
                     请结合只读 `/mnt/m3/briefings/latest.json`、`/mnt/m3/events/recent.json` 与 `m3_report` / `m3_events_for_ticker` 回答，并用 `[report:TICKER]` / `[xbrl:TICKER]` 标注相关标的。",
                    b.period_key,
                    b.period.as_str(),
                    b.strike_radar.len(),
                    b.filing_deltas.len(),
                    b.watchdog_hits.len(),
                    b.headline,
                ),
                UiLang::En => format!(
                    "[Active Scope: Circle Briefing {} ({}) | Strike Radar {}: {radar_label} | Filing Deltas {} | Watchdog Hits {} | Recent signals {recent_signals_count} / triages {recent_triages_count}]\n\
                     Headline: {}\n\
                     Summary: {exec_summary}\n\
                     Use read-only `/mnt/m3/briefings/latest.json`, `/mnt/m3/events/recent.json`, `m3_report`, and `m3_events_for_ticker`, citing `[report:TICKER]` / `[xbrl:TICKER]`.",
                    b.period_key,
                    b.period.as_str(),
                    b.strike_radar.len(),
                    b.filing_deltas.len(),
                    b.watchdog_hits.len(),
                    b.headline,
                ),
            }
        }
        None => match lang {
            UiLang::Zh => format!(
                "[当前研究范围：能力圈简报 `{target_id}`（暂未持久化简报）| 近 40 条信号 {recent_signals_count} / 研判 {recent_triages_count}]\n\
                 请读取只读 `/mnt/m3/briefings/latest.json`、`/mnt/m3/events/recent.json` 及 `m3_status` 核对最新观察池与事件动态。"
            ),
            UiLang::En => format!(
                "[Active Scope: Circle Briefing `{target_id}` (no persisted briefing yet) | Recent signals {recent_signals_count} / triages {recent_triages_count}]\n\
                 Inspect read-only `/mnt/m3/briefings/latest.json`, `/mnt/m3/events/recent.json`, and `m3_status` for current circle and event state."
            ),
        },
    }
}

fn build_portfolio_context_preamble(paths: &WorkspacePaths, lang: UiLang) -> String {
    let wl = store::load_watchlist(paths).ok();
    let journal = store::load_journal(paths).ok();
    let reports = store::load_or_compute_reports(paths).unwrap_or_default();
    let industries = industry::list_industries(paths).unwrap_or_default();

    let (core_n, watch_n, out_n, hard_n) = wl.as_ref().map_or((0, 0, 0, 0), |w| {
        let mut c = 0usize;
        let mut wa = 0usize;
        let mut o = 0usize;
        let mut h = 0usize;
        for co in &w.companies {
            match co.bucket {
                Bucket::Core => c += 1,
                Bucket::Watch => wa += 1,
                Bucket::Out => o += 1,
                Bucket::TooHard => h += 1,
            }
        }
        (c, wa, o, h)
    });

    let strike_tickers: Vec<String> = reports
        .iter()
        .filter(|r| {
            matches!(
                r.status,
                crate::model::StrikeZoneStatus::InStrikeZone
                    | crate::model::StrikeZoneStatus::NearStrikeZone
            )
        })
        .take(8)
        .map(|r| r.ticker.clone())
        .collect();
    let strike_summary = if strike_tickers.is_empty() {
        lang.t("暂无接近或进入击球区标的", "none in/near strike zone")
            .to_owned()
    } else {
        strike_tickers.join(", ")
    };

    let (positions_n, cash_label) = journal.as_ref().map_or((0, "—".to_owned()), |j| {
        (
            j.positions.len(),
            format!("{} {}", j.cash_balance.round_dp(2).normalize(), j.currency),
        )
    });

    match lang {
        UiLang::Zh => format!(
            "[当前研究范围：全局能力圈组合 | 观察池 IN={core_n}, WATCH={watch_n}, OUT={out_n}, TOO_HARD={hard_n} | 击球雷达：{strike_summary} | 模拟持仓 {positions_n} 笔 · 现金 {cash_label} | 行业研究 {} 个]\n\
             请调用 `m3_status`、`m3_universe_query`、`m3_industry_matrix` 或读取只读 `/mnt/m3/watchlist.json`、`/mnt/m3/journal.json`、`/mnt/m3/industries/index.json` 核对全盘数据；引用标的时标注 `[report:TICKER]`、`[xbrl:TICKER]`、`[dossier:TICKER]`，严禁篡改 `watchlist.json`、`journal.json` 或 `human_judgment`。",
            industries.len()
        ),
        UiLang::En => format!(
            "[Active Scope: Circle Portfolio | Watchlist IN={core_n}, WATCH={watch_n}, OUT={out_n}, TOO_HARD={hard_n} | Strike Radar: {strike_summary} | Paper Positions: {positions_n} · Cash {cash_label} | Industry Studies: {}]\n\
             Call `m3_status`, `m3_universe_query`, `m3_industry_matrix`, or read `/mnt/m3/watchlist.json`, `/mnt/m3/journal.json`, `/mnt/m3/industries/index.json`; cite `[report:TICKER]`, `[xbrl:TICKER]`, `[dossier:TICKER]`. Never mutate `watchlist.json`, `journal.json`, or `human_judgment`.",
            industries.len()
        ),
    }
}

/// Builds and boots Project-M3's isolated `EmbeddedAlan` V2 instance.
pub async fn boot_m3_embedded(
    paths: WorkspacePaths,
    fs: Arc<M3WorkbenchFs>,
    llm: LlmSource,
) -> Result<EmbeddedAlan> {
    let mut tools = ToolRegistry::new();
    tools.register(M3StatusTool {
        paths: paths.clone(),
    });
    tools.register(M3ReportTool {
        paths: paths.clone(),
    });
    tools.register(M3DossierTool {
        paths: paths.clone(),
    });
    tools.register(M3UpdateDraftTool {
        paths: paths.clone(),
        fs: Arc::clone(&fs),
    });
    tools.register(M3UniverseQueryTool {
        paths: paths.clone(),
    });
    tools.register(M3PeerPercentilesTool {
        paths: paths.clone(),
    });
    tools.register(M3SemanticNeighborsTool {
        paths: paths.clone(),
    });
    tools.register(M3EventsForTickerTool {
        paths: paths.clone(),
    });
    tools.register(M3IndustryMatrixTool { paths });
    tools.register(M3WebSearchTool);

    let process = AgentProcessConfig {
        trust_posture: TrustPosture::ConfinedTrusted,
        ..Default::default()
    };

    let config = EmbeddedConfig::ephemeral(llm)
        .with_process_config(process)
        .with_mount("/mnt/m3", fs as Arc<dyn FileServer>, Access::ReadOnly)
        .with_tools(tools)
        .with_tool_policy_hook(Arc::new(M3ToolPolicyHook))
        .with_builtin_skills(false)
        .with_system_prompt_mode(SystemPromptMode::Replace {
            prompt: M3_INTERROGATOR_SYSTEM_PROMPT.to_string(),
        });

    EmbeddedAlan::boot(config).await
}

pub fn friendly_m3_credentials_error(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("no connection service profile")
        || lower.contains("connection profile")
        || lower.contains("missing secret")
        || lower.contains("api key")
        || lower.contains("unconfigured")
        || lower.contains("not configured")
    {
        return "尚未配置 Alan 模型连接（不影响计算器底表与手动档案）。\n\
                可在终端运行：\n  \
                alan connection add openrouter --profile main\n  \
                alan connection set-secret main\n  \
                alan connection default set main\n\
                或直接点击右上角「复制审讯 Prompt (p)」交由外部会话运行。"
            .to_string();
    }
    raw.to_string()
}

/// Parses clickable Acme evidence tokens such as `[report:AAPL]`, `[xbrl:AAPL]`,
/// `[xbrl]`, and `[dossier:AAPL]` from Alan's response, anchored to real `MungerReport`s.
pub fn parse_acme_evidence_tokens(
    raw: &str,
    default_ticker: &str,
    reports: &[MungerReport],
    dossiers: &HashMap<String, Option<Dossier>>,
) -> (String, Vec<AcmeEvidenceToken>) {
    let mut cleaned = String::with_capacity(raw.len());
    let mut tokens = Vec::new();
    let mut seen = BTreeSet::new();
    let mut rest = raw;

    while let Some(start) = rest.find('[') {
        cleaned.push_str(&rest[..start]);
        let after_open = &rest[start + 1..];
        if let Some(end) = after_open.find(']') {
            let inner = after_open[..end].trim();
            if let Some((kind, ticker)) = parse_bracket_token(inner, default_ticker) {
                let id = format!("{}:{ticker}", kind.prefix());
                if seen.insert(id.clone()) {
                    tokens.push(build_evidence_token(
                        kind.clone(),
                        &ticker,
                        reports,
                        dossiers,
                    ));
                }
                cleaned.push_str(&format!("[{}:{ticker}]", kind.prefix()));
            } else {
                cleaned.push_str(&rest[start..start + 1 + end + 1]);
            }
            rest = &after_open[end + 1..];
        } else {
            cleaned.push('[');
            rest = after_open;
        }
    }
    cleaned.push_str(rest);

    // Ensure any mentioned known ticker also carries its clickable [report:TICKER] / [xbrl:TICKER].
    for rep in reports {
        if contains_ticker_word(&cleaned, &rep.ticker) {
            let id = format!("report:{}", rep.ticker);
            if seen.insert(id) {
                tokens.push(build_evidence_token(
                    AcmeEvidenceKind::Report,
                    &rep.ticker,
                    reports,
                    dossiers,
                ));
            }
        }
    }

    (cleaned, tokens)
}

fn parse_bracket_token(inner: &str, default_ticker: &str) -> Option<(AcmeEvidenceKind, String)> {
    let lower = inner.to_ascii_lowercase();
    if lower == "xbrl" && !default_ticker.trim().is_empty() {
        return Some((AcmeEvidenceKind::Xbrl, default_ticker.to_ascii_uppercase()));
    }
    if lower == "report" && !default_ticker.trim().is_empty() {
        return Some((
            AcmeEvidenceKind::Report,
            default_ticker.to_ascii_uppercase(),
        ));
    }
    if lower == "dossier" && !default_ticker.trim().is_empty() {
        return Some((
            AcmeEvidenceKind::Dossier,
            default_ticker.to_ascii_uppercase(),
        ));
    }
    let (prefix, sym) = inner.split_once(':')?;
    let sym = sym.trim().to_ascii_uppercase();
    if sym.is_empty()
        || sym.len() > 16
        || !sym
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    match prefix.trim().to_ascii_lowercase().as_str() {
        "report" => Some((AcmeEvidenceKind::Report, sym)),
        "xbrl" => Some((AcmeEvidenceKind::Xbrl, sym)),
        "dossier" => Some((AcmeEvidenceKind::Dossier, sym)),
        _ => None,
    }
}

fn contains_ticker_word(haystack: &str, ticker: &str) -> bool {
    if ticker.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(ticker) {
        let abs = start + pos;
        let before_ok = haystack[..abs]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        let after_ok = haystack[abs + ticker.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = abs + ticker.len();
    }
    false
}

fn build_evidence_token(
    kind: AcmeEvidenceKind,
    ticker: &str,
    reports: &[MungerReport],
    dossiers: &HashMap<String, Option<Dossier>>,
) -> AcmeEvidenceToken {
    let rep = reports
        .iter()
        .find(|r| r.ticker.eq_ignore_ascii_case(ticker));
    let dossier = dossiers.get(ticker).and_then(|d| d.as_ref());

    match kind {
        AcmeEvidenceKind::Report => {
            let summary = if let Some(r) = rep {
                let val = r.ttm_valuation.as_ref().or(r.fy_valuation.as_ref());
                let strike = val
                    .and_then(|v| v.strike_price.map(|s| format!("${}", s.round_dp(2))))
                    .unwrap_or_else(|| "—".into());
                let roic = r
                    .roic_5y_avg_pct
                    .map(|v| format!("{:.1}%", v))
                    .unwrap_or_else(|| "—".into());
                format!(
                    "{} ({}) · 分桶 {} · 状态 {} · 5Y ROIC均值 {roic} · 击球价 {strike} · 预警 {} 项",
                    r.name,
                    r.ticker,
                    r.bucket.code(),
                    r.status.label(),
                    r.warnings.len()
                )
            } else {
                format!("暂未在计算报告中找到 {ticker}。")
            };
            AcmeEvidenceToken {
                id: format!("report:{ticker}"),
                kind,
                ticker: ticker.to_string(),
                label: format!("[report:{ticker}]"),
                summary,
            }
        }
        AcmeEvidenceKind::Xbrl => {
            let summary = if let Some(r) = rep {
                let filing = r.latest_filing_end.as_deref().unwrap_or("—");
                let prov = r
                    .provenance_summary
                    .first()
                    .map(|p| format!("{} · {}", p.field, p.source_tag))
                    .unwrap_or_else(|| "SEC EDGAR XBRL 底表".into());
                let sha = r
                    .raw_sha256
                    .as_deref()
                    .map(|s| s.chars().take(12).collect::<String>())
                    .unwrap_or_else(|| "manual".into());
                format!(
                    "{} XBRL 底表 · 最近财报期 {filing} · 样本溯源 {prov} · SHA {sha} · 点击可直达「3 XBRL 底表」",
                    r.ticker
                )
            } else {
                format!("点击前往「3 XBRL 底表」查看 {ticker} 原始财务溯源。")
            };
            AcmeEvidenceToken {
                id: format!("xbrl:{ticker}"),
                kind,
                ticker: ticker.to_string(),
                label: format!("[xbrl:{ticker}]"),
                summary,
            }
        }
        AcmeEvidenceKind::Dossier => {
            let summary = if let Some(d) = dossier {
                let review = if d.human_judgment.pending_human_review {
                    "human_judgment 待人类答辩拍板"
                } else {
                    "已完成人类答辩"
                };
                format!(
                    "{} 审讯档案（更新 {} · {review}）：{}",
                    d.ticker, d.updated_at, d.three_sentence_business
                )
            } else {
                format!("尚无 workbench/dossiers/{ticker}.json 草稿，可要求 Alan 起草。")
            };
            AcmeEvidenceToken {
                id: format!("dossier:{ticker}"),
                kind,
                ticker: ticker.to_string(),
                label: format!("[dossier:{ticker}]"),
                summary,
            }
        }
    }
}

/// Resolves clickable [`AcmeEvidenceToken`]s into deterministic, deduplicated
/// [`InlineEvidenceCard`]s using saved computed reports, watchlist entries,
/// dossiers, and read-only DuckDB universe snapshots (zero LLM calls).
pub fn resolve_inline_evidence_cards(
    paths: &WorkspacePaths,
    tokens: &[AcmeEvidenceToken],
) -> Vec<InlineEvidenceCard> {
    if tokens.is_empty() {
        return Vec::new();
    }

    struct GroupedTicker {
        ticker: String,
        source_kind: AcmeEvidenceKind,
        has_dossier_token: bool,
    }

    let mut grouped: Vec<GroupedTicker> = Vec::new();
    let mut ticker_idx: HashMap<String, usize> = HashMap::new();
    for tok in tokens {
        let upper = tok.ticker.trim().to_ascii_uppercase();
        if upper.is_empty() {
            continue;
        }
        if let Some(&idx) = ticker_idx.get(&upper) {
            if tok.kind == AcmeEvidenceKind::Report {
                grouped[idx].source_kind = AcmeEvidenceKind::Report;
            } else if tok.kind == AcmeEvidenceKind::Xbrl
                && grouped[idx].source_kind == AcmeEvidenceKind::Dossier
            {
                grouped[idx].source_kind = AcmeEvidenceKind::Xbrl;
            }
            if tok.kind == AcmeEvidenceKind::Dossier {
                grouped[idx].has_dossier_token = true;
            }
        } else {
            ticker_idx.insert(upper.clone(), grouped.len());
            grouped.push(GroupedTicker {
                ticker: upper,
                source_kind: tok.kind.clone(),
                has_dossier_token: tok.kind == AcmeEvidenceKind::Dossier,
            });
        }
    }

    if grouped.is_empty() {
        return Vec::new();
    }

    let watchlist = store::load_watchlist(paths).ok();
    let mut lake_rows: HashMap<String, UniverseRow> = HashMap::new();
    let mut lake_loaded = false;

    let mut cards = Vec::with_capacity(grouped.len());
    for item in grouped {
        let ticker = &item.ticker;
        let wl_entry = watchlist.as_ref().and_then(|wl| {
            wl.companies
                .iter()
                .find(|c| c.ticker.eq_ignore_ascii_case(ticker))
        });

        let report = store::read_json::<MungerReport>(&paths.computed_report_path(ticker))
            .ok()
            .or_else(|| {
                wl_entry
                    .is_some()
                    .then(|| store::load_or_recompute_ticker(paths, ticker).ok())
                    .flatten()
            });

        let dossier = store::load_dossier(paths, ticker).ok().flatten();
        let has_dossier = dossier.is_some() || item.has_dossier_token;

        if let Some(rep) = report {
            let val = rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref());
            let fcf_yield =
                val.and_then(|v| v.strict_fcf_yield_mcap_pct.or(v.strict_fcf_yield_ev_pct));
            cards.push(InlineEvidenceCard {
                ticker: rep.ticker,
                name: rep.name,
                bucket: Some(rep.bucket),
                currency: Some(rep.currency),
                latest_price: rep.price.as_ref().map(|p| p.price),
                strike_price: val.and_then(|v| v.strike_price),
                strike_gap_pct: val.and_then(|v| v.distance_to_strike_pct),
                roic_5y: rep.roic_5y_avg_pct,
                fcf_conversion_or_yield: fcf_yield,
                warning_count: rep.warnings.len(),
                source_kind: item.source_kind,
                has_dossier,
            });
            continue;
        }

        if !lake_loaded {
            lake_loaded = true;
            if let Ok(Some(db)) = LakeDb::open_read_only_if_exists(paths) {
                let all_tickers: Vec<String> = vec![ticker.clone()];
                if let Ok(rows) = db.load_universe_rows_by_tickers(&all_tickers) {
                    for r in rows {
                        lake_rows.insert(r.ticker.to_ascii_uppercase(), r);
                    }
                }
            }
        } else if !lake_rows.contains_key(ticker)
            && let Ok(Some(db)) = LakeDb::open_read_only_if_exists(paths)
            && let Ok(rows) = db.load_universe_rows_by_tickers(std::slice::from_ref(ticker))
        {
            for r in rows {
                lake_rows.insert(r.ticker.to_ascii_uppercase(), r);
            }
        }

        if let Some(row) = lake_rows.get(ticker) {
            cards.push(InlineEvidenceCard {
                ticker: row.ticker.clone(),
                name: wl_entry
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| row.name.clone()),
                bucket: wl_entry.map(|c| c.bucket).or(row.bucket_opt),
                currency: wl_entry
                    .map(|c| c.currency.clone())
                    .or(Some("USD".to_owned())),
                latest_price: row.latest_price,
                strike_price: row.strike_price,
                strike_gap_pct: row.distance_to_strike_pct,
                roic_5y: row.roic_5y_avg_pct,
                fcf_conversion_or_yield: row.strict_fcf_yield_pct,
                warning_count: row.warnings_count,
                source_kind: item.source_kind,
                has_dossier,
            });
            continue;
        }

        if let Some(company) = wl_entry {
            let price_snap = store::read_json::<PriceSnapshot>(&paths.price_path(ticker)).ok();
            cards.push(InlineEvidenceCard {
                ticker: company.ticker.clone(),
                name: company.name.clone(),
                bucket: Some(company.bucket),
                currency: Some(company.currency.clone()),
                latest_price: price_snap.map(|p| p.price),
                strike_price: None,
                strike_gap_pct: None,
                roic_5y: None,
                fcf_conversion_or_yield: None,
                warning_count: 0,
                source_kind: item.source_kind,
                has_dossier,
            });
            continue;
        }

        if let Some(d) = dossier {
            cards.push(InlineEvidenceCard {
                ticker: d.ticker,
                name: d.name,
                bucket: None,
                currency: None,
                latest_price: None,
                strike_price: None,
                strike_gap_pct: None,
                roic_5y: None,
                fcf_conversion_or_yield: None,
                warning_count: 0,
                source_kind: item.source_kind,
                has_dossier: true,
            });
        }
    }

    cards
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AnnualFinancials, Bucket, CompanyEntry, DataSource, DecisionJournal, FieldProvenance,
        PriceSnapshot, RawCompanyFactsSummary, ValuationAssumptions, WatchlistFile,
    };
    use alan_llm::{GenerationResponse, MockLlmProvider, TokenUsage, ToolCall};
    use alan_os_host::embed::LlmClient;
    use rust_decimal::Decimal;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    fn temp_m3_workspace(label: &str) -> (PathBuf, WorkspacePaths) {
        let dir = std::env::temp_dir().join(format!("m3-alan-test-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        std::fs::create_dir_all(&paths.workbench_dir).unwrap();
        std::fs::create_dir_all(paths.workbench_dir.join("dossiers")).unwrap();
        std::fs::create_dir_all(paths.data_dir.join("raw/sec")).unwrap();
        std::fs::create_dir_all(paths.data_dir.join("raw/prices")).unwrap();
        std::fs::create_dir_all(paths.data_dir.join("computed")).unwrap();

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "AAPL".into(),
                name: "Apple Inc.".into(),
                bucket: Bucket::Core,
                source: DataSource::SecEdgar,
                cik: Some("0000320193".into()),
                currency: "USD".into(),
                circle: "消费生态 / 硬件+服务".into(),
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
        store::save_watchlist(&paths, &wl).unwrap();

        let journal = DecisionJournal {
            schema_version: 1,
            account_note: "TEST".into(),
            currency: "USD".into(),
            starting_cash: Decimal::from(100_000),
            cash_balance: Decimal::from(100_000),
            benchmark_symbol: "^GSPC".into(),
            benchmark_start_price: None,
            positions: Vec::new(),
            entries: Vec::new(),
        };
        store::save_journal(&paths, &journal).unwrap();

        let facts = RawCompanyFactsSummary {
            schema_version: crate::edgar::SEC_FACTS_SCHEMA_VERSION,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "AAPL".into(),
            entity_name: "Apple Inc.".into(),
            cik: "0000320193".into(),
            source_url: "https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json".into(),
            raw_sha256: "deadbeef12345678".into(),
            fetched_at_ms: 1_700_000_000_000,
            annuals: vec![AnnualFinancials {
                fiscal_year: 2025,
                period_end: "2025-09-27".into(),
                filed: Some("2025-11-01".into()),
                revenue: Decimal::from(400_000_000_000i64),
                gross_profit: Decimal::from(185_000_000_000i64),
                ebit: Decimal::from(125_000_000_000i64),
                tax_expense: Decimal::from(20_000_000_000i64),
                pretax_income: Decimal::from(124_000_000_000i64),
                ocf: Decimal::from(118_000_000_000i64),
                capex: Decimal::from(10_000_000_000i64),
                sbc: Decimal::from(12_000_000_000i64),
                buybacks: Decimal::from(90_000_000_000i64),
                cash_and_st: Some(Decimal::from(65_000_000_000i64)),
                total_debt: Decimal::from(100_000_000_000i64),
                equity: Decimal::from(60_000_000_000i64),
                diluted_shares: Decimal::from(15_200_000_000i64),
                ending_shares: Decimal::from(15_000_000_000i64),
                provenance: vec![FieldProvenance {
                    field: "ocf".into(),
                    source_tag: "us-gaap:NetCashProvidedByUsedInOperatingActivities".into(),
                    filed: Some("2025-11-01".into()),
                    period_end: "2025-09-27".into(),
                    note: None,
                }],
            }],
            ttm: None,
            manual_note: None,
        };
        store::write_json_atomic(&paths.sec_facts_summary_path("AAPL"), &facts).unwrap();

        let price = PriceSnapshot {
            schema_version: 1,
            symbol: "AAPL".into(),
            price: Decimal::from(180),
            previous_close: Some(Decimal::from(179)),
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(1_700_000_000_000),
            fetched_at_ms: 1_700_000_000_000,
            source_url: "https://query1.finance.yahoo.com/v8/finance/chart/AAPL".into(),
        };
        store::write_json_atomic(&paths.price_path("AAPL"), &price).unwrap();

        let dossier = Dossier {
            schema_version: 1,
            ticker: "AAPL".into(),
            name: "Apple Inc.".into(),
            updated_at: "2026-11-18".into(),
            draft_source: "manual_seed".into(),
            three_sentence_business: "端到端软硬件生态锁定高留存用户并用回购放大每股股东盈余。"
                .into(),
            latticework: Latticework {
                microeconomics: "生态转换成本高".into(),
                ecology: "端侧 AI 与服务黏性".into(),
                psychology_incentives: "以每股自由现金流为核心配资".into(),
                capital_allocation: "持续净回购压缩股本".into(),
            },
            premortem: vec!["硬件换机周期拉长叠加强监管拆分 App Store".into()],
            open_questions: vec!["服务业务毛利率能否维持 70% 以上？".into()],
            human_judgment: HumanJudgment {
                pending_human_review: true,
                my_stance: Some("WAITING_FOR_STRIKE_150".into()),
                weight_rationale: Some("核心能力圈保留".into()),
                falsification_metrics: vec!["活跃设备增长转负".into()],
                feynman_checked_at: Some("2026-11-18".into()),
            },
        };
        store::write_json_atomic(&paths.dossier_path("AAPL"), &dossier).unwrap();

        (dir, paths)
    }

    fn tool_call_response(id: &str, name: &str, arguments: Value) -> GenerationResponse {
        GenerationResponse {
            content: String::new(),
            thinking: None,
            thinking_signature: None,
            redacted_thinking: Vec::new(),
            tool_calls: vec![ToolCall {
                id: Some(id.to_string()),
                name: name.to_string(),
                arguments,
            }],
            usage: Some(TokenUsage {
                prompt_tokens: 16,
                cached_prompt_tokens: None,
                completion_tokens: 8,
                total_tokens: 24,
                reasoning_tokens: None,
                ..Default::default()
            }),
            finish_reason: None,
            warnings: Vec::new(),
            provider_response_id: None,
            provider_response_status: None,
        }
    }

    fn text_response(text: &str) -> GenerationResponse {
        GenerationResponse {
            content: text.to_string(),
            thinking: None,
            thinking_signature: None,
            redacted_thinking: Vec::new(),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                prompt_tokens: 24,
                cached_prompt_tokens: None,
                completion_tokens: 18,
                total_tokens: 42,
                reasoning_tokens: None,
                ..Default::default()
            }),
            finish_reason: None,
            warnings: Vec::new(),
            provider_response_id: None,
            provider_response_status: None,
        }
    }

    #[test]
    fn m3_tool_policy_hook_denies_all_redline_paths_and_human_judgment() {
        let hook = M3ToolPolicyHook;

        assert!(matches!(
            hook.evaluate(
                "m3_update_draft",
                &json!({
                    "ticker": "AAPL",
                    "human_judgment": {"my_stance": "BUY_NOW"}
                }),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_update_draft",
                &json!({
                    "ticker": "AAPL",
                    "pending_human_review": false
                }),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "write_file",
                &json!({
                    "path": "data/munger/computed/AAPL.json",
                    "content": "{}"
                }),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "write_file",
                &json!({
                    "path": "data/munger/raw/sec/AAPL_facts.json",
                    "content": "{}"
                }),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "edit_file",
                &json!({
                    "path": "workbench/journal.json",
                    "patch": "tamper"
                }),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_update_draft",
                &json!({
                    "ticker": "AAPL",
                    "open_questions": ["如果服务毛利跌破 65%，估值中枢下移多少？"]
                }),
                ToolCapability::Write
            ),
            HookDecision::Allow
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_report",
                &json!({"ticker": "AAPL"}),
                ToolCapability::Read
            ),
            HookDecision::Allow
        ));

        for forbidden_path in [
            "data/munger/lake/m3.duckdb",
            "lake/m3.duckdb",
            "data/munger/lake/parquet/companies.parquet",
            "parquet/facts_annual.parquet",
            "workbench/watchlist.json",
            "workbench/budget.json",
            "workbench/themes.json",
            "workbench/memos/AAPL.md",
            "memos/ADSK.md",
            "data/munger/something.json",
        ] {
            assert!(
                matches!(
                    hook.evaluate(
                        "write_file",
                        &json!({"path": forbidden_path, "content": "x"}),
                        ToolCapability::Write
                    ),
                    HookDecision::Deny(_)
                ),
                "expected deny for {forbidden_path}"
            );
            assert!(
                matches!(
                    hook.evaluate(
                        "read_file",
                        &json!({"path": forbidden_path}),
                        ToolCapability::Read
                    ),
                    HookDecision::Deny(_)
                ),
                "expected read-arg deny for {forbidden_path}"
            );
        }

        assert!(matches!(
            hook.evaluate(
                "m3_update_draft",
                &json!({"ticker": "AAPL", "extra_field": "bad"}),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "web_fetch",
                &json!({"url": "https://example.com"}),
                ToolCapability::Network
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate("mystery_tool", &json!({}), ToolCapability::Unknown),
            HookDecision::Deny(_)
        ));

        // Normal read queries on Lakehouse tools should pass without false positives.
        assert!(matches!(
            hook.evaluate(
                "m3_semantic_neighbors",
                &json!({"query": "CAD software subscription business", "limit": 10}),
                ToolCapability::Read
            ),
            HookDecision::Allow
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_universe_query",
                &json!({"min_roic_5y": 15, "exclude_financials": true, "limit": 20}),
                ToolCapability::Read
            ),
            HookDecision::Allow
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_peer_percentiles",
                &json!({"ticker": "ADSK", "limit": 10}),
                ToolCapability::Read
            ),
            HookDecision::Allow
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_events_for_ticker",
                &json!({"ticker": "ADSK"}),
                ToolCapability::Read
            ),
            HookDecision::Allow
        ));
    }

    #[test]
    fn embedded_m3_runs_report_and_dossier_and_blocks_redline_writes_in_process() {
        let handle = std::thread::Builder::new()
            .name("m3-test-embedded".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async {
        let (dir, paths) = temp_m3_workspace("e2e");
        let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));

        let provider = MockLlmProvider::new().with_responses(vec![
            tool_call_response("call_rep", "m3_report", json!({"ticker": "AAPL"})),
            tool_call_response("call_dos", "m3_dossier", json!({"ticker": "AAPL"})),
            // Blocked write attempt 1: touch human_judgment
            tool_call_response(
                "call_bad_judgment",
                "m3_update_draft",
                json!({
                    "ticker": "AAPL",
                    "human_judgment": {"my_stance": "FORCE_BUY", "pending_human_review": false}
                }),
            ),
            // Blocked write attempt 2: touch computed/*
            tool_call_response(
                "call_bad_computed",
                "write_computed",
                json!({
                    "path": "computed/AAPL.json",
                    "value": 999
                }),
            ),
            // Allowed AI draft update (must preserve existing human_judgment!)
            tool_call_response(
                "call_good_draft",
                "m3_update_draft",
                json!({
                    "ticker": "AAPL",
                    "open_questions": ["服务增速放缓时回购收益率还能支撑几倍估值？"]
                }),
            ),
            text_response(
                "已核对 AAPL 确定性报告 [report:AAPL]、XBRL 溯源 [xbrl:AAPL] 与审讯档案 [dossier:AAPL]。",
            ),
        ]);

        // Also register an extra dummy write tool to prove hook blocks non-draft write tools too.
        struct DummyComputedWriteTool(Arc<AtomicBool>);
        impl Tool for DummyComputedWriteTool {
            fn name(&self) -> &str {
                "write_computed"
            }
            fn description(&self) -> &str {
                "test-only forbidden write tool"
            }
            fn parameters_schema(&self) -> Value {
                json!({"type": "object"})
            }
            fn capability(&self, _arguments: &Value) -> ToolCapability {
                ToolCapability::Write
            }
            fn execute(&self, _arguments: Value, _ctx: &ToolContext) -> ToolResult {
                self.0.store(true, Ordering::SeqCst);
                Box::pin(async { Ok(json!({"hacked": true})) })
            }
        }

        let forbidden_ran = Arc::new(AtomicBool::new(false));
        let mut tools = ToolRegistry::new();
        tools.register(M3StatusTool {
            paths: paths.clone(),
        });
        tools.register(M3ReportTool {
            paths: paths.clone(),
        });
        tools.register(M3DossierTool {
            paths: paths.clone(),
        });
        tools.register(M3UpdateDraftTool {
            paths: paths.clone(),
            fs: Arc::clone(&fs),
        });
        tools.register(DummyComputedWriteTool(Arc::clone(&forbidden_ran)));

        let process = AgentProcessConfig {
            trust_posture: TrustPosture::ConfinedTrusted,
            ..Default::default()
        };

        let app = EmbeddedAlan::boot(
            EmbeddedConfig::ephemeral(LlmSource::Client(LlmClient::new(provider.clone())))
                .with_process_config(process)
                .with_mount("/mnt/m3", Arc::clone(&fs) as Arc<dyn FileServer>, Access::ReadOnly)
                .with_tools(tools)
                .with_tool_policy_hook(Arc::new(M3ToolPolicyHook))
                .with_builtin_skills(false)
                .with_system_prompt_mode(SystemPromptMode::Replace {
                    prompt: M3_INTERROGATOR_SYSTEM_PROMPT.to_string(),
                }),
        )
        .await
        .expect("boot m3 embedded");

        let client = app.root_agent();
        let mut stream = client.event_stream().await.unwrap();
        let sub_id = client
            .submit_text("请审讯 AAPL 的确定性财务与档案草稿")
            .await
            .unwrap();

        let mut finished_tools = Vec::new();
        let mut prose = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let ev = tokio::time::timeout_at(deadline, stream.next_event())
                .await
                .expect("timed out waiting for turn finish")
                .expect("event stream ok");
            match ev {
                EmbedEvent::TextDelta { chunk, .. } => prose.push_str(&chunk),
                EmbedEvent::ToolFinished {
                    name,
                    status,
                    preview,
                    ..
                } => {
                    finished_tools.push((name, status, preview.unwrap_or_default()));
                }
                EmbedEvent::TurnFinished {
                    submission_id,
                    status,
                    ..
                } if submission_id == sub_id => {
                    assert_eq!(status, UiInputStatus::Completed);
                    break;
                }
                _ => {}
            }
        }

        // Verify deterministic m3_report and m3_dossier ran cleanly.
        assert!(finished_tools
            .iter()
            .any(|(name, status, _)| name == "m3_report" && status == "completed"));
        assert!(finished_tools
            .iter()
            .any(|(name, status, _)| name == "m3_dossier" && status == "completed"));
        assert!(finished_tools
            .iter()
            .any(|(name, status, _)| name == "m3_update_draft" && status == "completed"));

        // Verify red-line hook prevented both write attempts cleanly.
        assert!(!forbidden_ran.load(Ordering::SeqCst));
        let recorded = provider.recorded_requests();
        assert!(
            recorded.iter().any(|req| {
                req.messages
                    .iter()
                    .any(|m| m.content.contains("红线拦截") && m.content.contains("human_judgment"))
            }),
            "expected ToolPolicyHook denial feedback for human_judgment"
        );
        assert!(
            recorded.iter().any(|req| {
                req.messages
                    .iter()
                    .any(|m| m.content.contains("红线拦截") && m.content.contains("computed/"))
            }),
            "expected ToolPolicyHook denial feedback for computed/"
        );

        // Verify human_judgment on disk stayed completely intact while open_questions updated.
        let disk_dossier = store::load_dossier(&paths, "AAPL").unwrap().unwrap();
        assert_eq!(
            disk_dossier.human_judgment.my_stance.as_deref(),
            Some("WAITING_FOR_STRIKE_150")
        );
        assert!(disk_dossier.human_judgment.pending_human_review);
        assert_eq!(
            disk_dossier.open_questions,
            vec!["服务增速放缓时回购收益率还能支撑几倍估值？".to_string()]
        );

        // Verify Acme evidence parser picks up clickable report/xbrl/dossier tokens.
        let reports = store::load_or_compute_reports(&paths).unwrap();
        let mut dossier_map = HashMap::new();
        dossier_map.insert("AAPL".to_string(), Some(disk_dossier));
        let (cleaned, tokens) = parse_acme_evidence_tokens(&prose, "AAPL", &reports, &dossier_map);
        assert!(cleaned.contains("[report:AAPL]"));
        assert!(tokens.iter().any(|t| t.id == "report:AAPL"));
        assert!(tokens.iter().any(|t| t.id == "xbrl:AAPL"));
        assert!(tokens.iter().any(|t| t.id == "dossier:AAPL"));

        let _ = app.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
                });
            })
            .expect("spawn test thread");
        handle.join().expect("embedded m3 test panicked");
    }

    #[test]
    fn parse_acme_evidence_tokens_supports_shorthand_xbrl_and_grounded_summaries() {
        let (dir, paths) = temp_m3_workspace("tokens");
        let reports = store::load_or_compute_reports(&paths).unwrap();
        let dossier = store::load_dossier(&paths, "AAPL").unwrap();
        let mut dossiers = HashMap::new();
        dossiers.insert("AAPL".to_string(), dossier);

        let (cleaned, tokens) = parse_acme_evidence_tokens(
            "参考 AAPL 现金流与 [xbrl] 数据",
            "AAPL",
            &reports,
            &dossiers,
        );
        assert_eq!(cleaned, "参考 AAPL 现金流与 [xbrl:AAPL] 数据");
        assert!(
            tokens
                .iter()
                .any(|t| t.id == "xbrl:AAPL" && t.summary.contains("XBRL 底表"))
        );
        assert!(
            tokens
                .iter()
                .any(|t| t.id == "report:AAPL" && t.summary.contains("Apple Inc."))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_m3_executes_lakehouse_readonly_tools_and_blocks_lake_and_budget_writes() {
        let handle = std::thread::Builder::new()
            .name("m3-test-lake-tools".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async {
        use crate::lake::{LakeCompanyRow, UniverseRow};
        use crate::lake_l1::{DocChunkRow, LakeEventRow, LakeFilingRow};
        use crate::latent::{
            LOCAL_EMBEDDING_DIM, LOCAL_EMBEDDING_MODEL, encode_embedding_json, local_embed,
            sha256_hex,
        };

        let (dir, paths) = temp_m3_workspace("lake_tools");
        {
            let mut db = LakeDb::open(&paths).unwrap();
            let mk_row = |ticker: &str, cik: &str, name: &str, roic: i64| UniverseRow {
                ticker: ticker.to_owned(),
                cik: cik.to_owned(),
                name: name.to_owned(),
                exchange: "NASDAQ".to_owned(),
                sic: Some(7372),
                sic_description: Some("Prepackaged Software".to_owned()),
                is_financial: false,
                latest_filing_end: Some("2026-01-31".to_owned()),
                years_of_data: 8,
                quality_score: Decimal::from(90),
                in_price_pool: true,
                in_watchlist: ticker == "AAPL",
                bucket_opt: (ticker == "AAPL").then_some(Bucket::Core),
                latest_price: Some(Decimal::from(180)),
                strike_price: Some(Decimal::from(200)),
                distance_to_strike_pct: Some(Decimal::from(-10)),
                strict_fcf_yield_pct: Some(Decimal::from(5)),
                roic_5y_avg_pct: Some(Decimal::from(roic)),
                roic_5y_stddev_pct: Some(Decimal::from(2)),
                gross_margin_5y_avg_pct: Some(Decimal::from(75)),
                gross_margin_5y_stddev_pct: Some(Decimal::from(1)),
                share_change_3y_pct: Some(Decimal::from(-2)),
                sbc_to_ocf_pct: Some(Decimal::from(8)),
                net_debt_to_oe: Some(Decimal::ZERO),
                strict_owner_earnings: Some(Decimal::from(2_000_000_000i64)),
                warnings_count: 0,
                warnings_json: "[]".to_owned(),
                raw_sha256: Some("abc123".to_owned()),
            };
            let rows = vec![
                mk_row("ADSK", "0000769397", "Autodesk Inc.", 42),
                mk_row("PTC", "0000857005", "PTC Inc.", 21),
                mk_row("CDNS", "0000813672", "Cadence Design", 36),
                mk_row("SNPS", "0000883241", "Synopsys Inc.", 28),
            ];
            let companies: Vec<LakeCompanyRow> = rows
                .iter()
                .map(|r| LakeCompanyRow {
                    cik: r.cik.clone(),
                    ticker: r.ticker.clone(),
                    name: r.name.clone(),
                    exchange: r.exchange.clone(),
                    sic: r.sic,
                    sic_description: r.sic_description.clone(),
                    is_financial: false,
                    is_active: true,
                })
                .collect();
            db.replace_sec_lake_data(&companies, &[], &[], &rows).unwrap();
            db.upsert_filings(&[LakeFilingRow {
                accession_no: "0000769397-26-000001".to_owned(),
                cik: "0000769397".to_owned(),
                ticker: "ADSK".to_owned(),
                form: "10-K".to_owned(),
                filing_date: "2026-03-15".to_owned(),
                report_date: Some("2026-01-31".to_owned()),
                primary_document: Some("adsk10k.htm".to_owned()),
                primary_doc_description: Some("Form 10-K".to_owned()),
                items_json: "[]".to_owned(),
                filing_url: "https://www.sec.gov/Archives/edgar/data/769397/adsk10k.htm".to_owned(),
                ingested_at_ms: 1_795_000_000_000,
            }])
            .unwrap();
            db.upsert_events(&[LakeEventRow {
                event_id: "ev-adsk-1".to_owned(),
                source: "sec_edgar".to_owned(),
                cik: Some("0000769397".to_owned()),
                ticker: Some("ADSK".to_owned()),
                event_type: "10-K".to_owned(),
                occurred_at_ms: 1_795_000_000_000,
                filing_date: Some("2026-03-15".to_owned()),
                title: "ADSK Annual Report 10-K".to_owned(),
                summary: "Subscription CAD and construction software".to_owned(),
                url: "https://www.sec.gov/Archives/edgar/data/769397/adsk10k.htm".to_owned(),
                metadata_json: "{}".to_owned(),
                processed_mask: 0,
            }])
            .unwrap();

            let mk_chunk = |id: &str, cik: &str, ticker: &str, text: &str| DocChunkRow {
                chunk_id: id.to_owned(),
                cik: cik.to_owned(),
                ticker: ticker.to_owned(),
                doc_type: "10-K".to_owned(),
                source_id: "0000769397-26-000001".to_owned(),
                period_end: Some("2026-01-31".to_owned()),
                filing_date: Some("2026-03-15".to_owned()),
                section: "item_1".to_owned(),
                title: format!("{ticker} Business"),
                content: text.to_owned(),
                content_sha256: sha256_hex(text.as_bytes()),
                source_url: format!("https://www.sec.gov/Archives/edgar/data/{cik}/{id}.htm"),
                embedding_model: LOCAL_EMBEDDING_MODEL.to_owned(),
                embedding_dim: LOCAL_EMBEDDING_DIM,
                embedding_json: encode_embedding_json(&local_embed(text)),
            };
            db.upsert_doc_chunks(&[
                mk_chunk(
                    "chk-adsk-1",
                    "0000769397",
                    "ADSK",
                    "Autodesk builds engineering CAD and BIM subscription software for architecture and manufacturing.",
                ),
                mk_chunk(
                    "chk-ptc-1",
                    "0000857005",
                    "PTC",
                    "PTC provides industrial CAD and PLM subscription software for discrete manufacturing.",
                ),
            ])
            .unwrap();
        }

        let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
        let provider = MockLlmProvider::new().with_responses(vec![
            tool_call_response(
                "call_uq",
                "m3_universe_query",
                json!({"min_roic_5y": 20, "exclude_financials": true, "limit": 10}),
            ),
            tool_call_response(
                "call_pp",
                "m3_peer_percentiles",
                json!({"ticker": "ADSK", "limit": 5}),
            ),
            tool_call_response(
                "call_sn",
                "m3_semantic_neighbors",
                json!({"ticker": "ADSK", "limit": 5}),
            ),
            tool_call_response(
                "call_ev",
                "m3_events_for_ticker",
                json!({"ticker": "ADSK", "filings_limit": 5, "events_limit": 5, "chunks_limit": 5}),
            ),
            // Blocked writes against DuckDB, Parquet, watchlist.json, journal.json, budget.json, human_judgment
            tool_call_response(
                "call_bad_duck",
                "write_forbidden",
                json!({"path": "data/munger/lake/m3.duckdb", "sql": "DROP TABLE research_runs"}),
            ),
            tool_call_response(
                "call_bad_parquet",
                "write_forbidden",
                json!({"path": "data/munger/lake/parquet/companies.parquet"}),
            ),
            tool_call_response(
                "call_bad_watch",
                "write_forbidden",
                json!({"path": "workbench/watchlist.json"}),
            ),
            tool_call_response(
                "call_bad_journal",
                "write_forbidden",
                json!({"path": "workbench/journal.json"}),
            ),
            tool_call_response(
                "call_bad_budget",
                "write_forbidden",
                json!({"path": "workbench/budget.json"}),
            ),
            tool_call_response(
                "call_bad_hj",
                "m3_update_draft",
                json!({"ticker": "AAPL", "human_judgment": {"my_stance": "BUY"}}),
            ),
            text_response("已通过只读湖仓工具完成 ADSK 同行与语义近邻检索，未触碰任何红线文件。"),
        ]);

        struct DummyForbiddenWriteTool(Arc<AtomicBool>);
        impl Tool for DummyForbiddenWriteTool {
            fn name(&self) -> &str {
                "write_forbidden"
            }
            fn description(&self) -> &str {
                "test-only forbidden write tool"
            }
            fn parameters_schema(&self) -> Value {
                json!({"type": "object"})
            }
            fn capability(&self, _arguments: &Value) -> ToolCapability {
                ToolCapability::Write
            }
            fn execute(&self, _arguments: Value, _ctx: &ToolContext) -> ToolResult {
                self.0.store(true, Ordering::SeqCst);
                Box::pin(async { Ok(json!({"hacked": true})) })
            }
        }

        let forbidden_ran = Arc::new(AtomicBool::new(false));
        let mut tools = ToolRegistry::new();
        tools.register(M3StatusTool { paths: paths.clone() });
        tools.register(M3ReportTool { paths: paths.clone() });
        tools.register(M3DossierTool { paths: paths.clone() });
        tools.register(M3UpdateDraftTool {
            paths: paths.clone(),
            fs: Arc::clone(&fs),
        });
        tools.register(M3UniverseQueryTool { paths: paths.clone() });
        tools.register(M3PeerPercentilesTool { paths: paths.clone() });
        tools.register(M3SemanticNeighborsTool { paths: paths.clone() });
        tools.register(M3EventsForTickerTool { paths: paths.clone() });
        tools.register(DummyForbiddenWriteTool(Arc::clone(&forbidden_ran)));

        let app = EmbeddedAlan::boot(
            EmbeddedConfig::ephemeral(LlmSource::Client(LlmClient::new(provider.clone())))
                .with_process_config(AgentProcessConfig {
                    trust_posture: TrustPosture::ConfinedTrusted,
                    ..Default::default()
                })
                .with_mount("/mnt/m3", Arc::clone(&fs) as Arc<dyn FileServer>, Access::ReadOnly)
                .with_tools(tools)
                .with_tool_policy_hook(Arc::new(M3ToolPolicyHook))
                .with_builtin_skills(false)
                .with_system_prompt_mode(SystemPromptMode::Replace {
                    prompt: M3_INTERROGATOR_SYSTEM_PROMPT.to_string(),
                }),
        )
        .await
        .expect("boot m3 embedded");

        let client = app.root_agent();
        let mut stream = client.event_stream().await.unwrap();
        let sub_id = client
            .submit_text("查询高 ROIC 公司并分析 ADSK 的同行与事件")
            .await
            .unwrap();

        let mut finished_tools: Vec<(String, String, String)> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let ev = tokio::time::timeout_at(deadline, stream.next_event())
                .await
                .expect("timed out waiting for turn finish")
                .expect("event stream ok");
            match ev {
                EmbedEvent::ToolFinished {
                    name,
                    status,
                    preview,
                    ..
                } => {
                    finished_tools.push((name, status, preview.unwrap_or_default()));
                }
                EmbedEvent::TurnFinished {
                    submission_id,
                    status,
                    ..
                } if submission_id == sub_id => {
                    assert_eq!(status, UiInputStatus::Completed);
                    break;
                }
                _ => {}
            }
        }

        for tool_name in [
            "m3_universe_query",
            "m3_peer_percentiles",
            "m3_semantic_neighbors",
            "m3_events_for_ticker",
        ] {
            assert!(
                finished_tools
                    .iter()
                    .any(|(n, st, _)| n == tool_name && st == "completed"),
                "expected {tool_name} to complete cleanly, got {finished_tools:?}"
            );
        }

        assert!(!forbidden_ran.load(Ordering::SeqCst));
        let recorded = provider.recorded_requests();
        for marker in [
            "m3.duckdb",
            "parquet/",
            "watchlist.json",
            "journal.json",
            "budget.json",
            "human_judgment",
        ] {
            assert!(
                recorded.iter().any(|req| {
                    req.messages
                        .iter()
                        .any(|m| m.content.contains("红线拦截") && m.content.contains(marker))
                }),
                "expected ToolPolicyHook denial feedback for {marker}"
            );
        }

        let _ = app.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
                });
            })
            .expect("spawn test thread");
        handle.join().expect("embedded lake tools test panicked");
    }

    #[test]
    fn policy_hook_allows_valid_web_search_and_blocks_industry_writes_and_other_network_tools() {
        let hook = M3ToolPolicyHook;
        assert!(matches!(
            hook.evaluate(
                "m3_web_search",
                &json!({"query": "EDA semiconductor software industries profit pool", "limit": 4}),
                ToolCapability::Network
            ),
            HookDecision::Allow
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_web_search",
                &json!({"query": "  "}),
                ToolCapability::Network
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "arbitrary_fetch",
                &json!({"url": "https://example.com"}),
                ToolCapability::Network
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "write_file",
                &json!({"path": "workbench/industries/eda.json"}),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "m3_update_draft",
                &json!({"ticker": "SNPS", "multiple_low": 15}),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
    }

    #[test]
    fn parse_ddg_html_results_decodes_uddg_urls_and_strips_tags() {
        let html = r#"
        <div class="result">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FElectronic_design_automation&amp;rut=1"><b>EDA</b> &amp; Duopoly</a>
          <a class="result__snippet" href="//duckduckgo.com/l/?uddg=...">Synopsys &amp; Cadence share &gt;70% of <em>core EDA</em>.</a>
        </div>
        <div class="result">
          <a class="result__a" href="https://sec.gov/example">SEC Filing Note</a>
          <div class="result__snippet">Direct URL item</div>
        </div>
        "#;
        let hits = parse_ddg_html_results(html, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "EDA & Duopoly");
        assert_eq!(
            hits[0].url,
            "https://en.wikipedia.org/wiki/Electronic_design_automation"
        );
        assert_eq!(
            hits[0].snippet,
            "Synopsys & Cadence share >70% of core EDA."
        );
        assert_eq!(hits[1].title, "SEC Filing Note");
        assert_eq!(hits[1].url, "https://sec.gov/example");
    }

    #[test]
    fn expanded_mnt_m3_exposes_industries_memos_briefings_events_read_only() {
        let handle = std::thread::Builder::new()
            .name("m3-test-mnt-expanded".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async {
                    use crate::industry::IndustryStatus;
                    use crate::lake_l1::{
                        SignalEvidence, SignalInboxRow, SignalKind, SignalStatus,
                    };
                    use crate::proactive::{
                        BriefingPeriod, CircleBriefing, SignalTriageRecord, StrikeRadarItem,
                        TriageQuote, TriageVerdict,
                    };

                    let (dir, paths) = temp_m3_workspace("mnt_expanded");

                    // 1. Seed industry + memos
                    let profile = IndustryProfile {
                        slug: "retail".to_owned(),
                        title: "Membership Warehouse Retail".to_owned(),
                        status: IndustryStatus::InsideCircle,
                        sic_codes: vec![5331],
                        keywords: vec!["club".to_owned()],
                        bound_tickers: vec!["AAPL".to_owned()],
                        multiple_low: Some(Decimal::from(18)),
                        multiple_high: Some(Decimal::from(24)),
                        key_kpis: vec!["Renewal Rate".to_owned()],
                        feynman_questions: vec!["Why do members renew at >90%?".to_owned()],
                        feynman_checked: vec![true],
                        human_notes: "Patiently wait for 20x".to_owned(),
                        created_at_ms: 1_700_000_000_000,
                        updated_at_ms: 1_700_000_100_000,
                        last_research_run_id: None,
                    };
                    industry::save_industry(&paths, &profile).unwrap();

                    let memos_dir = paths.workbench_dir.join("memos");
                    std::fs::create_dir_all(&memos_dir).unwrap();
                    std::fs::write(memos_dir.join("AAPL.md"), "# AAPL Memo\nCore moat.").unwrap();
                    std::fs::write(
                        memos_dir.join("AAPL-feynman.md"),
                        "# AAPL Feynman Defense\nRound 1 passed.",
                    )
                    .unwrap();

                    // First verify graceful fallback when DuckDB does not exist yet.
                    async fn read_vfs(fs: &M3WorkbenchFs, path: &[&str]) -> Vec<u8> {
                        let parts: Vec<String> = path.iter().map(|s| (*s).to_string()).collect();
                        let fid = Fid(10);
                        fs.walk(Fid::ROOT, fid, &parts).await.unwrap();
                        fs.open(fid, OpenMode::Read).await.unwrap();
                        let bytes = fs.read(fid, 0, 1_048_576).await.unwrap();
                        fs.clunk(fid).await.unwrap();
                        bytes
                    }
                    let fs_no_lake = M3WorkbenchFs::new(paths.clone());
                    let fallback_briefing: Value = serde_json::from_slice(
                        &read_vfs(&fs_no_lake, &["briefings", "latest.json"]).await,
                    )
                    .unwrap();
                    assert_eq!(fallback_briefing["available"], false);
                    assert!(fallback_briefing["briefing"].is_null());

                    let fallback_events: Value = serde_json::from_slice(
                        &read_vfs(&fs_no_lake, &["events", "recent.json"]).await,
                    )
                    .unwrap();
                    assert_eq!(fallback_events["available"], false);
                    assert_eq!(fallback_events["count"], 0);

                    // 2. Populate DuckDB with briefing + >40 L1 signals + L1.5 triage.
                    {
                        let db = LakeDb::open(&paths).unwrap();
                        let briefing = CircleBriefing {
                            briefing_id: "br-2026-11-26".to_owned(),
                            period: BriefingPeriod::Daily,
                            period_key: "2026-11-26".to_owned(),
                            generated_at_ms: 1_795_000_000_000,
                            lang: UiLang::Zh,
                            is_quiet: false,
                            headline: "AAPL approaching strike zone".to_owned(),
                            executive_summary: Some("Quiet compounder update.".to_owned()),
                            summary_tokens_used: 120,
                            summary_cost_usd: Decimal::new(2, 3),
                            strike_radar: vec![StrikeRadarItem {
                                ticker: "AAPL".to_owned(),
                                name: "Apple Inc.".to_owned(),
                                bucket: Bucket::Core,
                                currency: "USD".to_owned(),
                                latest_price: Some(Decimal::from(180)),
                                strike_price: Some(Decimal::from(175)),
                                distance_to_strike_pct: Some(Decimal::new(286, 2)),
                                status: crate::model::StrikeZoneStatus::NearStrikeZone,
                                in_strike_zone: false,
                            }],
                            filing_deltas: Vec::new(),
                            watchdog_hits: Vec::new(),
                        };
                        db.upsert_circle_briefing(&briefing).unwrap();

                        let mut signals = Vec::new();
                        for idx in 0..45u64 {
                            signals.push(SignalInboxRow {
                                signal_id: format!("sig-{idx:03}"),
                                kind: SignalKind::Strike,
                                score: 90.0 - idx as f64 * 0.1,
                                primary_ticker: "AAPL".to_owned(),
                                tickers: vec!["AAPL".to_owned()],
                                title: format!("Signal {idx}"),
                                summary: "10-K filing delta".to_owned(),
                                evidence: vec![SignalEvidence {
                                    chunk_id: Some("chk-aapl-1".to_owned()),
                                    ticker: "AAPL".to_owned(),
                                    doc_type: "10-K".to_owned(),
                                    section: "item_1".to_owned(),
                                    filing_date: Some("2025-11-01".to_owned()),
                                    title: "AAPL 10-K".to_owned(),
                                    quote: "Ecosystem retention".to_owned(),
                                    source_url: "https://sec.gov/aapl".to_owned(),
                                    similarity: 0.92,
                                }],
                                status: SignalStatus::Unread,
                                event_id: Some(format!("ev-{idx:03}")),
                                created_at_ms: 1_795_000_000_000 + idx,
                                updated_at_ms: 1_795_000_000_000 + idx,
                            });
                        }
                        db.upsert_signals(&signals).unwrap();
                        db.upsert_signal_triage(&SignalTriageRecord {
                            signal_id: "sig-000".to_owned(),
                            ticker: "AAPL".to_owned(),
                            verdict: TriageVerdict::Material,
                            headline: "Services margin expanded".to_owned(),
                            rationale: "72% gross margin".to_owned(),
                            matched_metrics: vec!["services_gm".to_owned()],
                            suggested_metrics: Vec::new(),
                            quotes: vec![TriageQuote {
                                section: "item_7".to_owned(),
                                filing_date: Some("2025-11-01".to_owned()),
                                quote: "Services margin increased.".to_owned(),
                                source_url: "https://sec.gov/aapl".to_owned(),
                            }],
                            fallback_no_metrics_warning: false,
                            model_used: "mock".to_owned(),
                            tokens_used: 50,
                            cost_usd: Decimal::ZERO,
                            created_at_ms: 1_795_000_000_100,
                        })
                        .unwrap();
                    }

                    let fs = M3WorkbenchFs::new(paths.clone());

                    let ind_idx: Value =
                        serde_json::from_slice(&read_vfs(&fs, &["industries", "index.json"]).await)
                            .unwrap();
                    assert_eq!(ind_idx["count"], 1);
                    assert_eq!(ind_idx["industries"][0]["slug"], "retail");

                    let ind_detail: Value = serde_json::from_slice(
                        &read_vfs(&fs, &["industries", "retail.json"]).await,
                    )
                    .unwrap();
                    assert_eq!(ind_detail["title"], "Membership Warehouse Retail");

                    let memo =
                        String::from_utf8(read_vfs(&fs, &["memos", "AAPL.md"]).await).unwrap();
                    assert!(memo.contains("Core moat."));

                    let feynman_memo =
                        String::from_utf8(read_vfs(&fs, &["memos", "AAPL-feynman.md"]).await)
                            .unwrap();
                    assert!(feynman_memo.contains("Round 1 passed."));

                    let latest_briefing: Value =
                        serde_json::from_slice(&read_vfs(&fs, &["briefings", "latest.json"]).await)
                            .unwrap();
                    assert_eq!(latest_briefing["available"], true);
                    assert_eq!(latest_briefing["briefing"]["briefing_id"], "br-2026-11-26");

                    let recent_events: Value =
                        serde_json::from_slice(&read_vfs(&fs, &["events", "recent.json"]).await)
                            .unwrap();
                    assert_eq!(recent_events["available"], true);
                    assert_eq!(recent_events["count"], RECENT_EVENTS_MOUNT_CAP);
                    assert_eq!(recent_events["signals"].as_array().unwrap().len(), 40);
                    assert_eq!(recent_events["triage_count"], 1);

                    // Verify VFS strictly denies writes / creates / removes on new paths.
                    let fid = Fid(99);
                    fs.walk(Fid::ROOT, fid, &["briefings".into(), "latest.json".into()])
                        .await
                        .unwrap();
                    assert_eq!(
                        fs.open(fid, OpenMode::Write).await,
                        Err(ErrorCode::NoAccess)
                    );
                    assert_eq!(fs.write(fid, 0, b"{}").await, Err(ErrorCode::NoAccess));
                    assert_eq!(fs.remove(fid).await, Err(ErrorCode::NoAccess));
                    fs.clunk(fid).await.unwrap();

                    // Verify M3ToolPolicyHook blocks writes to briefings/, events/, and /mnt/m3/.
                    let hook = M3ToolPolicyHook;
                    for p in [
                        "briefings/latest.json",
                        "events/recent.json",
                        "/mnt/m3/briefings/latest.json",
                        "/mnt/m3/events/recent.json",
                        "/mnt/m3/industries/retail.json",
                        "/mnt/m3/memos/AAPL.md",
                    ] {
                        assert!(
                            matches!(
                                hook.evaluate(
                                    "write_file",
                                    &json!({"path": p, "content": "hack"}),
                                    ToolCapability::Write
                                ),
                                HookDecision::Deny(_)
                            ),
                            "expected policy write deny for {p}"
                        );
                    }
                    // Read queries on events or /mnt/m3 briefings remain allowed.
                    assert!(matches!(
                        hook.evaluate(
                            "m3_semantic_neighbors",
                            &json!({"query": "events"}),
                            ToolCapability::Read
                        ),
                        HookDecision::Allow
                    ));

                    let _ = std::fs::remove_dir_all(&dir);
                });
            })
            .expect("spawn test thread");
        handle.join().expect("expanded mnt_m3 test panicked");
    }

    #[test]
    fn alan_scope_keys_labels_serde_preamble_and_scoped_interrogation() {
        let (dir, paths) = temp_m3_workspace("scopes");

        let scopes = [
            (
                AlanScope::Company(" cost ".to_owned()),
                "company-COST",
                "COST",
                "COST",
            ),
            (
                AlanScope::Industry("retail".to_owned()),
                "industry-retail",
                "retail",
                "retail",
            ),
            (
                AlanScope::Briefing("latest".to_owned()),
                "briefing-latest",
                "简报",
                "Briefing",
            ),
            (AlanScope::Portfolio, "portfolio", "全局能力圈", "Portfolio"),
        ];

        for (scope, expected_key, expected_zh, expected_en) in scopes {
            assert_eq!(scope.scope_key(), expected_key);
            assert_eq!(scope.label_for(UiLang::Zh), expected_zh);
            assert_eq!(scope.label_for(UiLang::En), expected_en);

            let serialized = serde_json::to_string(&scope).unwrap();
            let roundtrip: AlanScope = serde_json::from_str(&serialized).unwrap();
            assert_eq!(roundtrip, scope);

            let turn = scope.compose_interrogation_turn(&paths, UiLang::Zh, "核对护城河");
            assert!(turn.contains("核对护城河"));
            assert!(!scope.context_preamble(&paths, UiLang::En).is_empty());
        }

        // Unsafe path traversal characters in scope_key are neutralized.
        let dirty = AlanScope::Industry("../etc/passwd".to_owned());
        assert!(!dirty.scope_key().contains(".."));
        assert!(!dirty.scope_key().contains('/'));

        // End-to-end M3InterrogatorHandle::submit_scoped with MockLlmProvider.
        let provider = MockLlmProvider::new().with_responses(vec![text_response(
            "Portfolio 能力圈总览已核对 [report:AAPL]。",
        )]);
        let interrogator = M3InterrogatorHandle::start_with_llm(
            paths.clone(),
            LlmSource::Client(LlmClient::new(provider.clone())),
        );
        interrogator.submit_scoped(&AlanScope::Portfolio, UiLang::Zh, "总结当前能力圈击球区");

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut saw_done = false;
        let mut prose = String::new();
        while std::time::Instant::now() < deadline {
            for ev in interrogator.drain_events() {
                match ev {
                    InterrogatorEvent::TextDelta { chunk, .. } => prose.push_str(&chunk),
                    InterrogatorEvent::TurnDone { completed, .. } => {
                        assert!(completed);
                        saw_done = true;
                    }
                    InterrogatorEvent::Error { message } => {
                        panic!("unexpected interrogator error: {message}");
                    }
                    _ => {}
                }
            }
            if saw_done {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_done);
        assert!(prose.contains("[report:AAPL]"));
        interrogator.shutdown();

        let recorded = provider.recorded_requests();
        assert!(recorded.iter().any(|req| {
            req.messages.iter().any(|m| {
                m.content.contains("全局能力圈组合") && m.content.contains("总结当前能力圈击球区")
            })
        }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_inline_evidence_cards_dedupes_by_ticker_and_computes_zero_llm_summary() {
        let (dir, paths) = temp_m3_workspace("evidence_cards");
        let now = store::now_ms();
        let fresh_price = PriceSnapshot {
            schema_version: 1,
            symbol: "AAPL".into(),
            price: Decimal::from(180),
            previous_close: Some(Decimal::from(179)),
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(now),
            fetched_at_ms: now,
            source_url: "https://query1.finance.yahoo.com/v8/finance/chart/AAPL".into(),
        };
        store::write_json_atomic(&paths.price_path("AAPL"), &fresh_price).unwrap();

        let annuals: Vec<AnnualFinancials> = (2021..=2025)
            .map(|fy| AnnualFinancials {
                fiscal_year: fy,
                period_end: format!("{fy}-09-27"),
                filed: Some(format!("{fy}-11-01")),
                revenue: Decimal::from(400_000_000_000i64),
                gross_profit: Decimal::from(185_000_000_000i64),
                ebit: Decimal::from(125_000_000_000i64),
                tax_expense: Decimal::from(20_000_000_000i64),
                pretax_income: Decimal::from(124_000_000_000i64),
                ocf: Decimal::from(118_000_000_000i64),
                capex: Decimal::from(10_000_000_000i64),
                sbc: Decimal::from(12_000_000_000i64),
                buybacks: Decimal::from(90_000_000_000i64),
                cash_and_st: Some(Decimal::from(65_000_000_000i64)),
                total_debt: Decimal::from(100_000_000_000i64),
                equity: Decimal::from(60_000_000_000i64),
                diluted_shares: Decimal::from(15_200_000_000i64),
                ending_shares: Decimal::from(15_000_000_000i64),
                provenance: Vec::new(),
            })
            .collect();
        let facts = RawCompanyFactsSummary {
            schema_version: crate::edgar::SEC_FACTS_SCHEMA_VERSION,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "AAPL".into(),
            entity_name: "Apple Inc.".into(),
            cik: "0000320193".into(),
            source_url: "https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json".into(),
            raw_sha256: "deadbeef12345678".into(),
            fetched_at_ms: now,
            annuals,
            ttm: None,
            manual_note: None,
        };
        store::write_json_atomic(&paths.sec_facts_summary_path("AAPL"), &facts).unwrap();

        // Ensure computed report for AAPL exists on disk with fresh price & 5Y ROIC.
        let rep = store::recompute_ticker(&paths, "AAPL").unwrap();
        assert!(rep.price.is_some());
        assert!(rep.roic_5y_avg_pct.is_some());

        // Add a dossier-only company MSFT to verify fallback without computed report.
        let msft_dossier = Dossier {
            schema_version: 1,
            ticker: "MSFT".into(),
            name: "Microsoft Corp.".into(),
            updated_at: "2026-11-26".into(),
            draft_source: "manual".into(),
            three_sentence_business: "Enterprise cloud & productivity".into(),
            latticework: Latticework {
                microeconomics: String::new(),
                ecology: String::new(),
                psychology_incentives: String::new(),
                capital_allocation: String::new(),
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
        store::write_json_atomic(&paths.dossier_path("MSFT"), &msft_dossier).unwrap();

        let tokens = vec![
            AcmeEvidenceToken {
                id: "xbrl:AAPL".into(),
                kind: AcmeEvidenceKind::Xbrl,
                ticker: "AAPL".into(),
                label: "[xbrl:AAPL]".into(),
                summary: "XBRL".into(),
            },
            AcmeEvidenceToken {
                id: "report:AAPL".into(),
                kind: AcmeEvidenceKind::Report,
                ticker: "aapl".into(),
                label: "[report:AAPL]".into(),
                summary: "Report".into(),
            },
            AcmeEvidenceToken {
                id: "dossier:AAPL".into(),
                kind: AcmeEvidenceKind::Dossier,
                ticker: "AAPL".into(),
                label: "[dossier:AAPL]".into(),
                summary: "Dossier".into(),
            },
            AcmeEvidenceToken {
                id: "dossier:MSFT".into(),
                kind: AcmeEvidenceKind::Dossier,
                ticker: "MSFT".into(),
                label: "[dossier:MSFT]".into(),
                summary: "Dossier".into(),
            },
            AcmeEvidenceToken {
                id: "report:NOEXIST".into(),
                kind: AcmeEvidenceKind::Report,
                ticker: "NOEXIST".into(),
                label: "[report:NOEXIST]".into(),
                summary: "Missing".into(),
            },
        ];

        let cards = resolve_inline_evidence_cards(&paths, &tokens);
        assert_eq!(cards.len(), 2);

        let aapl = &cards[0];
        assert_eq!(aapl.ticker, "AAPL");
        assert_eq!(aapl.name, "Apple Inc.");
        assert_eq!(aapl.bucket, Some(Bucket::Core));
        assert_eq!(aapl.latest_price, Some(Decimal::from(180)));
        assert!(aapl.strike_price.is_some());
        assert!(aapl.strike_gap_pct.is_some());
        assert!(aapl.roic_5y.is_some());
        assert!(aapl.fcf_conversion_or_yield.is_some());
        assert_eq!(aapl.source_kind, AcmeEvidenceKind::Report);
        assert!(aapl.has_dossier);

        let msft = &cards[1];
        assert_eq!(msft.ticker, "MSFT");
        assert_eq!(msft.name, "Microsoft Corp.");
        assert_eq!(msft.bucket, None);
        assert_eq!(msft.source_kind, AcmeEvidenceKind::Dossier);
        assert!(msft.has_dossier);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
