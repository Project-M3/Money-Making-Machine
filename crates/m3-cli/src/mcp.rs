use crate::{
    MAX_FILTER_METRIC, MAX_L1_LIST_LIMIT, MAX_MIN_YEARS, MAX_PEERS_LIMIT, MAX_SCREEN_LIMIT,
    MAX_SIMILAR_LIMIT, MIN_FILTER_METRIC, cmd_budget_status, cmd_doctor_for_workspace, cmd_dossier,
    cmd_report, cmd_status, cmd_storage_with_updater, cmd_themes_list, cmd_universe_status,
    execute_inbox_list, execute_industry_list, execute_industry_show, execute_universe_peers,
    execute_universe_screen, execute_universe_similar,
};
use anyhow::{Context, Result, ensure};
use m3_core::{
    app_config,
    lake::{UniverseFilter, UniverseSortBy},
    lake_l1::{SignalKind, SignalStatus, SimilarSearchFilter},
    model::parse_decimal_token,
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

/// Read-only JSON-RPC 2.0 MCP server for Project M3 deterministic research tools.
pub struct McpServer {
    workspace: std::result::Result<WorkspacePaths, String>,
    updater_dir: Option<PathBuf>,
    initialized: bool,
}

impl McpServer {
    /// Construct a server using an optional explicit `--workspace` override or standard M3 workspace discovery.
    pub fn from_env(workspace_override: Option<PathBuf>) -> Result<Self> {
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
            initialized: false,
        })
    }

    /// Construct a server pinned to a known `WorkspacePaths` (primarily for isolated testing).
    pub fn with_workspace(paths: WorkspacePaths) -> Self {
        Self {
            workspace: Ok(paths),
            updater_dir: None,
            initialized: false,
        }
    }

    /// Construct a server without a resolved workspace (for testing `m3_doctor` and workspace error paths).
    pub fn without_workspace(err_msg: impl Into<String>) -> Self {
        Self {
            workspace: Err(err_msg.into()),
            updater_dir: None,
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
            "tools/list" => handle_tools_list(params),
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
            "instructions": "Project M3 deterministic value-investing workbench (read-only MCP tools). All financial arithmetic is computed deterministically in Rust using rust_decimal; never estimate or recompute financial figures in the LLM. Never fabricate or overwrite human_judgment, and never delete personal_workbench."
        }))
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

/// CLI entrypoint for `m3 mcp [--workspace <PATH>]`.
pub fn run_cli_mcp(args: &[String]) -> Result<i32> {
    let mut workspace_override: Option<PathBuf> = None;
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
            other => anyhow::bail!("unknown mcp flag: {other}"),
        }
        i += 1;
    }

    let mut server = McpServer::from_env(workspace_override)?;
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

fn handle_tools_list(params: Option<&Value>) -> std::result::Result<Value, JsonRpcError> {
    match params {
        None | Some(Value::Null) => {}
        Some(Value::Object(_)) => {}
        Some(_) => {
            return Err(JsonRpcError::invalid_params(
                "tools/list params must be an object or null",
            ));
        }
    }
    Ok(json!({ "tools": tool_definitions() }))
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

fn tool_def(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": read_only_annotations(),
    })
}

fn empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

/// Return the canonical list of 14 deterministic read-only M3 MCP tool definitions.
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
        let mut server = McpServer::with_workspace(paths);

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
        let expected_names = [
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
        ];
        assert_eq!(tools.len(), expected_names.len());
        let actual_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for name in expected_names {
            assert!(actual_names.contains(&name), "missing tool {name}");
        }
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
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
        assert_eq!(r3["result"]["tools"].as_array().unwrap().len(), 14);

        let r4: Value = serde_json::from_str(lines[3]).unwrap();
        assert_eq!(r4["id"], 4);
        assert_eq!(r4["result"]["isError"], false);
        let status_body: Value =
            serde_json::from_str(r4["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(status_body["data"]["companies"][0]["ticker"], "MCO");

        let _ = fs::remove_dir_all(&dir);
    }
}
