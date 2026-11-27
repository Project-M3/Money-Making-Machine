use crate::{
    DEFAULT_ALAN_TIMEOUT_SECS, MAX_ALAN_TIMEOUT_SECS, MAX_APP_TIMEOUT_SECS,
    MAX_BRIEFING_LIST_LIMIT, MAX_FILTER_METRIC, MAX_L1_LIST_LIMIT, MAX_MIN_YEARS, MAX_PEERS_LIMIT,
    MAX_RESEARCH_STATUS_LIMIT, MAX_SCREEN_LIMIT, MAX_SIMILAR_LIMIT, MAX_TRIAGE_LIMIT,
    MIN_FILTER_METRIC, cmd_budget_status, cmd_doctor_for_workspace, cmd_dossier, cmd_report,
    cmd_status, cmd_storage_with_updater, cmd_themes_list, cmd_universe_status, execute_alan_ask,
    execute_alan_session_clear, execute_alan_session_show, execute_alan_status, execute_alan_test,
    execute_app_control_at, execute_briefing_latest, execute_briefing_list, execute_briefing_run,
    execute_events_list, execute_falsification_update, execute_feynman_round, execute_inbox_list,
    execute_inbox_triage, execute_industry_list, execute_industry_show, execute_journal_list,
    execute_journal_record, execute_memo_read, execute_research_run, execute_research_status,
    execute_sync, execute_triage_list, execute_triage_run, execute_universe_peers,
    execute_universe_screen, execute_universe_similar, execute_watchlist_add,
    format_alan_scope_token, parse_alan_canvas_tab, parse_alan_scope, parse_alan_stage_mode,
    parse_bucket_token, parse_journal_action_token, parse_ui_lang, parse_workspace_tab,
};
use anyhow::{Context, Result, ensure};
use m3_core::{
    alan_interrogator::AlanScope,
    app_config,
    app_control::{self, AppControlRequest},
    feynman::{FeynmanRound, FeynmanScope},
    lake::{UniverseFilter, UniverseSortBy},
    lake_l1::{SignalKind, SignalStatus, SimilarSearchFilter},
    model::{Bucket, parse_decimal_token},
    proactive::{BriefingPeriod, TriageVerdict},
    store::{self, WorkspacePaths},
    update,
};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use std::{
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
    str::FromStr,
};

const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const JSONRPC_PARSE_ERROR: i64 = -32700;
const JSONRPC_INVALID_REQUEST: i64 = -32600;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;

#[derive(Debug, Clone, PartialEq, Eq)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl JsonRpcError {
    fn parse_error(msg: impl Into<String>) -> Self {
        Self {
            code: JSONRPC_PARSE_ERROR,
            message: msg.into(),
        }
    }

    fn invalid_request(msg: impl Into<String>) -> Self {
        Self {
            code: JSONRPC_INVALID_REQUEST,
            message: msg.into(),
        }
    }

    fn method_not_found(method: &str) -> Self {
        Self {
            code: JSONRPC_METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
        }
    }

    fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: JSONRPC_INVALID_PARAMS,
            message: msg.into(),
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "code": self.code,
            "message": self.message,
        })
    }
}

/// JSON-RPC 2.0 MCP server for Project M3 research, Alan, and desktop control tools.
pub struct McpServer {
    workspace: std::result::Result<WorkspacePaths, String>,
    updater_dir: Option<PathBuf>,
    app_control_socket: Option<PathBuf>,
    read_only: bool,
    initialized: bool,
}

impl McpServer {
    /// Construct a full-capability server using an optional `--workspace` override or standard discovery.
    pub fn from_env(workspace_override: Option<PathBuf>) -> Result<Self> {
        Self::from_env_with_mode(workspace_override, false)
    }

    /// Construct a server with explicit `--workspace` and `--read-only` mode.
    pub fn from_env_with_mode(
        workspace_override: Option<PathBuf>,
        read_only: bool,
    ) -> Result<Self> {
        let workspace = match workspace_override {
            Some(path) => {
                ensure!(
                    path.exists(),
                    "workspace path `{}` does not exist",
                    path.display()
                );
                let resolved = path.canonicalize().unwrap_or(path);
                Ok(WorkspacePaths::from_root(resolved))
            }
            None => store::resolve_repo_root()
                .map(WorkspacePaths::from_root)
                .map_err(|err| format!("{err:#}")),
        };
        let updater_dir = app_config::M3Paths::resolve().ok().map(|p| p.updater_dir());
        Ok(Self {
            workspace,
            updater_dir,
            app_control_socket: None,
            read_only,
            initialized: false,
        })
    }

    /// Construct a full-capability server pinned to a known `WorkspacePaths` (primarily for testing).
    pub fn with_workspace(paths: WorkspacePaths) -> Self {
        Self {
            workspace: Ok(paths),
            updater_dir: None,
            app_control_socket: None,
            read_only: false,
            initialized: false,
        }
    }

    /// Override read-only filtering mode (mainly for tests).
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Override desktop app control socket path (mainly for isolated socket tests).
    pub fn with_app_control_socket(mut self, socket_path: PathBuf) -> Self {
        self.app_control_socket = Some(socket_path);
        self
    }

    /// Construct a server without a resolved workspace (for testing `m3_doctor` and workspace error paths).
    pub fn without_workspace(err_msg: impl Into<String>) -> Self {
        Self {
            workspace: Err(err_msg.into()),
            updater_dir: None,
            app_control_socket: None,
            read_only: false,
            initialized: false,
        }
    }

