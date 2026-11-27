---
name: m3
description: Connect to Project M3's v0.6.5 local-first Munger value-investing workbench via the full-capability `m3 mcp` server (40 tools, or 24 with `--read-only`), `m3` JSON CLI (`m3 alan`, `m3 app`, `m3 doctor`), or live `M3.app` desktop control IPC. Use when screening US equities in DuckDB/Parquet, inspecting deterministic SEC EDGAR XBRL reports, running scoped Alan interrogations or Socratic Feynman rounds, driving the M3 desktop canvas, or managing watchlist/journal/briefing workflows under Munger and human-sovereignty guardrails.
---

# Project M3 Skill (`v0.6.5`)

Use this skill when interacting with a local **Project M3** (`m3`) value-investing workbench from an external agent (Claude Code, Codex, Antigravity, or standalone Alan).

## 1. Non-Negotiable Guardrails

1. **Zero-LLM Financial Arithmetic**
   - Never compute, estimate, interpolate, or "fix" financial numbers in your head or via ad-hoc scripts.
   - Always treat `m3_report`, `m3_universe_screen`, `m3_universe_peers`, and `m3_universe_similar` as the sole numerical source of truth. All figures are parsed from primary SEC EDGAR XBRL filings using `rust_decimal` (`Strict Owner Earnings = OCF - CapEx - SBC`).
2. **Human Sovereignty & Feynman Line**
   - **Never** write, overwrite, or fabricate `human_judgment.my_stance`, `weight_rationale`, or `feynman_checked_at` in `workbench/dossiers/*.json`.
   - `m3_feynman_round` returns Socratic critiques and blind spots only; it **never** auto-signs `feynman_checked_at`. Only use `m3_falsification_update` when the human principal explicitly asks to add/remove/replace objective falsification metrics.
   - **Never** overwrite `## Human Notes` sections in `workbench/memos/*.md` or `workbench/industries/*.md`.
   - **Never** output buy/sell recommendations or position-sizing directives. Present deterministic facts, red/yellow warnings, latticework analysis, and falsification questions for the human principal to decide.
3. **Protected Personal Workbench & Budget Gates**
   - Never delete `personal_workbench` (`watchlist.json`, `dossiers/`, `memos/`, `industries/`, `journal.json`, `budget.json`, `themes.json`).
   - Respect M3's daily proactive quotas and monthly USD/token ceilings on `m3_research_run`, `m3_briefing_run`, and `m3_triage_run`.
4. **Paper Research Scope**
   - M3 operates in local `PAPER RESEARCH` mode with no live brokerage execution.

---

## 2. Connecting via `m3 mcp` (40 Tools Default, `--read-only` Optional)

`m3 mcp` runs a dependency-free, line-delimited JSON-RPC 2.0 server over `stdio`.
- **Default mode**: Exposes all **40** tools (**24 read-only** + **16 controlled action, Alan, and desktop-control tools**), every one carrying `destructiveHint: false` and accurate `readOnlyHint`.
- **`--read-only` mode**: Filters `tools/list` to the **24** `readOnlyHint: true` tools and rejects write/action `tools/call` requests with JSON-RPC `-32602`.

Make sure `m3` is installed (`~/.local/bin/m3` or on `PATH`), or invoke it via `cargo run -q -p m3-cli -- mcp`. Optionally pass `--workspace <PATH>` or set `M3_REPO_ROOT=<PATH>`.

### Claude Code

```bash
claude mcp add m3 -- m3 mcp
# Or strictly read-only:
claude mcp add m3-readonly -- m3 mcp --read-only
```

Or in project `.mcp.json`:

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

### Codex CLI (`~/.codex/config.toml`)

```toml
[mcp_servers.m3]
command = "m3"
args = ["mcp"]
```

### Antigravity / Standard JSON MCP Config

