<p align="right">
  <strong>English</strong> | <a href="./README.zh-CN.md">简体中文</a>
</p>

# Project M3 — Money · Making · Machine

**Invest like Charlie Munger** — *Learn markets, study industries, pick compounding businesses, and evolve your value strategies into verifiable returns.*

[![CI](https://github.com/Project-M3/Money-Making-Machine/actions/workflows/ci.yml/badge.svg)](https://github.com/Project-M3/Money-Making-Machine/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/Project-M3/Money-Making-Machine?label=release)](https://github.com/Project-M3/Money-Making-Machine/releases/latest)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Platform: macOS Apple Silicon](https://img.shields.io/badge/platform-macOS%20Apple%20Silicon-black.svg)](https://github.com/Project-M3/Money-Making-Machine/releases)
[![Website](https://img.shields.io/badge/website-m3.thinksmart.systems-10b981.svg)](https://m3.thinksmart.systems)

Project M3 is a local-first desktop & CLI instrument that turns Charlie Munger's investment discipline into software: deterministic accounting over primary SEC EDGAR XBRL filings, cross-disciplinary adversarial research powered by built-in **Alan**, and an explicit path toward autonomous Agents and guarded programmatic execution.

> **Current Status (`v0.4.x`)**: Today M3 ships as a local-first research & paper-journaling workbench (`PAPER RESEARCH` — no brokerage credentials or live order execution yet). See [ROADMAP.md](./ROADMAP.md) for how `v0.5 → v1.0` adds zero-friction BYOK onboarding, `m3 mcp`, programmable value strategies, and autonomous portfolio execution behind a hardcoded Rust `RiskEngine`.

---

## Key Capabilities

- **Deterministic Charlie Calculator (`m3-core`)** — Computes multi-year ROIC, Strict Owner Earnings (`OCF - CapEx - SBC`), 3-year share count dilution/buyback rate, reverse-implied growth, Strike Price, and automated accounting red flags using fixed-point `rust_decimal` arithmetic with SHA-256 filing provenance.
- **Three-Layer Local-First Lakehouse**
  - **L0 Market Lakehouse**: Full-US SEC EDGAR XBRL (`companyfacts.zip` & `submissions.zip`) materialized into immutable ZSTD Parquet generations and embedded `m3.duckdb` for sub-second multi-factor screening and SIC peer percentile comparisons.
  - **L1 Event Inbox**: Zero-LLM-token filing watcher & 256-dimensional local semantic/BM25 indexer (`8-K`, `10-K`, `10-Q`, RSS news) surfacing *Discovery*, *Ripple*, *Strike*, and *Red Flag* cards with verbatim primary-source excerpts.
  - **L2 Alan Deep Research**: Opt-in 4-perspective parallel interrogation by built-in Alan (*Moat*, *Accounting*, *Latent Peers*, *5Y Pre-Mortem*) gated by hard monthly USD/token caps and weekly auto-research quotas. Human judgment (`human_judgment`) and `## Human Notes` are strictly protected from AI overwrite.
- **Native macOS Desktop Workbench (`apps/m3-nsh`)** — Built with Rust and Longbridge NSH `0.7.0`: bilingual English/Chinese UI, Circle-gated Strike Zone (`Core / Watch / Out / Too Hard`), Industry Research Studio, Canvas K-line & ROIC scatter charts, `⌘K` Command Palette, macOS-style Storage Manager, and SHA-256 verified atomic in-app updates.
- **Agent-Native JSON CLI (`m3`)** — Every workbench, lakehouse, storage, and updater workflow is exposed via structured schema-v1 JSON (`m3 --help`, `m3 doctor`, `m3 storage status`), auto-pretty on interactive TTYs and compact single-line JSON when piped.

---

## Quick Install (macOS Apple Silicon)

Install `M3.app` into `/Applications/M3.app` and symlink the `m3` CLI into `~/.local/bin/m3`:

```bash
# Latest stable release
curl -fsSL https://m3.thinksmart.systems/install.sh | bash

# Rolling beta channel (built on merges to main)
curl -fsSL https://m3.thinksmart.systems/install.sh | bash -s -- --channel beta
```

Prebuilt release archives and SHA-256 manifests are also available on [GitHub Releases](https://github.com/Project-M3/Money-Making-Machine/releases).

---

## CLI Quickstart

```bash
# Browse grouped commands and examples
m3 --help
m3 help universe

# Check local binary, workspace, lakehouse, and storage footprint
m3 doctor

# Initialize or refresh the L0 US market lakehouse (DuckDB + ZSTD Parquet)
m3 universe sync --full --prices-top 400
m3 universe screen --min-roic 15 --max-sbc-ocf 25 --limit 25

# Inspect L1 event signals and Circle status
m3 inbox list --status unread --limit 20
m3 add MCO --bucket core --circle "Credit Ratings & Analytics"
m3 status

# Reclaim rebuildable SEC/DuckDB cache without touching personal workbench data
m3 storage status
m3 storage clean sec_bulk_archives

# Manage release channel and check for updates
m3 update --check
```

When invoked interactively in a terminal, `m3` pretty-prints JSON automatically; when piped to `jq` or another script, it emits compact single-line JSON. Override at any time with `--pretty` or `--compact`.

---

## Workspace & Crate Architecture

| Path | Role |
| :--- | :--- |
| [`crates/m3-core`](./crates/m3-core) | Zero-UI deterministic engine: SEC EDGAR sync, `rust_decimal` Charlie Calculator, DuckDB/Parquet lakehouse (`L0`/`L1`/`L2`), Industry Studio, storage manager, bilingual i18n, self-updater |
| [`crates/m3-cli`](./crates/m3-cli) | Scriptable schema-v1 JSON CLI (`m3`) with structured help, `doctor`, `storage`, `industry`, and TTY progress bar for updates |
| [`apps/m3-nsh`](./apps/m3-nsh) | Native macOS desktop application powered by Longbridge NSH `0.7.0` and bundled with `m3` |
| [`website`](./website) | Public Solid + Vite landing page (build-time prerendered, en/zh) & interactive workbench showcase (`m3.thinksmart.systems`) |
| [`docs`](./docs) | Architecture specs, Munger human-agent workflow, and CLI/file contract documentation |
| [`scripts`](./scripts) | macOS bundle assembly, local installer (`install.sh`), release tooling, and manifest generator |

Data ownership is cleanly split on disk:
- `workbench/` holds human & curated workspace artifacts (`watchlist.json`, `dossiers/`, `industries/`, `memos/`, `themes.json`, `budget.json`, `journal.json`).
- `data/munger/` holds rebuildable primary caches and `m3.duckdb` / Parquet lakehouse generations.
- `~/Library/Application Support/M3/` stores persistent host configuration (`config.json`), updater receipts (`updater/`), and the default blank workbench when running outside a source checkout.

---

## Development

### Prerequisites

- macOS on Apple Silicon (`aarch64-apple-darwin`)
- Rust `1.98.1` with `rustfmt` and `clippy`
- Node.js + `pnpm` (only needed for `website/`)

### Common Commands

```bash
# Format check and strict linting (zero warnings + zero production .unwrap()/.expect())
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings

# Run full workspace test suite
cargo test --workspace --locked -j 4

# Build local macOS bundle into apps/m3-nsh/artifacts/M3.app
./apps/m3-nsh/build.sh

# Build static website
pnpm --dir website install && pnpm --dir website build
```

See [CONTRIBUTING.md](./CONTRIBUTING.md) for full CI parity checks and contribution guidelines.

---

## Roadmap (`v0.4.x → v1.0`)

| Milestone | Scope | Status |
| :--- | :--- | :--- |
| `v0.4.x` | Deterministic Charlie Calculator, 6,000+ US DuckDB/Parquet lakehouse, L1 Inbox, Industry Studio, NSH 0.7 macOS app, `m3` CLI | **Shipped** |
| `v0.5` | In-app BYOK LLM config, guided onboarding + starter pack, progressive L0 bootstrap, `m3 mcp` server & Agent Skills | **Next** |
| `v0.6` | Proactive event-driven research & falsification Agent, Feynman coaching, HK equity expansion | **Planned** |
| `v0.7` | Programmable value strategy engine, point-in-time historical backtester, built-in `Paper Broker` | **Planned** |
| `v0.8 – v1.0` | Hardcoded Rust `RiskEngine`, **Longbridge** + **Alpaca** adapters, autonomous portfolio sleeves under owner mandate | **Planned** |

Read the full milestone architecture and non-negotiable safety invariants in [ROADMAP.md](./ROADMAP.md).

---

## Documentation

- [Product & Architecture Roadmap (`v0.5 → v1.0`)](./ROADMAP.md)
- [Desktop App & Bundle Guide](./apps/m3-nsh/README.md)
- [Agent & CLI Contract](./docs/agent-interface.md)
- [System Architecture](./docs/architecture.md)
- [V2 Lakehouse & Event Architecture (Delivered Baseline)](./docs/v2-architecture.md)
- [Munger Human-Agent Workflow](./docs/munger-workflow.md)
- [Changelog](./CHANGELOG.md) · [Contributing](./CONTRIBUTING.md) · [Security Policy](./SECURITY.md)

---

## Disclaimer

Today (`v0.4.x`), Project M3 operates strictly as a local research, accounting verification, and paper-journaling instrument (`PAPER RESEARCH`) with **no brokerage integration, no trading credentials, and no order execution**. Future programmatic execution planned for `v0.8+` is strictly opt-in behind deterministic Rust risk guardrails and owner-authored mandates. Nothing produced by Project M3 constitutes investment advice.

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).
