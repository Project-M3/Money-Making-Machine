# M3 MCP Server (`m3 mcp`)

Project M3 provides a dependency-free **Model Context Protocol (MCP)** stdio server built directly into the `m3` CLI binary (`crates/m3-cli/src/mcp.rs`). External agents (Claude Code, Codex, Antigravity, standalone Alan, or any MCP-compatible host) can query deterministic M3 research data, run bounded Alan interrogations and Feynman rounds, update controlled operational state, and inspect or drive the running `M3.app` desktop window via local IPC.

By default, `m3 mcp` runs in **full-capability mode** with **40 tools** (**24 read-only** + **16 controlled action, Alan, and desktop-control tools**), every one annotated accurately with `readOnlyHint`, `destructiveHint: false`, `idempotentHint`, and `openWorldHint`. Pass `--read-only` to restrict the server strictly to the 24 read-only tools.

## Launch & Transport

```sh
m3 mcp [--workspace <PATH>] [--read-only]
```

- **Flags**:
  - `--workspace <PATH>`: Override the M3 workbench root directory.
  - `--read-only`: Filter `tools/list` to the 24 `readOnlyHint: true` tools and reject any non-read-only `tools/call` with JSON-RPC error `-32602`.
- **Transport**: Line-delimited JSON-RPC 2.0 over `stdin` / `stdout` (one JSON object per line, `\n` delimited, flushed per response).
- **Stdout Isolation**: `m3 mcp` bypasses the CLI's JSON envelope wrapper; `stdout` carries JSON-RPC 2.0 responses exclusively, while any diagnostics are written only to `stderr`.
- **Supported Methods**:
  - `initialize` (returns `protocolVersion`, `capabilities.tools`, `serverInfo`, and mode-appropriate M3 safety `instructions`)
  - `notifications/initialized` (or `initialized`; notification, no response emitted)
  - `ping`
  - `tools/list`
  - `tools/call`
- **Workspace Resolution** (used by workspace-scoped tools; `m3_doctor`, `m3_alan_status`, `m3_alan_test`, and `m3_app_*` also work without a resolved workspace):
  1. Explicit `--workspace <PATH>` flag on `m3 mcp`
  2. `M3_REPO_ROOT` environment variable
  3. Current working directory (or ancestor containing `workbench/watchlist.json`)
  4. Configured root via `m3 workspace set <PATH>` or default M3 workspace

## Client Configuration

### Claude Code

Full-capability (40 tools):

```sh
claude mcp add m3 -- m3 mcp
```

Read-only mode (24 tools):