```json
{
  "mcpServers": {
    "m3": {
      "command": "m3",
      "args": ["mcp", "--workspace", "/path/to/your/m3-workbench"]
    }
  }
}
```

---

## 3. Complete MCP Tool Catalog (`40` Tools)

### Read-Only Tools (`24`, `readOnlyHint: true`)

| Tool | Purpose | Key Arguments |
| :--- | :--- | :--- |
| `m3_status` | Circle watchlist summary, Strike Zone statuses, benchmark price, sync freshness, paper journal cash | — |
| `m3_doctor` | Binary, config, `alan_connection` health, stale Deep Research runs, workspace, DuckDB, storage, host tools | — |
| `m3_report` | Deterministic Charlie Calculator `MungerReport` (ROIC, Strict OE, reverse valuation, Strike Price, warnings) | `ticker` (required) |
| `m3_dossier` | Load company interrogation dossier (AI draft + `human_judgment`) | `ticker` (required) |
| `m3_universe_status` | L0 DuckDB/Parquet lakehouse company, fact, and price pool counts | — |
| `m3_universe_screen` | Filter & rank L0 US equities by deterministic quality metrics | `min_roic`, `max_sbc_ocf`, `max_share_change_3y`, `max_net_debt_to_oe`, `min_years`, `exclude_financials`, `positive_oe_only`, `price_pool_only`, `strike_only`, `query`, `sort`, `sort_asc`, `limit` |
| `m3_universe_peers` | Compare a ticker against SIC peers with percentile ranks | `ticker` (required), `limit` |
| `m3_universe_similar` | Hybrid financial hard-filter + 256d cosine/BM25 semantic search over L1 `doc_chunks` | `query` (required), `as_ticker`, `min_roic`, `max_sbc_ocf`, `exclude_financials`, `positive_oe_only`, `price_pool_only`, `limit` |
| `m3_inbox_list` | List L1 event-driven research signals with SEC filing evidence | `status`, `kind`, `limit` |
| `m3_industry_list` | List Industry Research Studio profiles and Feynman checklist progress | — |
| `m3_industry_show` | Load full industry profile JSON and Markdown primer | `slug` (required) |
| `m3_budget_status` | Inspect L2 Alan Deep Research monthly/per-run budget limits and spend | — |
| `m3_themes_list` | Read saved commercial archetype theme clusters (`workbench/themes.json`) | — |
| `m3_storage_status` | Scan disk usage across all 5 storage buckets (`personal_workbench` protected) | — |
| `m3_briefing_latest` | Fetch latest persisted calm Circle briefing (`daily` \| `weekly`) | `period` |
| `m3_briefing_list` | List persisted calm Circle briefings newest-first | `period`, `limit` |
| `m3_triage_list` | List L1.5 Circle Filing Watchdog triage verdicts and daily quota status | `ticker`, `verdict`, `limit` |
| `m3_alan_status` | Read Alan's active connection profile summary (non-secret `masked_hint`, provider, model) | — |
| `m3_alan_session_show` | Load persisted bounded Alan session for an `AlanScope` | `scope` (required) |
| `m3_journal_list` | List paper decision journal entries, cash balance, and open paper positions | — |
| `m3_memo_read` | Read `workbench/memos/<TICKER>.md` and `<TICKER>-feynman.md` | `ticker` (required) |
| `m3_events_list` | List ingested L1 SEC filing / news events from DuckDB | `ticker`, `event_type`, `unprocessed_only`, `limit` |
| `m3_research_status` | List L2 Deep Research audit runs & budget (reclaims `>= 15m` stale running rows if RW DuckDB is free) | `ticker`, `limit` |
| `m3_app_state` | Snapshot live state of the running `M3.app` desktop window via `control.sock` | `timeout_secs` |

### Controlled Action, Alan & Desktop Control Tools (`16`, `readOnlyHint: false`, `destructiveHint: false`)