    /// Run the line-delimited JSON-RPC 2.0 loop over an arbitrary reader and writer.
    pub fn serve_io<R: BufRead, W: Write>(&mut self, mut reader: R, mut writer: W) -> Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = reader
                .read_line(&mut line)
                .context("failed reading JSON-RPC line from stdin")?;
            if bytes == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(response) = self.handle_line(trimmed) {
                let encoded = serde_json::to_string(&response)
                    .context("failed serializing JSON-RPC response")?;
                writeln!(writer, "{encoded}")
                    .context("failed writing JSON-RPC response to stdout")?;
                writer
                    .flush()
                    .context("failed flushing JSON-RPC response to stdout")?;
            }
        }
        Ok(())
    }

    /// Process a single JSON-RPC 2.0 line and return `Some(response)` for requests or `None` for notifications.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let parsed: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                return Some(error_response(
                    Value::Null,
                    JsonRpcError::parse_error(format!("parse error: {err}")),
                ));
            }
        };

        let Some(obj) = parsed.as_object() else {
            return Some(error_response(
                Value::Null,
                JsonRpcError::invalid_request("JSON-RPC request must be an object"),
            ));
        };

        let has_id = obj.contains_key("id");
        let id = match obj.get("id") {
            None => Value::Null,
            Some(v) if is_valid_jsonrpc_id(v) => v.clone(),
            Some(_) => {
                return Some(error_response(
                    Value::Null,
                    JsonRpcError::invalid_request("JSON-RPC id must be a string, number, or null"),
                ));
            }
        };

        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(error_response(
                id,
                JsonRpcError::invalid_request("jsonrpc version must be \"2.0\""),
            ));
        }

        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return Some(error_response(
                id,
                JsonRpcError::invalid_request("missing or non-string JSON-RPC method"),
            ));
        };

        let params = obj.get("params");

        if !has_id {
            self.handle_notification(method, params);
            return None;
        }

        let outcome = match method {
            "initialize" => self.handle_initialize(params),
            "ping" => handle_ping(params),
            "tools/list" => self.handle_tools_list(params),
            "tools/call" => self.handle_tools_call(params),
            other => Err(JsonRpcError::method_not_found(other)),
        };

        Some(match outcome {
            Ok(result) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            }),
            Err(err) => error_response(id, err),
        })
    }

    fn handle_notification(&mut self, method: &str, _params: Option<&Value>) {
        if method == "notifications/initialized" || method == "initialized" {
            self.initialized = true;
        }
    }

    fn handle_initialize(
        &mut self,
        params: Option<&Value>,
    ) -> std::result::Result<Value, JsonRpcError> {
        let negotiated = match params {
            None | Some(Value::Null) => DEFAULT_PROTOCOL_VERSION,
            Some(Value::Object(map)) => match map.get("protocolVersion") {
                None | Some(Value::Null) => DEFAULT_PROTOCOL_VERSION,
                Some(Value::String(requested)) => SUPPORTED_PROTOCOL_VERSIONS
                    .iter()
                    .copied()
                    .find(|v| *v == requested.as_str())
                    .unwrap_or(DEFAULT_PROTOCOL_VERSION),
                Some(_) => {
                    return Err(JsonRpcError::invalid_params(
                        "initialize.protocolVersion must be a string",
                    ));
                }
            },
            Some(_) => {
                return Err(JsonRpcError::invalid_params(
                    "initialize params must be an object",
                ));
            }
        };

        self.initialized = true;
        let instructions = if self.read_only {
            "Project M3 deterministic value-investing workbench (--read-only MCP mode). All financial arithmetic is computed deterministically in Rust using rust_decimal; never estimate or recompute financial figures in the LLM. Write/action tools are disabled in this session. Never fabricate or overwrite human_judgment, and never delete personal_workbench."
        } else {
            "Project M3 deterministic value-investing workbench (full-capability MCP tools with readOnlyHint/destructiveHint annotations). All financial arithmetic is computed deterministically in Rust using rust_decimal; never estimate or recompute financial figures in the LLM. Never fabricate or overwrite human_judgment, never auto-sign feynman_checked_at, never overwrite ## Human Notes, never delete personal_workbench, and always respect AI research budget gates."
        };
        Ok(json!({
            "protocolVersion": negotiated,
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "m3",
                "version": update::current_binary_version(),
            },
            "instructions": instructions
        }))
    }

    fn handle_tools_list(
        &self,
        params: Option<&Value>,
    ) -> std::result::Result<Value, JsonRpcError> {
        match params {
            None | Some(Value::Null) => {}
            Some(Value::Object(_)) => {}
            Some(_) => {
                return Err(JsonRpcError::invalid_params(
                    "tools/list params must be an object or null",
                ));
            }
        }
        let tools = if self.read_only {
            read_only_tool_definitions()
        } else {
            tool_definitions()
        };
        Ok(json!({ "tools": tools }))
    }

    fn handle_tools_call(
        &self,
        params: Option<&Value>,
    ) -> std::result::Result<Value, JsonRpcError> {
        let map = params
            .and_then(Value::as_object)
            .ok_or_else(|| JsonRpcError::invalid_params("tools/call params must be an object"))?;

        let name = map
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                JsonRpcError::invalid_params("tools/call requires a non-empty string `name`")
            })?;

        if self.read_only && is_action_tool_name(name) {
            return Err(JsonRpcError::invalid_params(format!(
                "tool `{name}` is disabled in --read-only MCP mode"
            )));
        }

        let args = require_object_or_empty(map.get("arguments"))?;
        let domain_result = self.dispatch_tool(name, args)?;
        Ok(tool_call_result(domain_result))
    }

    fn dispatch_tool(
        &self,
        name: &str,
        args: &Map<String, Value>,
    ) -> std::result::Result<Result<Value>, JsonRpcError> {
        match name {
            "m3_status" => {
                reject_unknown_keys(args, &[])?;
                Ok(self.require_workspace().and_then(cmd_status))
            }
            "m3_doctor" => {
                reject_unknown_keys(args, &[])?;
                let ws_opt = self.workspace.as_ref().ok();
                Ok(cmd_doctor_for_workspace(
                    ws_opt,
                    self.updater_dir.as_deref(),
                ))
            }
            "m3_report" => {
                reject_unknown_keys(args, &["ticker"])?;
                let ticker = require_non_empty_string(args, "ticker")?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| cmd_report(paths, &[ticker])))
            }
            "m3_dossier" => {
                reject_unknown_keys(args, &["ticker"])?;
                let ticker = require_non_empty_string(args, "ticker")?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| cmd_dossier(paths, &[ticker])))
            }
            "m3_universe_status" => {
                reject_unknown_keys(args, &[])?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| cmd_universe_status(paths, &[])))
            }
            "m3_universe_screen" => {
                reject_unknown_keys(
                    args,
                    &[
                        "min_roic",
                        "max_sbc_ocf",
                        "max_share_change_3y",
                        "max_net_debt_to_oe",
                        "min_years",
                        "exclude_financials",
                        "positive_oe_only",
                        "price_pool_only",
                        "strike_only",
                        "query",
                        "sort",
                        "sort_asc",
                        "limit",
                    ],
                )?;
                let defaults = UniverseFilter::default();
                let filter = UniverseFilter {
                    min_roic_5y: optional_decimal_range(
                        args,
                        "min_roic",
                        MIN_FILTER_METRIC,
                        MAX_FILTER_METRIC,
                    )?,
                    max_sbc_to_ocf: optional_decimal_range(
                        args,
                        "max_sbc_ocf",
                        MIN_FILTER_METRIC,
                        MAX_FILTER_METRIC,
                    )?,
                    max_share_change_3y: optional_decimal_range(
                        args,
                        "max_share_change_3y",
                        MIN_FILTER_METRIC,
                        MAX_FILTER_METRIC,
                    )?,
                    max_net_debt_to_oe: optional_decimal_range(
                        args,
                        "max_net_debt_to_oe",
                        MIN_FILTER_METRIC,
                        MAX_FILTER_METRIC,
                    )?,
                    min_years_of_data: optional_usize_range(args, "min_years", 1, MAX_MIN_YEARS)?,
                    exclude_financials: optional_bool(args, "exclude_financials")?
                        .unwrap_or(defaults.exclude_financials),
                    positive_strict_oe_only: optional_bool(args, "positive_oe_only")?
                        .unwrap_or(defaults.positive_strict_oe_only),
                    in_price_pool_only: optional_bool(args, "price_pool_only")?
                        .unwrap_or(defaults.in_price_pool_only),
                    strike_zone_only: optional_bool(args, "strike_only")?
                        .unwrap_or(defaults.strike_zone_only),
                    query: optional_trimmed_string(args, "query")?,
                    sort_by: match optional_trimmed_string(args, "sort")? {
                        Some(sort_raw) => UniverseSortBy::from_str(&sort_raw).map_err(|err| {
                            JsonRpcError::invalid_params(format!("invalid sort: {err}"))
                        })?,
                        None => defaults.sort_by,
                    },
                    sort_asc: optional_bool(args, "sort_asc")?.unwrap_or(defaults.sort_asc),
                    limit: optional_usize_range(args, "limit", 1, MAX_SCREEN_LIMIT)?
                        .unwrap_or(defaults.limit),
                };
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_universe_screen(paths, &filter)))
            }
            "m3_universe_peers" => {
                reject_unknown_keys(args, &["ticker", "limit"])?;
                let ticker = require_non_empty_string(args, "ticker")?.to_ascii_uppercase();
                let limit = optional_usize_range(args, "limit", 1, MAX_PEERS_LIMIT)?.unwrap_or(15);
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_universe_peers(paths, &ticker, limit)))
            }
            "m3_universe_similar" => {
                reject_unknown_keys(
                    args,
                    &[
                        "query",
                        "as_ticker",
                        "min_roic",
                        "max_sbc_ocf",
                        "exclude_financials",
                        "positive_oe_only",
                        "price_pool_only",
                        "limit",
                    ],
                )?;
                let query = require_non_empty_string(args, "query")?;
                let as_ticker = optional_bool(args, "as_ticker")?.unwrap_or(false);
                let defaults = SimilarSearchFilter::default();
                let filter = SimilarSearchFilter {
                    min_roic_5y: optional_decimal_range(
                        args,
                        "min_roic",
                        MIN_FILTER_METRIC,
                        MAX_FILTER_METRIC,
                    )?,
                    max_sbc_to_ocf: optional_decimal_range(
                        args,
                        "max_sbc_ocf",
                        MIN_FILTER_METRIC,
                        MAX_FILTER_METRIC,
                    )?,
                    exclude_financials: optional_bool(args, "exclude_financials")?
                        .unwrap_or(defaults.exclude_financials),
                    positive_strict_oe_only: optional_bool(args, "positive_oe_only")?
                        .unwrap_or(defaults.positive_strict_oe_only),
                    in_price_pool_only: optional_bool(args, "price_pool_only")?
                        .unwrap_or(defaults.in_price_pool_only),
                    limit: optional_usize_range(args, "limit", 1, MAX_SIMILAR_LIMIT)?
                        .unwrap_or(defaults.limit),
                    min_score: defaults.min_score,
                };
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_universe_similar(paths, &query, as_ticker, &filter)))
            }
            "m3_inbox_list" => {
                reject_unknown_keys(args, &["status", "kind", "limit"])?;
                let (status_raw, status) = match optional_trimmed_string(args, "status")? {
                    None => ("all".to_owned(), None),
                    Some(raw) => {
                        let lower = raw.to_ascii_lowercase();
                        if lower == "all" {
                            ("all".to_owned(), None)
                        } else {
                            let parsed = SignalStatus::from_str(&lower).map_err(|err| {
                                JsonRpcError::invalid_params(format!("invalid status: {err}"))
                            })?;
                            (parsed.as_str().to_owned(), Some(parsed))
                        }
                    }
                };
                let kind = match optional_trimmed_string(args, "kind")? {
                    None => None,
                    Some(raw) => Some(SignalKind::from_str(&raw).map_err(|err| {
                        JsonRpcError::invalid_params(format!("invalid kind: {err}"))
                    })?),
                };
                let limit =
                    optional_usize_range(args, "limit", 1, MAX_L1_LIST_LIMIT)?.unwrap_or(50);
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_inbox_list(paths, &status_raw, status, kind, limit)))
            }
            "m3_industry_list" => {
                reject_unknown_keys(args, &[])?;
                Ok(self.require_workspace().and_then(execute_industry_list))
            }
            "m3_industry_show" => {
                reject_unknown_keys(args, &["slug"])?;
                let slug = require_non_empty_string(args, "slug")?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_industry_show(paths, &slug)))
            }
            "m3_budget_status" => {
                reject_unknown_keys(args, &[])?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| cmd_budget_status(paths, &[])))
            }
            "m3_themes_list" => {
                reject_unknown_keys(args, &[])?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| cmd_themes_list(paths, &[])))
            }
            "m3_storage_status" => {
                reject_unknown_keys(args, &[])?;
                Ok(self.require_workspace().and_then(|paths| {
                    cmd_storage_with_updater(paths, &["status".into()], self.updater_dir.as_deref())
                }))
            }
            "m3_briefing_latest" => {
                reject_unknown_keys(args, &["period"])?;
                let period = optional_briefing_period(args, "period")?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_briefing_latest(paths, period)))
            }
            "m3_briefing_list" => {
                reject_unknown_keys(args, &["period", "limit"])?;
                let period = optional_briefing_period(args, "period")?;
                let limit =
                    optional_usize_range(args, "limit", 1, MAX_BRIEFING_LIST_LIMIT)?.unwrap_or(10);
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_briefing_list(paths, period, limit)))
            }
            "m3_triage_list" => {
                reject_unknown_keys(args, &["ticker", "verdict", "limit"])?;
                let ticker =
                    optional_trimmed_string(args, "ticker")?.map(|t| t.to_ascii_uppercase());
                let verdict = match optional_trimmed_string(args, "verdict")? {
                    None => None,
                    Some(raw) => Some(parse_mcp_triage_verdict(&raw)?),
                };
                let limit = optional_usize_range(args, "limit", 1, MAX_TRIAGE_LIMIT)?.unwrap_or(50);
                Ok(self.require_workspace().and_then(|paths| {
                    execute_triage_list(paths, ticker.as_deref(), verdict, limit)
                }))
            }
            "m3_alan_status" => {
                reject_unknown_keys(args, &[])?;
                Ok(execute_alan_status())
            }
            "m3_alan_session_show" => {
                reject_unknown_keys(args, &["scope"])?;
                let scope = parse_mcp_alan_scope(args, "scope")?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_alan_session_show(paths, &scope)))
            }
            "m3_journal_list" => {
                reject_unknown_keys(args, &[])?;
                Ok(self.require_workspace().and_then(execute_journal_list))
            }
            "m3_memo_read" => {
                reject_unknown_keys(args, &["ticker"])?;
                let ticker = require_non_empty_string(args, "ticker")?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_memo_read(paths, &ticker)))
            }
            "m3_events_list" => {
                reject_unknown_keys(args, &["ticker", "event_type", "unprocessed_only", "limit"])?;
                let ticker = optional_trimmed_string(args, "ticker")?;
                let event_type = optional_trimmed_string(args, "event_type")?;
                let unprocessed_only = optional_bool(args, "unprocessed_only")?.unwrap_or(false);
                let limit =
                    optional_usize_range(args, "limit", 1, MAX_L1_LIST_LIMIT)?.unwrap_or(50);
                Ok(self.require_workspace().and_then(|paths| {
                    execute_events_list(
                        paths,
                        ticker.as_deref(),
                        event_type.as_deref(),
                        unprocessed_only,
                        limit,
                    )
                }))
            }
            "m3_research_status" => {
                reject_unknown_keys(args, &["ticker", "limit"])?;
                let ticker = optional_trimmed_string(args, "ticker")?;
                let limit = optional_usize_range(args, "limit", 1, MAX_RESEARCH_STATUS_LIMIT)?
                    .unwrap_or(20);
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_research_status(paths, ticker.as_deref(), limit)))
            }
            "m3_app_state" => {
                reject_unknown_keys(args, &["timeout_secs"])?;
                let timeout_secs = optional_app_timeout_secs(args)?;
                Ok(execute_app_control_at(
                    self.app_control_socket.as_deref(),
                    &AppControlRequest::GetState,
                    timeout_secs,
                ))
            }
            "m3_alan_test" => {
                reject_unknown_keys(args, &["profile_id"])?;
                let profile_id = optional_trimmed_string(args, "profile_id")?;
                Ok(execute_alan_test(profile_id.as_deref()))
            }
            "m3_alan_ask" => {
                reject_unknown_keys(args, &["scope", "question", "lang", "timeout_secs"])?;
                let scope = parse_mcp_alan_scope(args, "scope")?;
                let question = require_non_empty_string(args, "question")?;
                let lang = match optional_trimmed_string(args, "lang")? {
                    Some(raw) => parse_ui_lang(&raw)
                        .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?,
                    None => app_config::read_ui_language(),
                };
                let timeout_secs =
                    optional_usize_range(args, "timeout_secs", 1, MAX_ALAN_TIMEOUT_SECS)?
                        .map(|s| s as u64)
                        .unwrap_or(DEFAULT_ALAN_TIMEOUT_SECS);
                Ok(self.require_workspace().and_then(|paths| {
                    execute_alan_ask(paths, &scope, lang, &question, timeout_secs)
                }))
            }
            "m3_alan_session_clear" => {
                reject_unknown_keys(args, &["scope"])?;
                let scope = parse_mcp_alan_scope(args, "scope")?;
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_alan_session_clear(paths, &scope)))
            }
            "m3_watchlist_add" => {
                reject_unknown_keys(args, &["ticker", "bucket", "name", "circle", "reason"])?;
                let ticker = require_non_empty_string(args, "ticker")?;
                let bucket = match optional_trimmed_string(args, "bucket")? {
                    Some(raw) => parse_bucket_token(&raw)
                        .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?,
                    None => Bucket::Watch,
                };
                let name = optional_trimmed_string(args, "name")?;
                let circle = optional_trimmed_string(args, "circle")?;
                let reason = optional_trimmed_string(args, "reason")?;
                Ok(self.require_workspace().and_then(|paths| {
                    execute_watchlist_add(
                        paths,
                        &ticker,
                        bucket,
                        name.as_deref(),
                        circle.as_deref(),
                        reason.as_deref(),
                    )
                }))
            }
            "m3_sync" => {
                reject_unknown_keys(args, &["ticker", "prices_only"])?;
                let ticker = optional_trimmed_string(args, "ticker")?;
                let prices_only = optional_bool(args, "prices_only")?.unwrap_or(false);
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_sync(paths, ticker.as_deref(), prices_only)))
            }
            "m3_inbox_triage" => {
                reject_unknown_keys(args, &["signal_id", "status", "promote_watch"])?;
                let signal_id = require_non_empty_string(args, "signal_id")?;
                let status_raw = require_non_empty_string(args, "status")?;
                let target_status = SignalStatus::from_str(&status_raw).map_err(|err| {
                    JsonRpcError::invalid_params(format!("invalid status: {err}"))
                })?;
                let promote_watch = optional_bool(args, "promote_watch")?.unwrap_or(false);
                Ok(self.require_workspace().and_then(|paths| {
                    execute_inbox_triage(paths, &signal_id, target_status, promote_watch)
                }))
            }
            "m3_journal_record" => {
                reject_unknown_keys(
                    args,
                    &["ticker", "action", "note", "shares", "falsification"],
                )?;
                let ticker = require_non_empty_string(args, "ticker")?;
                let action_raw = require_non_empty_string(args, "action")?;
                let action = parse_journal_action_token(&action_raw)
                    .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?;
                let note = require_non_empty_string(args, "note")?;
                let shares = optional_decimal_any(args, "shares")?;
                let falsification = optional_trimmed_string(args, "falsification")?;
                Ok(self.require_workspace().and_then(|paths| {
                    execute_journal_record(
                        paths,
                        &ticker,
                        action,
                        &note,
                        shares,
                        falsification.as_deref(),
                    )
                }))
            }
            "m3_falsification_update" => {
                reject_unknown_keys(args, &["ticker", "action", "metric", "index", "metrics"])?;
                let ticker = require_non_empty_string(args, "ticker")?;
                let action = require_non_empty_string(args, "action")?.to_ascii_lowercase();
                let (metric, index, metrics) = match action.as_str() {
                    "add" => {
                        if args.get("index").is_some_and(|v| !v.is_null())
                            || args.get("metrics").is_some_and(|v| !v.is_null())
                        {
                            return Err(JsonRpcError::invalid_params(
                                "`add` action does not accept `index` or `metrics`",
                            ));
                        }
                        let m = require_non_empty_string(args, "metric")?;
                        (Some(m), None, None)
                    }
                    "remove" => {
                        if args.get("metric").is_some_and(|v| !v.is_null())
                            || args.get("metrics").is_some_and(|v| !v.is_null())
                        {
                            return Err(JsonRpcError::invalid_params(
                                "`remove` action does not accept `metric` or `metrics`",
                            ));
                        }
                        let idx = required_usize_index(args, "index")?;
                        (None, Some(idx), None)
                    }
                    "replace" => {
                        if args.get("metric").is_some_and(|v| !v.is_null())
                            || args.get("index").is_some_and(|v| !v.is_null())
                        {
                            return Err(JsonRpcError::invalid_params(
                                "`replace` action does not accept `metric` or `index`",
                            ));
                        }
                        let list = required_string_array(args, "metrics")?;
                        (None, None, Some(list))
                    }
                    _ => {
                        return Err(JsonRpcError::invalid_params(format!(
                            "invalid `action` `{action}`; expected add | remove | replace"
                        )));
                    }
                };
                Ok(self.require_workspace().and_then(|paths| {
                    execute_falsification_update(
                        paths,
                        &ticker,
                        &action,
                        metric.as_deref(),
                        index,
                        metrics,
                    )
                }))
            }
            "m3_research_run" => {
                reject_unknown_keys(args, &["ticker", "focus", "trigger"])?;
                let ticker = require_non_empty_string(args, "ticker")?;
                let focus = optional_trimmed_string(args, "focus")?;
                let trigger = optional_trimmed_string(args, "trigger")?
                    .unwrap_or_else(|| "cli".to_owned())
                    .to_ascii_lowercase();
                if !matches!(trigger.as_str(), "manual" | "inbox" | "cli") {
                    return Err(JsonRpcError::invalid_params(format!(
                        "invalid `trigger` `{trigger}`; expected manual|inbox|cli"
                    )));
                }
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_research_run(paths, &ticker, focus, &trigger)))
            }
            "m3_briefing_run" => {
                reject_unknown_keys(args, &["period", "no_llm"])?;
                let period =
                    optional_briefing_period(args, "period")?.unwrap_or(BriefingPeriod::Daily);
                let no_llm = optional_bool(args, "no_llm")?.unwrap_or(false);
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_briefing_run(paths, period, no_llm)))
            }
            "m3_triage_run" => {
                reject_unknown_keys(args, &["limit", "dry_run"])?;
                let limit = optional_usize_range(args, "limit", 1, MAX_TRIAGE_LIMIT)?.unwrap_or(10);
                let dry_run = optional_bool(args, "dry_run")?.unwrap_or(false);
                Ok(self
                    .require_workspace()
                    .and_then(|paths| execute_triage_run(paths, limit, dry_run)))
            }
            "m3_feynman_round" => {
                reject_unknown_keys(args, &["scope", "round", "answer", "lang", "draft_metrics"])?;
                let scope = parse_mcp_feynman_scope(args, "scope")?;
                let round_num = optional_usize_range(args, "round", 1, 3)?.ok_or_else(|| {
                    JsonRpcError::invalid_params("argument `round` (1..=3) is required")
                })?;
                let round = FeynmanRound::from_u8(round_num as u8).ok_or_else(|| {
                    JsonRpcError::invalid_params("argument `round` must be 1, 2, or 3")
                })?;
                let answer = require_non_empty_string(args, "answer")?;
                let lang = match optional_trimmed_string(args, "lang")? {
                    Some(raw) => parse_ui_lang(&raw)
                        .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?,
                    None => app_config::read_ui_language(),
                };
                let draft_metrics = match args.get("draft_metrics") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(_) => required_string_array(args, "draft_metrics")?,
                };
                Ok(self.require_workspace().and_then(|paths| {
                    execute_feynman_round(paths, &scope, round, &answer, lang, &draft_metrics)
                }))
            }
            "m3_app_navigate" => {
                reject_unknown_keys(args, &["tab", "ticker", "industry", "timeout_secs"])?;
                let tab = match optional_trimmed_string(args, "tab")? {
                    Some(raw) => Some(
                        parse_workspace_tab(&raw)
                            .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?,
                    ),
                    None => None,
                };
                let ticker = optional_string_allow_empty(args, "ticker")?.map(|s| {
                    let t = s.trim();
                    if t.is_empty() {
                        String::new()
                    } else {
                        t.to_ascii_uppercase()
                    }
                });
                let industry = optional_trimmed_string(args, "industry")?;
                if tab.is_none() && ticker.is_none() && industry.is_none() {
                    return Err(JsonRpcError::invalid_params(
                        "m3_app_navigate requires at least one of `tab`, `ticker`, or `industry`",
                    ));
                }
                let timeout_secs = optional_app_timeout_secs(args)?;
                Ok(execute_app_control_at(
                    self.app_control_socket.as_deref(),
                    &AppControlRequest::Navigate {
                        tab,
                        ticker,
                        industry,
                    },
                    timeout_secs,
                ))
            }
            "m3_app_alan" => {
                reject_unknown_keys(
                    args,
                    &[
                        "mode",
                        "tab",
                        "scope",
                        "pin",
                        "anchor_label",
                        "prompt",
                        "submit",
                        "timeout_secs",
                    ],
                )?;
                let mode = match optional_trimmed_string(args, "mode")? {
                    Some(raw) => Some(
                        parse_alan_stage_mode(&raw)
                            .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?,
                    ),
                    None => None,
                };
                let tab = match optional_trimmed_string(args, "tab")? {
                    Some(raw) => Some(
                        parse_alan_canvas_tab(&raw)
                            .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?,
                    ),
                    None => None,
                };
                let scope = match optional_trimmed_string(args, "scope")? {
                    Some(raw) => {
                        let parsed = parse_alan_scope(&raw)
                            .map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))?;
                        Some(format_alan_scope_token(&parsed))
                    }
                    None => None,
                };
                let pin = optional_bool(args, "pin")?;
                let anchor_label = optional_trimmed_string(args, "anchor_label")?;
                let prompt = optional_trimmed_string(args, "prompt")?;
                let submit = optional_bool(args, "submit")?.unwrap_or(false);
                if mode.is_none()
                    && tab.is_none()
                    && scope.is_none()
                    && pin.is_none()
                    && anchor_label.is_none()
                    && prompt.is_none()
                    && !submit
                {
                    return Err(JsonRpcError::invalid_params(
                        "m3_app_alan requires at least one state field or `submit: true`",
                    ));
                }
                let timeout_secs = optional_app_timeout_secs(args)?;
                Ok(execute_app_control_at(
                    self.app_control_socket.as_deref(),
                    &AppControlRequest::SetAlan {
                        mode,
                        tab,
                        scope,
                        pin,
                        anchor_label,
                        prompt,
                        submit,
                    },
                    timeout_secs,
                ))
            }
            "m3_app_action" => {
                reject_unknown_keys(args, &["action", "ticker", "timeout_secs"])?;
                let action = require_non_empty_string(args, "action")?.to_ascii_lowercase();
                let ticker =
                    optional_trimmed_string(args, "ticker")?.map(|t| t.to_ascii_uppercase());
                let timeout_secs = optional_app_timeout_secs(args)?;
                Ok(execute_app_control_at(
                    self.app_control_socket.as_deref(),
                    &AppControlRequest::TriggerAction { action, ticker },
                    timeout_secs,
                ))
            }
            "m3_app_screenshot" => {
                reject_unknown_keys(args, &["output_path", "timeout_secs"])?;
                let output_path = require_non_empty_string(args, "output_path")?;
                let timeout_secs = optional_app_timeout_secs(args)?;
                Ok(execute_app_control_at(
                    self.app_control_socket.as_deref(),
                    &AppControlRequest::Screenshot { output_path },
                    timeout_secs,
                ))
            }
            unknown => Err(JsonRpcError::invalid_params(format!(
                "unknown tool `{unknown}`"
            ))),
        }
    }

    fn require_workspace(&self) -> Result<&WorkspacePaths> {
        self.workspace
            .as_ref()
            .map_err(|msg| anyhow::anyhow!("{msg}"))
    }
}

