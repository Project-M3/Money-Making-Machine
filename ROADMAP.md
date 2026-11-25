<p align="right">
  <a href="./README.md">English README</a> | <a href="./README.zh-CN.md">简体中文 README</a>
</p>

# Project M3 Roadmap (`v0.5 → v1.0`)

> **Vision / 核心愿景**  
> **Invest like Charlie Munger** — Learn markets, study industries, pick compounding businesses, and evolve your value-investing strategies into verifiable returns through autonomous Agents and guarded programmatic execution.  
> **像查理·芒格一样投资** —— 学习市场、研究行业、挑选投资标的，并通过 Agent 与受控的程序化交易，逐步把你的价值投资策略转化为真正的收益。

---

## Where We Are Today (`v0.2.0 – v0.4.2`, Shipped)

Project M3 has completed its local-first research foundation across `v0.2`–`v0.4.2`:

- **Deterministic Financial Kernel (`m3-core`)**: Fixed-point `rust_decimal` Charlie Calculator (ROIC, Strict Owner Earnings `OCF - CapEx - SBC`, 3Y share dilution, reverse-implied growth, Strike Price, accounting red flags) with SEC EDGAR XBRL SHA-256 provenance.
- **L0 Full-US Lakehouse**: Bulk `companyfacts.zip` & `submissions.zip` ingestion into immutable ZSTD Parquet generations and embedded `m3.duckdb` (`6,000+` issuers), sub-second screening, and SIC peer percentiles.
- **L1 Event Inbox & Latent Search**: Zero-LLM-token filing watcher (`8-K`, `10-K`, `10-Q`, RSS news) and local 256d cosine + BM25 hybrid retrieval with verbatim primary-source evidence cards.
- **L2 `EmbeddedAlan` & Industry Studio**: Four-perspective adversarial interrogation (*Moat*, *Accounting*, *Latent Peers*, *5Y Pre-Mortem*) behind hard monthly USD/token and weekly quotas, preserving `human_judgment` and `## Human Notes`.
- **Native macOS Desktop (`apps/m3-nsh`) & Agent CLI (`m3`)**: Longbridge NSH `0.7.0` bilingual Zh/En workbench, Canvas K-line & scatter charts, `⌘K` Command Palette, macOS-style Storage Manager, SHA-256 atomic updater, and structured `m3` CLI (`doctor`, `storage`, `industry`, `--help`, TTY formatting).

> **Honest `0.x` Boundary**: As of `v0.4.2`, M3 is strictly a **research & paper-journaling instrument** (`PAPER RESEARCH`). It does not yet connect to brokerage accounts or execute live trades. Milestones `v0.5 → v1.0` close that loop deliberately and safely.

---

## Milestone Overview (`v0.4 → v1.0`)

| Version | Milestone | Focus | Status |
| :--- | :--- | :--- | :--- |
| **`v0.4.x`** | **Munger Research Workbench** | Full-US Lakehouse (`L0`/`L1`/`L2`), Industry Studio, NSH 0.7 macOS UI, Storage Manager, `m3` CLI | **Shipped** |
| **`v0.5`** | **Zero-Friction Onboarding & Agent Foundation** | In-app BYOK LLM setup, guided starter workspace, progressive L0 bootstrap, `m3 mcp` server & Agent Skills | **Next** |
| **`v0.6`** | **Proactive Research Agent & Multi-Market** | Event-driven auto-triage, falsification watchdog, scheduled Circle briefings, interactive Feynman tutor, HK equities | **Planned** |
| **`v0.7`** | **Programmable Strategy Engine & Paper Broker** | Declarative/scriptable value strategies, point-in-time historical backtesting on DuckDB/Parquet, built-in Paper Broker | **Planned** |
| **`v0.8`** | **Deterministic RiskEngine & Brokerage Adapters** | Hardcoded Rust `RiskEngine` guardrails + unified `BrokerAdapter` (**Longbridge OpenAPI** US/HK + **Alpaca** US) | **Planned** |
| **`v0.9 – v1.0`** | **Autonomous Portfolio Agent** | Owner-defined capital sleeves & investment mandates with `Research Only` / `Approval` / `Autonomous` execution | **Planned** |

---

## Detailed Milestones

### `v0.5` — Zero-Friction Onboarding & Agent Foundation (`Next`)

Make M3 immediately useful within two minutes of installation—both for first-time desktop users and for external AI agents:

- **In-App BYOK Provider Setup**: Configure model providers, API keys, and endpoints directly in Settings (stored securely in macOS Host Store / Keychain) so users no longer need to run external `alan` CLI setup commands first.
- **Guided First-Run Experience**:
  - Optional curated starter pack (e.g., classic Berkshire/Munger case studies) alongside a clean empty slate.
  - Non-blocking background progressive `L0` lakehouse bootstrap so new users can explore the UI right away instead of waiting on a multi-GB cold download.