| Tool | Purpose | Key Arguments |
| :--- | :--- | :--- |
| `m3_alan_test` | Probe an Alan LLM connection profile over HTTP and report status & latency | `profile_id` |
| `m3_alan_ask` | Run a scoped Alan turn (`company:TICKER`, `industry:SLUG`, `briefing:latest`, `portfolio`), persist session, return prose, Turing Tape, and `InlineEvidenceCard`s | `scope` (req), `question` (req), `lang`, `timeout_secs` |
| `m3_alan_session_clear` | Clear persisted Alan session file for a scope without touching dossiers or `human_judgment` | `scope` (req) |
| `m3_watchlist_add` | Add or update a Circle company (`core` \| `in` \| `watch` \| `out` \| `too_hard`, circle, reason) | `ticker` (req), `bucket`, `name`, `circle`, `reason` |
| `m3_sync` | Sync SEC EDGAR XBRL financials and Yahoo daily prices for watchlist or one ticker | `ticker`, `prices_only` |
| `m3_inbox_triage` | Update L1 signal status (`unread` \| `researching` \| `watch` \| `too_hard` \| `archived`) and optionally promote to `WATCH` | `signal_id` (req), `status` (req), `promote_watch` |
| `m3_journal_record` | Append a pre-committed paper decision (`wait` \| `out` \| `too_hard` \| `paper_buy` \| `paper_sell`) | `ticker` (req), `action` (req), `note` (req), `shares`, `falsification` |
| `m3_falsification_update` | Add, remove, or replace `human_judgment.falsification_metrics` (`add` \| `remove` \| `replace`) | `ticker` (req), `action` (req), `metric`, `index`, `metrics` |
| `m3_research_run` | Run budget-gated L2 4-lens Deep Research and write `workbench/memos/<TICKER>.md` (preserving `## Human Notes`) | `ticker` (req), `focus`, `trigger` |
| `m3_briefing_run` | Generate and persist a `daily` or `weekly` Circle briefing (`no_llm: true` for 0 tokens) | `period`, `no_llm` |
| `m3_triage_run` | Run L1.5 Circle Filing Watchdog triage (daily-quota gated; supports `dry_run`) | `limit`, `dry_run` |
| `m3_feynman_round` | Run one Socratic Feynman Defense round (`1..=3`) for `company:TICKER` or `industry:SLUG` (never auto-signs) | `scope` (req), `round` (req), `answer` (req), `lang`, `draft_metrics` |
| `m3_app_navigate` | Navigate `M3.app` to a tab (`inbox`\|`universe`\|`industry`\|`baskets`\|`journal`\|`settings`), company (`""` closes), or industry | `tab`, `ticker`, `industry`, `timeout_secs` |
| `m3_app_alan` | Configure unified Alan Canvas (`closed`\|`split`\|`immersive`, `interrogate`\|`feynman`, `scope`, `pin`, `anchor_label`, `prompt`, `submit`) | `mode`, `tab`, `scope`, `pin`, `anchor_label`, `prompt`, `submit`, `timeout_secs` |
| `m3_app_action` | Trigger UI action (`toggle_sidebar`, `open_palette`, `close_palette`, `close_company`, `sync`, `refresh_briefing`, `reset_alan`, `deep_research`) | `action` (req), `ticker`, `timeout_secs` |
| `m3_app_screenshot` | Capture a PNG screenshot of the running `M3.app` window to `output_path` | `output_path` (req), `timeout_secs` |

---

## 4. `AlanScope` Interrogation & `/mnt/m3` Virtual Filesystem

When running scoped interrogation via `m3_alan_ask` (or `m3 alan ask`), always specify one of the four canonical `AlanScope` strings:
- `company:<TICKER>` (e.g. `company:COST`) — grounds Alan in that company's `MungerReport`, Dossier, Living Memo, and recent events.
- `industry:<SLUG>` (e.g. `industry:retail`) — grounds Alan in `IndustryProfile`, SIC peer matrix, and Industry Primer.
- `briefing:latest` (or `briefing:<ID>`) — grounds Alan in the latest `CircleBriefing` and recent signals.
- `portfolio` — grounds Alan in watchlist buckets, Decision Journal ledger, and Circle exposure.

