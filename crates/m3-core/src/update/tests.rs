#[cfg(target_os = "macos")]
use super::install::{
    ArchiveManifest, SameFsStagingDir, atomic_swap_app_bundle_with_receipt_dir, encode_lower_hex,
    extract_and_verify_bundle, preflight_install_targets, resolve_target_app_bundle,
};
use super::passive_check::{
    LastCheckCache, PASSIVE_CHECK_TTL_BETA_SECS, PASSIVE_CHECK_TTL_STABLE_SECS,
    evaluate_cache_for_notice_with_target, should_skip_passive_check_with, write_cache_atomic,
};
use super::semver::{is_newer_version, parse_semver, target_triple_for};
use super::{ReleaseChannel, ReleaseManifest, TargetArtifact, validate_manifest_metadata};
use chrono::{Duration, Utc};
#[cfg(target_os = "macos")]
use flate2::Compression;
#[cfg(target_os = "macos")]
use flate2::write::GzEncoder;
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use tar::{Builder, Header};

fn make_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "m3-update-test-{name}-{}-{}",
        std::process::id(),
        crate::store::now_ms()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn semver_ordering_matches_spec_for_stable_and_beta() {
    assert!(is_newer_version("0.2.0-beta.20261124.1", "0.2.0").unwrap());
    assert!(!is_newer_version("0.2.0", "0.2.0-beta.20261124.1").unwrap());
    assert!(is_newer_version("0.2.0", "0.2.1-beta.20261124.2").unwrap());
    assert!(is_newer_version("0.2.1-beta.20261124.2", "0.2.1-beta.20261124.10").unwrap());
    assert!(is_newer_version("0.2.1-beta.20261124.10", "0.2.1").unwrap());
    assert!(!is_newer_version("0.2.1", "0.2.1").unwrap());

    assert!(parse_semver("0.2.0-01").is_err());
    assert!(parse_semver("01.2.0").is_err());
    assert!(parse_semver("0.2").is_err());
    assert_eq!(
        target_triple_for("macos", "aarch64").unwrap(),
        "aarch64-apple-darwin"
    );
    assert!(target_triple_for("linux", "x86_64").is_err());
}

#[test]
fn validate_manifest_checks_channel_and_product() {
    let mut targets = BTreeMap::new();
    targets.insert(
        "aarch64-apple-darwin".to_string(),
        TargetArtifact {
            asset: "m3-0.2.1-beta.20261124.1-aarch64-apple-darwin.tar.gz".to_string(),
            url: "https://example.test/m3.tar.gz".to_string(),
            sha256: "a".repeat(64),
            size: 123,
            signed: false,
            notarized: false,
        },
    );
    let manifest = ReleaseManifest {
        schema_version: 1,
        product: "m3".to_string(),
        channel: "beta".to_string(),
        version: "0.2.1-beta.20261124.1".to_string(),
        tag: "beta".to_string(),
        commit: Some("abcdef1".to_string()),
        published_at: None,
        release_url: None,
        notes_url: None,
        targets,
    };

    let (ver, ch) =
        validate_manifest_metadata(&manifest, None, Some(ReleaseChannel::Beta)).unwrap();
    assert_eq!(ver, "0.2.1-beta.20261124.1");
    assert_eq!(ch, ReleaseChannel::Beta);
    assert!(validate_manifest_metadata(&manifest, None, Some(ReleaseChannel::Stable)).is_err());
}

