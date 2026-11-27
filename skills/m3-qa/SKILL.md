---
name: m3-qa
description: Step-by-step live end-to-end QA playbook for Project M3 v0.6.5 (CLI, Alan connection, scoped `/mnt/m3` tools, `InlineEvidenceCard` bucket precedence, 40-tool & `--read-only` MCP server, and `M3.app` `control.sock` IPC + screenshot verification). Always runs against an isolated `/tmp` workspace without ever mutating real `workbench/` or `data/` files.
---

# Project M3 Live QA Playbook (`v0.6.5`)

Use this playbook to verify a real build of Project M3 end-to-end across `m3` CLI, live Alan LLM calls, `/mnt/m3` virtual filesystem tools, `m3 mcp` (full + `--read-only`), and `M3.app` desktop control IPC.

## Golden Rule: Zero Contamination of Real Workspace

- **NEVER** run mutating QA commands against the repository's real `workbench/` or `data/` directories.
- **ALWAYS** seed a fresh isolated workspace under `/tmp/m3-qa-<tag>` and pass `M3_REPO_ROOT=/tmp/m3-qa-<tag>` (plus `--workspace /tmp/m3-qa-<tag>` for `m3 mcp`).
- Capture `git status --short` before starting and again after cleanup to prove zero repo contamination.

---

## Step 1: Build Binary & Seed Isolated `/tmp` Workspace

Build the CLI binary and inspect pre-QA git state:

```bash
REPO="/Users/morris/Developer/github.com/Project-M3/Money-Making-Machine"
QA_WS="/tmp/m3-qa-$(date +%s)"
git -C "$REPO" status --short
cargo build --manifest-path "$REPO/Cargo.toml" -p m3-cli
M3_BIN="$REPO/target/debug/m3"
```

Seed `$QA_WS` by copying read-only baseline fixtures (`COST.json` computed report, `COST.json` dossier, `journal.json`) and writing canonical `watchlist.json`, `memos/COST.md`, and `industries/retail.json`.

> **Critical Canonical Schema Detail**:
> - In `workbench/watchlist.json`, the canonical JSON bucket value is **`"core"`** (not `"in"` — `"in"` is only a CLI/MCP input alias that maps to `Bucket::Core` = `"core"`).
> - Seeding `"bucket": "core"` in `watchlist.json` while copying a cached `data/munger/computed/COST.json` that still says `"bucket": "watch"` specifically tests that `resolve_inline_evidence_cards` overrides stale computed buckets with live `watchlist.json` state!
> - In `workbench/industries/retail.json`, use canonical `"status": "studying"` (`studying | inside_circle | too_hard`).

```bash
mkdir -p "$QA_WS/workbench/dossiers" "$QA_WS/workbench/memos" "$QA_WS/workbench/industries" "$QA_WS/data/munger/computed"

cp "$REPO/data/munger/computed/COST.json" "$QA_WS/data/munger/computed/COST.json"
cp "$REPO/workbench/dossiers/COST.json" "$QA_WS/workbench/dossiers/COST.json"
cp "$REPO/workbench/journal.json" "$QA_WS/workbench/journal.json"

cat > "$QA_WS/workbench/watchlist.json" <<'EOF'
{
  "schema_version": 1,
  "benchmark_symbol": "^GSPC",
  "default_target_return_pct": "10",
  "companies": [
    {
      "ticker": "COST",
      "name": "Costco Wholesale Corporation",
      "bucket": "core",
      "source": "sec_edgar",
      "cik": "0000909832",
      "currency": "USD",
      "circle": "Membership Warehouse Club",
      "too_hard_reason": null,
      "assumptions": {
        "multiple_low": "24",
        "multiple_high": "30",
        "target_return_pct": "10",
        "expected_growth_pct": "8"
      }
    }
  ]
}
EOF

cat > "$QA_WS/workbench/memos/COST.md" <<'EOF'
# COST Living Research Memo

- Scale-shared purchasing power passed back to members via capped markups (~14% max).
- Membership fee renewal (>90% US/Canada) underwrites operating margin durability.

## Human Notes
QA check: keep this human section intact.
EOF

cat > "$QA_WS/workbench/industries/retail.json" <<'EOF'
{
  "slug": "retail",
  "title": "Membership & Scale Retail",
  "status": "studying",
  "sic_codes": [5331],
  "keywords": ["warehouse club", "membership fee"],
  "bound_tickers": ["COST"],
  "multiple_low": "20",
  "multiple_high": "28",
  "key_kpis": ["Renewal Rate", "Sales per Sq Ft"],
  "feynman_questions": ["Why doesn't Costco raise gross margin above ~11%?"],
  "feynman_checked": [false],
  "human_notes": ""
}
EOF
```

