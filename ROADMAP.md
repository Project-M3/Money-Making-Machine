<p align="right">
  <a href="./README.md">English README</a> | <a href="./README.zh-CN.md">简体中文 README</a>
</p>

# Project M3 Roadmap (`v0.5 → v1.0`)

> **Vision / 核心愿景**  
> **Invest like Charlie Munger** — Learn markets, study industries, pick compounding businesses, and evolve your value-investing strategies into verifiable returns through autonomous Agents and guarded programmatic execution.  
> **像查理·芒格一样投资** —— 学习市场、研究行业、挑选投资标的，并通过 Agent 与受控的程序化交易，逐步把你的价值投资策略转化为真正的收益。

---

## Where We Are Today (`v0.2.0 – v0.6.0`, Shipped)

Project M3 has completed its local-first research, zero-friction onboarding, responsive desktop foundation, and proactive Munger analyst loop across `v0.2`–`v0.6.0`:

- **Deterministic Financial Kernel (`m3-core`)**: Fixed-point `rust_decimal` Charlie Calculator (ROIC, Strict Owner Earnings `OCF - CapEx - SBC`, 3Y share dilution, reverse-implied growth, Strike Price, accounting red flags) with SEC EDGAR XBRL SHA-256 provenance.
- **L0 Full-US Lakehouse & Progressive Bootstrap**: Bulk `companyfacts.zip` & `submissions.zip` ingestion into immutable ZSTD Parquet generations and embedded `m3.duckdb` (`6,000+` issuers), sub-second screening, SIC peer percentiles, and non-blocking 2-phase progressive L0 onboarding.
- **L1 Event Inbox & Latent Search**: Zero-LLM-token filing watcher (`8-K`, `10-K`, `10-Q`, RSS news) and local ~4x faster 256d cosine + BM25 hybrid retrieval (`search_similar`) with verbatim primary-source evidence cards.
- **L2 Alan Deep Research, 5-Provider Setup & Industry Studio**: In-app **Alan** setup across OpenRouter, OpenAI Responses, ChatGPT OAuth, Google Vertex AI & Magpie, four-perspective adversarial interrogation (*Moat*, *Accounting*, *Latent Peers*, *5Y Pre-Mortem*) behind hard monthly USD/token and weekly quotas, preserving `human_judgment` and `## Human Notes`.
- **Proactive Alan (`v0.6.0`)**: L1.5 Circle Filing Watchdog scoped strictly to `WATCH` + `IN` (`Noise | Material | FalsificationAlert`), calm 3-part daily/weekly Circle Briefings with 0-token quiet periods + NSH `0.7.0` Menu Bar Tray keep-alive, and 3-round Socratic Feynman Coach with `WATCH → IN` Soft Gate.
- **Native macOS Desktop (`apps/m3-nsh`), Agent CLI (`m3`) & `m3 mcp`**: Smoked-black glass Longbridge NSH `0.7.0` bilingual Zh/En workbench with zero-blank-screen startup hydration, off-thread Research Desk loading, canonical `WATCH → IN → OUT → TOO HARD` Circle order, non-destructive watchlist removal, Canvas K-line & scatter charts, `⌘K` Command Palette, Settings Modal (`⌘,`), macOS-style Storage Manager, SHA-256 atomic updater, structured `m3` CLI, and read-only `m3 mcp` server + Agent Skill.

> **Honest `0.x` Boundary**: As of `v0.6.0`, M3 is strictly a **research & paper-journaling instrument** (`PAPER RESEARCH`). It does not yet connect to brokerage accounts or execute live trades. Milestones `v0.6.5 → v1.0` close that loop deliberately and safely.

---

## Milestone Overview (`v0.4 → v1.0`)

