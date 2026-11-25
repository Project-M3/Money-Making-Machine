# Changelog

All notable changes to **Project M3 (Money · Making · Machine)** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Structured CLI help (`m3 --help`, `m3 help <command>`, `m3 <command> --help`) with grouped command discovery and examples.
- TTY-aware JSON formatting in `m3` (auto-pretty on interactive terminals, compact single-line JSON when piped) with `--pretty` and `--compact` overrides.
- Live `stderr` staged progress bar during interactive `m3 update`.
- `m3 storage [status|clean <category|all>]`, `m3 industry [list|show <slug>]`, and `m3 doctor` subcommands.
- Repository open-source documentation (`README.md`, `README.zh-CN.md`, `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`) and GitHub issue/PR/Dependabot templates.

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
- **L2 Deep Research & Budget Guardrails**: `EmbeddedAlan` 4-perspective parallel interrogation behind hard monthly USD/token and weekly quota gates, plus Commercial Archetype Themes and Industry Research Studio.
- **Bilingual Zh/En UI & Self-Updater**: Real-time language switching, `stable` / `beta` release channels, and signed/SHA-256 verified macOS distribution.

## [0.2.0] - 2026-11-19

### Added
- Initial **Munger Focus Value Research & Decision Workbench** (`v1`), deterministic SEC EDGAR XBRL & Yahoo Finance Charlie Calculator, `EmbeddedAlan` interrogator with `M3ToolPolicyHook`, and `m3` JSON CLI.

[Unreleased]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.4.2...main
[0.4.2]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Project-M3/Money-Making-Machine/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Project-M3/Money-Making-Machine/releases/tag/v0.2.0