/// CLI entrypoint for `m3 mcp [--workspace <PATH>] [--read-only]`.
pub fn run_cli_mcp(args: &[String]) -> Result<i32> {
    let mut workspace_override: Option<PathBuf> = None;
    let mut read_only = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace" => {
                i += 1;
                let raw = args
                    .get(i)
                    .context("--workspace requires a directory path")?;
                ensure!(!raw.trim().is_empty(), "--workspace path must not be empty");
                workspace_override = Some(PathBuf::from(raw));
            }
            "--read-only" => read_only = true,
            other => anyhow::bail!("unknown mcp flag: {other}"),
        }
        i += 1;
    }

    let mut server = McpServer::from_env_with_mode(workspace_override, read_only)?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let reader = BufReader::new(stdin.lock());
    let writer = BufWriter::new(stdout.lock());
    server.serve_io(reader, writer)?;
    Ok(0)
}

fn handle_ping(params: Option<&Value>) -> std::result::Result<Value, JsonRpcError> {
    match params {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::Object(map)) if map.is_empty() => Ok(json!({})),
        Some(Value::Object(_)) => Ok(json!({})),
        Some(_) => Err(JsonRpcError::invalid_params(
            "ping params must be an object or null",
        )),
    }
}

fn tool_call_result(domain_result: Result<Value>) -> Value {
    match domain_result {
        Ok(value) => {
            let is_error = value.get("ok").and_then(Value::as_bool) == Some(false);
            let text = serde_json::to_string(&value).unwrap_or_else(|_| value.to_string());
            json!({
                "content": [{
                    "type": "text",
                    "text": text,
                }],
                "isError": is_error,
            })
        }
        Err(err) => {
            let payload = json!({
                "schema_version": 1,
                "ok": false,
                "error": format!("{err:#}"),
            });
            let text = serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string());
            json!({
                "content": [{
                    "type": "text",
                    "text": text,
                }],
                "isError": true,
            })
        }
    }
}

fn error_response(id: Value, err: JsonRpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": err.to_value(),
    })
}

fn is_valid_jsonrpc_id(v: &Value) -> bool {
    matches!(v, Value::String(_) | Value::Number(_) | Value::Null)
}

fn read_only_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

fn action_annotations(idempotent: bool, open_world: bool) -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": idempotent,
        "openWorldHint": open_world,
    })
}

fn tool_def(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": read_only_annotations(),
    })
}

fn action_tool_def(
    name: &str,
    description: &str,
    input_schema: Value,
    idempotent: bool,
    open_world: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": action_annotations(idempotent, open_world),
    })
}

fn is_action_tool_name(name: &str) -> bool {
    tool_definitions().into_iter().any(|t| {
        t.get("name").and_then(Value::as_str) == Some(name)
            && t.get("annotations")
                .and_then(|a| a.get("readOnlyHint"))
                .and_then(Value::as_bool)
                == Some(false)
    })
}

/// Return only read-only (`annotations.readOnlyHint == true`) M3 MCP tool definitions.
pub fn read_only_tool_definitions() -> Vec<Value> {
    tool_definitions()
        .into_iter()
        .filter(|t| {
            t.get("annotations")
                .and_then(|a| a.get("readOnlyHint"))
                .and_then(Value::as_bool)
                == Some(true)
        })
        .collect()
}

fn empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

