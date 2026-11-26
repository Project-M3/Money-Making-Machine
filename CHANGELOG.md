# Changelog

All notable changes to **Project M3 (Money · Making · Machine)** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] - 2026-11-26

### Added
- **L1.5 Circle Filing Watchdog** (`crates/m3-core/src/proactive.rs`, DuckDB `signal_triage` table): monitors Circle `WATCH + IN` companies only (leaving `OUT`, `TOO HARD`, and 6,000+ Discovery signals at 0 LLM tokens), classifying unread events into `Noise | Material | FalsificationAlert` with 1–3 verbatim primary-source quotes, falling back cleanly to `Dossier.premortem` + deterministic red flags when `falsification_metrics` is empty, folding `Noise` non-destructively in the stream, and surfacing coral `FalsificationAlert` cards with a one-click manual 4-lens Deep Research trigger.
- **Calm Daily/Weekly Briefings** (`circle_briefings` DuckDB table): deterministic 3-part briefing skeleton (Strike Radar within `<= 15%` of Strike, Filing & Red-Flag Delta, and Watchdog Hits) with an optional 1-call Alan executive summary skipped at 0 tokens during quiet periods (`无球可挥 · 耐心等待`).
- **`⌘1` Dual-View Inbox & Cross-Surface Interfaces**: upgraded `⌘1 Briefing Inbox` with a segmented `简报 | 事件流` toggle while keeping sidebar navigation strictly `⌘1–⌘5`; added `m3 briefing`, `m3 triage`, and `m3 falsification` CLI subcommands plus 3 new read-only `m3 mcp` tools (`m3_briefing_latest`, `m3_briefing_list`, `m3_triage_list`, bringing MCP total to 17).
- **NSH Native Background Keep-Alive & Menu Bar Tray**: enabled `AppOptions::keep_alive_on_close: true`, automatic macOS Dock hide/restore (`NSApplicationActivationPolicy::Accessory ↔ Regular`) across `⌘W` / reopen, and a monochrome template `PIXEL_M3` Menu Bar tray icon that lights up status dots only on active Circle alerts or Strike Zone hits.
- **Interactive 3-Round Socratic Feynman Coach & `WATCH → IN` Soft Gate** (`crates/m3-core/src/feynman.rs`): added structured 3-round defense coaching in Company Desk (`2 Deep Research & Alan`) and Industry Studio (`⌘3`) compatible with macOS native Dictation, explicit human sign-off (`my_stance`, `falsification_metrics`, `feynman_checked_at`, archived Markdown defense history), in-app `falsification_metrics` CRUD editor with one-click Alan suggestion adoption, and a non-blocking `WATCH → IN` Soft Gate across Company Desk, Circle Overview, context menu, and `⌘K` Command Palette.
- **Proactive Alan Quotas & Settings Controls** (`crates/m3-core/src/budget.rs`): added backward-compatible daily quotas (`proactive_triage_enabled = true`, `daily_triage_quota = 10`, `daily_briefing_summary_quota = 1`, `background_refresh_hours = 4`) stacked under combined L2 + L1.5 monthly USD/token ceilings and configurable in `Settings → AI`.

### Changed
- Scoped `v0.6.0` exclusively to Proactive Alan (Branches A + B + C) and moved HKEX / multi-market expansion to `v0.6.5` (`ROADMAP.md`, `CONTEXT.md`, `docs/v0.6-proactive-alan.md`).
- Restricted `smoked_glass::install()` to normal titled windows (`NSNormalWindowLevel` + `NSWindowStyleMask::Titled`) so macOS Menu Bar `NSStatusBarWindow` renders cleanly without a translucent box backdrop.

## [0.5.1] - 2026-11-26

### Added
- Non-destructive "Remove from Circle" (`remove_company_from_watchlist` / `remove_company_from_circle`) across Company Desk, context menu, Circle Overview row actions, and Universe Radar drawer (preserving human-authored dossiers, memos, and journal entries).

### Changed
- Standardized Circle tier order to `WATCH → IN → OUT → TOO HARD` across core `Bucket::display_order()`, sidebar, Circle Overview, Company Desk, menus, and website simulator.
- Hydrated primary Inbox and Circle state before frame 1 and added `startup_hydrated` empty-state guards to eliminate startup empty-screen flashes.

### Performance
- Eliminated UI-thread freezes when opening or switching Company Research Desk by moving DuckDB context queries (`research_runs`, `peers`, `search_similar`, budget) off-thread with request-sequence guards.
- Accelerated `LakeDb::search_similar` (~4x faster on local lakehouses via zero-allocation BM25 scanning, streaming JSON cosine similarity, and deferred top-K snippet extraction).

## [0.5.0] - 2026-11-26