Inside every scoped turn, Alan has access to `/mnt/m3` via its internal strictly read-only virtual filesystem tools (`m3_fs_list` and `m3_fs_read`, capped at **64 KiB** per read under `M3ToolPolicyHook`) covering `/mnt/m3/watchlist.json`, `/mnt/m3/journal.json`, `/mnt/m3/computed/<TICKER>.json`, `/mnt/m3/dossiers/<TICKER>.json`, `/mnt/m3/industries/*`, `/mnt/m3/memos/*`, `/mnt/m3/briefings/latest.json`, and `/mnt/m3/events/recent.json`.

Include `[report:TICKER]`, `[xbrl:TICKER]`, or `[dossier:TICKER]` citations in answers when grounded evidence cards are desired: `resolve_inline_evidence_cards` attaches deterministic `InlineEvidenceCard`s using live `watchlist.json` buckets.

---

## 5. Live Desktop Copilot Workflow (`m3_app_*` / `m3 app`)

When the user has `M3.app` open (`m3 app ping` succeeds over `control.sock`), you can act as a calm copilot directly alongside their visual workspace:

1. **Inspect Context First**: Call `m3_app_state` (or `m3 app state`) to see which tab, company, industry, and Alan canvas state (`Closed | Split | Immersive`, `alan_scope`, `alan_context_pinned`) the user is currently looking at.
2. **Focus the Canvas**: Call `m3_app_navigate` to open the relevant tab or ticker, then `m3_app_alan` (e.g. `mode: "split"`, `scope: "company:COST"`, `pin: true`) to bring up the unified Alan Canvas without cluttering overview tabs.
3. **Visual Verification**: Call `m3_app_screenshot` (or `m3 app screenshot /tmp/m3-window.png`) when you need to verify rendered charts, evidence cards, or layout state visually.

---

## 6. CLI Equivalents (`m3 --json ...`)

All MCP capabilities are also exposed directly by the `m3` CLI (`--compact` / `--json` when piped):

```bash
m3 --json doctor
m3 --json alan status
m3 --json alan test
m3 --json alan ask --scope company:COST "Verify Costco moat & bucket [report:COST]" --lang en
m3 --json alan session show --scope company:COST
m3 --json app state
m3 --json app navigate --tab universe --ticker COST
m3 --json app alan --mode split --scope company:COST --pin true
m3 --json app screenshot /tmp/m3-window.png
```

---

## 7. Recommended Munger Research Flow

1. **Health & Environment**: Check `m3_doctor` (confirming `alan_connection` and zero `stale_research_runs`), `m3_status`, and `m3_universe_status`.
2. **Deterministic Screening**: Narrow candidates with `m3_universe_screen`, compare SIC peers with `m3_universe_peers`, and search filing analogs with `m3_universe_similar`.
3. **Primary Facts & Context**: Call `m3_report` for exact `rust_decimal` metrics and automatic red/yellow warnings, `m3_dossier`, `m3_memo_read`, `m3_industry_show`, and `m3_briefing_latest` / `m3_inbox_list`.
4. **Scoped Alan Interrogation**: Use `m3_alan_ask` (with `AlanScope`) to stress-test 3-sentence business clarity, latticework moats, and 5-year pre-mortems grounded in `/mnt/m3` artifacts and `InlineEvidenceCard`s.
5. **Feynman Defense & Human Sign-Off**: Coach the human owner through 3 rounds with `m3_feynman_round`, optionally record explicit falsification criteria with `m3_falsification_update` at their direction, and leave `my_stance`, `feynman_checked_at`, and `m3_journal_record` decisions strictly under human sovereignty.