/// Return the canonical list of 40 full-capability M3 MCP tool definitions (24 read-only + 16 controlled action/Alan/desktop tools).
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool_def(
            "m3_status",
            "Get Circle watchlist summary, Strike Zone statuses, benchmark price, sync freshness, and paper decision journal balance (read-only).",
            empty_object_schema(),
        ),
        tool_def(
            "m3_doctor",
            "Diagnose M3 binary version, config, workspace, DuckDB lakehouse counts, storage footprint, and host tools (read-only; works without an initialized workspace).",
            empty_object_schema(),
        ),
        tool_def(
            "m3_report",
            "Compute and return the deterministic Charlie Calculator MungerReport (rust_decimal ROIC, Strict Owner Earnings = OCF - CapEx - SBC, reverse valuation, Strike Price, warnings, and XBRL provenance) for a watchlist ticker.",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Stock ticker symbol in the Circle watchlist (e.g. AAPL, MCO)."
                    }
                },
                "required": ["ticker"],
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_dossier",
            "Load a company's human-agent interrogation dossier (AI cross-disciplinary draft, 5-year pre-mortem, and human_judgment fields) read-only. Never mutates human_judgment.",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Stock ticker symbol (e.g. AAPL, MCO)."
                    }
                },
                "required": ["ticker"],
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_universe_status",
            "Show L0 full-US DuckDB/Parquet lakehouse summary counts (companies, annual/TTM facts, price pool, sync timestamps). Returns zeros if not yet synced.",
            empty_object_schema(),
        ),
        tool_def(
            "m3_universe_screen",
            "Screen and rank companies in the local L0 DuckDB lakehouse by deterministic financial quality metrics (ROIC, SBC/OCF, 3Y share dilution, net debt / Owner Earnings, Strike Zone).",
            json!({
                "type": "object",
                "properties": {
                    "min_roic": {
                        "type": "number",
                        "description": "Minimum 5-year average ROIC (%). Range: -10000 to 100000."
                    },
                    "max_sbc_ocf": {
                        "type": "number",
                        "description": "Maximum SBC / OCF (%). Range: -10000 to 100000."
                    },
                    "max_share_change_3y": {
                        "type": "number",
                        "description": "Maximum 3-year share count change (%). Range: -10000 to 100000."
                    },
                    "max_net_debt_to_oe": {
                        "type": "number",
                        "description": "Maximum Net Debt / Strict Owner Earnings ratio. Range: -10000 to 100000."
                    },
                    "min_years": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Minimum years of SEC financial data (1..=20)."
                    },
                    "exclude_financials": {
                        "type": "boolean",
                        "default": true,
                        "description": "Exclude SIC 6000-6799 financials (default: true)."
                    },
                    "positive_oe_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Require positive Strict Owner Earnings."
                    },
                    "price_pool_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Require a latest daily price snapshot."
                    },
                    "strike_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Filter to companies inside the Strike Zone."
                    },
                    "query": {
                        "type": "string",
                        "description": "Case-insensitive ticker, company name, or SIC filter."
                    },
                    "sort": {
                        "type": "string",
                        "enum": [
                            "quality",
                            "roic",
                            "fcf_yield",
                            "distance",
                            "share_change",
                            "sbc_ocf",
                            "price",
                            "strike",
                            "warnings",
                            "ticker",
                            "name",
                            "sic",
                            "oe"
                        ],
                        "default": "quality",
                        "description": "Sort column (default: quality)."
                    },
                    "sort_asc": {
                        "type": "boolean",
                        "default": false,
                        "description": "Sort ascending when true, descending when false (default: false)."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 5000,
                        "default": 200,
                        "description": "Maximum rows returned (1..=5000, default: 200)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_universe_peers",
            "Compare a company against its SIC peers in the local L0 DuckDB lakehouse (ROIC, gross margin, and strict FCF yield percentiles plus top peers).",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Target stock ticker symbol (e.g. MCO, ADSK)."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "default": 15,
                        "description": "Maximum peer rows returned (1..=200, default: 15)."
                    }
                },
                "required": ["ticker"],
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_universe_similar",
            "Hybrid deterministic financial hard-filter + 256d cosine/BM25 semantic similarity search across local L1 SEC doc_chunks.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Natural-language moat/business query, or a ticker symbol when as_ticker is true."
                    },
                    "as_ticker": {
                        "type": "boolean",
                        "default": false,
                        "description": "Treat query as a ticker symbol in the lakehouse rather than free text."
                    },
                    "min_roic": {
                        "type": "number",
                        "description": "Minimum 5-year average ROIC (%). Range: -10000 to 100000."
                    },
                    "max_sbc_ocf": {
                        "type": "number",
                        "description": "Maximum SBC / OCF (%). Range: -10000 to 100000."
                    },
                    "exclude_financials": {
                        "type": "boolean",
                        "default": true,
                        "description": "Exclude SIC 6000-6799 financials (default: true)."
                    },
                    "positive_oe_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Require positive Strict Owner Earnings."
                    },
                    "price_pool_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Require a latest daily price snapshot."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "default": 20,
                        "description": "Maximum similarity hits returned (1..=200, default: 20)."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_inbox_list",
            "List L1 event-driven research signals (discovery, ripple, strike, red_flag) with traceable filing evidence from DuckDB (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["all", "unread", "researching", "watch", "too_hard", "archived"],
                        "default": "all",
                        "description": "Filter by signal status (default: all)."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["discovery", "ripple", "strike", "red_flag"],
                        "description": "Optional signal kind filter."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 5000,
                        "default": 50,
                        "description": "Maximum signals returned (1..=5000, default: 50)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_industry_list",
            "List all saved Industry Research Studio profiles (slug, status, SIC codes, bound tickers, valuation anchor, Feynman progress).",
            empty_object_schema(),
        ),
        tool_def(
            "m3_industry_show",
            "Load a full Industry Research Studio profile JSON and its Markdown primer (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Industry profile slug (e.g. enterprise-cad-software)."
                    }
                },
                "required": ["slug"],
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_budget_status",
            "Inspect monthly/per-run L2 Alan Deep Research budget limits, current month token/USD usage, and remaining auto-trigger quota.",
            empty_object_schema(),
        ),
        tool_def(
            "m3_themes_list",
            "Read saved commercial archetype theme clusters from workbench/themes.json (read-only).",
            empty_object_schema(),
        ),
        tool_def(
            "m3_storage_status",
            "Inspect local disk usage across M3's 5 storage buckets (including protected personal_workbench). Read-only; never deletes personal_workbench.",
            empty_object_schema(),
        ),
        tool_def(
            "m3_briefing_latest",
            "Fetch the latest persisted calm Circle briefing (Strike Radar, Filing & Red-Flag Delta, Alan L1.5 Watchdog Hits) from DuckDB (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "period": {
                        "type": "string",
                        "enum": ["daily", "weekly"],
                        "description": "Optional briefing cadence filter (daily or weekly)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_briefing_list",
            "List persisted calm Circle briefings newest-first from DuckDB (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "period": {
                        "type": "string",
                        "enum": ["daily", "weekly"],
                        "description": "Optional briefing cadence filter (daily or weekly)."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10,
                        "description": "Maximum briefings returned (1..=50, default: 10)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_triage_list",
            "List persisted L1.5 Circle Filing Watchdog triage evaluations and daily triage quota status from DuckDB (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Optional stock ticker symbol filter (e.g. MCO)."
                    },
                    "verdict": {
                        "type": "string",
                        "enum": ["noise", "material", "falsification_alert"],
                        "description": "Optional Watchdog triage verdict filter."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 50,
                        "description": "Maximum triage records returned (1..=100, default: 50)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_alan_status",
            "Read Alan's active connection profile summary (default profile, provider, model, non-secret hint) without exposing secrets (read-only).",
            empty_object_schema(),
        ),
        tool_def(
            "m3_alan_session_show",
            "Load persisted Alan interrogation session messages for a scope (`company:TICKER|industry:SLUG|briefing:latest|portfolio`) read-only.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "minLength": 1,
                        "description": "AlanScope (`company:TICKER`, `industry:SLUG`, `briefing:latest`, or `portfolio`)."
                    }
                },
                "required": ["scope"],
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_journal_list",
            "List paper decision journal entries, cash balance, and open paper positions (read-only).",
            empty_object_schema(),
        ),
        tool_def(
            "m3_memo_read",
            "Read a company's saved Living Research Memo (`workbench/memos/<TICKER>.md`) and Feynman Defense history (`workbench/memos/<TICKER>-feynman.md`) read-only.",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Stock ticker symbol (e.g. COST, MCO)."
                    }
                },
                "required": ["ticker"],
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_events_list",
            "List ingested L1 SEC filing / news events from DuckDB with optional ticker, event_type, and unprocessed filters (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Optional stock ticker filter."
                    },
                    "event_type": {
                        "type": "string",
                        "description": "Optional event type filter (e.g. sec_8k, sec_10k)."
                    },
                    "unprocessed_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Filter to unprocessed events only."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 5000,
                        "default": 50,
                        "description": "Maximum events returned (1..=5000, default: 50)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_research_status",
            "List L2 Deep Research audit runs and monthly AI budget status, opportunistically reclaiming stale >= 15m running rows when RW DuckDB is free (read-only view).",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Optional stock ticker symbol filter."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 20,
                        "description": "Maximum research runs returned (1..=500, default: 20)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        tool_def(
            "m3_app_state",
            "Inspect the live state snapshot of the running M3 desktop window over local control IPC (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 120,
                        "default": 5,
                        "description": "IPC timeout in seconds (1..=120, default: 5)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        action_tool_def(
            "m3_alan_test",
            "Probe an Alan LLM connection profile over the network and report HTTP status and latency.",
            json!({
                "type": "object",
                "properties": {
                    "profile_id": {
                        "type": "string",
                        "description": "Optional profile ID (defaults to active default profile)."
                    }
                },
                "additionalProperties": false
            }),
            true,
            true,
        ),
        action_tool_def(
            "m3_alan_ask",
            "Run a scoped Alan interrogation turn (`company:TICKER|industry:SLUG|briefing:latest|portfolio`), persist the bounded session, and return prose, Turing Tape, and verified InlineEvidenceCards.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "minLength": 1,
                        "description": "AlanScope (`company:TICKER`, `industry:SLUG`, `briefing:latest`, or `portfolio`)."
                    },
                    "question": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Question to ask Alan within the selected scope."
                    },
                    "lang": {
                        "type": "string",
                        "enum": ["zh", "en"],
                        "description": "Optional response language override (defaults to configured UI language)."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "default": 120,
                        "description": "Maximum seconds to wait for Alan reply (1..=600, default: 120)."
                    }
                },
                "required": ["scope", "question"],
                "additionalProperties": false
            }),
            false,
            true,
        ),
        action_tool_def(
            "m3_alan_session_clear",
            "Clear the persisted Alan interrogation session file for a scope without touching dossiers or human_judgment.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "minLength": 1,
                        "description": "AlanScope (`company:TICKER`, `industry:SLUG`, `briefing:latest`, or `portfolio`)."
                    }
                },
                "required": ["scope"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
        action_tool_def(
            "m3_watchlist_add",
            "Add or update a company's Circle bucket (`core|in|watch|out|too_hard`), circle label, or Too Hard rationale in watchlist.json and DuckDB.",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Stock ticker symbol (e.g. COST, MCO)."
                    },
                    "bucket": {
                        "type": "string",
                        "enum": ["core", "in", "watch", "out", "too_hard"],
                        "default": "watch",
                        "description": "Target Circle bucket (default: watch)."
                    },
                    "name": {
                        "type": "string",
                        "description": "Optional company display name override."
                    },
                    "circle": {
                        "type": "string",
                        "description": "Optional circle/thesis label."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Optional note or Too Hard rationale."
                    }
                },
                "required": ["ticker"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
        action_tool_def(
            "m3_sync",
            "Sync SEC EDGAR XBRL financials and Yahoo daily prices for the Circle watchlist (or a single ticker).",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Optional single ticker symbol to sync."
                    },
                    "prices_only": {
                        "type": "boolean",
                        "default": false,
                        "description": "Refresh daily prices only without fetching SEC XBRL."
                    }
                },
                "additionalProperties": false
            }),
            true,
            true,
        ),
        action_tool_def(
            "m3_inbox_triage",
            "Update an L1 signal status (`unread|researching|watch|too_hard|archived`) and optionally promote its primary_ticker to WATCH.",
            json!({
                "type": "object",
                "properties": {
                    "signal_id": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Target signal_id in signal_inbox."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["unread", "researching", "watch", "too_hard", "archived"],
                        "description": "Target signal status."
                    },
                    "promote_watch": {
                        "type": "boolean",
                        "default": false,
                        "description": "Promote primary_ticker into watchlist WATCH bucket."
                    }
                },
                "required": ["signal_id", "status"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
        action_tool_def(
            "m3_journal_record",
            "Append a pre-committed paper decision journal entry (`wait|out|too_hard|paper_buy|paper_sell`). Never mutates human_judgment.",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Stock ticker symbol."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["wait", "out", "too_hard", "paper_buy", "paper_sell"],
                        "description": "Paper decision action."
                    },
                    "note": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Pre-commitment thesis or wait rationale."
                    },
                    "shares": {
                        "type": ["number", "string"],
                        "description": "Optional share count (required for paper_buy / paper_sell)."
                    },
                    "falsification": {
                        "type": "string",
                        "description": "Optional falsification condition."
                    }
                },
                "required": ["ticker", "action", "note"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        action_tool_def(
            "m3_falsification_update",
            "Add, remove, or replace `human_judgment.falsification_metrics` on a dossier without ever modifying `feynman_checked_at`, `my_stance`, or `## Human Notes`.",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Stock ticker symbol."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["add", "remove", "replace"],
                        "description": "Update operation: add | remove | replace."
                    },
                    "metric": {
                        "type": "string",
                        "description": "Falsification metric text (required when action is `add`)."
                    },
                    "index": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "0-based index (required when action is `remove`)."
                    },
                    "metrics": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Replacement metrics list (required when action is `replace`)."
                    }
                },
                "required": ["ticker", "action"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        action_tool_def(
            "m3_research_run",
            "Run budget-gated L2 4-perspective Deep Research for a ticker and write `workbench/memos/<TICKER>.md` while preserving any `## Human Notes` footer.",
            json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Stock ticker symbol."
                    },
                    "focus": {
                        "type": "string",
                        "description": "Optional research focus prompt."
                    },
                    "trigger": {
                        "type": "string",
                        "enum": ["manual", "inbox", "cli"],
                        "default": "cli",
                        "description": "Run trigger label (default: cli)."
                    }
                },
                "required": ["ticker"],
                "additionalProperties": false
            }),
            false,
            true,
        ),
        action_tool_def(
            "m3_briefing_run",
            "Generate and persist a daily or weekly Circle briefing (with optional 0-token `--no-llm` mode).",
            json!({
                "type": "object",
                "properties": {
                    "period": {
                        "type": "string",
                        "enum": ["daily", "weekly"],
                        "default": "daily",
                        "description": "Briefing cadence: daily | weekly."
                    },
                    "no_llm": {
                        "type": "boolean",
                        "default": false,
                        "description": "Skip Alan executive summary (0 LLM tokens)."
                    }
                },
                "additionalProperties": false
            }),
            false,
            true,
        ),
        action_tool_def(
            "m3_triage_run",
            "Run L1.5 Circle Filing Watchdog triage against WATCH/CORE falsification metrics (respecting daily quota and supporting dry_run).",
            json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 10,
                        "description": "Maximum pending signals to evaluate (1..=100, default: 10)."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "default": false,
                        "description": "Preview without calling Alan or writing DuckDB."
                    }
                },
                "additionalProperties": false
            }),
            false,
            true,
        ),
        action_tool_def(
            "m3_feynman_round",
            "Run one Socratic Feynman Defense critique round (`1..=3`) for a company or industry scope. Never auto-signs `feynman_checked_at`.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Feynman scope (`company:TICKER` or `industry:SLUG`)."
                    },
                    "round": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 3,
                        "description": "Feynman defense round number (1, 2, or 3)."
                    },
                    "answer": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Human investor's plain-language explanation for this round."
                    },
                    "lang": {
                        "type": "string",
                        "enum": ["zh", "en"],
                        "description": "Optional language override."
                    },
                    "draft_metrics": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional draft falsification metrics for company context."
                    }
                },
                "required": ["scope", "round", "answer"],
                "additionalProperties": false
            }),
            false,
            true,
        ),
        action_tool_def(
            "m3_app_navigate",
            "Navigate the running M3 desktop window to a workspace tab, company dossier (or `\"\"` to close), or industry study.",
            json!({
                "type": "object",
                "properties": {
                    "tab": {
                        "type": "string",
                        "enum": ["inbox", "universe", "industry", "baskets", "journal", "settings"],
                        "description": "Target workspace tab."
                    },
                    "ticker": {
                        "type": "string",
                        "description": "Company ticker to open (or empty string `\"\"` to close company overlay)."
                    },
                    "industry": {
                        "type": "string",
                        "description": "Industry profile slug to select."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 120,
                        "default": 5,
                        "description": "IPC timeout in seconds (1..=120, default: 5)."
                    }
                },
                "additionalProperties": false
            }),
            true,
            false,
        ),
        action_tool_def(
            "m3_app_alan",
            "Configure the unified Alan Studio canvas (`closed|split|immersive`, `interrogate|feynman`, scope, pin lock, anchor, prompt, submit) in the running M3 desktop window.",
            json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["closed", "split", "immersive"],
                        "description": "Alan stage mode."
                    },
                    "tab": {
                        "type": "string",
                        "enum": ["interrogate", "feynman"],
                        "description": "Alan canvas tab."
                    },
                    "scope": {
                        "type": "string",
                        "description": "AlanScope (`company:TICKER`, `industry:SLUG`, `briefing:latest`, `portfolio`)."
                    },
                    "pin": {
                        "type": "boolean",
                        "description": "Lock or unlock Alan auto-follow scope pin."
                    },
                    "anchor_label": {
                        "type": "string",
                        "description": "Optional UI anchor label."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Prompt text to pre-fill in Alan input."
                    },
                    "submit": {
                        "type": "boolean",
                        "default": false,
                        "description": "Submit the prompt immediately to Alan."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 120,
                        "default": 5,
                        "description": "IPC timeout in seconds (1..=120, default: 5)."
                    }
                },
                "additionalProperties": false
            }),
            false,
            false,
        ),
        action_tool_def(
            "m3_app_action",
            "Trigger a named UI action (`toggle_sidebar`, `open_palette`, `close_palette`, `close_company`, `sync`, `refresh_briefing`, `reset_alan`, `deep_research`) in the running M3 desktop window.",
            json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Action identifier."
                    },
                    "ticker": {
                        "type": "string",
                        "description": "Optional ticker parameter (used by `deep_research`)."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 120,
                        "default": 5,
                        "description": "IPC timeout in seconds (1..=120, default: 5)."
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        action_tool_def(
            "m3_app_screenshot",
            "Capture a PNG screenshot of the running M3 desktop window to `output_path` via local control IPC.",
            json!({
                "type": "object",
                "properties": {
                    "output_path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Destination PNG file path."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 120,
                        "default": 5,
                        "description": "IPC timeout in seconds (1..=120, default: 5)."
                    }
                },
                "required": ["output_path"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
    ]
}

fn require_object_or_empty(
    value: Option<&Value>,
) -> std::result::Result<&Map<String, Value>, JsonRpcError> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    match value {
        None | Some(Value::Null) => Ok(EMPTY.get_or_init(Map::new)),
        Some(Value::Object(map)) => Ok(map),
        Some(_) => Err(JsonRpcError::invalid_params(
            "tools/call `arguments` must be an object",
        )),
    }
}

fn reject_unknown_keys(
    map: &Map<String, Value>,
    allowed: &[&str],
) -> std::result::Result<(), JsonRpcError> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(JsonRpcError::invalid_params(format!(
                "unexpected argument `{key}`"
            )));
        }
    }
    Ok(())
}