#[test]
fn passive_check_cache_handles_fresh_stale_and_channel_switch() {
    let dir = make_temp_dir("passive");
    let cache_path = dir.join("last-check.json");
    let now = Utc::now();

    let (_, needs_refresh) = evaluate_cache_for_notice_with_target(
        &cache_path,
        "0.2.0",
        ReleaseChannel::Stable,
        ReleaseChannel::Stable,
        now,
    );
    assert!(needs_refresh);

    // Fresh beta cache with newer version.
    let cache = LastCheckCache {
        checked_at_rfc3339: (now - Duration::seconds(PASSIVE_CHECK_TTL_BETA_SECS / 2)).to_rfc3339(),
        channel: "beta".to_string(),
        latest_version: "0.2.1-beta.20261124.5".to_string(),
        tag: "beta".to_string(),
        release_url: Some("https://github.com/thinksmart-systems/m3/releases/tag/beta".to_string()),
    };
    write_cache_atomic(&cache_path, &cache).unwrap();

    let (notice, refresh) = evaluate_cache_for_notice_with_target(
        &cache_path,
        "0.2.0",
        ReleaseChannel::Stable,
        ReleaseChannel::Beta,
        now,
    );
    assert!(!refresh);
    let notice = notice.expect("should emit switch/upgrade notice");
    assert!(notice.is_channel_switch);
    assert_eq!(notice.latest_version, "0.2.1-beta.20261124.5");

    // Stale stable cache triggers background refresh.
    let stale_cache = LastCheckCache {
        checked_at_rfc3339: (now - Duration::seconds(PASSIVE_CHECK_TTL_STABLE_SECS + 10))
            .to_rfc3339(),
        channel: "stable".to_string(),
        latest_version: "0.2.0".to_string(),
        tag: "v0.2.0".to_string(),
        release_url: None,
    };
    write_cache_atomic(&cache_path, &stale_cache).unwrap();
    let (notice, refresh) = evaluate_cache_for_notice_with_target(
        &cache_path,
        "0.2.0",
        ReleaseChannel::Stable,
        ReleaseChannel::Stable,
        now,
    );
    assert!(notice.is_none());
    assert!(refresh);

    assert!(should_skip_passive_check_with(false, Some("1"), None));
    assert!(should_skip_passive_check_with(false, None, Some("true")));
    assert!(!should_skip_passive_check_with(
        false,
        Some("0"),
        Some("false")
    ));

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "macos")]
#[test]
fn resolve_target_app_ignores_repo_artifacts_build_dir() {
    let dev_exe = Path::new("/Users/test/repo/apps/m3-nsh/artifacts/M3.app/Contents/MacOS/m3-nsh");
    let resolved = resolve_target_app_bundle(None, None, Some(dev_exe)).unwrap();
    assert_eq!(resolved, PathBuf::from("/Applications/M3.app"));

    let legacy_dev_exe =
        Path::new("/Users/test/repo/apps/m3-desktop/artifacts/M3.app/Contents/MacOS/m3-desktop");
    let legacy_resolved = resolve_target_app_bundle(None, None, Some(legacy_dev_exe)).unwrap();
    assert_eq!(legacy_resolved, PathBuf::from("/Applications/M3.app"));

    let custom_dir = make_temp_dir("appdir");
    let custom_resolved =
        resolve_target_app_bundle(Some(&custom_dir), None, Some(dev_exe)).unwrap();
    assert_eq!(custom_resolved, custom_dir.join("M3.app"));
    let _ = fs::remove_dir_all(&custom_dir);
}

