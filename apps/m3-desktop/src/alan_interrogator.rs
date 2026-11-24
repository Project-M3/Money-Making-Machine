//! In-process `EmbeddedAlan` V2 interrogator for Project-M3 (`2 Alan 审讯台`).
//!
//! - Mounts read-only `/mnt/m3` exposing `watchlist.json`, `journal.json`,
//!   `computed/<TICKER>.json`, and `dossiers/<TICKER>.json`.
//! - Registers deterministic M3 tools (`m3_status`, `m3_report`, `m3_dossier`,
//!   `m3_update_draft`, `m3_universe_query`, `m3_peer_percentiles`,
//!   `m3_semantic_neighbors`, `m3_events_for_ticker`) backed directly by
//!   `store.rs`, `calculator.rs`, `lake.rs`, and `lake_l1.rs`.
//! - Enforces [`M3ToolPolicyHook`] unconditionally denying any tool call
//!   touching `human_judgment`, `computed/`, `raw/`, `data/munger/`, `m3.duckdb`,
//!   `.parquet`, `lake/`, `journal.json`, `watchlist.json`, `budget.json`,
//!   `themes.json`, or `memos/`.
//! - Emits live streamed prose, `alanworks.app`-style Turing Tape rows, and
//!   clickable Acme evidence tokens (`[report:TICKER]`, `[xbrl:TICKER]`, `[dossier:TICKER]`).

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
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, mpsc as tokio_mpsc};

use crate::industry::{self, IndustryProfile};
use crate::lake::{LakeDb, UniverseFilter, UniverseSortBy};
use crate::lake_l1::SimilarSearchFilter;
use crate::model::{Bucket, Dossier, HumanJudgment, Latticework, MungerReport};
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcmeEvidenceToken {
    pub id: String,
    pub kind: AcmeEvidenceKind,
    pub ticker: String,
    pub label: String,
    pub summary: String,
}

/// Events drained on the GPUI timer into `MungerWorkbench`.
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

        let dossiers_dir = paths.workbench_dir.join("dossiers");
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

        let mut dirs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        {
            let root = dirs.entry(String::new()).or_default();
            root.insert("computed".to_string());
            root.insert("dossiers".to_string());
        }
        dirs.entry("computed".to_string()).or_default();
        dirs.entry("dossiers".to_string()).or_default();

        let mut path_ids = HashMap::new();
        path_ids.insert(String::new(), 0);
        let mut next_id = 1u64;
        path_ids.insert("computed".to_string(), next_id);
        next_id += 1;
        path_ids.insert("dossiers".to_string(), next_id);
        next_id += 1;

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
    SubmitPrompt {
        display_prompt: String,
        full_prompt: String,
    },
    Interrupt,
    Reset,
    RefreshFs,
    Shutdown,
}

/// Asynchronous controller hosting `EmbeddedAlan` V2 for `2 Alan 审讯台`.
#[derive(Clone)]
pub struct M3InterrogatorHandle {
    fs: Arc<M3WorkbenchFs>,
    cmd_tx: tokio_mpsc::UnboundedSender<InterrogatorCommand>,
    events: Arc<StdMutex<Vec<InterrogatorEvent>>>,
    #[allow(dead_code)]
    usage: Arc<StdMutex<(u64, u64)>>,
}

impl M3InterrogatorHandle {
    pub fn start_shared_credentials(paths: WorkspacePaths) -> Self {
        Self::start_with_llm(paths, LlmSource::SharedProductCredentials)
    }

    pub fn start_with_llm(paths: WorkspacePaths, llm: LlmSource) -> Self {
        let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
        let (cmd_tx, mut cmd_rx) = tokio_mpsc::unbounded_channel::<InterrogatorCommand>();
        let events = Arc::new(StdMutex::new(Vec::new()));
        let usage = Arc::new(StdMutex::new((0u64, 0u64)));
        let fs_worker = Arc::clone(&fs);
        let events_worker = Arc::clone(&events);
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
                    let app =
                        match boot_m3_embedded(paths, Arc::clone(&fs_worker), llm).await {
                            Ok(app) => app,
                            Err(err) => {
                                push_event(
                                    &events_worker,
                                    InterrogatorEvent::Error {
                                        message: friendly_m3_credentials_error(&format!(
                                            "{err:#}"
                                        )),
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
                &events,
                InterrogatorEvent::Error {
                    message: format!("无法启动 Alan 审讯线程: {err}"),
                },
            );
        }

        Self {
            fs,
            cmd_tx,
            events,
            usage,
        }
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

fn push_event(queue: &Arc<StdMutex<Vec<InterrogatorEvent>>>, event: InterrogatorEvent) {
    if let Ok(mut guard) = queue.lock() {
        guard.push(event);
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
}