| Version | Milestone | Focus | Status |
| :--- | :--- | :--- | :--- |
| **`v0.4.x`** | **Munger Research Workbench** | Full-US Lakehouse (`L0`/`L1`/`L2`), Industry Studio, NSH 0.7 macOS UI, Storage Manager, `m3` CLI | **Shipped** |
| **`v0.5`** | **Zero-Friction Onboarding & Alan Setup** | In-app 5-provider Alan setup, guided starter workspace, progressive L0 bootstrap, `m3 mcp` server & Agent Skills, smoked-glass UI | **Shipped** |
| **`v0.5.1`** | **Startup & Circle Polish** | Zero-blank-screen startup (`e23259f`), off-thread Research Desk & 4x faster `search_similar`, non-destructive Circle removal, `WATCH → IN → OUT → TOO HARD` | **Shipped** |
| **`v0.6.0`** | **Proactive Alan** | Branch A (L1.5 Circle Filing Watchdog), Branch B (Calm Briefings + NSH `0.7.0` Tray Keep-Alive), Branch C (3-Round Socratic Feynman Coach & `WATCH → IN` Soft Gate) | **Shipped** |
| **`v0.6.5`** | **Unified Alan** | App-Shell Dual Canvas (`Closed \| Split \| Immersive`), unified `AlanScope` + `📌` context pin lock, inline deterministic M3 evidence cards & single-line collapsible Turing Tape, retirement of Company Desk Tab 2 & Industry bottom chat, expanded `/mnt/m3` mount tree, 40-tool `m3 mcp` server & desktop `control.sock` IPC | **Shipped** |
| **`v0.7`** | **Programmable Strategy Engine & Paper Broker** | Declarative/scriptable value strategies, point-in-time historical backtesting on DuckDB/Parquet, built-in Paper Broker | **Planned** |
| **`v0.7.5`** | **Multi-Market & HKEX Expansion** | `HKEX` filing ingestion alongside US SEC EDGAR, dual-market data pipelines, and multi-currency accounting (deferred post-`v0.7` alongside brokerage work) | **Planned** |
| **`v0.8`** | **Deterministic RiskEngine & Brokerage Adapters** | Hardcoded Rust `RiskEngine` guardrails + unified `BrokerAdapter` (**Longbridge OpenAPI** US/HK + **Alpaca** US) | **Planned** |
| **`v0.9 – v1.0`** | **Autonomous Alan Portfolio Sleeves** | Owner-defined capital sleeves & investment mandates driven by built-in Alan (`Research Only` / `Approval` / `Autonomous`) | **Planned** |

---

## Detailed Milestones

### `v0.5` — Zero-Friction Onboarding & Agent Foundation (`Shipped`)

Make M3 immediately useful within two minutes of installation—both for first-time desktop users (without external CLI prerequisites) and for external AI agents:

- **In-App Alan Connection Setup**: Configure Alan's model providers, API keys, and endpoints directly in Settings (stored securely in macOS Host Store / Keychain) so users no longer need to run external `alan` CLI setup commands first.
- **Guided First-Run Experience**:
  - Optional curated starter pack (e.g., classic Berkshire/Munger case studies) alongside a clean empty slate.
  - Non-blocking background progressive `L0` lakehouse bootstrap so new users can explore the UI right away instead of waiting on a multi-GB cold download.
- **`m3 mcp` Server & Agent Skill**: Expose M3's deterministic financial reports, `L0` screener, `L1` event inbox, dossiers, and industry studies cleanly to external agents (`Claude Code`, `Codex`, external `Alan` sessions, etc.) via Model Context Protocol (`m3 mcp`) and a first-party Skill.

---

### `v0.5.1` — Startup Hydration, Research Desk Performance & Canonical Circle Order (`Shipped` · `e23259f`)

Eliminate startup flashes and UI-thread stalls while aligning Circle ordering and non-destructive editing:

- **Zero-Blank-Screen Startup**: Hydrate primary Inbox and Circle state before frame 1 with `startup_hydrated` empty-state guards.
- **Off-Thread Research Desk & ~4x Faster `search_similar`**: Move DuckDB context queries (`research_runs`, `peers`, `search_similar`, budget) off the UI thread with request-sequence guards, and speed up `search_similar` via zero-allocation BM25 scanning and streaming cosine similarity.
- **Non-Destructive Circle Removal & Canonical Order**: Add non-destructive "Remove from Circle" (preserving dossiers, memos, and journal entries) and standardize bucket order to **`WATCH → IN → OUT → TOO HARD`** across core, desktop, and website.

---

### `v0.6.0` — Proactive Alan (`Shipped`)

Turn **Alan** inside M3 from an on-demand responder into a calm, bounded, human-sovereign Munger analyst across three focused branches (see [docs/v0.6-proactive-alan.md](./docs/v0.6-proactive-alan.md)):

