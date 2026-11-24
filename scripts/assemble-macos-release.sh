#!/usr/bin/env bash
set -euo pipefail

# Build, optionally Developer-ID sign & notarize, verify, and package M3.app + manifest.json
# for macOS release distribution.

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_CRATE_DIR="$ROOT_DIR/apps/m3-desktop"

M3_TARGET="${M3_TARGET:-aarch64-apple-darwin}"
M3_BUILD_CHANNEL="${M3_BUILD_CHANNEL:-stable}"
M3_RELEASE_VERSION="${M3_RELEASE_VERSION:-$(python3 -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["package"]["version"])' "$APP_CRATE_DIR/Cargo.toml")}"
M3_BUILD_VERSION="${M3_BUILD_VERSION:-$M3_RELEASE_VERSION}"
M3_BUILD_COMMIT="${M3_BUILD_COMMIT:-$(git -C "$ROOT_DIR" rev-parse --short=7 HEAD 2>/dev/null || true)}"
M3_RELEASE_OUT_DIR="${M3_RELEASE_OUT_DIR:-$ROOT_DIR/dist}"
M3_REQUIRE_NOTARIZE="${M3_REQUIRE_NOTARIZE:-0}"

case "$M3_BUILD_CHANNEL" in
  stable|beta) ;;
  *)
    echo "error: invalid M3_BUILD_CHANNEL '$M3_BUILD_CHANNEL' (expected stable or beta)" >&2
    exit 1
    ;;
esac

if [ -z "$M3_RELEASE_VERSION" ]; then
  echo "error: M3_RELEASE_VERSION cannot be empty" >&2
  exit 1
fi

mkdir -p "$M3_RELEASE_OUT_DIR"

echo "Building m3-desktop target=$M3_TARGET version=$M3_BUILD_VERSION channel=$M3_BUILD_CHANNEL commit=$M3_BUILD_COMMIT"
(
  cd "$APP_CRATE_DIR"
  export M3_BUILD_VERSION
  export M3_BUILD_CHANNEL
  export M3_BUILD_COMMIT
  cargo build --release --locked --target "$M3_TARGET" -j 4
)

TARGET_DIR="$(cd "$APP_CRATE_DIR" && cargo metadata --no-deps --format-version 1 | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"
BINARY_PATH="$TARGET_DIR/$M3_TARGET/release/m3-desktop"
if [ ! -x "$BINARY_PATH" ]; then
  echo "error: built binary missing at $BINARY_PATH" >&2
  exit 1
fi

STAGE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/m3-release-stage.XXXXXX")"
cleanup() {
  rm -rf "$STAGE_ROOT"
}
trap cleanup EXIT

BUNDLE_DIR="$STAGE_ROOT/M3.app"
CONTENTS_DIR="$BUNDLE_DIR/Contents"
mkdir -p "$CONTENTS_DIR/MacOS" "$CONTENTS_DIR/Resources"

cp "$APP_CRATE_DIR/Info.plist" "$CONTENTS_DIR/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $M3_RELEASE_VERSION" "$CONTENTS_DIR/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $M3_RELEASE_VERSION" "$CONTENTS_DIR/Info.plist"
/usr/bin/plutil -lint "$CONTENTS_DIR/Info.plist" >/dev/null

cp "$BINARY_PATH" "$CONTENTS_DIR/MacOS/m3-desktop"
chmod 755 "$CONTENTS_DIR/MacOS/m3-desktop"

if [ -f "$APP_CRATE_DIR/assets/AppIcon.icns" ]; then
  cp "$APP_CRATE_DIR/assets/AppIcon.icns" "$CONTENTS_DIR/Resources/AppIcon.icns"
fi

SIGNED="false"
NOTARIZED="false"
NOTARY_STATUS="none"

if [ -n "${M3_SIGN_IDENTITY:-}" ]; then
  echo "Signing M3.app with Developer ID identity: $M3_SIGN_IDENTITY"
  sign_cmd=(
    codesign --force --deep --options runtime --timestamp
    --sign "$M3_SIGN_IDENTITY"
  )
  if [ -n "${M3_SIGN_KEYCHAIN:-}" ]; then
    sign_cmd+=(--keychain "$M3_SIGN_KEYCHAIN")
  fi
  "${sign_cmd[@]}" "$BUNDLE_DIR"
  codesign --verify --deep --strict --verbose=2 "$BUNDLE_DIR"
  SIGNED="true"

  if [ -n "${M3_NOTARY_KEY:-}" ] && [ -n "${M3_NOTARY_KEY_ID:-}" ] && [ -n "${M3_NOTARY_ISSUER:-}" ]; then
    NOTARY_ZIP="$STAGE_ROOT/M3-notary.zip"
    NOTARY_JSON="${M3_NOTARY_JSON_OUT:-$M3_RELEASE_OUT_DIR/notary-${M3_TARGET}.json}"
    ditto -c -k --keepParent "$BUNDLE_DIR" "$NOTARY_ZIP"
    xcrun notarytool submit "$NOTARY_ZIP" \
      --key "$M3_NOTARY_KEY" \
      --key-id "$M3_NOTARY_KEY_ID" \
      --issuer "$M3_NOTARY_ISSUER" \
      --wait \
      --output-format json > "$NOTARY_JSON"
    cat "$NOTARY_JSON"
    NOTARY_STATUS="$(python3 -c '