### Added
- In-app **Alan** model connection setup & live connectivity verification in Settings across all 5 first-party providers (`OpenRouter`, `OpenAI Responses`, `ChatGPT` browser OAuth, `Google Vertex AI` ADC/Express, `Magpie`).
- Guided first-run onboarding card with curated Berkshire × Munger non-financial classics starter pack and 2-phase progressive `L0` lakehouse bootstrap (`64 KiB` byte-streaming download progress and early `lake_ready` activation before Top-400 quotes).
- Read-only stdio MCP server (`m3 mcp`, 14 deterministic tools), first-party M3 Agent Skill, and one-click MCP setup copy (`Claude Code` / `Codex` / `Cursor / JSON`) in Settings.
- Smoked-black macOS glass shell (`M3SmokedGlassView`), morphing topbar Command Box, compact topbar Message Box with determinate sync progress, centered Settings Modal (`⌘,` with `General` / `AI` / `Data` tabs), IconMCP SVG icon language, and zero-layout-shift hold-`⌘` shortcut overlays.
- Structured CLI help (`m3 --help`, `m3 help <command>`, `m3 <command> --help`) with grouped command discovery and examples.
- TTY-aware JSON formatting in `m3` (auto-pretty on interactive terminals, compact single-line JSON when piped) with `--pretty` and `--compact` overrides.
- Live `stderr` staged progress bar during interactive `m3 update`.
- `m3 storage [status|clean <category|all>]`, `m3 industry [list|show <slug>]`, and `m3 doctor` subcommands.
- Repository open-source documentation (`README.md`, `README.zh-CN.md`, `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`) and GitHub issue/PR/Dependabot templates.

### Changed
- Cross-view UI consistency polish across all 6 desktop views (`Inbox`, `Universe`, `Industry`, `Circle`, `Themes & Journal`, `Company Desk`): unified `42px` pinned top telemetry ribbons, consistent filter strips, and IconMCP vector icons replacing raw text glyphs.
- Direct public GitHub Releases updater (removing `gh` auth and REST API quota dependency) and unified **Alan** branding across app, CLI, docs, and website.

### Fixed
- Fixed Google Vertex AI connectivity probe (`v1beta1/publishers/google/models?pageSize=1` with regional host resolution and `x-goog-user-project` for ADC, and `:countTokens` for Express API keys).
- Detached `async_bridge` NSH tasks from transient view owners, bounded update-check timeouts, guarded worker panics, and resolved update-check state machine races.

## [0.4.2] - 2026-11-25

### Added
- Live staged self-update progress bars (`Resolving`, `Downloading`, `Verifying`, `Extracting`, `Installing`, `Completed`) in both the Settings panel and sidebar dock.
- macOS Storage-style workspace Storage Manager in Settings with Unix hardlink deduplication, protected personal workbench reporting, and one-click cache reclamation.
- Circular `?` concept tooltips (`concept_hint`) across the desktop app and website to replace verbose inline explanations.
- Berkshire × Munger Non-Financial Classics static dataset fixture (`MCO`, `COST`, `AAPL`, `KO`, `VRSN`, `DPZ`, `AAL`, `INTC`) for the public website simulator.

### Fixed
- Resolved a runtime hang (`"正在处理更新任务，请稍候…"`) caused by dropping `reqwest::blocking::Client` inside a Tokio `spawn_blocking` thread, and raised archive download total timeout to `600s`.
- Fixed the `⌘K` Command Palette candidate list collapsing to zero height inside the NSH modal container and improved ASCII `M3` query matching.

## [0.4.1] - 2026-11-25

### Added
- Refined `M³` ("Money · Making · Machine") brand micro-interactions and subtle easter eggs in the desktop workbench and website.
- Interactive Stable / Beta channel toggle on the website installation card.

## [0.4.0] - 2026-11-25

### Changed
- **Workspace & Desktop Rewrite**: Split deterministic engine (`crates/m3-core`), JSON CLI (`crates/m3-cli`), and macOS desktop app (`apps/m3-nsh`) built with **Longbridge NSH `0.7.0`**, retiring the previous GPUI frontend.
- Added Canvas K-line and ROIC × FCF Yield scatter plots in the native NSH workbench.
- Standardized repository journal defaults to an empty clean slate while preserving seamless `M3.app` in-app updates.

## [0.3.0] - 2026-11-24

### Added
- **L0 Full-US Lakehouse**: Bulk SEC EDGAR `submissions.zip` & `companyfacts.zip` ingestion into immutable ZSTD Parquet generations and embedded `m3.duckdb`, multi-factor screener, and SIC peer percentiles.
- **L1 Event Inbox**: Zero-LLM-token SEC filing & news watcher with local 256-dimensional cosine + BM25 hybrid lookalike search and verbatim evidence cards.
- **L2 Deep Research & Budget Guardrails**: Built-in **Alan** 4-perspective parallel interrogation behind hard monthly USD/token and weekly quota gates, plus Commercial Archetype Themes and Industry Research Studio.
- **Bilingual Zh/En UI & Self-Updater**: Real-time language switching, `stable` / `beta` release channels, and signed/SHA-256 verified macOS distribution.

## [0.2.0] - 2026-11-19

### Added
- Initial **Munger Focus Value Research & Decision Workbench** (`v1`), deterministic SEC EDGAR XBRL & Yahoo Finance Charlie Calculator, built-in **Alan** interrogator with `M3ToolPolicyHook`, and `m3` JSON CLI.

[Unreleased]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.5.1...main
[0.5.1]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Project-M3/Money-Making-Machine/releases/tag/v0.2.0
