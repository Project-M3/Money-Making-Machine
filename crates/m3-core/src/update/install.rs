//! Safe archive extraction, preflight checks, atomic `M3.app` bundle swap, and CLI symlink setup.

use super::ReleaseChannel;
use crate::{app_config, store};
use anyhow::{Context, Result, bail, ensure};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use tar::{Archive, EntryType};

const LOCK_DIR_NAME: &str = ".install.lock";
const RECEIPT_FILE_NAME: &str = "install-receipt.json";
const MAX_EXTRACT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const LSREGISTER_PATH: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// Archive-internal `manifest.json` written by `scripts/assemble-macos-release.sh`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveManifest {
    pub schema_version: u64,
    pub version: String,
    pub channel: String,
    pub target: String,
    #[serde(default)]
    pub commit: String,
    pub bundle_id: String,
    pub binary_sha256: String,
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub notarized: bool,
}

/// Receipt written under `.../M3/updater/install-receipt.json` after each install or update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallReceipt {
    pub schema_version: u32,
    pub version: String,
    pub channel: String,
    pub tag: String,
    pub archive_sha256: String,
    pub binary_sha256: String,
    pub app_bundle_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_link_path: Option<String>,
    pub installed_at_rfc3339: String,
}

/// RAII lock guard owning `<updater_dir>/.install.lock`.
pub struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    /// Acquire `<updater_dir>/.install.lock` atomically.
    pub fn acquire() -> Result<Self> {
        let updater_dir = app_config::updater_dir()?;
        fs::create_dir_all(&updater_dir)
            .with_context(|| format!("create updater directory `{}`", updater_dir.display()))?;
        let lock_path = updater_dir.join(LOCK_DIR_NAME);
        match fs::create_dir(&lock_path) {
            Ok(()) => Ok(Self { path: lock_path }),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => bail!(
                "更新锁 `{}` 已存在（可能有另一实例正在更新）。若确认无并发更新，可手动删除该锁目录后重试。",
                lock_path.display()
            ),
            Err(err) => Err(err).with_context(|| {
                format!("create updater lock directory `{}`", lock_path.display())
            }),
        }
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

/// RAII temporary staging directory inside `app_parent` so `fs::rename` is same-filesystem atomic.
pub struct SameFsStagingDir {
    path: PathBuf,
}

impl SameFsStagingDir {
    /// Create a fresh hidden staging directory inside `app_parent`.
    pub fn create_in(app_parent: &Path) -> Result<Self> {
        fs::create_dir_all(app_parent)
            .with_context(|| format!("create target app parent `{}`", app_parent.display()))?;
        let dir_name = format!(
            ".m3-update-stage-{}-{}",
            std::process::id(),
            store::now_ms()
        );
        let path = app_parent.join(dir_name);
        create_private_dir(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SameFsStagingDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .with_context(|| format!("create temporary directory `{}`", path.display()))
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
            .with_context(|| format!("create temporary directory `{}`", path.display()))
    }
}

/// Return `true` when `value` is a valid 64-character hexadecimal SHA-256 digest.
pub fn is_valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Encode bytes as lowercase hex.
pub fn encode_lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// Compute lowercase hex SHA-256 digest by streaming a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("open `{}` for SHA-256 check", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buf)
            .with_context(|| format!("read `{}` for SHA-256 check", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(encode_lower_hex(&hasher.finalize()))
}

/// Resolve the target `M3.app` bundle path to install or replace.
///
/// Priority:
/// 1. Explicit `app_dir` (`--app-dir` or `M3_APP_DIR`)
/// 2. Currently running `M3.app` if not inside a repo `artifacts/` or `target/` build directory
/// 3. `/Applications/M3.app`
pub fn resolve_target_app_bundle(
    explicit_app_dir: Option<&Path>,
    env_app_dir: Option<&str>,
    current_exe: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(dir) = explicit_app_dir {
        return normalize_app_bundle_target(dir);
    }
    if let Some(trimmed) = env_app_dir.map(str::trim).filter(|s| !s.is_empty()) {
        return normalize_app_bundle_target(Path::new(trimmed));
    }

    if let Some(exe) = current_exe {
        let resolved = fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
        if let Some(app_bundle) = bundle_root_from_exe(&resolved)
            && !is_dev_build_bundle(&app_bundle)
        {
            return Ok(app_bundle);
        }
    }

    Ok(PathBuf::from("/Applications/M3.app"))
}

fn normalize_app_bundle_target(path: &Path) -> Result<PathBuf> {
    ensure!(!path.as_os_str().is_empty(), "`--app-dir` cannot be empty");
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve current directory for `--app-dir`")?
            .join(path)
    };
    let candidate = if abs.file_name() == Some(OsStr::new("M3.app")) {
        abs
    } else {
        abs.join("M3.app")
    };
    ensure!(
        candidate.parent().is_some(),
        "invalid target app bundle path `{}`",
        candidate.display()
    );
    Ok(candidate)
}

fn bundle_root_from_exe(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    if macos_dir.file_name() != Some(OsStr::new("MacOS")) {
        return None;
    }
    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name() != Some(OsStr::new("Contents")) {
        return None;
    }
    let bundle_dir = contents_dir.parent()?;
    if bundle_dir.extension() != Some(OsStr::new("app")) {
        return None;
    }
    Some(bundle_dir.to_path_buf())
}

fn is_dev_build_bundle(app_bundle: &Path) -> bool {
    let Some(parent) = app_bundle.parent() else {
        return false;
    };
    let parent_name = parent.file_name().and_then(OsStr::to_str);
    if parent_name == Some("artifacts") {
        return true;
    }
    app_bundle
        .components()
        .any(|c| matches!(c, Component::Normal(s) if s == "target"))
}

/// Resolve the CLI symlink destination (`~/.local/bin/m3` or `<bin_dir>/m3`).
pub fn resolve_cli_link_path(explicit_bin_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = explicit_bin_dir {
        ensure!(!dir.as_os_str().is_empty(), "`--bin-dir` cannot be empty");
        let abs = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir()
                .context("resolve current directory for `--bin-dir`")?
                .join(dir)
        };
        return Ok(abs.join("m3"));
    }
    let home = std::env::var("HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .context("cannot resolve HOME for ~/.local/bin/m3 symlink")?;
    Ok(PathBuf::from(home).join(".local").join("bin").join("m3"))
}

/// Verify preflight permissions & ownership before mutating disk.
pub fn preflight_install_targets(
    target_app: &Path,
    cli_link: Option<&Path>,
    force: bool,
) -> Result<()> {
    let app_parent = target_app
        .parent()
        .context("target app bundle has no parent directory")?;
    ensure_writable_dir(app_parent)?;

    if let Ok(meta) = fs::symlink_metadata(target_app) {
        if meta.file_type().is_symlink() {
            ensure!(
                force,
                "拒绝覆盖符号链接 `{}`；如确认替换请加 `--force`",
                target_app.display()
            );
        } else if !meta.is_dir() {
            bail!(
                "目标路径 `{}` 不是目录/App 包，无法安装",
                target_app.display()
            );
        }
    }

    if let Some(link) = cli_link {
        if let Some(bin_parent) = link.parent() {
            fs::create_dir_all(bin_parent)
                .with_context(|| format!("create CLI bin directory `{}`", bin_parent.display()))?;
            ensure_writable_dir(bin_parent)?;
        }
        if let Ok(meta) = fs::symlink_metadata(link)
            && !meta.file_type().is_symlink()
        {
            ensure!(
                force,
                "拒绝覆盖非符号链接的 `{}`；如确认替换为符号链接请加 `--force`",
                link.display()
            );
            ensure!(
                meta.is_file(),
                "`{}` 不是普通文件或符号链接，无法替换",
                link.display()
            );
        }
    }

    Ok(())
}

fn ensure_writable_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create directory `{}`", dir.display()))?;
    let probe = dir.join(format!(
        ".m3-write-probe-{}-{}",
        std::process::id(),
        store::now_ms()
    ));
    match File::create(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            bail!(
                "目录 `{}` 不可写（Permission Denied）。请调整目录权限或使用 `--app-dir` 指定其他安装目录。",
                dir.display()
            );
        }
        Err(err) => Err(err).with_context(|| format!("check writability of `{}`", dir.display())),
    }
}

