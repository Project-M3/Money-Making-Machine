//! Self-update engine, release channel resolution, and private GitHub Release downloads.

pub mod channel;
#[cfg(target_os = "macos")]
pub mod install;
pub mod passive_check;
pub mod semver;
#[cfg(test)]
mod tests;

pub use channel::{ReleaseChannel, effective_channel};
pub use passive_check::PassiveUpdateNotice;

#[cfg(target_os = "macos")]
use anyhow::Context as _;
use anyhow::{Result, bail, ensure};
#[cfg(target_os = "macos")]
use install::{
    InstallLock, SameFsStagingDir, atomic_swap_app_bundle, encode_lower_hex,
    extract_and_verify_bundle, is_valid_sha256_hex, preflight_install_targets,
    resolve_cli_link_path, resolve_target_app_bundle,
};
#[cfg(target_os = "macos")]
use passive_check::{LastCheckCache, passive_check_cache_path, write_cache_atomic};
#[cfg(target_os = "macos")]
use reqwest::blocking::Client;
#[cfg(target_os = "macos")]
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::fs::File;
#[cfg(target_os = "macos")]
use std::io::{BufWriter, Read as _, Write as _};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use semver::{current_target_triple, display_tag, is_newer_version};
use semver::{normalize_version_str, parse_semver};

/// Default GitHub owner/repo hosting M3 releases (private repository supported via `gh` token).
pub const DEFAULT_GITHUB_REPO: &str = "Project-M3/Money-Making-Machine";
/// Default GitHub REST API base URL.
pub const GITHUB_API_BASE: &str = "https://api.github.com";

#[cfg(target_os = "macos")]
const MANIFEST_TIMEOUT_SECS: u64 = 15;
#[cfg(target_os = "macos")]
const ARCHIVE_CONNECT_TIMEOUT_SECS: u64 = 30;
#[cfg(target_os = "macos")]
const ARCHIVE_READ_TIMEOUT_SECS: u64 = 60;
#[cfg(target_os = "macos")]
const ARCHIVE_MAX_TOTAL_TIMEOUT_SECS: u64 = 600;
#[cfg(target_os = "macos")]
const MAX_GITHUB_JSON_BYTES: usize = 2 * 1024 * 1024;
#[cfg(target_os = "macos")]
const GH_CANDIDATES: [&str; 3] = ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"];

/// Return the semantic version compiled into this binary.
pub fn current_binary_version() -> &'static str {
    env!("M3_VERSION")
}

/// Return the long display version compiled into this binary (`0.2.0-beta... (beta, sha)`).
pub fn current_long_version() -> &'static str {
    env!("M3_LONG_VERSION")
}

/// Return the short commit SHA compiled into this binary (empty if unset).
pub fn current_build_commit() -> &'static str {
    env!("M3_BUILD_COMMIT")
}

/// Return the release channel compiled into this binary.
pub fn current_binary_channel() -> ReleaseChannel {
    match env!("M3_RELEASE_CHANNEL") {
        "beta" => ReleaseChannel::Beta,
        _ => ReleaseChannel::Stable,
    }
}

pub(crate) fn default_channel() -> String {
    "stable".to_string()
}

/// Canonical `latest.json` manifest published on each GitHub Release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema_version: u64,
    #[serde(default)]
    pub product: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    pub version: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub release_url: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    pub targets: BTreeMap<String, TargetArtifact>,
}

/// Per-target archive entry inside `latest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetArtifact {
    #[serde(default)]
    pub asset: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub notarized: bool,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubReleaseAsset>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    url: String,
    #[serde(default)]
    size: u64,
}

/// Options for checking or applying an update.
#[derive(Debug, Clone, Default)]
pub struct UpdateOptions {
    pub check_only: bool,
    pub channel: Option<ReleaseChannel>,
    pub version: Option<String>,
    pub force: bool,
    pub app_dir: Option<PathBuf>,
    pub bin_dir: Option<PathBuf>,
    pub no_cli_link: bool,
    pub manifest_url: Option<String>,
}

