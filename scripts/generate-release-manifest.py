#!/usr/bin/env python3
"""Generate and verify M3 macOS release manifest (latest.json) and checksums.txt."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys
import tarfile

SUPPORTED_TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
)
SUPPORTED_CHANNELS = ("stable", "beta")
DEFAULT_REPO = "Project-M3/Money-Making-Machine"
SHA256_LINE_RE = re.compile(r"^([0-9a-fA-F]{64})(?:[ \t]+\*?(.+))?$")
HEX64_RE = re.compile(r"^[0-9a-f]{64}$")
RFC3339_UTC_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$")


class ManifestError(ValueError):
    """Raised when release assets, metadata, or manifest inputs are invalid."""


def compute_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(65536)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def verify_sha256_sidecar(sidecar: Path, asset_name: str, actual_digest: str) -> None:
    lines = [line.strip() for line in sidecar.read_text(encoding="utf-8").splitlines() if line.strip()]
    if len(lines) != 1:
        raise ManifestError(f"Expected 1 checksum line in {sidecar}, got {len(lines)}")
    match = SHA256_LINE_RE.match(lines[0])
    if not match:
        raise ManifestError(f"Malformed SHA-256 sidecar {sidecar}: {lines[0]!r}")
    expected_digest = match.group(1).lower()
    recorded_name = match.group(2)
    if recorded_name is not None and Path(recorded_name.strip()).name != asset_name:
        raise ManifestError(
            f"Sidecar {sidecar} references {recorded_name!r}, expected {asset_name!r}"
        )
    if expected_digest != actual_digest:
        raise ManifestError(
            f"SHA-256 mismatch for {asset_name}: sidecar {expected_digest} != actual {actual_digest}"
        )


def verify_archive_contents(
    archive_path: Path, version: str, channel: str, target: str
) -> dict:
    has_binary = False
    has_plist = False
    manifest_bytes: bytes | None = None

    try:
        with tarfile.open(archive_path, "r:gz") as tar:
            for member in tar.getmembers():
                raw_name = member.name
                if raw_name in ("", "."):
                    continue
                parts = Path(raw_name).parts
                if raw_name.startswith("/") or ".." in parts:
                    raise ManifestError(
                        f"Unsafe archive entry path in {archive_path}: {raw_name!r}"
                    )
                if any(part.startswith("._") for part in parts):
                    raise ManifestError(
                        f"Forbidden AppleDouble metadata entry in {archive_path}: {raw_name!r}"
                    )
                if member.issym() or member.islnk():
                    raise ManifestError(
                        f"Forbidden symlink/hardlink in {archive_path}: {raw_name!r}"
                    )
                normalized = raw_name[2:] if raw_name.startswith("./") else raw_name
                if normalized in ("M3.app/Contents/MacOS/m3-desktop", "M3.app/Contents/MacOS/m3-nsh"):
                    if not member.isreg() or member.size <= 0:
                        raise ManifestError(
                            f"Archive {archive_path} binary {raw_name!r} must be non-empty regular file"
                        )
                    has_binary = True
                elif normalized == "M3.app/Contents/MacOS/m3":
                    if not member.isreg() or member.size <= 0:
                        raise ManifestError(
                            f"Archive {archive_path} CLI binary {raw_name!r} must be non-empty regular file"
                        )
                elif normalized == "M3.app/Contents/Info.plist":
                    if not member.isreg() or member.size <= 0:
                        raise ManifestError(
                            f"Archive {archive_path} Info.plist {raw_name!r} invalid"
                        )
                    has_plist = True
                elif normalized == "manifest.json":
                    if not member.isreg():
                        raise ManifestError(f"Archive {archive_path} manifest.json must be regular file")
                    extracted = tar.extractfile(member)
                    if extracted is None:
                        raise ManifestError(f"Could not read manifest.json from {archive_path}")
                    manifest_bytes = extracted.read()
    except tarfile.TarError as exc:
        raise ManifestError(f"Invalid gzip tar archive {archive_path}: {exc}") from exc

    if not has_binary:
        raise ManifestError(
            f"Archive {archive_path} missing M3.app/Contents/MacOS/{{m3-desktop,m3-nsh}}"
        )
    if not has_plist:
        raise ManifestError(f"Archive {archive_path} missing M3.app/Contents/Info.plist")
    if manifest_bytes is None:
        raise ManifestError(f"Archive {archive_path} missing manifest.json")

    try:
        meta = json.loads(manifest_bytes.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ManifestError(f"Invalid manifest.json in {archive_path}: {exc}") from exc

    if meta.get("schema_version") != 1:
        raise ManifestError(f"Unexpected archive manifest schema_version in {archive_path}")
    if meta.get("version") != version:
        raise ManifestError(
            f"Archive {archive_path} manifest version {meta.get('version')!r} != {version!r}"
        )
    if meta.get("channel") != channel:
        raise ManifestError(
            f"Archive {archive_path} manifest channel {meta.get('channel')!r} != {channel!r}"
        )
    if meta.get("target") != target:
        raise ManifestError(
            f"Archive {archive_path} manifest target {meta.get('target')!r} != {target!r}"
        )
    if not HEX64_RE.match(str(meta.get("binary_sha256", ""))):
        raise ManifestError(f"Archive {archive_path} manifest missing valid binary_sha256")
    return meta


def atomic_write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = path.with_name(f".{path.name}.tmp")
    tmp_path.write_text(content, encoding="utf-8")
    tmp_path.replace(path)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets-dir", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--tag")
    parser.add_argument("--channel", choices=SUPPORTED_CHANNELS, default="stable")
    parser.add_argument("--commit")
    parser.add_argument("--repo", default=DEFAULT_REPO)
    parser.add_argument("--published-at")
    parser.add_argument("--targets", default="aarch64-apple-darwin")
    parser.add_argument("--require-apple-notarized", action="store_true")
    parser.add_argument("--verify-archives", action="store_true")
    parser.add_argument("--output-manifest", type=Path)
    parser.add_argument("--output-checksums", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    version = args.version.strip()
    if not version or version.startswith(("v", "V")) or any(ch.isspace() for ch in version):
        raise ManifestError(f"Invalid --version {version!r} (must not start with 'v')")

    tag = args.tag.strip() if args.tag else ("beta" if args.channel == "beta" else f"v{version}")
    published_at = (
        args.published_at.strip()
        if args.published_at
        else datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    )
    if not RFC3339_UTC_RE.match(published_at):
        raise ManifestError(f"Invalid --published-at timestamp: {published_at!r}")

    target_list = [t.strip() for t in args.targets.split(",") if t.strip()]
    if not target_list:
        raise ManifestError("--targets cannot be empty")
    for t in target_list:
        if t not in SUPPORTED_TARGETS:
            raise ManifestError(f"Unsupported target {t!r}")

    download_base = f"https://github.com/{args.repo}/releases/download/{tag}"
    release_url = f"https://github.com/{args.repo}/releases/tag/{tag}"

    targets_payload: dict[str, dict] = {}
    checksum_lines: list[str] = []

    for target in target_list:
        asset_name = f"m3-{version}-{target}.tar.gz"
        asset_path = args.assets_dir / asset_name
        if not asset_path.is_file() or asset_path.is_symlink():
            raise ManifestError(f"Missing release archive: {asset_path}")

        actual_digest = compute_sha256(asset_path)
        sidecar = args.assets_dir / f"{asset_name}.sha256"
        if sidecar.exists():
            verify_sha256_sidecar(sidecar, asset_name, actual_digest)

        signed = False
        notarized = False
        if args.verify_archives:
            meta = verify_archive_contents(asset_path, version, args.channel, target)
            signed = bool(meta.get("signed", False))
            notarized = bool(meta.get("notarized", False))

        if args.require_apple_notarized and not (signed and notarized):
            raise ManifestError(
                f"Target {target} requires Apple Developer ID signing and notarization"
            )

        targets_payload[target] = {
            "asset": asset_name,
            "url": f"{download_base}/{asset_name}",
            "sha256": actual_digest,
            "size": asset_path.stat().st_size,
            "signed": signed,
            "notarized": notarized,
        }
        checksum_lines.append(f"{actual_digest}  {asset_name}")

    manifest: dict = {
        "schema_version": 1,
        "product": "m3",
        "channel": args.channel,
        "version": version,
        "tag": tag,
        "published_at": published_at,
        "release_url": release_url,
        "notes_url": release_url,
        "targets": targets_payload,
    }
    if args.commit and args.commit.strip():
        manifest["commit"] = args.commit.strip()

    manifest_path = args.output_manifest or (args.assets_dir / "latest.json")
    checksums_path = args.output_checksums or (args.assets_dir / "checksums.txt")
    atomic_write_text(manifest_path, json.dumps(manifest, indent=2) + "\n")
    atomic_write_text(checksums_path, "\n".join(checksum_lines) + "\n")
    print(f"Wrote {manifest_path} and {checksums_path} ({len(targets_payload)} targets)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ManifestError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
