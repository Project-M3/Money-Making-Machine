//! Persisted release channel selection (`~/Library/Application Support/M3/updater/channel.json`)
//! and resolution of the effective channel.

use super::{current_binary_channel, current_binary_version};
use crate::{app_config, store};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

const CHANNEL_FILE_NAME: &str = "channel.json";
const LAST_CHECK_FILE_NAME: &str = "last-check.json";
const MAX_CHANNEL_BYTES: u64 = 4096;

/// Supported release distribution channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    /// Tagged semantic releases (`vX.Y.Z`).
    Stable,
    /// Rolling pre-release builds from `main` (`beta`).
    Beta,
}

impl ReleaseChannel {
    /// Canonical lowercase channel identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }

    /// Short Chinese display label for UI badges.
    pub const fn label_cn(self) -> &'static str {
        match self {
            Self::Stable => "稳定版 (stable)",
            Self::Beta => "测试版 (beta)",
        }
    }

    /// Parse a strict lowercase channel identifier (`"stable"` or `"beta"`).
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "stable" => Ok(Self::Stable),
            "beta" => Ok(Self::Beta),
            other => bail!("unknown release channel `{other}` (expected `stable` or `beta`)"),
        }
    }
}

impl fmt::Display for ReleaseChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Source of the active effective release channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSource {
    /// Explicitly saved in `updater/channel.json`.
    Configured,
    /// Inherited from the compiled channel of the running binary.
    BinaryDefault,
}

impl ChannelSource {
    /// Lowercase machine-readable identifier for JSON reports and display.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::BinaryDefault => "binary_default",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedChannel {
    channel: String,
}

/// Structured status output emitted for `m3 update channel`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelStatusReport {
    pub channel: String,
    pub source: String,
    pub binary_channel: String,
    pub binary_version: String,
    pub switch_pending: bool,
}

/// Resolve `<updater_dir>/channel.json` under explicit `paths`.
pub fn configured_channel_path_in(paths: &app_config::M3Paths) -> PathBuf {
    paths.updater_dir().join(CHANNEL_FILE_NAME)
}

/// Resolve `.../M3/updater/channel.json`.
pub fn configured_channel_path() -> Result<PathBuf> {
    Ok(app_config::M3Paths::resolve()?
        .updater_dir()
        .join(CHANNEL_FILE_NAME))
}

/// Read the persisted release channel selection under explicit `paths`.
pub fn read_configured_channel_in(paths: &app_config::M3Paths) -> Option<ReleaseChannel> {
    let path = configured_channel_path_in(paths);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_CHANNEL_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let parsed = serde_json::from_slice::<PersistedChannel>(&bytes).ok()?;
    ReleaseChannel::parse(&parsed.channel).ok()
}

/// Read the persisted release channel selection, ignoring missing or invalid files cleanly.
pub fn read_configured_channel() -> Option<ReleaseChannel> {
    let paths = app_config::M3Paths::resolve().ok()?;
    read_configured_channel_in(&paths)
}

/// Atomically persist the selected release channel under explicit `paths` and invalidate any
/// cached `updater/last-check.json` passive update notice.
pub fn write_configured_channel_in(
    paths: &app_config::M3Paths,
    channel: ReleaseChannel,
) -> Result<()> {
    let channel_path = configured_channel_path_in(paths);
    let parent = channel_path
        .parent()
        .context("configured channel path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create updater config directory `{}`", parent.display()))?;
    if let Ok(meta) = std::fs::symlink_metadata(&channel_path) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "refusing to overwrite non-regular channel file `{}`",
            channel_path.display()
        );
    }

    store::write_json_atomic(
        &channel_path,
        &PersistedChannel {
            channel: channel.as_str().to_string(),
        },
    )
    .with_context(|| format!("persist release channel `{}`", channel_path.display()))?;

    let cache_path = parent.join(LAST_CHECK_FILE_NAME);
    if let Ok(meta) = std::fs::symlink_metadata(&cache_path)
        && meta.is_file()
        && !meta.file_type().is_symlink()
    {
        let _ = std::fs::remove_file(&cache_path);
    }

    Ok(())
}

/// Atomically persist the selected release channel to `updater/channel.json` and invalidate any
/// cached `updater/last-check.json` passive update notice.
pub fn write_configured_channel(channel: ReleaseChannel) -> Result<()> {
    let paths = app_config::M3Paths::resolve()?;
    write_configured_channel_in(&paths, channel)
}

/// Return the effective channel under explicit `paths`.
pub fn effective_channel_in(paths: &app_config::M3Paths) -> (ReleaseChannel, ChannelSource) {
    match read_configured_channel_in(paths) {
        Some(ch) => (ch, ChannelSource::Configured),
        None => (current_binary_channel(), ChannelSource::BinaryDefault),
    }
}

/// Return the effective channel (`configured` if set, otherwise running binary channel).
pub fn effective_channel() -> (ReleaseChannel, ChannelSource) {
    match read_configured_channel() {
        Some(ch) => (ch, ChannelSource::Configured),
        None => (current_binary_channel(), ChannelSource::BinaryDefault),
    }
}

/// Build the structured status report under explicit `paths`.
pub fn channel_status_report_in(paths: &app_config::M3Paths) -> ChannelStatusReport {
    let binary_channel = current_binary_channel();
    let binary_version = current_binary_version().to_string();
    let (channel, source) = effective_channel_in(paths);
    ChannelStatusReport {
        channel: channel.as_str().to_string(),
        source: source.as_str().to_string(),
        binary_channel: binary_channel.as_str().to_string(),
        binary_version,
        switch_pending: channel != binary_channel,
    }
}

/// Build the structured status report for `m3 update channel`.
pub fn channel_status_report() -> ChannelStatusReport {
    let binary_channel = current_binary_channel();
    let binary_version = current_binary_version().to_string();
    let (channel, source) = effective_channel();
    ChannelStatusReport {
        channel: channel.as_str().to_string(),
        source: source.as_str().to_string(),
        binary_channel: binary_channel.as_str().to_string(),
        binary_version,
        switch_pending: channel != binary_channel,
    }
}