#[cfg(target_os = "macos")]
#[test]
fn extract_rejects_path_traversal_and_symlinks() {
    let dir = make_temp_dir("unsafe-tar");
    let traversal_archive = dir.join("evil-traversal.tar.gz");
    {
        let file = fs::File::create(&traversal_archive).unwrap();
        let enc = GzEncoder::new(file, Compression::default());
        let mut tar = Builder::new(enc);
        let payload = b"evil";
        let mut header = Header::new_gnu();
        let name = b"../escape.txt";
        header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append(&header, &payload[..]).unwrap();
        tar.finish().unwrap();
    }

    let staging = SameFsStagingDir::create_in(&dir).unwrap();
    let err = extract_and_verify_bundle(
        &traversal_archive,
        staging.path(),
        "9.9.9",
        ReleaseChannel::Stable,
        "aarch64-apple-darwin",
    )
    .unwrap_err();
    assert!(err.to_string().contains("unsafe path"));

    let symlink_archive = dir.join("evil-symlink.tar.gz");
    {
        let file = fs::File::create(&symlink_archive).unwrap();
        let enc = GzEncoder::new(file, Compression::default());
        let mut tar = Builder::new(enc);
        let mut header = Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        tar.append_link(&mut header, "M3.app/Contents/MacOS/m3-nsh", "/bin/sh")
            .unwrap();
        tar.finish().unwrap();
    }
    let err2 = extract_and_verify_bundle(
        &symlink_archive,
        staging.path(),
        "9.9.9",
        ReleaseChannel::Stable,
        "aarch64-apple-darwin",
    )
    .unwrap_err();
    assert!(err2.to_string().contains("non-regular"));

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "macos")]
#[test]
fn end_to_end_extract_smoke_test_and_atomic_swap_with_cli_symlink() {
    let dir = make_temp_dir("swap");
    let app_dir = dir.join("Applications");
    let bin_dir = dir.join("bin");
    let support_dir = dir.join("AppSupport");
    fs::create_dir_all(&app_dir).unwrap();
    fs::create_dir_all(&bin_dir).unwrap();

    // Seed an existing legacy m3-desktop bundle and CLI symlink pointing to m3-desktop.
    let target_app = app_dir.join("M3.app");
    let old_bin = target_app.join("Contents/MacOS/m3-desktop");
    fs::create_dir_all(old_bin.parent().unwrap()).unwrap();
    fs::write(&old_bin, b"#!/bin/sh\necho \"m3 0.1.0\"\n").unwrap();

    let cli_link = bin_dir.join("m3");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&old_bin, &cli_link).unwrap();

    let app_script_body =
        b"#!/bin/sh\nif [ \"$1\" = \"-V\" ]; then echo \"m3 9.9.9\"; exit 0; fi\nexit 0\n";
    let cli_script_body =
        b"#!/bin/sh\nif [ \"$1\" = \"-V\" ]; then echo \"m3 9.9.9\"; exit 0; fi\nexit 0\n";
    let bin_sha = encode_lower_hex(&Sha256::digest(app_script_body));
    let plist_body = b"<plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>m3-nsh</string><key>CFBundleShortVersionString</key><string>9.9.9</string></dict></plist>";
    let nsh_app_json = br#"{"name":"M3","id":"systems.thinksmart.m3","version":"9.9.9"}"#;
    let archive_manifest = ArchiveManifest {
        schema_version: 1,
        version: "9.9.9".to_string(),
        channel: "beta".to_string(),
        target: "aarch64-apple-darwin".to_string(),
        commit: "deadbee".to_string(),
        bundle_id: "systems.thinksmart.m3".to_string(),
        binary_sha256: bin_sha.clone(),
        signed: false,
        notarized: false,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&archive_manifest).unwrap();

    let archive_path = dir.join("m3-9.9.9-aarch64-apple-darwin.tar.gz");
    {
        let file = fs::File::create(&archive_path).unwrap();
        let enc = GzEncoder::new(file, Compression::default());
        let mut tar = Builder::new(enc);

        for (rel, bytes, mode) in [
            ("M3.app/Contents/MacOS/m3-nsh", &app_script_body[..], 0o755),
            ("M3.app/Contents/MacOS/m3", &cli_script_body[..], 0o755),
            ("M3.app/Contents/Info.plist", &plist_body[..], 0o644),
            (
                "M3.app/Contents/Resources/nsh/app.json",
                &nsh_app_json[..],
                0o644,
            ),
            ("manifest.json", &manifest_bytes[..], 0o644),
        ] {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            tar.append_data(&mut header, rel, bytes).unwrap();
        }
        tar.finish().unwrap();
    }

    let staging = SameFsStagingDir::create_in(&app_dir).unwrap();
    let (staged_app, parsed_manifest) = extract_and_verify_bundle(
        &archive_path,
        staging.path(),
        "9.9.9",
        ReleaseChannel::Beta,
        "aarch64-apple-darwin",
    )
    .unwrap();
    assert_eq!(parsed_manifest.commit, "deadbee");

    fs::create_dir_all(&support_dir).unwrap();
    let archive_sha = super::install::sha256_file(&archive_path).unwrap();

    // Existing symlink cli_link pointing to old m3-desktop must be replaceable without --force.
    assert!(preflight_install_targets(&target_app, Some(&cli_link), false).is_ok());

    let receipt = atomic_swap_app_bundle_with_receipt_dir(
        &staged_app,
        &target_app,
        Some(&cli_link),
        false,
        "9.9.9",
        ReleaseChannel::Beta,
        "beta",
        &archive_sha,
        &bin_sha,
        &support_dir,
    )
    .unwrap();
    assert_eq!(receipt.version, "9.9.9");
    assert!(support_dir.join("install-receipt.json").exists());
    assert!(!target_app.join("Contents/MacOS/m3-desktop").exists());
    assert!(target_app.join("Contents/MacOS/m3-nsh").is_file());
    assert!(target_app.join("Contents/MacOS/m3").is_file());
    assert_eq!(
        fs::read_link(&cli_link).unwrap(),
        target_app.join("Contents/MacOS/m3")
    );
    let out = std::process::Command::new(&cli_link)
        .arg("-V")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "m3 9.9.9");

    // Regular non-symlink file in cli_link must still fail preflight unless --force.
    fs::remove_file(&cli_link).unwrap();
    fs::write(&cli_link, b"custom").unwrap();
    assert!(preflight_install_targets(&target_app, Some(&cli_link), false).is_err());

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "macos")]
#[test]
fn extract_and_swap_supports_legacy_m3_desktop_archive_for_rollback() {
    let dir = make_temp_dir("legacy-rollback");
    let app_dir = dir.join("Applications");
    let bin_dir = dir.join("bin");
    let support_dir = dir.join("AppSupport");
    fs::create_dir_all(&app_dir).unwrap();
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&support_dir).unwrap();

    let script_body =
        b"#!/bin/sh\nif [ \"$1\" = \"-V\" ]; then echo \"m3 0.3.0\"; exit 0; fi\nexit 0\n";
    let bin_sha = encode_lower_hex(&Sha256::digest(script_body));
    let plist_body = b"<plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>m3-desktop</string></dict></plist>";
    let archive_manifest = ArchiveManifest {
        schema_version: 1,
        version: "0.3.0".to_string(),
        channel: "stable".to_string(),
        target: "aarch64-apple-darwin".to_string(),
        commit: "0000001".to_string(),
        bundle_id: "com.morris.m3.desktop".to_string(),
        binary_sha256: bin_sha.clone(),
        signed: false,
        notarized: false,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&archive_manifest).unwrap();

    let archive_path = dir.join("m3-0.3.0-aarch64-apple-darwin.tar.gz");
    {
        let file = fs::File::create(&archive_path).unwrap();
        let enc = GzEncoder::new(file, Compression::default());
        let mut tar = Builder::new(enc);
        for (rel, bytes, mode) in [
            ("M3.app/Contents/MacOS/m3-desktop", &script_body[..], 0o755),
            ("M3.app/Contents/Info.plist", &plist_body[..], 0o644),
            ("manifest.json", &manifest_bytes[..], 0o644),
        ] {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            tar.append_data(&mut header, rel, bytes).unwrap();
        }
        tar.finish().unwrap();
    }

    let staging = SameFsStagingDir::create_in(&app_dir).unwrap();
    let (staged_app, _) = extract_and_verify_bundle(
        &archive_path,
        staging.path(),
        "0.3.0",
        ReleaseChannel::Stable,
        "aarch64-apple-darwin",
    )
    .unwrap();
    let target_app = app_dir.join("M3.app");
    let cli_link = bin_dir.join("m3");
    let archive_sha = super::install::sha256_file(&archive_path).unwrap();

    let receipt = atomic_swap_app_bundle_with_receipt_dir(
        &staged_app,
        &target_app,
        Some(&cli_link),
        false,
        "0.3.0",
        ReleaseChannel::Stable,
        "v0.3.0",
        &archive_sha,
        &bin_sha,
        &support_dir,
    )
    .unwrap();
    assert_eq!(receipt.version, "0.3.0");
    assert_eq!(
        fs::read_link(&cli_link).unwrap(),
        target_app.join("Contents/MacOS/m3-desktop")
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn channel_selection_roundtrips_with_explicit_m3_paths() {
    let dir = make_temp_dir("channel-m3paths");
    let m3_paths = crate::app_config::M3Paths::from_root(&dir);

    assert_eq!(
        super::channel::effective_channel_in(&m3_paths),
        (
            super::current_binary_channel(),
            super::channel::ChannelSource::BinaryDefault
        )
    );
    super::channel::write_configured_channel_in(&m3_paths, ReleaseChannel::Beta).unwrap();
    assert_eq!(
        super::channel::read_configured_channel_in(&m3_paths),
        Some(ReleaseChannel::Beta)
    );
    assert_eq!(
        super::channel::effective_channel_in(&m3_paths),
        (
            ReleaseChannel::Beta,
            super::channel::ChannelSource::Configured
        )
    );

    let report = super::channel::channel_status_report_in(&m3_paths);
    assert_eq!(report.channel, "beta");
    assert_eq!(report.source, "configured");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn update_progress_fractions_increase_monotonically_across_stages() {
    use super::{UpdateProgress, UpdateStage};

    let resolving = UpdateProgress::for_stage(UpdateStage::Resolving, 0, None);
    let dl_start = UpdateProgress::for_stage(UpdateStage::Downloading, 0, Some(1_000));
    let dl_mid = UpdateProgress::for_stage(UpdateStage::Downloading, 500, Some(1_000));
    let dl_end = UpdateProgress::for_stage(UpdateStage::Downloading, 1_000, Some(1_000));
    let verifying = UpdateProgress::for_stage(UpdateStage::Verifying, 1_000, Some(1_000));
    let extracting = UpdateProgress::for_stage(UpdateStage::Extracting, 1_000, Some(1_000));
    let installing = UpdateProgress::for_stage(UpdateStage::Installing, 1_000, Some(1_000));
    let completed = UpdateProgress::for_stage(UpdateStage::Completed, 1_000, Some(1_000));

    assert!(resolving.fraction <= dl_start.fraction);
    assert!(dl_start.fraction < dl_mid.fraction);
    assert!(dl_mid.fraction < dl_end.fraction);
    assert!(dl_end.fraction < verifying.fraction);
    assert!(verifying.fraction < extracting.fraction);
    assert!(extracting.fraction < installing.fraction);
    assert!(installing.fraction < completed.fraction);
    assert!((completed.fraction - 1.0).abs() < f32::EPSILON);
}

#[test]
fn os_worker_thread_drops_reqwest_blocking_client_cleanly_from_tokio_runtime() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let res = rt.block_on(async {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("m3-worker-test".into())
            .spawn(move || {
                let built = reqwest::blocking::Client::builder()
                    .build()
                    .map(|_client| ())
                    .map_err(|e| e.to_string());
                let _ = tx.send(built);
            })
            .unwrap();
        rx.await.unwrap()
    });

    assert!(res.is_ok(), "unexpected error: {res:?}");
}

#[test]
fn default_manifest_url_builds_public_release_endpoints() {
    let stable = super::default_manifest_url(
        super::DEFAULT_GITHUB_BASE_URL,
        super::DEFAULT_GITHUB_REPO,
        ReleaseChannel::Stable,
        None,
    )
    .unwrap();
    assert_eq!(
        stable,
        "https://github.com/thinksmart-systems/m3/releases/latest/download/latest.json"
    );

    let beta = super::default_manifest_url(
        super::DEFAULT_GITHUB_BASE_URL,
        super::DEFAULT_GITHUB_REPO,
        ReleaseChannel::Beta,
        None,
    )
    .unwrap();
    assert_eq!(
        beta,
        "https://github.com/thinksmart-systems/m3/releases/download/beta/latest.json"
    );

    for ver in ["0.4.2", "v0.4.2"] {
        let pinned = super::default_manifest_url(
            super::DEFAULT_GITHUB_BASE_URL,
            super::DEFAULT_GITHUB_REPO,
            ReleaseChannel::Stable,
            Some(ver),
        )
        .unwrap();
        assert_eq!(
            pinned,
            "https://github.com/thinksmart-systems/m3/releases/download/v0.4.2/latest.json"
        );
    }
}
