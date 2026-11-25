# M3 MCP Server (`m3 mcp`)

Project M3 provides a dependency-free, read-only **Model Context Protocol (MCP)** stdio server built directly into the `m3` CLI binary (`crates/m3-cli/src/mcp.rs`). External agents (Claude Code, Codex, Antigravity, standalone Alan, or any MCP-compatible host) can query deterministic M3 research data without parsing CLI flags or touching screenshots.

## Launch & Transport

```sh
m3 mcp [--workspace <PATH>]
```

- **Transport**: Line-delimited JSON-RPC 2.0 over `stdin` / `stdout` (one JSON object per line, `\n` delimited, flushed per response).
- **Stdout Isolation**: `m3 mcp` bypasses the CLI's JSON envelope wrapper; `stdout` carries JSON-RPC 2.0 responses exclusively, while any diagnostics are written only to `stderr`.
- **Supported Methods**:
  - `initialize` (returns `protocolVersion`, `capabilities.tools`, `serverInfo`, and M3 safety `instructions`)
  - `notifications/initialized` (notification; no response emitted)
  - `ping`
  - `tools/list`
  - `tools/call`
- **Workspace Resolution**:
  1. Explicit `--workspace <PATH>` flag on `m3 mcp`
  2. `M3_REPO_ROOT` environment variable
  3. Current working directory (or ancestor containing `workbench/watchlist.json`)
  4. Configured root via `m3 workspace set <PATH>` or default M3 workspace

## Client Configuration

### Claude Code

```sh
claude mcp add m3 -- m3 mcp
```

Or `.mcp.json` in your project root:

```json
{
  "mcpServers": {
    "m3": {
      "command": "m3",
      "args": ["mcp"]
    }
  }
}
```

### Codex CLI

In `~/.codex/config.toml`:

```toml
[mcp_servers.m3]
command = "m3"
args = ["mcp"]
```

### Antigravity / Standard `mcpServers` JSON

```json
{
  "mcpServers": {
    "m3": {
      "command": "m3",
      "args": ["mcp", "--workspace", "/path/to/m3-workbench"]
    }
  }
}
```

## Exposed Tools (Read-Only)

All 14 tools set MCP `annotations.readOnlyHint = true` and `destructiveHint = false`:

| Tool | Description | Required Params | Optional Params |
| :--- | :--- | :--- | :--- |
| `m3_status` | Watchlist summary, Strike Zone statuses, benchmark, sync freshness, journal cash | — | — |
| `m3_doctor` | Binary, config, workspace, DuckDB counts, storage, host tool diagnostics | — | — |
| `m3_report` | Deterministic Charlie Calculator `MungerReport` (`rust_decimal` + XBRL provenance) | `ticker` | — |
| `m3_dossier` | Load company interrogation dossier (AI draft + `human_judgment`) | `ticker` | — |
| `m3_universe_status` | L0 DuckDB/Parquet lakehouse company, fact, and price counts | — | — |
| `m3_universe_screen` | Filter & rank US equities in DuckDB lakehouse | — | `min_roic`, `max_sbc_ocf`, `max_share_change_3y`, `max_net_debt_to_oe`, `min_years`, `exclude_financials`, `positive_oe_only`, `price_pool_only`, `strike_only`, `query`, `sort`, `sort_asc`, `limit` |
| `m3_universe_peers` | SIC peer comparison with ROIC, gross margin, and strict FCF yield percentiles | `ticker` | `limit` |
| `m3_universe_similar` | Financial hard-filter + 256d cosine/BM25 hybrid search over L1 `doc_chunks` | `query` | `as_ticker`, `min_roic`, `max_sbc_ocf`, `exclude_financials`, `positive_oe_only`, `price_pool_only`, `limit` |
| `m3_inbox_list` | List L1 `discovery` / `ripple` / `strike` / `red_flag` signals with evidence | — | `status`, `kind`, `limit` |
| `m3_industry_list` | List Industry Research Studio profiles and Feynman progress | — | — |
| `m3_industry_show` | Load full `IndustryProfile` JSON and Markdown primer | `slug` | — |
| `m3_budget_status` | Inspect L2 Alan Deep Research monthly/per-run budget limits and usage | — | — |
| `m3_themes_list` | Read saved commercial archetype theme clusters (`workbench/themes.json`) | — | — |
| `m3_storage_status` | Scan disk usage across all 5 storage buckets (`personal_workbench` protected) | — | — |

## Error Handling & Safety Guarantees

- **JSON-RPC Protocol Errors**: Malformed JSON (`-32700`), invalid request envelopes (`-32600`), unknown RPC methods (`-32601`), and unknown tool names or schema/range violations (`-32602`) return standard JSON-RPC errors without terminating the server.
- **Tool Domain Errors**: Missing watchlist tickers, unsynced lakehouses, missing dossiers/industries, or uninitialized workspaces return a `tools/call` result with `"isError": true` and structured `{"schema_version": 1, "ok": false, "error": "..."}` JSON text.
- **Read-Only Safety**: `m3 mcp` exposes no write, sync, cleanup, or trade tools. It **never mutates `human_judgment`** and **never deletes `personal_workbench`**.

For agent prompt guidance, see [skills/m3/SKILL.md](../skills/m3/SKILL.md) and [docs/agent-interface.md](agent-interface.md).
