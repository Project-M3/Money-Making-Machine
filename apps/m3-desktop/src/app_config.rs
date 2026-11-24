//! Persistent macOS Application Support configuration for installed `M3.app` / `m3` CLI.

use crate::i18n::UiLang;
use crate::model::{DecisionJournal, WatchlistFile};
use crate::store;
use anyhow::{Context, Result, bail, ensure};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.json";
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Persistent desktop/CLI host configuration stored at
/// `~/Library/Application Support/M3/config.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<UiLang>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            workspace_root: None,
            language: None,
        }
    }
}

/// Resolve M3's Application Support directory (`M3_APP_SUPPORT_DIR` or
/// `~/Library/Application Support/M3`).
pub fn app_support_dir() -> Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("M3_APP_SUPPORT_DIR") {
        let trimmed = override_dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    let home = std::env::var("HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .context("cannot resolve HOME directory for M3 application support state")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("M3"))
}

/// Resolve the updater state directory under Application Support (`.../M3/updater`).
pub fn updater_dir() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("updater"))
}

/// Resolve the default standalone empty workspace (`.../M3/workspace`).
pub fn default_workspace_dir() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("workspace"))
}

/// Reject symlinked workspace directories or files and initialize an empty workspace if missing.
pub fn ensure_initialized_workspace(root: &Path) -> Result<PathBuf> {
    ensure!(
        !root.as_os_str().is_empty(),
        "workspace root path cannot be empty"
    );
    match std::fs::symlink_metadata(root) {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "refusing non-directory or symlinked workspace root `{}`",
            root.display()
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err)
                .with_context(|| format!("inspect workspace root `{}`", root.display()));
        }
    }

    let workbench_dir = root.join("workbench");
    match std::fs::symlink_metadata(&workbench_dir) {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "refusing non-directory or symlinked workbench directory `{}`",
            workbench_dir.display()
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err)
                .with_context(|| format!("inspect workbench dir `{}`", workbench_dir.display()));
        }
    }

    std::fs::create_dir_all(workbench_dir.join("dossiers")).with_context(|| {
        format!(
            "create workspace dossiers directory `{}`",
            workbench_dir.join("dossiers").display()
        )
    })?;

    let watchlist_path = workbench_dir.join("watchlist.json");
    match std::fs::symlink_metadata(&watchlist_path) {
        Ok(meta) => ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "refusing symlinked or non-regular watchlist `{}`",
            watchlist_path.display()
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let empty_wl = WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".to_owned(),
                default_target_return_pct: Decimal::from(10),
                companies: Vec::new(),
            };
            store::write_json_atomic(&watchlist_path, &empty_wl)?;
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("inspect watchlist `{}`", watchlist_path.display()));
        }
    }

    let journal_path = workbench_dir.join("journal.json");
    match std::fs::symlink_metadata(&journal_path) {
        Ok(meta) => ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "refusing symlinked or non-regular journal `{}`",
            journal_path.display()
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let empty_journal = DecisionJournal {
                schema_version: 1,
                account_note: "PAPER RESEARCH FIXTURE — NO LIVE ACCOUNT / NO BROKERAGE CONNECTION"
                    .to_owned(),
                currency: "USD".to_owned(),
                starting_cash: Decimal::from(100_000),
                cash_balance: Decimal::from(100_000),
                benchmark_symbol: "^GSPC".to_owned(),
                benchmark_start_price: None,
                positions: Vec::new(),
                entries: Vec::new(),
            };
            store::write_json_atomic(&journal_path, &empty_journal)?;
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("inspect journal `{}`", journal_path.display()));
        }
    }

    Ok(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
}

/// Resolve `~/Library/Application Support/M3/config.json`.
pub fn config_path() -> Result<PathBuf> {
    Ok(app_support_dir()?.join(CONFIG_FILE_NAME))
}

/// Read `config.json`, refusing symlinks or oversized non-regular files.
pub fn read_app_config() -> Result<Option<AppConfig>> {
    let path = config_path()?;
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("inspect config file `{}`", path.display()));
        }
    };
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "refusing to read non-regular or symlinked config `{}`",
        path.display()
    );
    ensure!(
        meta.len() <= MAX_CONFIG_BYTES,
        "config file `{}` exceeds {MAX_CONFIG_BYTES} byte limit",
        path.display()
    );
    let bytes =
        std::fs::read(&path).with_context(|| format!("read config file `{}`", path.display()))?;
    let parsed = serde_json::from_slice::<AppConfig>(&bytes)
        .with_context(|| format!("parse config file `{}`", path.display()))?;
    Ok(Some(parsed))
}

/// Validate and persist `workspace_root` to `config.json`.
pub fn write_workspace_root(path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty(),
        "workspace root path cannot be empty"
    );
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve current working directory for workspace path")?
            .join(path)
    };
    ensure!(
        candidate.join("workbench/watchlist.json").exists(),
        "工作区 `{}` 缺少 workbench/watchlist.json",
        candidate.display()
    );
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("canonicalize workspace root `{}`", candidate.display()))?;

    let cfg_path = config_path()?;
    if let Ok(meta) = std::fs::symlink_metadata(&cfg_path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        bail!(
            "refusing to overwrite non-regular config `{}`",
            cfg_path.display()
        );
    }

    let mut cfg = read_app_config().ok().flatten().unwrap_or_default();
    cfg.schema_version = 1;
    cfg.workspace_root = Some(canonical.display().to_string());
    store::write_json_atomic(&cfg_path, &cfg)?;
    Ok(canonical)
}