- **Branch A — L1.5 Circle Filing Watchdog**:
  - Watches **`WATCH` (`待审讯`) + `IN` (`核心击球`)** Circle companies only (`OUT`, `TOO HARD`, and non-Circle Discovery signals remain at 0 LLM tokens).
  - Classifies newly ingested filings into `Noise | Material | FalsificationAlert` with verbatim primary-source quotes (`TriageQuote`).
  - **`Noise`** is collapsed in the UI and **never auto-deleted or archived** (`SignalStatus` untouched); **`FalsificationAlert`** is surfaced in coral with a one-click 4-lens Deep Research trigger (**never auto-triggered**).
  - Includes an in-app `falsification_metrics` CRUD editor in Company Desk and deterministic `Dossier.premortem` fallback (`fallback_no_metrics_warning`) when explicit falsification metrics are not yet registered.
- **Branch B — Calm Daily/Weekly Briefings + NSH `0.7.0` Native Background Keep-Alive**:
  - Single-process background lifecycle via NSH `0.7.0` `AppOptions::keep_alive_on_close` + macOS Menu Bar `SetTray`, avoiding DuckDB (`m3.duckdb`) cross-process file lock conflicts without separate `launchd` daemons.
  - Builds a deterministic 3-part briefing skeleton (*Strike Radar* for `WATCH + IN` in Strike Zone or `<=15%` above Strike, *Filing & Red-Flag Delta*, *Alan L1.5 Watchdog Hits*) plus at most 1 lightweight Alan Munger-style executive summary call—skipped at **0 LLM tokens** during quiet periods (`"无球可挥 · 耐心等待"` / `"No Fat Pitches · Stay Patient"`).
  - Persisted locally in DuckDB (`circle_briefings`) and surfaced via the `⌘1` segmented `简报 | 事件流` Briefing view, `m3 briefing` CLI, and read-only `m3 mcp` tools.
- **Branch C — Interactive 3-Round Socratic Feynman Coach & `WATCH → IN` Soft Gate**:
  - Text-first 3-round Socratic defense (*Moat & Unit Economics* → *5-Year Failure Modes & Testable Falsification Metrics* → *Valuation Margin of Safety & Kill Criteria*) inside Company Desk and Industry Studio, compatible with macOS native Dictation.
  - Explicit human sign-off atomically writes `my_stance`, `falsification_metrics`, and `feynman_checked_at`, clears `pending_human_review`, and archives the defense summary to Markdown (`workbench/memos/<TICKER>-feynman.md` or `workbench/industries/<slug>/feynman-defense.md`).
  - Moving a company from `WATCH` (`待审讯`) to `IN` (`核心击球`) invokes a calm **`WATCH → IN` Soft Gate** dialog when Feynman verification is missing or pending review.

---

### `v0.6.5` — Unified Alan (`Shipped`)

Consolidate all interactive **Alan** research, interrogation, and Feynman coaching into one calm App-Shell Dual Canvas (see [docs/v0.6.5-unified-alan.md](./docs/v0.6.5-unified-alan.md)):

- **App-Shell Dual Canvas (`Closed | Split | Immersive`)**:
  - Default `Closed` globally and on overview tabs (`Inbox`, `Universe`, `Baskets`, `Journal`), with remembered `Split` preference in deep-research views (`Company Desk` and `Industry Studio`) and auto-collapsed left sidebar in `Split` mode.
  - Global keyboard rhythm (`⌘L` for `Closed ↔ Split`, `⇧⌘L` for `Split ↔ Immersive`, `Esc` stepping `Immersive → Split` first) plus a single unified topbar Alan toggle next to Message Box.
- **Unified `AlanScope` + `📌` Context Pin Lock**:
  - One context-aware Alan Canvas serving `Company(ticker)`, `Industry(slug)`, `Briefing(...)`, and `Portfolio` scopes, auto-following the active view unless frozen via the `📌` pin lock.
  - Unifies `审讯对话 | 费曼答辩` under one roof, retiring Company Desk Tab 2 (`2 深度研究与 Alan`—folding 4-lens Deep Research output into Living Memo with an async trigger) and removing Industry Studio's separate bottom chat box and inline Feynman block.
- **Inline Deterministic M3 Evidence Cards & Collapsible Turing Tape**:
  - Resolves `[report:TICKER]`, `[xbrl:TICKER]`, and `[dossier:TICKER]` tokens into native deterministic M3 summary cards directly under Alan replies, while collapsing Turing Tape rows into a single-line expandable status capsule by default.