```sh
claude mcp add m3-readonly -- m3 mcp --read-only
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

For strictly read-only inspection, set `args = ["mcp", "--read-only"]`.

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

Add `"--read-only"` to `args` when you want read-only guarantees enforced by the MCP server itself.

## Exposed Tools (`40` Total, All `destructiveHint = false`)

### Read-Only Tools (`24`)

All 24 read-only tools set `annotations.readOnlyHint = true`, `destructiveHint = false`, `idempotentHint = true`, and `openWorldHint = false`. They remain enabled in `--read-only` mode.

| Tool | Description | Required Params | Optional Params |
| :--- | :--- | :--- | :--- |
| `m3_status` | Watchlist summary, Strike Zone statuses, benchmark, sync freshness, journal cash | — | — |
| `m3_doctor` | Binary, config, Alan connection, stale Deep Research runs, workspace, DuckDB, storage, host tools | — | — |
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
| `m3_briefing_latest` | Fetch latest persisted calm Circle briefing (`daily` / `weekly`) from DuckDB | — | `period` |
| `m3_briefing_list` | List persisted calm Circle briefings newest-first from DuckDB | — | `period`, `limit` |
| `m3_triage_list` | List persisted L1.5 Circle Filing Watchdog triage evaluations and daily quota status | — | `ticker`, `verdict`, `limit` |
| `m3_alan_status` | Read Alan's active connection profile summary (non-secret `masked_hint`, provider, model) | — | — |
| `m3_alan_session_show` | Load persisted Alan interrogation session for a scope (`company:TICKER`, `industry:SLUG`, `briefing:latest`, `portfolio`) | `scope` | — |
| `m3_journal_list` | List paper decision journal entries, cash balance, and open paper positions | — | — |
| `m3_memo_read` | Read a company's Living Research Memo (`workbench/memos/<TICKER>.md`) and Feynman Defense history (`workbench/memos/<TICKER>-feynman.md`) | `ticker` | — |
| `m3_events_list` | List ingested L1 SEC filing / news events from DuckDB | — | `ticker`, `event_type`, `unprocessed_only`, `limit` |
| `m3_research_status` | List L2 Deep Research audit runs and monthly budget status (opportunistically reclaims stale `>= 15m` running rows when RW DuckDB is free) | — | `ticker`, `limit` |
| `m3_app_state` | Snapshot the live state of the running `M3.app` desktop window over `control.sock` | — | `timeout_secs` |

### Controlled Action, Alan & Desktop Control Tools (`16`)

All 16 action tools set `annotations.readOnlyHint = false` and `destructiveHint = false`. Calling any of them when `m3 mcp` was launched with `--read-only` returns JSON-RPC `-32602` (`tool ... is disabled in --read-only MCP mode`).

| Tool | Description | Required Params | Optional Params | Annotations (`idempotent` / `openWorld`) |
| :--- | :--- | :--- | :--- | :--- |
| `m3_alan_test` | Probe an Alan LLM connection profile over the network and report HTTP status & latency | — | `profile_id` | `true` / `true` |
| `m3_alan_ask` | Run a scoped Alan interrogation turn (`company:TICKER`, `industry:SLUG`, `briefing:latest`, `portfolio`), persist the bounded session, and return prose, Turing Tape, and `InlineEvidenceCard`s | `scope`, `question` | `lang` (`zh` \| `en`), `timeout_secs` (`1..=600`, default `120`) | `false` / `true` |
| `m3_alan_session_clear` | Clear the persisted Alan session file for a scope without touching dossiers or `human_judgment` | `scope` | — | `true` / `false` |
| `m3_watchlist_add` | Add or update a company's Circle bucket (`core` \| `in` \| `watch` \| `out` \| `too_hard`), circle label, or Too Hard rationale in `watchlist.json` and DuckDB | `ticker` | `bucket` (default `watch`), `name`, `circle`, `reason` | `true` / `false` |
| `m3_sync` | Sync SEC EDGAR XBRL financials and Yahoo daily prices for the Circle watchlist or one ticker | — | `ticker`, `prices_only` | `true` / `true` |
| `m3_inbox_triage` | Update an L1 signal status (`unread` \| `researching` \| `watch` \| `too_hard` \| `archived`) and optionally promote `primary_ticker` to `WATCH` | `signal_id`, `status` | `promote_watch` | `true` / `false` |
| `m3_journal_record` | Append a pre-committed paper decision journal entry (`wait` \| `out` \| `too_hard` \| `paper_buy` \| `paper_sell`) | `ticker`, `action`, `note` | `shares`, `falsification` | `false` / `false` |
| `m3_falsification_update` | Add, remove, or replace `human_judgment.falsification_metrics` on a dossier without ever modifying `feynman_checked_at`, `my_stance`, or `## Human Notes` | `ticker`, `action` (`add` \| `remove` \| `replace`) | `metric` (for `add`), `index` (for `remove`), `metrics` (for `replace`) | `false` / `false` |
| `m3_research_run` | Run budget-gated L2 4-perspective Deep Research and write `workbench/memos/<TICKER>.md` (preserving `## Human Notes`) | `ticker` | `focus`, `trigger` (`manual` \| `inbox` \| `cli`, default `cli`) | `false` / `true` |
| `m3_briefing_run` | Generate and persist a `daily` or `weekly` Circle briefing (supports 0-token `no_llm: true`) | — | `period` (`daily` \| `weekly`), `no_llm` | `false` / `true` |
| `m3_triage_run` | Run L1.5 Circle Filing Watchdog triage against `WATCH`/`CORE` falsification metrics (daily-quota gated; supports `dry_run`) | — | `limit` (`1..=100`, default `10`), `dry_run` | `false` / `true` |
| `m3_feynman_round` | Run one Socratic Feynman Defense critique round (`1..=3`) for `company:TICKER` or `industry:SLUG`; never auto-signs `feynman_checked_at` | `scope`, `round` (`1..=3`), `answer` | `lang` (`zh` \| `en`), `draft_metrics` | `false` / `true` |
| `m3_app_navigate` | Navigate the running `M3.app` window to a workspace tab (`inbox` \| `universe` \| `industry` \| `baskets` \| `journal` \| `settings`), open a company dossier (or pass `""` to close), or select an industry slug | — | `tab`, `ticker`, `industry`, `timeout_secs` (`1..=120`, default `5`) | `true` / `false` |
| `m3_app_alan` | Configure the unified Alan Canvas (`closed` \| `split` \| `immersive`, `interrogate` \| `feynman`, `scope`, `pin`, `anchor_label`, `prompt`, `submit`) in the running `M3.app` window | — | `mode`, `tab`, `scope`, `pin`, `anchor_label`, `prompt`, `submit`, `timeout_secs` (`1..=120`, default `5`) | `false` / `false` |
| `m3_app_action` | Trigger a named UI action (`toggle_sidebar`, `open_palette`, `close_palette`, `close_company`, `sync`, `refresh_briefing`, `reset_alan`, `deep_research`) in the running `M3.app` window | `action` | `ticker`, `timeout_secs` (`1..=120`, default `5`) | `false` / `false` |
| `m3_app_screenshot` | Capture a PNG screenshot of the running `M3.app` desktop window to `output_path` via local control IPC | `output_path` | `timeout_secs` (`1..=120`, default `5`) | `true` / `false` |