/// Return the persisted workspace root if configured and valid (`workbench/watchlist.json` exists).
pub fn read_workspace_root() -> Option<PathBuf> {
    let cfg = read_app_config().ok().flatten()?;
    let raw = cfg.workspace_root?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if !path.join("workbench/watchlist.json").exists() {
        return None;
    }
    path.canonicalize().ok().or(Some(path))
}

/// Return raw `workspace_root` string from `config.json` if configured and non-empty.
pub fn read_configured_workspace_root() -> Option<String> {
    let cfg = read_app_config().ok().flatten()?;
    cfg.workspace_root
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Clear explicit `workspace_root` in `config.json` while preserving other host settings.
pub fn clear_workspace_root() -> Result<()> {
    let cfg_path = config_path()?;
    if let Ok(meta) = std::fs::symlink_metadata(&cfg_path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        bail!(
            "refusing to overwrite non-regular config `{}`",
            cfg_path.display()
        );
    }
    let mut cfg = read_app_config().ok().flatten().unwrap_or_default();
    cfg.schema_version = 1;
    cfg.workspace_root = None;
    store::write_json_atomic(&cfg_path, &cfg)
}

/// Read persisted UI language preference, falling back to OS/env detection.
pub fn read_ui_language() -> UiLang {
    read_app_config()
        .ok()
        .flatten()
        .and_then(|c| c.language)
        .unwrap_or_else(UiLang::detect_system_lang)
}

/// Persist UI language preference to `config.json` without clobbering `workspace_root`.
pub fn write_ui_language(lang: UiLang) -> Result<()> {
    let cfg_path = config_path()?;
    if let Ok(meta) = std::fs::symlink_metadata(&cfg_path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        bail!(
            "refusing to overwrite non-regular config `{}`",
            cfg_path.display()
        );
    }
    let mut cfg = read_app_config().ok().flatten().unwrap_or_default();
    cfg.schema_version = 1;
    cfg.language = Some(lang);
    store::write_json_atomic(&cfg_path, &cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bucket, CompanyEntry, DataSource, ValuationAssumptions};

    #[test]
    fn app_config_language_and_workspace_roundtrip_without_clobbering() {
        let dir = std::env::temp_dir().join(format!(
            "m3-app-cfg-test-{}",
            crate::store::atomic_temp_path(Path::new("cfg"))
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("x")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let ws = ensure_initialized_workspace(&dir.join("ws")).unwrap();
        let cfg_file = dir.join("config.json");
        let cfg = AppConfig {
            schema_version: 1,
            workspace_root: Some(ws.display().to_string()),
            language: Some(UiLang::En),
        };
        store::write_json_atomic(&cfg_file, &cfg).unwrap();
        let loaded: AppConfig = store::read_json(&cfg_file).unwrap();
        assert_eq!(loaded.language, Some(UiLang::En));
        assert_eq!(
            loaded.workspace_root.as_deref(),
            cfg.workspace_root.as_deref()
        );

        // Legacy config JSON without `language` field still deserializes cleanly.
        let legacy: AppConfig =
            serde_json::from_str(r#"{"schema_version":1,"workspace_root":"/tmp/m3"}"#).unwrap();
        assert_eq!(legacy.language, None);
        assert_eq!(legacy.workspace_root.as_deref(), Some("/tmp/m3"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_initialized_workspace_creates_empty_files_and_preserves_existing() {
        let dir = std::env::temp_dir().join(format!(
            "m3-empty-ws-test-{}",
            crate::store::atomic_temp_path(Path::new("ws"))
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("x")
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let init_path = ensure_initialized_workspace(&dir).unwrap();
        assert!(init_path.join("workbench/dossiers").is_dir());

        let paths = crate::store::WorkspacePaths::from_root(init_path.clone());
        let wl = crate::store::load_watchlist(&paths).unwrap();
        assert!(wl.companies.is_empty());
        assert_eq!(wl.benchmark_symbol, "^GSPC");

        let journal = crate::store::load_journal(&paths).unwrap();
        assert!(journal.entries.is_empty());

        let reports = crate::store::load_or_compute_reports(&paths).unwrap();
        assert!(reports.is_empty());

        // Populate watchlist with one user entry, then call ensure_initialized_workspace again.
        let mut wl_mod = wl;
        wl_mod.companies.push(CompanyEntry {
            ticker: "COST".to_owned(),
            name: "Costco".to_owned(),
            bucket: Bucket::Watch,
            source: DataSource::SecEdgar,
            cik: Some("0000909832".to_owned()),
            currency: "USD".to_owned(),
            circle: "会员制零售".to_owned(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(18),
                multiple_high: Decimal::from(22),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        });
        crate::store::save_watchlist(&paths, &wl_mod).unwrap();

        ensure_initialized_workspace(&dir).unwrap();
        let wl_after = crate::store::load_watchlist(&paths).unwrap();
        assert_eq!(wl_after.companies.len(), 1);
        assert_eq!(wl_after.companies[0].ticker, "COST");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