- **Expanded Read-Only `/mnt/m3` & Bounded Session Persistence**:
  - Extends `/mnt/m3` (`Access::ReadOnly`) with `industries/index.json`, `industries/<slug>.json`, `memos/<TICKER>.md`, `memos/<TICKER>-feynman.md`, `briefings/latest.json`, and `events/recent.json`, alongside atomic bounded session files (`workbench/alan_sessions/<scope_key>.json`).
  - Stays pinned to `alan` `v0.1.4` in `crates/m3-core/Cargo.toml` while keeping `/mnt/m3` FileServer and `M3ToolPolicyHook` boundaries forward-compatible with newer `alan9`.

---

### `v0.7` — Programmable Value Strategy Engine & Paper Broker (`Planned`)

Bridge qualitative value research and systematic capital allocation—before risking a dollar of live capital:

- **Declarative / Scriptable Strategy Engine**: Express Munger/Buffett-style value rules programmatically (e.g., 5Y ROIC floor, strict Owner Earnings yield, net buyback threshold, Circle-graduation gate, phased scaling into Strike Zones, and falsification exits).
- **Point-in-Time Historical Backtester**: Replay strategies over years of local SEC EDGAR filings and daily price history in DuckDB/Parquet with strict filing-date anti-lookahead guarantees.
- **Unified `BrokerAdapter` & Built-In Paper Broker**: Introduce `BrokerAdapter` (`crates/m3-core`) backed first by a zero-credential local `Paper Broker` tracking live paper fills, cash drag, dividends, benchmarks (`^GSPC`), and verified strategy track records.

---

### `v0.7.5` — Multi-Market & HKEX Expansion (`Planned`, Post-`v0.7`)

Deferred past `v0.7` to align multi-market financial pipelines with brokerage execution support:

- **Hong Kong Equities (`HKEX`) Filings**: Ingest `HKEX` corporate filings alongside US SEC EDGAR into the shared `m3-core` and DuckDB/Parquet lakehouse.
- **Dual-Market Data & Multi-Currency Accounting**: Support USD/HKD reporting, FX-aware `rust_decimal` valuation metrics, and unified cross-market Circle screening without compromising single-market provenance.

---

### `v0.8` — Deterministic Rust `RiskEngine` & Live Brokerage Adapters

Connect real markets behind non-bypassable deterministic code guardrails:

- **Deterministic Rust `RiskEngine`**: Every order intent—whether emitted by a strategy rule or by Alan—must pass a deterministic Rust validation gate that **cannot be overridden by prompts or LLMs**:
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

### `v0.9 – v1.0` — Autonomous Alan Portfolio Sleeves & Closed-Loop Machine

Graduate M3 to an autonomous, owner-governed **Money · Making · Machine**:

- **Owner Investment Mandate & Capital Sleeves**: Partition capital into independent sleeves with explicit autonomy levels for Alan:
  - `Research Only` — insights and dossier updates only
  - `Approval Required` — Alan prepares sized limit-order drafts & rationale for one-click owner sign-off
  - `Autonomous within Mandate` — Alan continuously screens, conducts deep research, rebalances, and executes strategy rules inside the owner's hard `RiskEngine` boundaries
- **Closed-Loop Attribution**: Track real post-execution cash flows, dividend reinvestment, tax-lot holding periods, opportunity cost vs. cash & index benchmarks, and strategy drift inside one calm obsidian ledger.

---

## Non-Negotiable Architecture Invariants (`0.x → 1.0`)

1. **Zero-LLM Financial Arithmetic**: All accounting metrics, valuations, backtest returns, position sizing math, and risk checks run deterministically in Rust with `rust_decimal` and SHA-256 provenance.
2. **Built-In Agent = Alan**: M3 does not invent a separate proprietary agent runtime—all built-in agent capabilities (deep research, Feynman coaching, proactive monitoring, and sleeve execution) run on **Alan**, embedded in-process and strictly scoped by M3 tools and policies.
3. **Deterministic Execution Gate**: In `v0.8+`, `RiskEngine` sits in deterministic Rust between any Alan/strategy intent and `BrokerAdapter`. No LLM can alter risk limits, increase leverage, or disable the Kill Switch.
4. **Owner Sovereignty**: The human owner writes the Investment Mandate, risk ceilings, Circle definitions, and `human_judgment`. Autonomy is always opt-in, scoped, and reversible.
5. **Local-First Ownership**: Filings, DuckDB/Parquet lakehouse, research memos, strategy logs, and keys live on the user's machine—with zero mandatory cloud backend.