## Live Desktop Control (`m3_app_*`)

`M3.app` (`apps/m3-nsh`) starts a local Unix domain socket server (`control.sock`, permissions `0600`) at `<M3Paths::config_dir>/control.sock`—or the path set by `M3_CONTROL_SOCKET`—dispatching `AppControlRequest` payloads onto the NSH engine thread:

- Inspect UI state anytime with `m3_app_state` (returns `active_tab`, `company_open`, `selected_ticker`, `selected_industry_slug`, `sidebar_collapsed`, `palette_open`, `alan_stage_mode`, `alan_canvas_tab`, `alan_scope`, `alan_context_pinned`, `alan_streaming`, `alan_message_count`, `last_alan_preview`, and `workspace_root`).
- Drive navigation and Alan Studio with `m3_app_navigate`, `m3_app_alan`, and `m3_app_action` (for example, opening `COST`, switching to `split` mode, locking the `📌` pin, and pre-filling or submitting a question).
- Capture crisp window-scoped PNG screenshots on macOS with `m3_app_screenshot` (creating parent directories automatically and invoking CoreGraphics window discovery + `screencapture -l <wid> -o -x`).

If `M3.app` is not running, `m3_app_*` tools return a structured `isError: true` error pointing to the missing socket without crashing the MCP server.

## Error Handling & Safety Guarantees

- **JSON-RPC Protocol Errors**: Malformed JSON (`-32700`), invalid request envelopes (`-32600`), unknown RPC methods (`-32601`), unknown tool names, schema/range violations, or action tools invoked in `--read-only` mode (`-32602`) return standard JSON-RPC errors without terminating the server.
- **Tool Domain Errors**: Missing watchlist tickers, unsynced lakehouses, missing dossiers/industries/memos, blocked budgets, or unlaunched desktop sockets return a `tools/call` result with `"isError": true` and structured `{"schema_version": 1, "ok": false, "error": "..."}` JSON text.
- **Human Sovereignty & Asset Protection**:
  - `m3_falsification_update` only edits `human_judgment.falsification_metrics` when explicitly invoked; no MCP tool ever mutates `my_stance`, `weight_rationale`, or `feynman_checked_at`, and `m3_feynman_round` never auto-signs graduation.
  - `m3_research_run` and `m3_alan_ask` never overwrite `## Human Notes` sections in `workbench/memos/*.md` or `workbench/industries/*.md`.
  - No MCP tool can delete `personal_workbench`.
  - `m3_research_run`, `m3_briefing_run`, and `m3_triage_run` enforce M3's daily quotas and monthly USD/token hard ceilings before calling an LLM.

For agent prompt guidance, see [skills/m3/SKILL.md](../skills/m3/SKILL.md), [skills/m3-qa/SKILL.md](../skills/m3-qa/SKILL.md), and [docs/agent-interface.md](agent-interface.md).
