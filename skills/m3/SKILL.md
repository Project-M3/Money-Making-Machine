---
name: m3
description: Connect to Project M3's local-first Munger value-investing workbench via the `m3 mcp` stdio server or `m3` JSON CLI. Use when screening US equities in the DuckDB/Parquet lakehouse, inspecting deterministic SEC EDGAR XBRL financial reports, reviewing L1 event inbox signals, reading company dossiers or industry primers, or checking research budgets while obeying M3's zero-LLM arithmetic and human-sovereignty rules.
---

# Project M3 Skill

Use this skill when interacting with a local **Project M3** (`m3`) value-investing workbench from an external agent (Claude Code, Codex, Antigravity, or standalone Alan).

## 1. Non-Negotiable Guardrails

1. **Zero-LLM Financial Arithmetic**
   - Never compute, estimate, interpolate, or "fix" financial numbers in your head or via ad-hoc scripts.
   - Always treat `m3_report`, `m3_universe_screen`, `m3_universe_peers`, and `m3_universe_similar` as the sole numerical source of truth. All figures are parsed from primary SEC EDGAR XBRL filings using `rust_decimal` (`Strict Owner Earnings = OCF - CapEx - SBC`).
2. **Human Sovereignty & Feynman Line**
   - **Never** write, overwrite, or fabricate `human_judgment` (`my_stance`, `weight_rationale`, `falsification_metrics`, `feynman_checked_at`) in `workbench/dossiers/*.json`.
   - **Never** overwrite `## Human Notes` sections in `workbench/memos/*.md` or `workbench/industries/*.md`.
   - **Never** output buy/sell recommendations or position-sizing directives. Present deterministic facts, red/yellow warnings, latticework analysis, and falsification questions for the human principal to decide.
3. **Protected Personal Workbench**
   - Never delete `personal_workbench` (`watchlist.json`, `dossiers/`, `memos/`, `industries/`, `journal.json`, `budget.json`, `themes.json`).
4. **Paper Research Scope**
   - M3 operates in local `PAPER RESEARCH` mode with no live brokerage execution.

---

## 2. Connecting via `m3 mcp` (Preferred Read-Only Interface)

`m3 mcp` exposes 14 deterministic, read-only tools over line-delimited JSON-RPC 2.0 on `stdio`.

Make sure `m3` is installed (`~/.local/bin/m3` or on `PATH`), or invoke it via `cargo run -p m3-cli -- mcp` from the M3 repository root. Optionally pass `--workspace <PATH>` or set `M3_REPO_ROOT=<PATH>` if running from outside your workbench directory.

### Claude Code

```bash
claude mcp add m3 -- m3 mcp
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

### Codex CLI

Add to `~/.codex/config.toml`:

```toml
[mcp_servers.m3]
command = "m3"
args = ["mcp"]
```

### Antigravity / Standard JSON MCP Config

Add to your MCP server configuration (`mcp_config.json` / client settings):

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

### Standalone Alan

Configure `m3 mcp` as a `stdio` MCP server in Alan's tool/MCP configuration, or invoke `m3 <command> --json` directly as a shell tool when write/sync commands are required.

---

## 3. Available MCP Tools (All Read-Only)

| Tool | Purpose | Key Arguments |
| :--- | :--- | :--- |
| `m3_status` | Circle watchlist summary, Strike Zone statuses, benchmark price, sync freshness, paper journal cash | — |
| `m3_doctor` | Diagnose binary version, config, workspace, lakehouse counts, storage, and host tools | — |
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

---

## 4. Fallback / Write Operations via `m3` JSON CLI

`m3 mcp` is strictly read-only. When the user explicitly asks to sync SEC data, add a watchlist ticker, catch up L1 events, triage inbox signals, or run budget-gated Deep Research, use the `m3` CLI (`--compact` / `--json` when piped):

```bash
m3 doctor
m3 status
m3 sync --ticker MCO
m3 add MCO --bucket watch --circle "Credit Ratings & Analytics"
m3 universe sync --full --prices-top 400
m3 events catchup --bootstrap --max-ciks 25 --max-docs 12
m3 inbox triage <SIGNAL_ID> --status watch --promote-watch
m3 research run MCO --focus "Pricing power and private credit shift" --trigger cli
```

All CLI business commands emit `{"schema_version": 1, "ok": true, "data": ...}` on success and `{"schema_version": 1, "ok": false, "error": "..."}` with non-zero exit code on failure.

---

## 5. Recommended Munger Research Flow

1. **Check Environment & Freshness**: Call `m3_doctor` and `m3_status` (plus `m3_universe_status` for full-US lakehouse availability).
2. **Screen Deterministically First**: Narrow candidates with `m3_universe_screen` (e.g., `min_roic: 15`, `max_sbc_ocf: 25`, `exclude_financials: true`), cross-check industry peers via `m3_universe_peers`, and find structural analogs via `m3_universe_similar`.
3. **Inspect Primary Facts & Context**: Call `m3_report` for exact `rust_decimal` metrics and automatic red/yellow warnings, `m3_dossier` for prior notes, `m3_industry_show` for sector primers, and `m3_inbox_list` for filing-backed signals.
4. **Apply the Munger Interrogation**:
   - **3-Sentence Business Filter**: What it sells, why customers do not leave, and where the cash goes.
   - **Latticework Mapping**: Microeconomics/moat, ecosystem role, management incentives, and capital allocation history.
   - **5-Year Pre-Mortem**: Concrete failure paths anchored to the warnings in `m3_report`.
   - **Feynman Handoff**: Present open questions so the human principal can author and verify their own `human_judgment`.