fn sanitize_archive_rel_path(raw: &Path) -> Result<Option<PathBuf>> {
    let mut clean = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(part) => {
                let s = part
                    .to_str()
                    .context("release archive entry contains non-UTF-8 path")?;
                ensure!(
                    !s.chars().any(char::is_control),
                    "release archive entry contains control characters"
                );
                ensure!(
                    !s.starts_with("._"),
                    "refusing AppleDouble metadata entry `{s}` in release archive"
                );
                clean.push(part);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "refusing unsafe path `{}` in release archive",
                    raw.display()
                );
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Ok(None);
    }
    let first = clean
        .components()
        .next()
        .and_then(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .unwrap_or("");
    let allowed = first == "M3.app" || clean == Path::new("manifest.json");
    ensure!(
        allowed,
        "unexpected top-level archive entry `{}` (expected `M3.app/...` or `manifest.json`)",
        clean.display()
    );
    Ok(Some(clean))
}

const PRIMARY_BUNDLE_BINARIES: [&str; 2] = ["m3-nsh", "m3-desktop"];
const CLI_BUNDLE_BINARIES: [&str; 3] = ["m3", "m3-nsh", "m3-desktop"];

fn resolve_bundle_executable(bundle_dir: &Path, candidates: &[&str]) -> Result<PathBuf> {
    let macos_dir = bundle_dir.join("Contents/MacOS");
    for name in candidates {
        let candidate = macos_dir.join(name);
        if let Ok(meta) = fs::symlink_metadata(&candidate) {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink() && meta.len() > 0,
                "bundle binary `{}` is not a non-empty regular file",
                candidate.display()
            );
            return Ok(candidate);
        }
    }
    bail!(
        "bundle `{}` missing executable in `Contents/MacOS/` (expected one of: {})",
        bundle_dir.display(),
        candidates.join(", ")
    );
}