import json, pathlib, sys
raw = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
data = json.loads(raw[raw.find("{"):])
print(data.get("status", "Unknown"))
' "$NOTARY_JSON")"
    if [ "$NOTARY_STATUS" != "Accepted" ]; then
      echo "error: Apple notarization returned status=$NOTARY_STATUS" >&2
      exit 1
    fi
    xcrun stapler staple "$BUNDLE_DIR"
    xcrun stapler validate "$BUNDLE_DIR"
    spctl --assess --type execute --verbose=4 "$BUNDLE_DIR"
    NOTARIZED="true"
  elif [ "$M3_REQUIRE_NOTARIZE" = "1" ]; then
    echo "error: M3_REQUIRE_NOTARIZE=1 set but App Store Connect notary credentials missing" >&2
    exit 1
  fi
else
  if [ "$M3_REQUIRE_NOTARIZE" = "1" ]; then
    echo "error: M3_REQUIRE_NOTARIZE=1 set but M3_SIGN_IDENTITY missing" >&2
    exit 1
  fi
  echo "Applying ad-hoc signature to M3.app (Apple Developer ID secrets not configured)"
  codesign --force --deep --sign - "$BUNDLE_DIR"
  codesign --verify --deep --strict "$BUNDLE_DIR"
fi

if [ -n "${M3_SIGN_PROOF_OUT:-}" ]; then
  {
    printf 'target=%s\n' "$M3_TARGET"
    printf 'version=%s\n' "$M3_RELEASE_VERSION"
    printf 'channel=%s\n' "$M3_BUILD_CHANNEL"
    printf 'signed=%s\n' "$SIGNED"
    printf 'notarized=%s\n' "$NOTARIZED"
    printf 'notary_status=%s\n' "$NOTARY_STATUS"
  } > "$M3_SIGN_PROOF_OUT"
fi

BINARY_SHA256="$(shasum -a 256 "$CONTENTS_DIR/MacOS/m3-desktop" | awk '{print $1}')"

# Native smoke test on matching host arch before archiving.
host_arch="$(uname -m)"
if { [ "$host_arch" = "arm64" ] && [ "$M3_TARGET" = "aarch64-apple-darwin" ]; } || \
   { [ "$host_arch" = "x86_64" ] && [ "$M3_TARGET" = "x86_64-apple-darwin" ]; }; then
  short_ver="$("$CONTENTS_DIR/MacOS/m3-desktop" -V)"
  if [ "$short_ver" != "m3 $M3_RELEASE_VERSION" ]; then
    echo "error: smoke test failed: expected 'm3 $M3_RELEASE_VERSION', got '$short_ver'" >&2
    exit 1
  fi
  "$CONTENTS_DIR/MacOS/m3-desktop" --version
fi

python3 - "$STAGE_ROOT/manifest.json" "$M3_RELEASE_VERSION" "$M3_BUILD_CHANNEL" "$M3_TARGET" "$M3_BUILD_COMMIT" "$BINARY_SHA256" "$SIGNED" "$NOTARIZED" <<'PY'
import json, pathlib, sys
out_path, version, channel, target, commit, bin_sha, signed, notarized = sys.argv[1:]
doc = {
    "schema_version": 1,
    "version": version,
    "channel": channel,
    "target": target,
    "commit": commit,
    "bundle_id": "com.morris.m3.desktop",
    "binary_sha256": bin_sha,
    "signed": signed == "true",
    "notarized": notarized == "true",
}
pathlib.Path(out_path).write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")
PY

ASSET_NAME="m3-${M3_RELEASE_VERSION}-${M3_TARGET}.tar.gz"
ARCHIVE_PATH="$M3_RELEASE_OUT_DIR/$ASSET_NAME"
rm -f "$ARCHIVE_PATH" "${ARCHIVE_PATH}.sha256"

# Prevent macOS tar from embedding AppleDouble (._*) extended-attribute entries.
COPYFILE_DISABLE=1 tar -C "$STAGE_ROOT" --no-xattrs --no-mac-metadata -czf "$ARCHIVE_PATH" M3.app manifest.json 2>/dev/null || \
  COPYFILE_DISABLE=1 tar -C "$STAGE_ROOT" -czf "$ARCHIVE_PATH" M3.app manifest.json

(
  cd "$M3_RELEASE_OUT_DIR"
  shasum -a 256 "$ASSET_NAME" > "${ASSET_NAME}.sha256"
  shasum -a 256 -c "${ASSET_NAME}.sha256"
)

echo "Packaged $ARCHIVE_PATH (binary_sha256=$BINARY_SHA256 signed=$SIGNED notarized=$NOTARIZED)"