fn require_non_empty_string(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<String, JsonRpcError> {
    let raw = map
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            JsonRpcError::invalid_params(format!("argument `{key}` must be a non-empty string"))
        })?;
    Ok(raw.to_owned())
}

fn optional_trimmed_string(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<String>, JsonRpcError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
        }
        Some(_) => Err(JsonRpcError::invalid_params(format!(
            "argument `{key}` must be a string"
        ))),
    }
}

fn optional_bool(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<bool>, JsonRpcError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(v)) => Ok(Some(*v)),
        Some(_) => Err(JsonRpcError::invalid_params(format!(
            "argument `{key}` must be a boolean"
        ))),
    }
}

fn optional_usize_range(
    map: &Map<String, Value>,
    key: &str,
    min: usize,
    max: usize,
) -> std::result::Result<Option<usize>, JsonRpcError> {
    let Some(val) = map.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(raw_u64) = val.as_u64() else {
        return Err(JsonRpcError::invalid_params(format!(
            "argument `{key}` must be an integer between {min} and {max}"
        )));
    };
    let parsed = usize::try_from(raw_u64).map_err(|_| {
        JsonRpcError::invalid_params(format!(
            "argument `{key}` must be an integer between {min} and {max}"
        ))
    })?;
    if parsed < min || parsed > max {
        return Err(JsonRpcError::invalid_params(format!(
            "argument `{key}` must be between {min} and {max}, got {parsed}"
        )));
    }
    Ok(Some(parsed))
}

fn optional_decimal_range(
    map: &Map<String, Value>,
    key: &str,
    min_i64: i64,
    max_i64: i64,
) -> std::result::Result<Option<Decimal>, JsonRpcError> {
    let Some(val) = map.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let token = match val {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => {
            return Err(JsonRpcError::invalid_params(format!(
                "argument `{key}` must be a number"
            )));
        }
    };
    let parsed = parse_decimal_token(&token).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid `{key}` decimal value: {err}"))
    })?;
    let min = Decimal::from(min_i64);
    let max = Decimal::from(max_i64);
    if parsed < min || parsed > max {
        return Err(JsonRpcError::invalid_params(format!(
            "argument `{key}` must be between {min} and {max}, got {parsed}"
        )));
    }
    Ok(Some(parsed))
}

fn optional_briefing_period(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<BriefingPeriod>, JsonRpcError> {
    match optional_trimmed_string(map, key)? {
        None => Ok(None),
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "daily" => Ok(Some(BriefingPeriod::Daily)),
            "weekly" => Ok(Some(BriefingPeriod::Weekly)),
            _ => Err(JsonRpcError::invalid_params(format!(
                "invalid `{key}`: expected `daily` or `weekly`, got `{raw}`"
            ))),
        },
    }
}

fn parse_mcp_triage_verdict(raw: &str) -> std::result::Result<TriageVerdict, JsonRpcError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "noise" => Ok(TriageVerdict::Noise),
        "material" => Ok(TriageVerdict::Material),
        "falsification_alert" => Ok(TriageVerdict::FalsificationAlert),
        _ => Err(JsonRpcError::invalid_params(format!(
            "invalid `verdict`: expected noise|material|falsification_alert, got `{raw}`"
        ))),
    }
}

fn parse_mcp_alan_scope(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<AlanScope, JsonRpcError> {
    let raw = require_non_empty_string(map, key)?;
    parse_alan_scope(&raw).map_err(|err| JsonRpcError::invalid_params(format!("{err:#}")))
}

fn parse_mcp_feynman_scope(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<FeynmanScope, JsonRpcError> {
    let parsed = parse_mcp_alan_scope(map, key)?;
    match parsed {
        AlanScope::Company(ticker) => Ok(FeynmanScope::Company { ticker }),
        AlanScope::Industry(slug) => Ok(FeynmanScope::Industry { slug }),
        _ => Err(JsonRpcError::invalid_params(
            "feynman scope must be `company:TICKER` or `industry:SLUG`",
        )),
    }
}

fn optional_string_allow_empty(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<String>, JsonRpcError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => Ok(Some(raw.clone())),
        Some(_) => Err(JsonRpcError::invalid_params(format!(
            "argument `{key}` must be a string"
        ))),
    }
}

fn optional_app_timeout_secs(map: &Map<String, Value>) -> std::result::Result<u64, JsonRpcError> {
    Ok(
        optional_usize_range(map, "timeout_secs", 1, MAX_APP_TIMEOUT_SECS)?
            .map(|s| s as u64)
            .unwrap_or(app_control::DEFAULT_APP_CONTROL_TIMEOUT.as_secs()),
    )
}

fn optional_decimal_any(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<Decimal>, JsonRpcError> {
    let Some(val) = map.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let token = match val {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => {
            return Err(JsonRpcError::invalid_params(format!(
                "argument `{key}` must be a number or decimal string"
            )));
        }
    };
    let parsed = parse_decimal_token(&token).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid `{key}` decimal value: {err}"))
    })?;
    Ok(Some(parsed))
}