---

## Step 2: Verify `m3 doctor` & Stale Research Reclamation

```bash
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json doctor
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json research status
```

Pass criteria:
- `data.workspace.root` points to `$QA_WS` (never the real repo `workbench/`).
- `checks` includes `alan_connection` (`ok: true`, non-secret `masked_hint` in `data.alan_connection`).
- `checks` includes `stale_research_runs` (`ok: true`, `0` stale runs).
- `research status` returns `ok: true` and `data.reclaimed_stale_runs`.

---

## Step 3: Verify `m3 alan test`, `/mnt/m3` `m3_fs_read`, `InlineEvidenceCard`, & Session Persistence

1. Probe live Alan connectivity (verifies HTTP 200 OK and zero Tokio nested-runtime panic):

```bash
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json alan status
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json alan test
```

2. Run a live scoped interrogation forcing `/mnt/m3` reads and an inline report citation:

```bash
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json alan ask \
  --scope company:COST \
  --timeout 90 \
  "请调用 m3_fs_read 读取 /mnt/m3/memos/COST.md 和 /mnt/m3/watchlist.json，核实 COST 的 bucket 并附上 [report:COST]"
```

Pass criteria:
- `ok: true` with non-empty `data.prose`.
- `data.turing_tape` contains `m3_fs_read` entries with `"status": "completed"` (proving Alan actually called `/mnt/m3` read tools).
- `data.evidence_cards` contains a `COST` card whose `"bucket"` is **`"core"`** (not stale `"watch"` from `computed/COST.json`).
- `$QA_WS/workbench/alan_sessions/company-COST.json` exists on disk, and `M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json alan session show --scope company:COST` returns the persisted turn.

---

## Step 4: Verify Full-Capability `m3 mcp` vs `--read-only`

Probe both server modes over stdio JSON-RPC 2.0:

```bash
# 1. Default full-capability mode (expect 40 tools: 24 readOnlyHint=true, 16 readOnlyHint=false, all destructiveHint=false)
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"m3_alan_status","arguments":{}}}' \
  '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"m3_alan_session_show","arguments":{"scope":"company:COST"}}}' \
  '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"m3_falsification_update","arguments":{"ticker":"COST","action":"add","metric":"US/Canada renewal rate < 88%"}}}' \
| M3_REPO_ROOT="$QA_WS" "$M3_BIN" mcp --workspace "$QA_WS"

# 2. --read-only mode (expect 24 tools in tools/list, and -32602 rejection on action tools)
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"m3_watchlist_add","arguments":{"ticker":"MCO","bucket":"watch"}}}' \
| M3_REPO_ROOT="$QA_WS" "$M3_BIN" mcp --workspace "$QA_WS" --read-only
```

Pass criteria:
- Default mode: `id: 2` lists **40** tools; `id: 3`, `4`, `5` return `"isError": false`; `COST.json` in `$QA_WS` preserves `feynman_checked_at: null` and `my_stance`.
- `--read-only` mode: `id: 2` lists **24** tools (every tool has `annotations.readOnlyHint == true`); `id: 3` returns JSON-RPC `"error": {"code": -32602, ...}`.
- `stdout` contains pure JSON-RPC lines only (zero diagnostic pollution).

---

## Step 5: Verify Live `M3.app` `control.sock` IPC & Screenshot Inspection

When testing the desktop app (using an isolated `M3_CONTROL_SOCKET="$QA_WS/control.sock"` or the running `M3.app` instance):

1. Check connectivity and initial state:

```bash
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json app ping
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json app state
```

2. Drive navigation and unified Alan Canvas, then verify state and capture a screenshot:

```bash
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json app navigate --tab universe --ticker COST
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json app alan --mode split --tab interrogate --scope company:COST --pin true
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json app state
M3_REPO_ROOT="$QA_WS" "$M3_BIN" --json app screenshot "$QA_WS/m3-split-cost.png"
```

Pass criteria:
- `app state` confirms `company_open: true`, `selected_ticker: "COST"`, `alan_stage_mode: "split"`, `alan_scope: "company:COST"`, `alan_context_pinned: true`.
- `$QA_WS/m3-split-cost.png` exists and is a non-empty PNG; inspect the image directly to confirm Dual Canvas `Split` rendering, collapsed left sidebar, and clean typography.

---

## Step 6: Cleanup & Contamination Check

```bash
rm -rf "$QA_WS"
git -C "$REPO" status --short
```

Confirm no QA artifacts remain in the repository working tree.
