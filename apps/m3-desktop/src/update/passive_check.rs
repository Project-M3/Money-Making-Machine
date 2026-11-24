//! Non-blocking passive update check cached under Application Support (`updater/last-check.json`).

use super::semver::{display_tag, is_newer_version, parse_semver};
use super::{ReleaseChannel, current_binary_channel, default_channel};
use crate::app_config;
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Cache TTL for `beta` channel (1 hour).
pub const PASSIVE_CHECK_TTL_BETA_SECS: i64 = 3600;
/// Cache TTL for `stable` channel (6 hours).
pub const PASSIVE_CHECK_TTL_STABLE_SECS: i64 = 6 * 3600;
const MAX_CACHE_BYTES: u64 = 64 * 1024;

/// Persisted passive update check entry at `updater/last-check.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastCheckCache {
    pub checked_at_rfc3339: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    pub latest_version: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
}

/// Notice returned to the desktop shell or toolbar when a newer release (or pending channel
/// switch) is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassiveUpdateNotice {
    pub latest_version: String,
    pub channel: ReleaseChannel,
    pub tag: String,
    pub is_channel_switch: bool,
    pub release_url: Option<String>,
}

/// Resolve `.../M3/updater/last-check.json`.
pub fn passive_check_cache_path() -> Result<PathBuf> {
    Ok(app_config::updater_dir()?.join("last-check.json"))
}

/// Return `true` when background startup update checks should be skipped.
pub fn should_skip_passive_check() -> bool {
    let no_update_check = std::env::var("M3_NO_UPDATE_CHECK").ok();
    let ci = std::env::var("CI").ok();
    should_skip_passive_check_with(cfg!(test), no_update_check.as_deref(), ci.as_deref())
}

/// Pure helper evaluating passive check skip rules for deterministic tests.
pub fn should_skip_passive_check_with(
    in_test: bool,
    no_update_check_env: Option<&str>,
    ci_env: Option<&str>,
) -> bool {
    in_test || str_flag_is_truthy(no_update_check_env) || str_flag_is_truthy(ci_env)
}

fn str_flag_is_truthy(value: Option<&str>) -> bool {
    value.is_some_and(|raw| {
        let trimmed = raw.trim();
        !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
    })
}

/// TTL duration for the requested channel.
pub fn ttl_for_channel(channel: ReleaseChannel) -> Duration {
    match channel {
        ReleaseChannel::Beta => Duration::seconds(PASSIVE_CHECK_TTL_BETA_SECS),
        ReleaseChannel::Stable => Duration::seconds(PASSIVE_CHECK_TTL_STABLE_SECS),
    }
}

/// Evaluate cached `last-check.json` synchronously (`< 1ms`) against the running binary channel
/// and selected target channel, returning `(notice, needs_refresh)`.
pub fn evaluate_cache_for_notice_with_target(
    cache_path: &Path,
    current_version: &str,
    current_channel: ReleaseChannel,
    target_channel: ReleaseChannel,
    now: DateTime<Utc>,
) -> (Option<PassiveUpdateNotice>, bool) {
    let Ok(metadata) = std::fs::symlink_metadata(cache_path) else {
        return (None, true);
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_CACHE_BYTES
    {
        return (None, true);
    }
    let Ok(bytes) = std::fs::read(cache_path) else {
        return (None, true);
    };
    let Ok(cache) = serde_json::from_slice::<LastCheckCache>(&bytes) else {
        return (None, true);
    };
    let Ok(cache_channel) = ReleaseChannel::parse(&cache.channel) else {
        return (None, true);
    };
    if cache_channel != target_channel {
        return (None, true);
    }
    if parse_semver(&cache.latest_version).is_err() {
        return (None, true);
    }
    let Ok(checked_at) = DateTime::parse_from_rfc3339(&cache.checked_at_rfc3339) else {
        return (None, true);
    };

    let age = now.signed_duration_since(checked_at.with_timezone(&Utc));
    let ttl = ttl_for_channel(target_channel);
    let needs_refresh = age < Duration::zero() || age >= ttl;

    let is_channel_switch = target_channel != current_channel;
    let is_newer = is_newer_version(current_version, &cache.latest_version).unwrap_or(false);
    let notice = if is_channel_switch || is_newer {
        let tag = if cache.tag.trim().is_empty() {
            display_tag(&cache.latest_version)
        } else {
            cache.tag.clone()
        };
        Some(PassiveUpdateNotice {
            latest_version: cache.latest_version,
            channel: target_channel,
            tag,
            is_channel_switch,
            release_url: cache.release_url,
        })
    } else {
        None
    };

    (notice, needs_refresh)
}

/// Evaluate the configured passive cache path for the currently running binary.
pub fn evaluate_local_passive_notice() -> (Option<PassiveUpdateNotice>, bool) {
    let Ok(cache_path) = passive_check_cache_path() else {
        return (None, false);
    };
    let binary_channel = current_binary_channel();
    let (target_channel, _) = super::channel::effective_channel();
    evaluate_cache_for_notice_with_target(
        &cache_path,
        super::current_binary_version(),
        binary_channel,
        target_channel,
        Utc::now(),
    )
}

/// Atomically persist `last-check.json`, refusing non-regular files or symlinks.
pub fn write_cache_atomic(cache_path: &Path, cache: &LastCheckCache) -> Result<()> {
    let parent = cache_path
        .parent()
        .context("passive check cache path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create updater cache directory `{}`", parent.display()))?;
    if let Ok(meta) = std::fs::symlink_metadata(cache_path) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "refusing to write updater cache over non-regular file `{}`",
            cache_path.display()
        );
    }
    crate::store::write_json_atomic(cache_path, cache).with_context(|| {
        format!(
            "atomically publish updater cache `{}`",
            cache_path.display()
        )
    })
}