/// Safely unpack a SHA-256-verified `.tar.gz` into `stage_dir`, verify `manifest.json` +
/// `binary_sha256`, and smoke-test `M3.app/Contents/MacOS/{m3-nsh|m3-desktop}` (plus `m3` if bundled).
pub fn extract_and_verify_bundle(
    archive_path: &Path,
    stage_dir: &Path,
    expected_version: &str,
    expected_channel: ReleaseChannel,
    expected_target: &str,
) -> Result<(PathBuf, ArchiveManifest)> {
    let extract_dir = stage_dir.join("extract");
    create_private_dir(&extract_dir)?;

    let file = File::open(archive_path)
        .with_context(|| format!("open verified archive `{}`", archive_path.display()))?;
    let decoder = GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = Archive::new(decoder);

    let mut total_bytes: u64 = 0;
    let mut file_count: usize = 0;

    let entries = archive
        .entries()
        .context("read entries from gzip tar archive")?;
    for entry_res in entries {
        let mut entry = entry_res.context("read archive entry header")?;
        let entry_type = entry.header().entry_type();
        ensure!(
            entry_type == EntryType::Directory
                || entry_type == EntryType::Regular
                || entry_type == EntryType::Continuous,
            "refusing unsupported non-regular/non-directory archive entry type ({entry_type:?})"
        );

        let raw_path = entry
            .path()
            .context("read archive entry path")?
            .into_owned();
        let Some(clean_rel) = sanitize_archive_rel_path(&raw_path)? else {
            continue;
        };
        let dest = extract_dir.join(&clean_rel);

        if entry_type == EntryType::Directory {
            fs::create_dir_all(&dest)
                .with_context(|| format!("create directory `{}`", dest.display()))?;
            continue;
        }

        let size = entry.header().size().context("read archive entry size")?;
        total_bytes = total_bytes.saturating_add(size);
        ensure!(
            total_bytes <= MAX_EXTRACT_TOTAL_BYTES,
            "release archive exceeds maximum unpacked byte limit ({MAX_EXTRACT_TOTAL_BYTES})"
        );

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent `{}`", parent.display()))?;
        }
        let mode = entry.header().mode().unwrap_or(0o644);
        let mut out = File::create(&dest)
            .with_context(|| format!("create extracted file `{}`", dest.display()))?;
        let copied = std::io::copy(&mut entry, &mut out)
            .with_context(|| format!("unpack entry `{}`", clean_rel.display()))?;
        out.flush()
            .with_context(|| format!("flush extracted file `{}`", dest.display()))?;
        drop(out);
        ensure!(
            copied == size,
            "truncated entry `{}` (expected {size} bytes, unpacked {copied})",
            clean_rel.display()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let sanitized_mode = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
            fs::set_permissions(&dest, fs::Permissions::from_mode(sanitized_mode))
                .with_context(|| format!("set permissions on `{}`", dest.display()))?;
        }

        file_count += 1;
    }

    ensure!(file_count > 0, "release archive contained no files");

    let manifest_path = extract_dir.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path).with_context(|| {
        format!(
            "missing `manifest.json` in archive `{}`",
            archive_path.display()
        )
    })?;
    let archive_manifest: ArchiveManifest =
        serde_json::from_slice(&manifest_bytes).context("parse archive `manifest.json`")?;
    ensure!(
        archive_manifest.schema_version == 1,
        "unsupported archive manifest schema_version {}",
        archive_manifest.schema_version
    );
    ensure!(
        archive_manifest.version == expected_version,
        "archive manifest version `{}` != expected `{expected_version}`",
        archive_manifest.version
    );
    ensure!(
        archive_manifest.channel == expected_channel.as_str(),
        "archive manifest channel `{}` != expected `{}`",
        archive_manifest.channel,
        expected_channel.as_str()
    );
    ensure!(
        archive_manifest.target == expected_target,
        "archive manifest target `{}` != expected `{expected_target}`",
        archive_manifest.target
    );
    ensure!(
        is_valid_sha256_hex(&archive_manifest.binary_sha256),
        "archive manifest has invalid binary_sha256 `{}`",
        archive_manifest.binary_sha256
    );

    let staged_app = extract_dir.join("M3.app");
    let info_plist = staged_app.join("Contents/Info.plist");
    ensure!(
        fs::symlink_metadata(&info_plist).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()),
        "staged bundle missing `Contents/Info.plist`"
    );

    let staged_bin = resolve_bundle_executable(&staged_app, &PRIMARY_BUNDLE_BINARIES)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged_bin, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod +x staged binary `{}`", staged_bin.display()))?;
    }

    let actual_bin_sha = sha256_file(&staged_bin)?;
    ensure!(
        actual_bin_sha == archive_manifest.binary_sha256.to_ascii_lowercase(),
        "staged binary SHA-256 mismatch: expected {}, got {actual_bin_sha}",
        archive_manifest.binary_sha256
    );

    smoke_test_staged_binary(&staged_bin, expected_version)?;

    let staged_cli = staged_app.join("Contents/MacOS/m3");
    if let Ok(cli_meta) = fs::symlink_metadata(&staged_cli) {
        ensure!(
            cli_meta.is_file() && !cli_meta.file_type().is_symlink() && cli_meta.len() > 0,
            "staged CLI binary `{}` is missing or empty",
            staged_cli.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged_cli, fs::Permissions::from_mode(0o755)).with_context(
                || format!("chmod +x staged CLI binary `{}`", staged_cli.display()),
            )?;
        }
        smoke_test_staged_binary(&staged_cli, expected_version)?;
    }

    Ok((staged_app, archive_manifest))
}