- **`m3 mcp` Server & Agent Skill**: Expose M3's deterministic financial reports, `L0` screener, `L1` event inbox, dossiers, and industry studies cleanly to external agents (`Claude Code`, `Codex`, `Alan`, etc.) via Model Context Protocol (`m3 mcp`) and a first-party Skill.

---

### `v0.6` — Proactive Research Agent & Multi-Market Foundation

Turn M3 from a passive workbench you query into a calm, proactive Munger-style analyst:

- **Event-Driven Research Agent**: Automatically triage newly ingested `8-K` / `10-K` / `10-Q` filings against your active Circle companies and pre-registered falsification criteria (`falsification_metrics`).
- **Scheduled Briefings & Falsification Watchdog**: Surface calm daily/weekly digests only when fundamentals move meaningfully or a valuation approaches Strike Price—never noisy intraday tick alerts.
- **Interactive Feynman Coach**: Structured voice/text Socratic defense drills inside Industry Studio and Company Desk to test understanding before graduating a company into `Core`.
- **Hong Kong Equities (`HKEX`) Expansion**: Unified multi-currency accounting & filing ingestion alongside US SEC EDGAR inside the shared `m3-core` model and DuckDB lakehouse.

---

### `v0.7` — Programmable Value Strategy Engine & Paper Broker

Bridge qualitative value research and systematic capital allocation—before risking a dollar of live capital:

- **Declarative / Scriptable Strategy Engine**: Express Munger/Buffett-style value rules programmatically (e.g., 5Y ROIC floor, strict Owner Earnings yield, net buyback threshold, Circle-graduation gate, phased scaling into Strike Zones, and falsification exits).
- **Point-in-Time Historical Backtester**: Replay strategies over years of local SEC EDGAR filings and daily price history in DuckDB/Parquet with strict filing-date anti-lookahead guarantees.
- **Unified `BrokerAdapter` & Built-In Paper Broker**: Introduce `BrokerAdapter` (`crates/m3-core`) backed first by a zero-credential local `Paper Broker` tracking live paper fills, cash drag, dividends, benchmarks (`^GSPC`), and verified strategy track records.

---

### `v0.8` — Deterministic Rust `RiskEngine` & Live Brokerage Adapters

Connect real markets behind non-bypassable deterministic code guardrails:

- **Deterministic Rust `RiskEngine`**: Every order intent—whether emitted by a strategy rule or an autonomous Agent—must pass a deterministic Rust validation gate that **cannot be overridden by prompts or LLMs**:
  - Dedicated capital sleeve ceiling (`max_sleeve_capital`)
  - Single-position weight ceiling (`max_position_pct`) & Circle membership check
  - Strict zero-margin / zero-naked-leverage enforcement
  - Limit-only execution with spread & price-collar bounds
  - Drawdown & daily-turnover circuit breakers
  - Immutable local audit ledger (`orders`, `fills`, `risk_decisions`)
  - Instant one-click / CLI **Kill Switch** (`m3 broker halt`)
- **First Live Brokerage Adapters**:
  - **Longbridge OpenAPI** (native Rust SDK supporting US + HK equities)
  - **Alpaca** (clean US paper + live REST API for global open-source contributors)

---

### `v0.9 – v1.0` — Autonomous Portfolio Agent & Closed-Loop Machine

Graduate M3 to an autonomous, owner-governed **Money · Making · Machine**:

- **Owner Investment Mandate & Capital Sleeves**: Partition capital into independent sleeves with explicit autonomy levels:
  - `Research Only` — insights and dossier updates only
  - `Approval Required` — agent prepares sized limit-order drafts & rationale for one-click owner sign-off
  - `Autonomous within Mandate` — agent continuously screens, conducts deep research, rebalances, and executes strategy rules inside the owner's hard `RiskEngine` boundaries
- **Closed-Loop Attribution**: Track real post-execution cash flows, dividend reinvestment, tax-lot holding periods, opportunity cost vs. cash & index benchmarks, and strategy drift inside one calm obsidian ledger.

---

## Non-Negotiable Architecture Invariants (`0.x → 1.0`)

1. **Zero-LLM Financial Arithmetic**: All accounting metrics, valuations, backtest returns, position sizing math, and risk checks run deterministically in Rust with `rust_decimal` and SHA-256 provenance.
2. **Deterministic Execution Gate**: In `v0.8+`, `RiskEngine` sits in deterministic Rust between any Agent/strategy intent and `BrokerAdapter`. No LLM can alter risk limits, increase leverage, or disable the Kill Switch.
3. **Owner Sovereignty**: The human owner writes the Investment Mandate, risk ceilings, Circle definitions, and `human_judgment`. Autonomy is always opt-in, scoped, and reversible.
4. **Local-First Ownership**: Filings, DuckDB/Parquet lakehouse, research memos, strategy logs, and keys live on the user's machine—with zero mandatory cloud backend.
