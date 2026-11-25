# Contributing to Project M3

Thank you for your interest in contributing to **Project M3 (Money · Making · Machine)**.

## Project Layout

- `crates/m3-core`: Zero-UI deterministic financial engine, SEC/Yahoo sync, DuckDB/Parquet lakehouse, Industry Studio, storage manager, i18n, and self-updater.
- `crates/m3-cli`: Structured schema-v1 JSON CLI (`m3`).
- `apps/m3-nsh`: Native macOS desktop workbench built with Longbridge NSH `0.7.0`.
- `website`: Public Vite + TypeScript landing page and interactive workbench showcase (`m3.thinksmart.systems`).
- `scripts`: macOS packaging, install (`scripts/install.sh`), release, and manifest tools.
- `docs`: Architecture specs, Munger workflow, and Agent/CLI contracts.

## Prerequisites

- **macOS** on Apple Silicon (`aarch64-apple-darwin`) for desktop builds and release checks
- **Rust `1.98.1`** with `rustfmt` and `clippy` components
- **Node.js + `pnpm`** if modifying `website/`

## Architectural & Code Standards

1. **Deterministic Arithmetic Only**: Never use floating-point numbers (`f32`/`f64`) or LLM estimates for financial figures (ROIC, Owner Earnings, Strike Price, dilution, margins). Use `rust_decimal::Decimal` with explicit provenance in `crates/m3-core`.
2. **Zero Production `.unwrap()` / `.expect()`**: CI enforces that **no production file** under `crates/*/src` or `apps/*/src` contains `.unwrap(` or `.expect(` outside `#[cfg(test)]` modules or `tests.rs`. Always propagate structured `anyhow::Result` or handle `Option` cleanly.
3. **Human Sovereignty**: `human_judgment` fields in dossiers and `## Human Notes` sections in living memos/industry primers must never be overwritten programmatically by automated tools.
4. **Preserve CLI Schema Compatibility**: Existing `m3` JSON output contracts (`{"schema_version": 1, "ok": true, "data": ...}`) must remain backward-compatible for agent and script callers.
5. **Installer Parity**: If you edit `scripts/install.sh`, keep `website/public/install.sh` byte-for-byte identical.

## Local Verification (Run Before Opening a PR)

Match the CI workflow (`.github/workflows/ci.yml`):

```bash
# 1. Formatting and strict Clippy
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings

# 2. Full Rust test suite
cargo test --workspace --locked -j 4

# 3. Release script syntax & plist validation
bash -n scripts/assemble-macos-release.sh scripts/install.sh scripts/release.sh apps/m3-nsh/build.sh apps/m3-nsh/m3
/usr/bin/plutil -lint apps/m3-nsh/Info.plist
python3 scripts/generate-release-manifest.py --help >/dev/null

# 4. If you touched website/
pnpm --dir website build
```

If you build or test local Alan changes via `[patch."https://github.com/realmorrisliu/alan.git"]`, keep the patch strictly local (for example in an untracked `.cargo/config.toml`) and never commit host-specific absolute paths.

## Commit & Pull Request Conventions

- Use Conventional Commits: `feat(...)`, `fix(...)`, `docs(...)`, `chore(...)`, `refactor(...)`, `ci(...)`.
- Keep PRs focused on a single concern, include a clear explanation of why the change is needed, and confirm all CI checks above pass locally.
- Please read and follow our [Code of Conduct](./CODE_OF_CONDUCT.md).