fn required_usize_index(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<usize, JsonRpcError> {
    let val = map.get(key).ok_or_else(|| {
        JsonRpcError::invalid_params(format!(
            "argument `{key}` is required as a non-negative integer"
        ))
    })?;
    let raw_u64 = val.as_u64().ok_or_else(|| {
        JsonRpcError::invalid_params(format!("argument `{key}` must be a non-negative integer"))
    })?;
    usize::try_from(raw_u64)
        .map_err(|_| JsonRpcError::invalid_params(format!("argument `{key}` integer out of range")))
}

fn required_string_array(
    map: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Vec<String>, JsonRpcError> {
    let arr = map.get(key).and_then(Value::as_array).ok_or_else(|| {
        JsonRpcError::invalid_params(format!("argument `{key}` must be an array of strings"))
    })?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item.as_str().ok_or_else(|| {
            JsonRpcError::invalid_params(format!("every element of `{key}` must be a string"))
        })?;
        out.push(s.to_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use m3_core::{
        budget::{BudgetConfig, ThemeCluster, ThemesFile, save_budget_config, save_themes},
        industry::{
            IndustryProfile, IndustryStatus, save_industry,
            write_industry_primer_preserving_human_notes,
        },
        lake::{LakeDb, UniverseRow},
        lake_l1::{DocChunkRow, SignalEvidence, SignalInboxRow},
        latent::{
            LOCAL_EMBEDDING_DIM, LOCAL_EMBEDDING_MODEL, encode_embedding_json, local_embed,
            sha256_hex,
        },
        model::{
            Bucket, CompanyEntry, DataSource, DecisionJournal, Dossier, HumanJudgment, Latticework,
            ValuationAssumptions, WarningFlag, WarningSeverity, WatchlistFile,
        },
        proactive::{
            BriefingPeriod, CircleBriefing, SignalTriageRecord, TriageQuote, TriageVerdict,
            WatchdogHitItem,
        },
    };
    use std::fs;
    use std::io::Cursor;

    fn temp_workspace_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "m3-mcp-{label}-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench/dossiers")).unwrap();
        fs::create_dir_all(dir.join("workbench/industries")).unwrap();
        dir
    }

    fn seed_synthetic_workspace(dir: &std::path::Path) -> WorkspacePaths {
        let paths = WorkspacePaths::from_root(dir.to_path_buf());
        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![CompanyEntry {
                    ticker: "MCO".into(),
                    name: "Moody's Corporation".into(),
                    bucket: Bucket::Core,
                    source: DataSource::Manual,
                    cik: Some("0001059556".into()),
                    currency: "USD".into(),
                    circle: "Credit Ratings & Analytics".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(20),
                        multiple_high: Decimal::from(24),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(8),
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

        let dossier = Dossier {
            schema_version: 1,
            ticker: "MCO".into(),
            name: "Moody's Corporation".into(),
            updated_at: "2026-11-25T00:00:00Z".into(),
            draft_source: "agent_draft".into(),
            three_sentence_business: "Sells bond ratings and credit analytics.".into(),
            latticework: Latticework {
                microeconomics: "Toll-bridge oligopoly".into(),
                ecology: "Debt capital markets".into(),
                psychology_incentives: "Issuer-pays credential".into(),
                capital_allocation: "Buybacks and dividends".into(),
            },
            premortem: vec!["Regulatory unbundling of rating mandates".into()],
            open_questions: vec!["Private credit rating share shift?".into()],
            human_judgment: HumanJudgment {
                pending_human_review: false,
                my_stance: Some("Keep human stance untouched".into()),
                weight_rationale: Some("Toll bridge remains intact".into()),
                falsification_metrics: vec!["Operating margin < 35%".into()],
                feynman_checked_at: Some("2026-11-01".into()),
            },
        };
        store::write_json_atomic(&paths.dossier_path("MCO"), &dossier).unwrap();

        save_budget_config(&paths, &BudgetConfig::default()).unwrap();
        save_themes(
            &paths,
            &ThemesFile {
                schema_version: 1,
                updated_at_ms: 1_700_000_000_000,
                total_indexed_companies: 2,
                clusters: vec![ThemeCluster {
                    theme_id: "toll-bridge".into(),
                    title: "Ratings Toll Bridges".into(),
                    archetype: "Toll Bridge".into(),
                    sic_codes: vec![7320],
                    keywords: vec!["ratings".into(), "credit".into()],
                    summary: "Oligopoly toll bridges".into(),
                    tickers: vec!["MCO".into(), "SPGI".into()],
                    members: Vec::new(),
                    avg_similarity: 0.92,
                    median_roic_5y_pct: Some(Decimal::from(35)),
                    strike_zone_count: 1,
                    user_notes: "Human note preserved".into(),
                }],
            },
        )
        .unwrap();

        let profile = IndustryProfile {
            slug: "credit-ratings".into(),
            title: "Credit Ratings Oligopoly".into(),
            status: IndustryStatus::InsideCircle,
            sic_codes: vec![7320],
            keywords: vec!["ratings".into(), "royalty".into()],
            bound_tickers: vec!["MCO".into()],
            multiple_low: Some(Decimal::from(20)),
            multiple_high: Some(Decimal::from(25)),
            key_kpis: vec!["Debt issuance".into()],
            feynman_questions: vec!["Why dual ratings?".into()],
            feynman_checked: vec![true],
            human_notes: "Regulatory moat.".into(),
            created_at_ms: 1000,
            updated_at_ms: 2000,
            last_research_run_id: None,
        };
        save_industry(&paths, &profile).unwrap();
        write_industry_primer_preserving_human_notes(
            &paths,
            "credit-ratings",
            "# Credit Ratings\nToll-bridge economics.",
            Some("Human memo"),
        )
        .unwrap();

        let rows = vec![
            UniverseRow {
                ticker: "MCO".into(),
                cik: "0001059556".into(),
                name: "Moody's Corporation".into(),
                exchange: "NYSE".into(),
                sic: Some(7320),
                sic_description: Some("Consumer Credit Reporting".into()),
                is_financial: false,
                latest_filing_end: Some("2025-12-31".into()),
                years_of_data: 8,
                quality_score: Decimal::from(92),
                in_price_pool: true,
                in_watchlist: true,
                bucket_opt: Some(Bucket::Core),
                latest_price: Some(Decimal::from(400)),
                strike_price: Some(Decimal::from(420)),
                distance_to_strike_pct: Some(Decimal::new(-48, 1)),
                strict_fcf_yield_pct: Some(Decimal::new(35, 1)),
                roic_5y_avg_pct: Some(Decimal::from(35)),
                roic_5y_stddev_pct: Some(Decimal::new(15, 1)),
                gross_margin_5y_avg_pct: Some(Decimal::from(72)),
                gross_margin_5y_stddev_pct: Some(Decimal::new(9, 1)),
                share_change_3y_pct: Some(Decimal::from(-3)),
                sbc_to_ocf_pct: Some(Decimal::from(7)),
                net_debt_to_oe: Some(Decimal::new(18, 1)),
                strict_owner_earnings: Some(Decimal::from(2_500_000_000i64)),
                warnings_count: 1,
                warnings_json: serde_json::to_string(&vec![WarningFlag {
                    code: "INFO".into(),
                    severity: WarningSeverity::Info,
                    title: "Note".into(),
                    detail: "Synthetic".into(),
                }])
                .unwrap(),
                raw_sha256: Some("sha-mco".into()),
            },
            UniverseRow {
                ticker: "SPGI".into(),
                cik: "0000064040".into(),
                name: "S&P Global Inc.".into(),
                exchange: "NYSE".into(),
                sic: Some(7320),
                sic_description: Some("Consumer Credit Reporting".into()),
                is_financial: false,
                latest_filing_end: Some("2025-12-31".into()),
                years_of_data: 8,
                quality_score: Decimal::from(90),
                in_price_pool: true,
                in_watchlist: false,
                bucket_opt: None,
                latest_price: Some(Decimal::from(450)),
                strike_price: Some(Decimal::from(440)),
                distance_to_strike_pct: Some(Decimal::new(23, 1)),
                strict_fcf_yield_pct: Some(Decimal::new(32, 1)),
                roic_5y_avg_pct: Some(Decimal::from(30)),
                roic_5y_stddev_pct: Some(Decimal::new(18, 1)),
                gross_margin_5y_avg_pct: Some(Decimal::from(68)),
                gross_margin_5y_stddev_pct: Some(Decimal::new(11, 1)),
                share_change_3y_pct: Some(Decimal::from(-2)),
                sbc_to_ocf_pct: Some(Decimal::from(6)),
                net_debt_to_oe: Some(Decimal::new(20, 1)),
                strict_owner_earnings: Some(Decimal::from(4_000_000_000i64)),
                warnings_count: 0,
                warnings_json: "[]".into(),
                raw_sha256: Some("sha-spgi".into()),
            },
        ];

        let mut db = LakeDb::open(&paths).unwrap();
        db.replace_sec_lake_data(&[], &[], &[], &rows).unwrap();
        db.ensure_l1_schema(&paths).unwrap();

        let mco_text = "Moody's operates a credit ratings toll bridge with high switching costs.";
        let spgi_text =
            "S&P Global provides credit ratings benchmarks and index licensing toll bridges.";
        db.upsert_doc_chunks(&[
            DocChunkRow {
                chunk_id: "mco-1".into(),
                cik: "0001059556".into(),
                ticker: "MCO".into(),
                doc_type: "10-K".into(),
                source_id: "0001059556-26-000001".into(),
                period_end: Some("2025-12-31".into()),
                filing_date: Some("2026-02-15".into()),
                section: "item_1".into(),
                title: "Business".into(),
                content: mco_text.into(),
                content_sha256: sha256_hex(mco_text.as_bytes()),
                source_url: "https://www.sec.gov/Archives/edgar/data/1059556/000105955626000001"
                    .into(),
                embedding_model: LOCAL_EMBEDDING_MODEL.into(),
                embedding_dim: LOCAL_EMBEDDING_DIM,
                embedding_json: encode_embedding_json(&local_embed(mco_text)),
            },
            DocChunkRow {
                chunk_id: "spgi-1".into(),
                cik: "0000064040".into(),
                ticker: "SPGI".into(),
                doc_type: "10-K".into(),
                source_id: "0000064040-26-000001".into(),
                period_end: Some("2025-12-31".into()),
                filing_date: Some("2026-02-14".into()),
                section: "item_1".into(),
                title: "Business".into(),
                content: spgi_text.into(),
                content_sha256: sha256_hex(spgi_text.as_bytes()),
                source_url: "https://www.sec.gov/Archives/edgar/data/64040/000006404026000001"
                    .into(),
                embedding_model: LOCAL_EMBEDDING_MODEL.into(),
                embedding_dim: LOCAL_EMBEDDING_DIM,
                embedding_json: encode_embedding_json(&local_embed(spgi_text)),
            },
        ])
        .unwrap();

        db.upsert_signals(&[SignalInboxRow {
            signal_id: "sig-mco-strike".into(),
            kind: SignalKind::Strike,
            score: 0.88,
            primary_ticker: "MCO".into(),
            tickers: vec!["MCO".into(), "SPGI".into()],
            title: "MCO entered Strike Zone".into(),
            summary: "Strict FCF yield 3.5% with 35% 5Y ROIC".into(),
            evidence: vec![SignalEvidence {
                chunk_id: Some("mco-1".into()),
                ticker: "MCO".into(),
                doc_type: "10-K".into(),
                section: "item_1".into(),
                filing_date: Some("2026-02-15".into()),
                title: "Business".into(),
                quote: mco_text.into(),
                source_url: "https://www.sec.gov/Archives/edgar/data/1059556/000105955626000001"
                    .into(),
                similarity: 0.95,
            }],
            status: SignalStatus::Unread,
            event_id: None,
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
        }])
        .unwrap();

        paths
    }

    fn call_tool(server: &mut McpServer, id: i64, name: &str, arguments: Value) -> Value {
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
            }
        });
        server.handle_line(&req.to_string()).unwrap()
    }

    fn parse_tool_payload(resp: &Value) -> (bool, Value) {
        let result = &resp["result"];
        let is_error = result["isError"].as_bool().unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        (is_error, payload)
    }

    #[test]
    fn mcp_initialize_ping_notification_and_tools_list() {
        let dir = temp_workspace_dir("init");
        let paths = seed_synthetic_workspace(&dir);
        let mut server = McpServer::with_workspace(paths.clone());

        let init_resp = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0"}}}"#,
            )
            .unwrap();
        assert_eq!(init_resp["jsonrpc"], "2.0");
        assert_eq!(init_resp["id"], 1);
        assert_eq!(init_resp["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init_resp["result"]["serverInfo"]["name"], "m3");
        assert!(
            init_resp["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("human_judgment")
        );

        let notif = server.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert!(notif.is_none());

        let ping_resp = server
            .handle_line(r#"{"jsonrpc":"2.0","id":"ping-1","method":"ping"}"#)
            .unwrap();
        assert_eq!(ping_resp["id"], "ping-1");
        assert_eq!(ping_resp["result"], json!({}));

        let list_resp = server
            .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap();
        let tools = list_resp["result"]["tools"].as_array().unwrap();
        let expected_read_only = [
            "m3_status",
            "m3_doctor",
            "m3_report",
            "m3_dossier",
            "m3_universe_status",
            "m3_universe_screen",
            "m3_universe_peers",
            "m3_universe_similar",
            "m3_inbox_list",
            "m3_industry_list",
            "m3_industry_show",
            "m3_budget_status",
            "m3_themes_list",
            "m3_storage_status",
            "m3_briefing_latest",
            "m3_briefing_list",
            "m3_triage_list",
            "m3_alan_status",
            "m3_alan_session_show",
            "m3_journal_list",
            "m3_memo_read",
            "m3_events_list",
            "m3_research_status",
            "m3_app_state",
        ];
        let expected_actions = [
            "m3_alan_test",
            "m3_alan_ask",
            "m3_alan_session_clear",
            "m3_watchlist_add",
            "m3_sync",
            "m3_inbox_triage",
            "m3_journal_record",
            "m3_falsification_update",
            "m3_research_run",
            "m3_briefing_run",
            "m3_triage_run",
            "m3_feynman_round",
            "m3_app_navigate",
            "m3_app_alan",
            "m3_app_action",
            "m3_app_screenshot",
        ];
        assert_eq!(
            tools.len(),
            expected_read_only.len() + expected_actions.len()
        );
        let actual_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for name in expected_read_only {
            assert!(
                actual_names.contains(&name),
                "missing read-only tool {name}"
            );
            let tool = tools.iter().find(|t| t["name"] == name).unwrap();
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
        }
        for name in expected_actions {
            assert!(actual_names.contains(&name), "missing action tool {name}");
            let tool = tools.iter().find(|t| t["name"] == name).unwrap();
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["annotations"]["readOnlyHint"], false);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
        }

        // `--read-only` mode filters `tools/list` to readOnlyHint=true and rejects non-read-only calls.
        let mut ro_server = McpServer::with_workspace(paths).with_read_only(true);
        let ro_list = ro_server
            .handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#)
            .unwrap();
        let ro_tools = ro_list["result"]["tools"].as_array().unwrap();
        assert_eq!(ro_tools.len(), expected_read_only.len());
        for tool in ro_tools {
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
        }
        for blocked_name in [
            "m3_watchlist_add",
            "m3_alan_ask",
            "m3_app_navigate",
            "m3_journal_record",
        ] {
            let blocked = call_tool(&mut ro_server, 4, blocked_name, json!({}));
            assert_eq!(blocked["error"]["code"], JSONRPC_INVALID_PARAMS);
            assert!(
                blocked["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("--read-only")
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_tools_call_executes_all_read_only_tools_and_preserves_sovereignty() {
        let dir = temp_workspace_dir("tools");
        let paths = seed_synthetic_workspace(&dir);
        let dossier_before = fs::read_to_string(paths.dossier_path("MCO")).unwrap();
        let mut server = McpServer::with_workspace(paths.clone());

        let (err, status) = parse_tool_payload(&call_tool(&mut server, 1, "m3_status", json!({})));
        assert!(!err);
        assert_eq!(status["ok"], true);
        assert_eq!(status["data"]["companies"][0]["ticker"], "MCO");

        let (err, doctor) = parse_tool_payload(&call_tool(&mut server, 2, "m3_doctor", json!({})));
        assert!(!err);
        assert_eq!(doctor["ok"], true);
        assert_eq!(doctor["data"]["healthy"], true);

        let (err, report) = parse_tool_payload(&call_tool(
            &mut server,
            3,
            "m3_report",
            json!({"ticker": "MCO"}),
        ));
        assert!(!err);
        assert_eq!(report["ok"], true);
        assert_eq!(report["data"]["ticker"], "MCO");

        let (err, dossier) = parse_tool_payload(&call_tool(
            &mut server,
            4,
            "m3_dossier",
            json!({"ticker": "MCO"}),
        ));
        assert!(!err);
        assert_eq!(dossier["ok"], true);
        assert_eq!(
            dossier["data"]["human_judgment"]["my_stance"],
            "Keep human stance untouched"
        );

        let (err, u_status) =
            parse_tool_payload(&call_tool(&mut server, 5, "m3_universe_status", json!({})));
        assert!(!err);
        assert_eq!(u_status["data"]["total_companies"], 2);

        let (err, screen) = parse_tool_payload(&call_tool(
            &mut server,
            6,
            "m3_universe_screen",
            json!({"min_roic": 32, "sort": "roic", "limit": 10}),
        ));
        assert!(!err);
        assert_eq!(screen["data"]["count"], 1);
        assert_eq!(screen["data"]["rows"][0]["ticker"], "MCO");

        let (err, peers) = parse_tool_payload(&call_tool(
            &mut server,
            7,
            "m3_universe_peers",
            json!({"ticker": "MCO", "limit": 5}),
        ));
        assert!(!err);
        assert_eq!(peers["data"]["target"]["ticker"], "MCO");
        assert_eq!(peers["data"]["top_peers"][0]["ticker"], "SPGI");

        let (err, similar) = parse_tool_payload(&call_tool(
            &mut server,
            8,
            "m3_universe_similar",
            json!({"query": "credit ratings toll bridge", "min_roic": 20, "limit": 5}),
        ));
        assert!(!err);
        assert!(similar["data"]["count"].as_u64().unwrap() >= 1);

        let (err, inbox) = parse_tool_payload(&call_tool(
            &mut server,
            9,
            "m3_inbox_list",
            json!({"status": "unread", "kind": "strike", "limit": 10}),
        ));
        assert!(!err);
        assert_eq!(inbox["data"]["count"], 1);
        assert_eq!(inbox["data"]["signals"][0]["signal_id"], "sig-mco-strike");

        let (err, ind_list) =
            parse_tool_payload(&call_tool(&mut server, 10, "m3_industry_list", json!({})));
        assert!(!err);
        assert_eq!(ind_list["data"]["count"], 1);

        let (err, ind_show) = parse_tool_payload(&call_tool(
            &mut server,
            11,
            "m3_industry_show",
            json!({"slug": "credit-ratings"}),
        ));
        assert!(!err);
        assert_eq!(
            ind_show["data"]["profile"]["title"],
            "Credit Ratings Oligopoly"
        );

        let (err, budget) =
            parse_tool_payload(&call_tool(&mut server, 12, "m3_budget_status", json!({})));
        assert!(!err);
        assert_eq!(budget["ok"], true);

        let (err, themes) =
            parse_tool_payload(&call_tool(&mut server, 13, "m3_themes_list", json!({})));
        assert!(!err);
        assert_eq!(themes["data"]["clusters"][0]["theme_id"], "toll-bridge");

        let (err, storage_res) =
            parse_tool_payload(&call_tool(&mut server, 14, "m3_storage_status", json!({})));
        assert!(!err);
        assert_eq!(storage_res["ok"], true);

        // Domain error (missing ticker) returns isError: true without crashing the server.
        let (domain_err, missing_rep) = parse_tool_payload(&call_tool(
            &mut server,
            15,
            "m3_report",
            json!({"ticker": "ZZZZ"}),
        ));
        assert!(domain_err);
        assert_eq!(missing_rep["ok"], false);
        assert!(missing_rep["error"].as_str().unwrap().contains("ZZZZ"));

        // Verify human_judgment and personal_workbench files were never mutated or deleted.
        let dossier_after = fs::read_to_string(paths.dossier_path("MCO")).unwrap();
        assert_eq!(dossier_before, dossier_after);
        assert!(paths.watchlist_path().exists());
        assert!(paths.journal_path().exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_jsonrpc_errors_and_doctor_without_workspace() {
        let mut server = McpServer::without_workspace("synthetic missing workspace");

        // Parse error
        let parse_err = server.handle_line("{not json").unwrap();
        assert_eq!(parse_err["error"]["code"], JSONRPC_PARSE_ERROR);
        assert_eq!(parse_err["id"], Value::Null);

        // Invalid request (missing jsonrpc 2.0)
        let invalid_req = server
            .handle_line(r#"{"jsonrpc":"1.0","id":10,"method":"ping"}"#)
            .unwrap();
        assert_eq!(invalid_req["error"]["code"], JSONRPC_INVALID_REQUEST);
        assert_eq!(invalid_req["id"], 10);

        // Method not found
        let method_err = server
            .handle_line(r#"{"jsonrpc":"2.0","id":11,"method":"resources/list"}"#)
            .unwrap();
        assert_eq!(method_err["error"]["code"], JSONRPC_METHOD_NOT_FOUND);

        // Unknown tool -> JSON-RPC -32602
        let unknown_tool = call_tool(&mut server, 12, "m3_nonexistent", json!({}));
        assert_eq!(unknown_tool["error"]["code"], JSONRPC_INVALID_PARAMS);

        // Bad params (missing required ticker, wrong type, unknown arg, out of range)
        let missing_ticker = call_tool(&mut server, 13, "m3_report", json!({}));
        assert_eq!(missing_ticker["error"]["code"], JSONRPC_INVALID_PARAMS);

        let wrong_type = call_tool(&mut server, 14, "m3_report", json!({"ticker": 42}));
        assert_eq!(wrong_type["error"]["code"], JSONRPC_INVALID_PARAMS);

        let extra_arg = call_tool(&mut server, 15, "m3_status", json!({"bogus": true}));
        assert_eq!(extra_arg["error"]["code"], JSONRPC_INVALID_PARAMS);

        let bad_limit = call_tool(
            &mut server,
            16,
            "m3_universe_screen",
            json!({"limit": 99999}),
        );
        assert_eq!(bad_limit["error"]["code"], JSONRPC_INVALID_PARAMS);

        let bad_sort = call_tool(
            &mut server,
            17,
            "m3_universe_screen",
            json!({"sort": "magic"}),
        );
        assert_eq!(bad_sort["error"]["code"], JSONRPC_INVALID_PARAMS);

        // m3_doctor still succeeds without a workspace
        let (doc_err, doc_payload) =
            parse_tool_payload(&call_tool(&mut server, 18, "m3_doctor", json!({})));
        assert!(!doc_err);
        assert_eq!(doc_payload["ok"], true);
        assert_eq!(doc_payload["data"]["healthy"], false);

        // Workspace tool returns isError: true cleanly when workspace is unavailable
        let (ws_err, ws_payload) =
            parse_tool_payload(&call_tool(&mut server, 19, "m3_status", json!({})));
        assert!(ws_err);
        assert_eq!(ws_payload["ok"], false);
        assert!(
            ws_payload["error"]
                .as_str()
                .unwrap()
                .contains("synthetic missing workspace")
        );
    }

    #[test]
    fn mcp_stdio_roundtrip_over_cursor_and_vec() {
        let dir = temp_workspace_dir("stdio");
        let paths = seed_synthetic_workspace(&dir);
        let mut server = McpServer::with_workspace(paths);

        let input = concat!(
            "\n",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"m3_status","arguments":{}}}"#,
            "\n"
        );

        let cursor = Cursor::new(input.as_bytes());
        let mut out = Vec::<u8>::new();
        server.serve_io(cursor, &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        // Notification produced no response, so 4 requests -> 4 response lines.
        assert_eq!(lines.len(), 4);

        let r1: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(r1["id"], 1);
        assert_eq!(r1["result"]["protocolVersion"], "2024-11-05");

        let r2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(r2["id"], 2);
        assert_eq!(r2["result"], json!({}));

        let r3: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(r3["id"], 3);
        assert_eq!(r3["result"]["tools"].as_array().unwrap().len(), 40);

        let r4: Value = serde_json::from_str(lines[3]).unwrap();
        assert_eq!(r4["id"], 4);
        assert_eq!(r4["result"]["isError"], false);
        let status_body: Value =
            serde_json::from_str(r4["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(status_body["data"]["companies"][0]["ticker"], "MCO");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_proactive_tools_handle_empty_and_seeded_workspaces_read_only() {
        let dir = temp_workspace_dir("proactive-tools");
        let paths = WorkspacePaths::from_root(dir.clone());
        let mut empty_server = McpServer::with_workspace(paths.clone());

        // Workspace without m3.duckdb returns None / empty lists gracefully without crashing.
        let (err, pre_latest) = parse_tool_payload(&call_tool(
            &mut empty_server,
            100,
            "m3_briefing_latest",
            json!({"period": "daily"}),
        ));
        assert!(!err);
        assert_eq!(pre_latest["data"]["briefing"], Value::Null);

        let (err, pre_list) = parse_tool_payload(&call_tool(
            &mut empty_server,
            101,
            "m3_briefing_list",
            json!({}),
        ));
        assert!(!err);
        assert_eq!(pre_list["data"]["count"], 0);

        let (err, pre_triage) = parse_tool_payload(&call_tool(
            &mut empty_server,
            102,
            "m3_triage_list",
            json!({}),
        ));
        assert!(!err);
        assert_eq!(pre_triage["data"]["count"], 0);
        assert_eq!(pre_triage["data"]["daily_quota"], 10);
        assert!(!paths.lake_db_path().exists());

        let paths = seed_synthetic_workspace(&dir);
        let mut server = McpServer::with_workspace(paths.clone());

        // Seed proactive briefing & triage records using LakeDb wrappers.
        let now = store::now_ms();
        {
            let db = LakeDb::open(&paths).unwrap();
            db.upsert_signal_triage(&SignalTriageRecord {
                signal_id: "sig-mco-strike".into(),
                ticker: "MCO".into(),
                verdict: TriageVerdict::Material,
                headline: "MIS issuance acceleration".into(),
                rationale: "Debt issuance rebounded cleanly.".into(),
                matched_metrics: vec!["Operating margin < 35%".into()],
                suggested_metrics: vec![],
                quotes: vec![TriageQuote {
                    section: "item_7".into(),
                    filing_date: Some("2026-02-15".into()),
                    quote: "Corporate issuance grew 18% YoY.".into(),
                    source_url: "https://www.sec.gov/mco".into(),
                }],
                fallback_no_metrics_warning: false,
                model_used: "test-model".into(),
                tokens_used: 1200,
                cost_usd: Decimal::new(1, 2),
                created_at_ms: now,
            })
            .unwrap();

            db.upsert_circle_briefing(&CircleBriefing {
                briefing_id: "briefing-daily-2026-11-26".into(),
                period: BriefingPeriod::Daily,
                period_key: "2026-11-26".into(),
                generated_at_ms: now,
                lang: m3_core::UiLang::En,
                is_quiet: false,
                headline: "Circle Daily Briefing · 1 Watchdog Hits".into(),
                executive_summary: Some("Watchdog flagged MIS issuance acceleration.".into()),
                summary_tokens_used: 300,
                summary_cost_usd: Decimal::new(2, 3),
                strike_radar: vec![],
                filing_deltas: vec![],
                watchdog_hits: vec![WatchdogHitItem {
                    signal_id: "sig-mco-strike".into(),
                    ticker: "MCO".into(),
                    verdict: TriageVerdict::Material,
                    headline: "MIS issuance acceleration".into(),
                    rationale: "Debt issuance rebounded cleanly.".into(),
                    fallback_no_metrics_warning: false,
                    quotes: vec![],
                    created_at_ms: now,
                }],
            })
            .unwrap();
        }

        let (err, latest) = parse_tool_payload(&call_tool(
            &mut server,
            103,
            "m3_briefing_latest",
            json!({"period": "daily"}),
        ));
        assert!(!err);
        assert_eq!(
            latest["data"]["briefing"]["briefing_id"],
            "briefing-daily-2026-11-26"
        );

        let (err, list) = parse_tool_payload(&call_tool(
            &mut server,
            104,
            "m3_briefing_list",
            json!({"period": "daily", "limit": 5}),
        ));
        assert!(!err);
        assert_eq!(list["data"]["count"], 1);

        let (err, triage) = parse_tool_payload(&call_tool(
            &mut server,
            105,
            "m3_triage_list",
            json!({"ticker": "mco", "verdict": "material", "limit": 10}),
        ));
        assert!(!err);
        assert_eq!(triage["data"]["count"], 1);
        assert_eq!(triage["data"]["triages_today"], 1);
        assert_eq!(triage["data"]["remaining_today"], 9);
        assert_eq!(triage["data"]["triages"][0]["signal_id"], "sig-mco-strike");

        // Invalid params yield JSON-RPC -32602.
        let bad_period = call_tool(
            &mut server,
            106,
            "m3_briefing_latest",
            json!({"period": "monthly"}),
        );
        assert_eq!(bad_period["error"]["code"], JSONRPC_INVALID_PARAMS);

        let bad_verdict = call_tool(
            &mut server,
            107,
            "m3_triage_list",
            json!({"verdict": "bogus"}),
        );
        assert_eq!(bad_verdict["error"]["code"], JSONRPC_INVALID_PARAMS);

        let bad_limit = call_tool(&mut server, 108, "m3_briefing_list", json!({"limit": 999}));
        assert_eq!(bad_limit["error"]["code"], JSONRPC_INVALID_PARAMS);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_full_capability_read_action_and_app_tools_work_and_preserve_sovereignty() {
        use m3_core::{
            alan_session::{
                AlanMessageRole, PersistedAlanMessage, PersistedAlanSession, save_alan_session,
            },
            app_control::{AppControlResponse, AppControlState},
            budget::BudgetConfig,
            lake_l1::LakeEventRow,
            lake_l2::{DEFAULT_STALE_RESEARCH_RUN_MS, ResearchRunRow, ResearchRunStatus},
            research,
        };

        let dir = temp_workspace_dir("full-capability");
        let paths = seed_synthetic_workspace(&dir);

        // Seed memo, Feynman history, Alan session, L1 event, and stale running research run.
        let memo_path = research::memo_path_for(&paths, "MCO");
        fs::create_dir_all(memo_path.parent().unwrap()).unwrap();
        fs::write(
            &memo_path,
            "# MCO Memo\n## Human Notes\nKeep my original human note.\n",
        )
        .unwrap();

        let scope = AlanScope::Company("MCO".into());
        let mut session = PersistedAlanSession::empty(&scope);
        session.messages.push(PersistedAlanMessage {
            id: "u-1".into(),
            role: AlanMessageRole::User,
            text: "Toll bridge check".into(),
            raw_text: "Toll bridge check".into(),
            created_at_ms: 1_700_000_000_000,
        });
        save_alan_session(&paths, &session).unwrap();

        let now = store::now_ms();
        {
            let db = LakeDb::open(&paths).unwrap();
            db.upsert_events(&[LakeEventRow {
                event_id: "ev-mco-10k".into(),
                source: "sec_edgar".into(),
                cik: Some("0001059556".into()),
                ticker: Some("MCO".into()),
                event_type: "sec_10k".into(),
                occurred_at_ms: now,
                filing_date: Some("2026-02-15".into()),
                title: "Moody's 10-K".into(),
                summary: "Annual report".into(),
                url: "https://www.sec.gov/mco10k".into(),
                metadata_json: "{}".into(),
                processed_mask: 0,
            }])
            .unwrap();
            db.insert_research_run(&ResearchRunRow {
                run_id: "run-stale-mcp".into(),
                ticker: "MCO".into(),
                trigger: "cli".into(),
                focus: None,
                model: "test-model".into(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: Decimal::ZERO,
                started_at_ms: now.saturating_sub(DEFAULT_STALE_RESEARCH_RUN_MS + 10_000),
                finished_at_ms: None,
                status: ResearchRunStatus::Running,
                error: None,
                memo_path: None,
                perspectives_json: "[]".into(),
                citations_json: "[]".into(),
            })
            .unwrap();
        }

        // Mock desktop app Unix control socket for m3_app_* tools (keep path < SUN_LEN on macOS).
        #[cfg(unix)]
        let sock_path = PathBuf::from(format!(
            "/tmp/m3-mcp-{}-{}.sock",
            std::process::id(),
            store::now_ms()
        ));
        #[cfg(unix)]
        let _ = fs::remove_file(&sock_path);
        #[cfg(unix)]
        let app_server_thread = {
            use std::io::{BufRead, BufReader, Write};
            use std::os::unix::net::UnixListener;

            let listener = UnixListener::bind(&sock_path).unwrap();
            std::thread::spawn(move || {
                for stream_res in listener.incoming().take(5) {
                    let mut stream = stream_res.unwrap();
                    let mut line = String::new();
                    {
                        let mut reader = BufReader::new(&stream);
                        reader.read_line(&mut line).unwrap();
                    }
                    let req: AppControlRequest = serde_json::from_str(line.trim()).unwrap();
                    let resp = match req {
                        AppControlRequest::GetState => AppControlResponse {
                            ok: true,
                            state: Some(AppControlState {
                                active_tab: "universe".into(),
                                selected_ticker: "MCO".into(),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        AppControlRequest::Navigate { tab, ticker, .. } => AppControlResponse {
                            ok: true,
                            state: Some(AppControlState {
                                active_tab: tab.unwrap_or_default(),
                                selected_ticker: ticker.unwrap_or_default(),
                                company_open: true,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        AppControlRequest::SetAlan {
                            mode, scope, pin, ..
                        } => AppControlResponse {
                            ok: true,
                            state: Some(AppControlState {
                                alan_stage_mode: mode.unwrap_or_default(),
                                alan_scope: scope.unwrap_or_default(),
                                alan_context_pinned: pin.unwrap_or(false),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        AppControlRequest::TriggerAction { action, .. } => AppControlResponse {
                            ok: true,
                            message: Some(format!("triggered {action}")),
                            ..Default::default()
                        },
                        AppControlRequest::Screenshot { output_path } => AppControlResponse {
                            ok: true,
                            screenshot_path: Some(output_path),
                            ..Default::default()
                        },
                        _ => AppControlResponse::default(),
                    };
                    let mut out = serde_json::to_string(&resp).unwrap();
                    out.push('\n');
                    stream.write_all(out.as_bytes()).unwrap();
                }
            })
        };

        #[cfg(unix)]
        let mut server =
            McpServer::with_workspace(paths.clone()).with_app_control_socket(sock_path.clone());
        #[cfg(not(unix))]
        let mut server = McpServer::with_workspace(paths.clone());

        // New read-only tools.
        let (err, alan_status) =
            parse_tool_payload(&call_tool(&mut server, 200, "m3_alan_status", json!({})));
        assert!(!err);
        assert_eq!(alan_status["ok"], true);

        let (err, session_show) = parse_tool_payload(&call_tool(
            &mut server,
            201,
            "m3_alan_session_show",
            json!({"scope": "company:mco"}),
        ));
        assert!(!err);
        assert_eq!(session_show["data"]["message_count"], 1);

        let (err, journal_list) =
            parse_tool_payload(&call_tool(&mut server, 202, "m3_journal_list", json!({})));
        assert!(!err);
        assert_eq!(journal_list["data"]["cash_balance"], "100000");

        let (err, memo) = parse_tool_payload(&call_tool(
            &mut server,
            203,
            "m3_memo_read",
            json!({"ticker": "mco"}),
        ));
        assert!(!err);
        assert_eq!(memo["data"]["memo_exists"], true);
        assert!(
            memo["data"]["memo_markdown"]
                .as_str()
                .unwrap()
                .contains("## Human Notes")
        );

        let (err, events) = parse_tool_payload(&call_tool(
            &mut server,
            204,
            "m3_events_list",
            json!({"ticker": "mco", "event_type": "sec_10k", "limit": 10}),
        ));
        assert!(!err);
        assert_eq!(events["data"]["count"], 1);

        let (err, r_status) = parse_tool_payload(&call_tool(
            &mut server,
            205,
            "m3_research_status",
            json!({"ticker": "MCO", "limit": 10}),
        ));
        assert!(!err);
        assert_eq!(r_status["data"]["reclaimed_stale_runs"], 1);
        assert_eq!(r_status["data"]["runs"][0]["status"], "failed");

        // Action tools.
        let (err, session_clear) = parse_tool_payload(&call_tool(
            &mut server,
            206,
            "m3_alan_session_clear",
            json!({"scope": "company:MCO"}),
        ));
        assert!(!err);
        assert_eq!(session_clear["data"]["cleared"], true);

        let (err, wl_add) = parse_tool_payload(&call_tool(
            &mut server,
            207,
            "m3_watchlist_add",
            json!({"ticker": "CSU.TO", "bucket": "watch", "name": "Constellation Software Inc.", "circle": "Vertical Market Software"}),
        ));
        assert!(!err);
        assert_eq!(wl_add["data"]["ticker"], "CSU.TO");
        assert_eq!(wl_add["data"]["bucket"], "watch");

        let (err, inbox_tr) = parse_tool_payload(&call_tool(
            &mut server,
            208,
            "m3_inbox_triage",
            json!({"signal_id": "sig-mco-strike", "status": "watch"}),
        ));
        assert!(!err);
        assert_eq!(inbox_tr["data"]["signal"]["status"], "watch");

        let (err, journal_rec) = parse_tool_payload(&call_tool(
            &mut server,
            209,
            "m3_journal_record",
            json!({"ticker": "MCO", "action": "wait", "note": "Wait for 20x OE"}),
        ));
        assert!(!err);
        assert_eq!(journal_rec["data"]["ticker"], "MCO");
        assert_eq!(journal_rec["data"]["action"], "WAIT");

        // `m3_falsification_update` preserves `feynman_checked_at` and `my_stance`.
        let (err, fals_add) = parse_tool_payload(&call_tool(
            &mut server,
            210,
            "m3_falsification_update",
            json!({"ticker": "MCO", "action": "add", "metric": "MIS margin < 50%"}),
        ));
        assert!(!err);
        assert_eq!(fals_add["data"]["count"], 2);
        let reloaded_dossier = store::load_dossier(&paths, "MCO").unwrap().unwrap();
        assert_eq!(
            reloaded_dossier
                .human_judgment
                .feynman_checked_at
                .as_deref(),
            Some("2026-11-01")
        );
        assert_eq!(
            reloaded_dossier.human_judgment.my_stance.as_deref(),
            Some("Keep human stance untouched")
        );

        // `m3_briefing_run --no-llm` & `m3_triage_run --dry-run`.
        let (err, brief_run) = parse_tool_payload(&call_tool(
            &mut server,
            211,
            "m3_briefing_run",
            json!({"period": "daily", "no_llm": true}),
        ));
        assert!(!err);
        assert_eq!(brief_run["data"]["source"], "generated");

        let (err, triage_run) = parse_tool_payload(&call_tool(
            &mut server,
            212,
            "m3_triage_run",
            json!({"limit": 5, "dry_run": true}),
        ));
        assert!(!err);
        assert_eq!(triage_run["data"]["dry_run"], true);

        // `m3_research_run` respects budget gate cleanly.
        save_budget_config(
            &paths,
            &BudgetConfig {
                monthly_budget_usd: Decimal::ZERO,
                ..BudgetConfig::default()
            },
        )
        .unwrap();
        let (blocked_err, blocked_run) = parse_tool_payload(&call_tool(
            &mut server,
            213,
            "m3_research_run",
            json!({"ticker": "MCO", "focus": "moat"}),
        ));
        assert!(blocked_err);
        assert_eq!(blocked_run["data"]["status"], "blocked_budget");

        // Desktop app IPC tools over isolated socket.
        #[cfg(unix)]
        {
            let (err, app_state) =
                parse_tool_payload(&call_tool(&mut server, 214, "m3_app_state", json!({})));
            assert!(!err);
            assert_eq!(app_state["data"]["state"]["active_tab"], "universe");

            let (err, app_nav) = parse_tool_payload(&call_tool(
                &mut server,
                215,
                "m3_app_navigate",
                json!({"tab": "baskets", "ticker": "mco"}),
            ));
            assert!(!err);
            assert_eq!(app_nav["data"]["state"]["selected_ticker"], "MCO");

            let (err, app_alan) = parse_tool_payload(&call_tool(
                &mut server,
                216,
                "m3_app_alan",
                json!({"mode": "split", "scope": "company:mco", "pin": true}),
            ));
            assert!(!err);
            assert_eq!(app_alan["data"]["state"]["alan_stage_mode"], "split");
            assert_eq!(app_alan["data"]["state"]["alan_scope"], "company:MCO");

            let (err, app_act) = parse_tool_payload(&call_tool(
                &mut server,
                217,
                "m3_app_action",
                json!({"action": "toggle_sidebar"}),
            ));
            assert!(!err);
            assert_eq!(app_act["data"]["message"], "triggered toggle_sidebar");

            let (err, app_shot) = parse_tool_payload(&call_tool(
                &mut server,
                218,
                "m3_app_screenshot",
                json!({"output_path": "/tmp/m3-mcp-shot.png"}),
            ));
            assert!(!err);
            assert_eq!(app_shot["data"]["screenshot_path"], "/tmp/m3-mcp-shot.png");

            app_server_thread.join().unwrap();
        }

        // Invalid params checks on new tools.
        assert_eq!(
            call_tool(
                &mut server,
                219,
                "m3_feynman_round",
                json!({"scope": "portfolio", "round": 1, "answer": "hi"})
            )["error"]["code"],
            JSONRPC_INVALID_PARAMS
        );
        assert_eq!(
            call_tool(
                &mut server,
                220,
                "m3_alan_ask",
                json!({"scope": "bogus", "question": "hi"})
            )["error"]["code"],
            JSONRPC_INVALID_PARAMS
        );
        assert_eq!(
            call_tool(&mut server, 221, "m3_app_navigate", json!({}))["error"]["code"],
            JSONRPC_INVALID_PARAMS
        );
        assert_eq!(
            call_tool(
                &mut server,
                222,
                "m3_falsification_update",
                json!({"ticker": "MCO", "action": "add", "metric": "x", "index": 0})
            )["error"]["code"],
            JSONRPC_INVALID_PARAMS
        );

        #[cfg(unix)]
        let _ = fs::remove_file(&sock_path);
        let _ = fs::remove_dir_all(&dir);
    }
}
