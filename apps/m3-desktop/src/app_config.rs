//! Persistent macOS Application Support configuration for installed `M3.app` / `m3` CLI.

use crate::store;
use anyhow::{Context, Result, bail, ensure};
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            workspace_root: None,
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