/// Structured update check/apply report returned by CLI and background UI tasks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateReport {
    pub status: String,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_channel: Option<String>,
    pub current_version: String,
    pub target_version: String,
    pub tag: String,
    pub target: String,
    pub update_available: bool,
    pub updated: bool,
    pub checked_only: bool,
    pub app_bundle_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_link_path: Option<String>,
    pub source_label: String,
    pub asset: String,
    pub archive_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    pub signed: bool,
    pub notarized: bool,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
enum DownloadSource {
    GitHubApiAsset { api_asset_url: String },
    DirectUrl { url: String },
}

#[cfg(target_os = "macos")]
struct ResolvedRelease {
    manifest: ReleaseManifest,
    artifact: TargetArtifact,
    source_label: String,
    archive_download: DownloadSource,
}

/// Resolve a GitHub bearer token from env vars or `gh auth token` (Finder-safe absolute paths).
#[cfg(target_os = "macos")]
pub fn resolve_github_token() -> Option<String> {
    for var in ["M3_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    for bin in GH_CANDIDATES {
        let Ok(out) = Command::new(bin)
            .args(["auth", "token"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        if out.status.success() {
            let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn github_repo() -> String {
    std::env::var("M3_GITHUB_REPO")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_REPO.to_string())
}

#[cfg(target_os = "macos")]
fn github_api_base() -> String {
    std::env::var("M3_GITHUB_API_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| GITHUB_API_BASE.to_string())
}

#[cfg(target_os = "macos")]
fn build_clients() -> Result<(Client, Client)> {
    let ua = format!("m3/{}", current_binary_version());
    let standard = Client::builder()
        .user_agent(ua.clone())
        .connect_timeout(Duration::from_secs(ARCHIVE_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(ARCHIVE_READ_TIMEOUT_SECS))
        .build()
        .context("build HTTP client for M3 updater")?;
    let no_redirect = Client::builder()
        .user_agent(ua)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(MANIFEST_TIMEOUT_SECS))
        .timeout(Duration::from_secs(MANIFEST_TIMEOUT_SECS))
        .build()
        .context("build no-redirect HTTP client for M3 updater")?;
    Ok((standard, no_redirect))
}

#[cfg(target_os = "macos")]
fn validate_http_url(label: &str, url: &str) -> Result<()> {
    ensure!(
        url.starts_with("https://") || url.starts_with("http://"),
        "{label} must use an `http://` or `https://` URL: `{url}`"
    );
    Ok(())
}

/// Validate parsed `ReleaseManifest` invariants.
pub fn validate_manifest_metadata(
    manifest: &ReleaseManifest,
    requested_version: Option<&str>,
    expected_channel: Option<ReleaseChannel>,
) -> Result<(String, ReleaseChannel)> {
    ensure!(
        manifest.schema_version == 1,
        "unsupported release manifest schema_version {}",
        manifest.schema_version
    );
    if !manifest.product.trim().is_empty() {
        ensure!(
            manifest.product == "m3",
            "unexpected manifest product `{}` (expected `m3`)",
            manifest.product
        );
    }
    let manifest_channel = ReleaseChannel::parse(&manifest.channel)?;
    if let Some(expected) = expected_channel {
        ensure!(
            manifest_channel == expected,
            "release manifest channel `{}` != expected `{}`",
            manifest_channel.as_str(),
            expected.as_str()
        );
    }
    let normalized = normalize_version_str(&manifest.version)?;
    let _ = parse_semver(&normalized)?;
    ensure!(
        !manifest.tag.trim().is_empty(),
        "release manifest `tag` cannot be empty"
    );
    if let Some(req_ver) = requested_version {
        let expected_ver = normalize_version_str(req_ver)?;
        ensure!(
            normalized == expected_ver,
            "release manifest version `{normalized}` != requested `{expected_ver}`"
        );
    }
    Ok((normalized, manifest_channel))
}

#[cfg(target_os = "macos")]
fn is_github_api_url(url: &str, api_base: &str) -> bool {
    url.starts_with(api_base) || url.starts_with(GITHUB_API_BASE)
}

#[cfg(target_os = "macos")]
fn fetch_json_bytes_with_github_redirect(
    standard: &Client,
    no_redirect: &Client,
    asset_api_url: &str,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    let mut req = no_redirect
        .get(asset_api_url)
        .header("Accept", "application/octet-stream")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }
    let resp = req
        .send()
        .with_context(|| format!("request GitHub release asset `{asset_api_url}`"))?;
    let status = resp.status();
    if status.is_redirection() {
        let loc = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .context("GitHub release asset redirect missing Location header")?
            .to_string();
        let follow = standard
            .get(&loc)
            .send()
            .context("download redirected GitHub release asset")?
            .error_for_status()
            .context("redirected GitHub release asset returned HTTP error")?;
        let bytes = follow
            .bytes()
            .context("read redirected GitHub release asset bytes")?;
        ensure!(
            bytes.len() <= MAX_GITHUB_JSON_BYTES,
            "manifest asset exceeds {MAX_GITHUB_JSON_BYTES} bytes"
        );
        return Ok(bytes.to_vec());
    }
    if !status.is_success() {
        bail!("GitHub release asset request failed with HTTP {status}");
    }
    let bytes = resp.bytes().context("read GitHub release asset bytes")?;
    ensure!(
        bytes.len() <= MAX_GITHUB_JSON_BYTES,
        "manifest asset exceeds {MAX_GITHUB_JSON_BYTES} bytes"
    );
    Ok(bytes.to_vec())
}

#[cfg(target_os = "macos")]
fn resolve_release(
    standard: &Client,
    no_redirect: &Client,
    opts: &UpdateOptions,
    target_channel: ReleaseChannel,
    target_triple: &str,
) -> Result<ResolvedRelease> {
    let env_manifest_url = std::env::var("M3_MANIFEST_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let explicit_manifest_url = opts
        .manifest_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(env_manifest_url);

    let api_base = github_api_base();
    let token = resolve_github_token();

    if let Some(manifest_url) = explicit_manifest_url {
        validate_http_url("manifest_url", &manifest_url)?;
        let bytes = if is_github_api_url(&manifest_url, &api_base) {
            fetch_json_bytes_with_github_redirect(
                standard,
                no_redirect,
                &manifest_url,
                token.as_deref(),
            )?
        } else {
            let resp = standard
                .get(&manifest_url)
                .send()
                .with_context(|| format!("fetch release manifest `{manifest_url}`"))?
                .error_for_status()
                .with_context(|| format!("manifest endpoint `{manifest_url}` returned error"))?;
            let b = resp.bytes().context("read release manifest bytes")?;
            ensure!(
                b.len() <= MAX_GITHUB_JSON_BYTES,
                "release manifest exceeds {MAX_GITHUB_JSON_BYTES} bytes"
            );
            b.to_vec()
        };
        let manifest: ReleaseManifest =
            serde_json::from_slice(&bytes).context("parse release manifest JSON")?;
        validate_manifest_metadata(
            &manifest,
            opts.version.as_deref(),
            if opts.version.is_some() {
                None
            } else {
                Some(target_channel)
            },
        )?;
        let artifact = manifest
            .targets
            .get(target_triple)
            .with_context(|| format!("manifest has no target `{target_triple}`"))?
            .clone();
        validate_http_url("target download url", &artifact.url)?;
        ensure!(
            is_valid_sha256_hex(&artifact.sha256),
            "invalid target sha256 `{}`",
            artifact.sha256
        );
        return Ok(ResolvedRelease {
            manifest,
            archive_download: if is_github_api_url(&artifact.url, &api_base) {
                DownloadSource::GitHubApiAsset {
                    api_asset_url: artifact.url.clone(),
                }
            } else {
                DownloadSource::DirectUrl {
                    url: artifact.url.clone(),
                }
            },
            artifact,
            source_label: manifest_url,
        });
    }

    let repo = github_repo();
    let release_endpoint = if let Some(ver) = opts.version.as_deref() {
        let norm = normalize_version_str(ver)?;
        let tag = display_tag(&norm);
        format!("{api_base}/repos/{repo}/releases/tags/{tag}")
    } else {
        match target_channel {
            ReleaseChannel::Stable => format!("{api_base}/repos/{repo}/releases/latest"),
            ReleaseChannel::Beta => format!("{api_base}/repos/{repo}/releases/tags/beta"),
        }
    };

    let mut req = standard
        .get(&release_endpoint)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(tok) = token.as_deref() {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req
        .send()
        .with_context(|| format!("query GitHub Release `{release_endpoint}`"))?;
    let status = resp.status();
    if matches!(status.as_u16(), 401 | 403) {
        bail!(
            "GitHub API 请求受限或鉴权失败（HTTP {status}）。如遇匿名限流，请运行 `gh auth login` 或设置环境变量 `M3_GITHUB_TOKEN`。"
        );
    }
    if status.as_u16() == 404 {
        if token.is_none() {
            bail!(
                "未找到仓库 `{repo}` 在 `{target_channel}` 通道的 Release（HTTP 404）。若使用私有镜像仓库，请先执行 `gh auth login` 或设置 `M3_GITHUB_TOKEN`。"
            );
        }
        bail!(
            "未找到仓库 `{repo}` 在 `{target_channel}` 通道的 Release（{release_endpoint} 返回 404）。"
        );
    }
    if !status.is_success() {
        bail!("GitHub Release API `{release_endpoint}` 返回异常状态 HTTP {status}");
    }

    let release: GitHubRelease = resp
        .json()
        .context("parse GitHub Release API response JSON")?;
    let manifest_asset = release
        .assets
        .iter()
        .find(|a| a.name == "latest.json")
        .with_context(|| format!("Release `{}` 缺少清单文件 `latest.json`", release.tag_name))?;

    let manifest_bytes = fetch_json_bytes_with_github_redirect(
        standard,
        no_redirect,
        &manifest_asset.url,
        token.as_deref(),
    )
    .with_context(|| format!("fetch `latest.json` from release `{}`", release.tag_name))?;
    let mut manifest: ReleaseManifest =
        serde_json::from_slice(&manifest_bytes).context("parse `latest.json` from release")?;
    if manifest.release_url.is_none() {
        manifest.release_url = release.html_url;
    }

    validate_manifest_metadata(
        &manifest,
        opts.version.as_deref(),
        if opts.version.is_some() {
            None
        } else {
            Some(target_channel)
        },
    )?;

    let artifact = manifest
        .targets
        .get(target_triple)
        .with_context(|| format!("release manifest has no target `{target_triple}`"))?
        .clone();
    ensure!(
        is_valid_sha256_hex(&artifact.sha256),
        "release manifest target `{target_triple}` has invalid sha256 `{}`",
        artifact.sha256
    );

    let archive_asset_name = if artifact.asset.trim().is_empty() {
        format!("m3-{}-{target_triple}.tar.gz", manifest.version)
    } else {
        artifact.asset.clone()
    };

    let archive_download =
        if let Some(gh_asset) = release.assets.iter().find(|a| a.name == archive_asset_name) {
            DownloadSource::GitHubApiAsset {
                api_asset_url: gh_asset.url.clone(),
            }
        } else if !artifact.url.trim().is_empty() {
            validate_http_url("target download url", &artifact.url)?;
            if is_github_api_url(&artifact.url, &api_base) {
                DownloadSource::GitHubApiAsset {
                    api_asset_url: artifact.url.clone(),
                }
            } else {
                DownloadSource::DirectUrl {
                    url: artifact.url.clone(),
                }
            }
        } else {
            bail!(
                "Release `{}` 中缺少目标资产 `{archive_asset_name}`",
                release.tag_name
            );
        };
    let mut artifact = artifact;
    if artifact.size == 0
        && let Some(gh_asset) = release.assets.iter().find(|a| a.name == archive_asset_name)
    {
        artifact.size = gh_asset.size;
    }

    Ok(ResolvedRelease {
        manifest,
        artifact,
        source_label: release_endpoint,
        archive_download,
    })
}

#[cfg(target_os = "macos")]
fn open_archive_stream(
    standard: &Client,
    no_redirect: &Client,
    source: &DownloadSource,
    token: Option<&str>,
) -> Result<reqwest::blocking::Response> {
    match source {
        DownloadSource::DirectUrl { url } => standard
            .get(url)
            .send()
            .with_context(|| format!("download release archive from `{url}`"))?
            .error_for_status()
            .with_context(|| format!("release archive download failed for `{url}`")),
        DownloadSource::GitHubApiAsset { api_asset_url } => {
            let mut req = no_redirect
                .get(api_asset_url)
                .header("Accept", "application/octet-stream")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(tok) = token {
                req = req.header("Authorization", format!("Bearer {tok}"));
            }
            let resp = req
                .send()
                .with_context(|| format!("request GitHub release archive `{api_asset_url}`"))?;
            let status = resp.status();
            if status.is_redirection() {
                let loc = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .context("GitHub release archive redirect missing Location header")?
                    .to_string();
                return standard
                    .get(&loc)
                    .send()
                    .context("download redirected GitHub release archive")?
                    .error_for_status()
                    .context("redirected GitHub release archive returned HTTP error");
            }
            if !status.is_success() {
                bail!("GitHub release archive API returned HTTP {status}");
            }
            Ok(resp)
        }
    }
}

#[cfg(target_os = "macos")]
fn download_and_verify_archive(
    standard: &Client,
    no_redirect: &Client,
    source: &DownloadSource,
    expected_sha256: &str,
    expected_size: u64,
    archive_path: &Path,
    token: Option<&str>,
) -> Result<()> {
    let mut response = open_archive_stream(standard, no_redirect, source, token)?;
    let file = File::create(archive_path)
        .with_context(|| format!("create staged archive `{}`", archive_path.display()))?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let started = Instant::now();
    let deadline = Duration::from_secs(ARCHIVE_MAX_TOTAL_TIMEOUT_SECS);
    let mut buf = [0u8; 64 * 1024];

    loop {
        ensure!(
            started.elapsed() <= deadline,
            "release archive download exceeded {ARCHIVE_MAX_TOTAL_TIMEOUT_SECS}s timeout"
        );
        let read = response
            .read(&mut buf)
            .context("stream release archive download")?;
        if read == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(read as u64);
        if expected_size > 0 && downloaded > expected_size {
            bail!(
                "downloaded archive exceeds expected size ({expected_size} bytes < {downloaded})"
            );
        }
        writer
            .write_all(&buf[..read])
            .with_context(|| format!("write staged archive `{}`", archive_path.display()))?;
        hasher.update(&buf[..read]);
    }

    writer
        .flush()
        .with_context(|| format!("flush staged archive `{}`", archive_path.display()))?;
    drop(writer);

    if expected_size > 0 {
        ensure!(
            downloaded == expected_size,
            "downloaded archive size mismatch (expected {expected_size} bytes, got {downloaded})"
        );
    }

    let actual_sha256 = encode_lower_hex(&hasher.finalize());
    let expected_lower = expected_sha256.to_ascii_lowercase();
    ensure!(
        actual_sha256 == expected_lower,
        "SHA-256 checksum mismatch (expected {expected_lower}, got {actual_sha256})"
    );

    Ok(())
}

#[cfg(target_os = "macos")]
fn determine_effective_target_channel(opts: &UpdateOptions) -> ReleaseChannel {
    if let Some(ch) = opts.channel {
        return ch;
    }
    if let Ok(env_ch) = std::env::var("M3_CHANNEL")
        && let Ok(parsed) = ReleaseChannel::parse(&env_ch)
    {
        return parsed;
    }
    channel::effective_channel().0
}

#[cfg(target_os = "macos")]
fn write_last_check_cache_best_effort(manifest: &ReleaseManifest, channel: ReleaseChannel) {
    if let Ok(path) = passive_check_cache_path() {
        let cache = LastCheckCache {
            checked_at_rfc3339: chrono::Utc::now().to_rfc3339(),
            channel: channel.as_str().to_string(),
            latest_version: manifest.version.clone(),
            tag: manifest.tag.clone(),
            release_url: manifest.release_url.clone(),
        };
        let _ = write_cache_atomic(&path, &cache);
    }
}

/// Execute `m3 update` check or apply against GitHub Releases (or explicit manifest URL).
#[cfg(target_os = "macos")]
pub fn run_update(opts: &UpdateOptions) -> Result<UpdateReport> {
    let target_triple = current_target_triple()?;
    let current_version = current_binary_version();
    let binary_channel = current_binary_channel();
    let target_channel = determine_effective_target_channel(opts);

    let current_exe = std::env::current_exe().ok();
    let env_app_dir = std::env::var("M3_APP_DIR").ok();
    let target_app = resolve_target_app_bundle(
        opts.app_dir.as_deref(),
        env_app_dir.as_deref(),
        current_exe.as_deref(),
    )?;
    let cli_link = if opts.no_cli_link {
        None
    } else {
        Some(resolve_cli_link_path(opts.bin_dir.as_deref())?)
    };

    let (standard, no_redirect) = build_clients()?;
    let resolved = resolve_release(&standard, &no_redirect, opts, target_channel, target_triple)?;
    let manifest_channel = ReleaseChannel::parse(&resolved.manifest.channel)?;

    let is_channel_switch = manifest_channel != binary_channel && opts.version.is_none();
    let is_newer = is_newer_version(current_version, &resolved.manifest.version)?;
    let update_available = is_newer || is_channel_switch;

    if opts.version.is_none() {
        write_last_check_cache_best_effort(&resolved.manifest, manifest_channel);
    }

    let asset_name = if resolved.artifact.asset.trim().is_empty() {
        format!("m3-{}-{target_triple}.tar.gz", resolved.manifest.version)
    } else {
        resolved.artifact.asset.clone()
    };

    if opts.check_only || (!update_available && !opts.force) {
        let status = if update_available {
            if is_channel_switch {
                "channel_switch"
            } else {
                "update_available"
            }
        } else if is_newer_version(&resolved.manifest.version, current_version).unwrap_or(false)
            && opts.version.is_some()
        {
            "downgrade_selected"
        } else {
            "up_to_date"
        };
        return Ok(UpdateReport {
            status: status.to_string(),
            channel: manifest_channel.as_str().to_string(),
            previous_channel: (manifest_channel != binary_channel)
                .then(|| binary_channel.as_str().to_string()),
            current_version: current_version.to_string(),
            target_version: resolved.manifest.version,
            tag: resolved.manifest.tag,
            target: target_triple.to_string(),
            update_available,
            updated: false,
            checked_only: opts.check_only,
            app_bundle_path: target_app.display().to_string(),
            cli_link_path: cli_link.map(|p| p.display().to_string()),
            source_label: resolved.source_label,
            asset: asset_name,
            archive_sha256: resolved.artifact.sha256.to_ascii_lowercase(),
            binary_sha256: None,
            release_url: resolved.manifest.release_url,
            signed: resolved.artifact.signed,
            notarized: resolved.artifact.notarized,
        });
    }

    if is_newer_version(&resolved.manifest.version, current_version)?
        && opts.version.is_some()
        && !opts.force
        && !is_channel_switch
    {
        bail!(
            "目标版本 `{}` 低于当前版本 `{current_version}`；如需降级请添加 `--force`。",
            resolved.manifest.version
        );
    }

    preflight_install_targets(&target_app, cli_link.as_deref(), opts.force)?;
    let _lock = InstallLock::acquire()?;
    let app_parent = target_app
        .parent()
        .context("target app path has no parent directory")?;
    let staging = SameFsStagingDir::create_in(app_parent)?;
    let archive_path = staging.path().join(&asset_name);
    let token = resolve_github_token();

    download_and_verify_archive(
        &standard,
        &no_redirect,
        &resolved.archive_download,
        &resolved.artifact.sha256,
        resolved.artifact.size,
        &archive_path,
        token.as_deref(),
    )?;

    let (staged_app, archive_manifest) = extract_and_verify_bundle(
        &archive_path,
        staging.path(),
        &resolved.manifest.version,
        manifest_channel,
        target_triple,
    )?;

    let receipt = atomic_swap_app_bundle(
        &staged_app,
        &target_app,
        cli_link.as_deref(),
        opts.force,
        &resolved.manifest.version,
        manifest_channel,
        &resolved.manifest.tag,
        &resolved.artifact.sha256.to_ascii_lowercase(),
        &archive_manifest.binary_sha256.to_ascii_lowercase(),
    )?;

    if opts.version.is_none() {
        write_last_check_cache_best_effort(&resolved.manifest, manifest_channel);
    }

    Ok(UpdateReport {
        status: "updated".to_string(),
        channel: manifest_channel.as_str().to_string(),
        previous_channel: (manifest_channel != binary_channel)
            .then(|| binary_channel.as_str().to_string()),
        current_version: current_version.to_string(),
        target_version: resolved.manifest.version,
        tag: resolved.manifest.tag,
        target: target_triple.to_string(),
        update_available,
        updated: true,
        checked_only: false,
        app_bundle_path: receipt.app_bundle_path,
        cli_link_path: receipt.cli_link_path,
        source_label: resolved.source_label,
        asset: asset_name,
        archive_sha256: receipt.archive_sha256,
        binary_sha256: Some(receipt.binary_sha256),
        release_url: resolved.manifest.release_url,
        signed: resolved.artifact.signed,
        notarized: resolved.artifact.notarized,
    })
}

/// Execute `m3 update` check or apply (non-macOS platforms are unsupported).
#[cfg(not(target_os = "macos"))]
pub fn run_update(_opts: &UpdateOptions) -> Result<UpdateReport> {
    bail!("self-update is only supported on macOS")
}

/// Passive background refresh entry point called on desktop launch when `last-check.json` is stale.
#[cfg(target_os = "macos")]
pub fn refresh_passive_cache_once(
    target_channel: ReleaseChannel,
) -> Result<Option<PassiveUpdateNotice>> {
    let target_triple = current_target_triple()?;
    let (standard, no_redirect) = build_clients()?;
    let opts = UpdateOptions {
        check_only: true,
        channel: Some(target_channel),
        ..Default::default()
    };
    let resolved = resolve_release(
        &standard,
        &no_redirect,
        &opts,
        target_channel,
        target_triple,
    )?;
    let manifest_channel = ReleaseChannel::parse(&resolved.manifest.channel)?;
    write_last_check_cache_best_effort(&resolved.manifest, manifest_channel);

    let current_ver = current_binary_version();
    let binary_channel = current_binary_channel();
    let is_channel_switch = manifest_channel != binary_channel;
    let is_newer = is_newer_version(current_ver, &resolved.manifest.version)?;
    if is_channel_switch || is_newer {
        Ok(Some(PassiveUpdateNotice {
            latest_version: resolved.manifest.version,
            channel: manifest_channel,
            tag: resolved.manifest.tag,
            is_channel_switch,
            release_url: resolved.manifest.release_url,
        }))
    } else {
        Ok(None)
    }
}

/// Passive background refresh stub for non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn refresh_passive_cache_once(
    _target_channel: ReleaseChannel,
) -> Result<Option<PassiveUpdateNotice>> {
    Ok(None)
}