fn smoke_test_staged_binary(staged_bin: &Path, expected_version: &str) -> Result<()> {
    let out = Command::new(staged_bin)
        .arg("-V")
        .env("M3_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("smoke-test staged binary `{}` -V", staged_bin.display()))?;
    ensure!(
        out.status.success(),
        "staged binary `-V` smoke test failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let expected = format!("m3 {expected_version}");
    ensure!(
        stdout == expected,
        "staged binary `-V` output mismatch: expected `{expected}`, got `{stdout}`"
    );
    Ok(())
}

/// Atomically replace `target_app` with `staged_app`, update `cli_link` (if requested),
/// refresh LaunchServices, and write `install-receipt.json` under `M3/updater`.
#[expect(
    clippy::too_many_arguments,
    reason = "install transaction records complete provenance into install-receipt.json"
)]
pub fn atomic_swap_app_bundle(
    staged_app: &Path,
    target_app: &Path,
    cli_link: Option<&Path>,
    force: bool,
    version: &str,
    channel: ReleaseChannel,
    tag: &str,
    archive_sha256: &str,
    binary_sha256: &str,
) -> Result<InstallReceipt> {
    let updater_dir = app_config::updater_dir()?;
    atomic_swap_app_bundle_with_receipt_dir(
        staged_app,
        target_app,
        cli_link,
        force,
        version,
        channel,
        tag,
        archive_sha256,
        binary_sha256,
        &updater_dir,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "testable transaction helper accepts explicit receipt directory for sandbox isolation"
)]
pub(crate) fn atomic_swap_app_bundle_with_receipt_dir(
    staged_app: &Path,
    target_app: &Path,
    cli_link: Option<&Path>,
    force: bool,
    version: &str,
    channel: ReleaseChannel,
    tag: &str,
    archive_sha256: &str,
    binary_sha256: &str,
    receipt_dir: &Path,
) -> Result<InstallReceipt> {
    ensure!(
        target_app.file_name() == Some(OsStr::new("M3.app")),
        "target app path `{}` must end with `M3.app`",
        target_app.display()
    );
    let parent = target_app
        .parent()
        .context("target app path has no parent directory")?;
    let backup_app = parent.join(format!(
        ".M3.app.backup.{}.{}",
        std::process::id(),
        store::now_ms()
    ));

    let had_existing = if let Ok(meta) = fs::symlink_metadata(target_app) {
        if meta.file_type().is_symlink() {
            ensure!(
                force,
                "refusing to overwrite symlink `{}` without --force",
                target_app.display()
            );
            fs::remove_file(target_app)
                .with_context(|| format!("remove existing symlink `{}`", target_app.display()))?;
            false
        } else {
            fs::rename(target_app, &backup_app).with_context(|| {
                format!(
                    "backup existing `{}` to `{}`",
                    target_app.display(),
                    backup_app.display()
                )
            })?;
            true
        }
    } else {
        false
    };

    if let Err(err) = fs::rename(staged_app, target_app) {
        if had_existing {
            let _ = fs::rename(&backup_app, target_app);
        }
        return Err(err).with_context(|| {
            format!(
                "atomically swap `{}` into `{}`",
                staged_app.display(),
                target_app.display()
            )
        });
    }

    if had_existing {
        let _ = fs::remove_dir_all(&backup_app);
    }

    let linked_path = if let Some(link) = cli_link {
        let cli_target = resolve_bundle_executable(target_app, &CLI_BUNDLE_BINARIES)?;
        install_cli_symlink(&cli_target, link, force)?;
        Some(link.display().to_string())
    } else {
        None
    };

    if Path::new(LSREGISTER_PATH).exists() {
        let _ = Command::new(LSREGISTER_PATH)
            .arg("-f")
            .arg(target_app)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    let receipt = InstallReceipt {
        schema_version: 1,
        version: version.to_string(),
        channel: channel.as_str().to_string(),
        tag: tag.to_string(),
        archive_sha256: archive_sha256.to_string(),
        binary_sha256: binary_sha256.to_string(),
        app_bundle_path: target_app.display().to_string(),
        cli_link_path: linked_path,
        installed_at_rfc3339: chrono::Utc::now().to_rfc3339(),
    };
    let receipt_path = receipt_dir.join(RECEIPT_FILE_NAME);
    store::write_json_atomic(&receipt_path, &receipt)?;

    Ok(receipt)
}

fn install_cli_symlink(target_bin: &Path, link_path: &Path, force: bool) -> Result<()> {
    let parent = link_path
        .parent()
        .context("CLI symlink path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create CLI directory `{}`", parent.display()))?;

    if let Ok(meta) = fs::symlink_metadata(link_path)
        && !meta.file_type().is_symlink()
    {
        ensure!(
            force,
            "refusing to replace non-symlink `{}` without --force",
            link_path.display()
        );
    }

    #[cfg(unix)]
    {
        let tmp_link = parent.join(format!(
            ".m3.link.tmp.{}.{}",
            std::process::id(),
            store::now_ms()
        ));
        std::os::unix::fs::symlink(target_bin, &tmp_link).with_context(|| {
            format!(
                "create temporary CLI symlink `{}` -> `{}`",
                tmp_link.display(),
                target_bin.display()
            )
        })?;
        if let Err(err) = fs::rename(&tmp_link, link_path) {
            let _ = fs::remove_file(&tmp_link);
            return Err(err).with_context(|| {
                format!("atomically install CLI symlink `{}`", link_path.display())
            });
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (target_bin, force);
        bail!("CLI symlink installation is only supported on Unix/macOS")
    }
}

/// Spawn a detached relaunch of `app_bundle` so the caller can exit cleanly and open the updated app.
pub fn relaunch_app_bundle(app_bundle: &Path) -> Result<()> {
    Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 0.4; /usr/bin/open -n \"$1\"")
        .arg("_")
        .arg(app_bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn relaunch for `{}`", app_bundle.display()))?;
    Ok(())
}
