#!/usr/bin/env bash
set -euo pipefail

# M3 macOS installer (supports public GitHub Releases by default; optional gh/token for rate limits, plus --local builds).
# Installs M3.app to /Applications/M3.app (by default) and symlinks ~/.local/bin/m3,
# verifies SHA-256 checksums before swapping, and persists workspace + release channel.

DEFAULT_REPO="thinksmart-systems/m3"
REPO="${M3_GITHUB_REPO:-$DEFAULT_REPO}"
CHANNEL="${M3_CHANNEL:-stable}"
REQUESTED_VERSION="${M3_VERSION:-}"
APP_DIR="${M3_APP_DIR:-/Applications}"
BIN_DIR="${M3_BIN_DIR:-$HOME/.local/bin}"
WORKSPACE_DIR=""
USE_LOCAL=0
DRY_RUN=0
FORCE=0

usage() {
  cat <<'EOF'
Usage: install.sh [OPTIONS]

Install M3.app and the `m3` CLI symlink on macOS from verified GitHub Releases (or local build).

Options:
  --channel <stable|beta>   Release channel to install and track (default: stable)
  --version <VER_OR_TAG>    Install a specific version (e.g. 0.2.0 or v0.2.0)
  --local                   Install from local apps/m3-nsh/artifacts/M3.app
  --workspace <DIR>         Bind persistent workspace root (default: auto-initialized empty workspace)
  --app-dir <DIR>           Target Applications directory (default: /Applications)
  --bin-dir <DIR>           Target CLI symlink directory (default: ~/.local/bin)
  --dry-run                 Resolve manifest & verify target paths without modifying disk
  --force                   Overwrite existing non-symlink CLI file or symlink M3.app
  -h, --help                Show this help message

Environment:
  M3_GITHUB_TOKEN / GH_TOKEN / GITHUB_TOKEN   Optional token for GitHub API rate limits or private forks (falls back to `gh auth token`)
  M3_GITHUB_REPO                              GitHub owner/repo (default: thinksmart-systems/m3)
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --channel)
      CHANNEL="${2:?--channel requires value}"
      shift 2
      ;;
    --channel=*)
      CHANNEL="${1#--channel=}"
      shift
      ;;
    --version)
      REQUESTED_VERSION="${2:?--version requires value}"
      shift 2
      ;;
    --version=*)
      REQUESTED_VERSION="${1#--version=}"
      shift
      ;;
    --workspace)
      WORKSPACE_DIR="${2:?--workspace requires value}"
      shift 2
      ;;
    --workspace=*)
      WORKSPACE_DIR="${1#--workspace=}"
      shift
      ;;
    --app-dir)
      APP_DIR="${2:?--app-dir requires value}"
      shift 2
      ;;
    --app-dir=*)
      APP_DIR="${1#--app-dir=}"
      shift
      ;;
    --bin-dir)
      BIN_DIR="${2:?--bin-dir requires value}"
      shift 2
      ;;
    --bin-dir=*)
      BIN_DIR="${1#--bin-dir=}"
      shift
      ;;
    --local)
      USE_LOCAL=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --force)
      FORCE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

case "$CHANNEL" in
  stable|beta) ;;
  *)
    echo "error: invalid channel '$CHANNEL' (expected stable or beta)" >&2
    exit 1
    ;;
esac

SCRIPT_DIR=""
REPO_ROOT_GUESS=""
if [ -f "$0" ]; then
  SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
  REPO_ROOT_GUESS="$(cd "$SCRIPT_DIR/.." && pwd)"
fi
if [ -z "$WORKSPACE_DIR" ]; then
  if [ -f "$PWD/workbench/watchlist.json" ]; then
    WORKSPACE_DIR="$PWD"
  elif [ -n "$REPO_ROOT_GUESS" ] && [ -f "$REPO_ROOT_GUESS/workbench/watchlist.json" ]; then
    WORKSPACE_DIR="$REPO_ROOT_GUESS"
  fi
fi
if [ -n "$WORKSPACE_DIR" ] && [ ! -f "$WORKSPACE_DIR/workbench/watchlist.json" ]; then
  echo "error: --workspace '$WORKSPACE_DIR' missing workbench/watchlist.json" >&2
  exit 1
fi

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *)
    echo "error: unsupported macOS architecture '$ARCH'" >&2
    exit 1
    ;;
esac

resolve_token() {
  for var in M3_GITHUB_TOKEN GH_TOKEN GITHUB_TOKEN; do
    eval "val=\${$var-}"
    if [ -n "$val" ]; then
      printf '%s' "$val"
      return 0
    fi
  done
  for gh_bin in gh /opt/homebrew/bin/gh /usr/local/bin/gh; do
    if command -v "$gh_bin" >/dev/null 2>&1 || [ -x "$gh_bin" ]; then
      tok="$("$gh_bin" auth token 2>/dev/null || true)"
      if [ -n "$tok" ]; then
        printf '%s' "$tok"
        return 0
      fi
    fi
  done
  return 0
}

TARGET_APP="${APP_DIR%/}/M3.app"
CLI_LINK="${BIN_DIR%/}/m3"

if [ -L "$TARGET_APP" ] && [ "$FORCE" -ne 1 ]; then
  echo "error: refusing to overwrite symlink '$TARGET_APP' without --force" >&2
  exit 1
fi
if [ -e "$CLI_LINK" ] && [ ! -L "$CLI_LINK" ] && [ "$FORCE" -ne 1 ]; then
  echo "error: refusing to replace non-symlink '$CLI_LINK' without --force" >&2
  exit 1
fi

mkdir -p "$APP_DIR"
STAGE_ROOT="$(mktemp -d "${APP_DIR%/}/.m3-install-stage.XXXXXX")"
BACKUP_APP=""
cleanup() {
  rm -rf "$STAGE_ROOT"
}
trap cleanup EXIT

STAGED_APP=""
INSTALLED_VERSION=""

resolve_bundle_primary_bin() {
  local app_bundle="$1"
  for candidate in m3-nsh m3-desktop; do
    if [ -x "$app_bundle/Contents/MacOS/$candidate" ]; then
      printf '%s' "$app_bundle/Contents/MacOS/$candidate"
      return 0
    fi
  done
  return 1
}

resolve_bundle_cli_bin() {
  local app_bundle="$1"
  for candidate in m3 m3-nsh m3-desktop; do
    if [ -x "$app_bundle/Contents/MacOS/$candidate" ]; then
      printf '%s' "$app_bundle/Contents/MacOS/$candidate"
      return 0
    fi
  done
  return 1
}

if [ "$USE_LOCAL" -eq 1 ]; then
  if [ -z "$REPO_ROOT_GUESS" ]; then
    echo "error: --local requires running install.sh directly from the repository checkout" >&2
    exit 1
  fi
  LOCAL_APP="$REPO_ROOT_GUESS/apps/m3-nsh/artifacts/M3.app"
  LOCAL_BIN="$(resolve_bundle_primary_bin "$LOCAL_APP" || true)"
  if [ -z "$LOCAL_BIN" ]; then
    echo "error: local bundle not found at '$LOCAL_APP'; run ./apps/m3-nsh/build.sh first" >&2
    exit 1
  fi
  INSTALLED_VERSION="$("$LOCAL_BIN" -V | awk '{print $2}')"
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "Dry run (--local): version=$INSTALLED_VERSION channel=$CHANNEL target_app=$TARGET_APP cli_link=$CLI_LINK workspace=${WORKSPACE_DIR:-<unchanged>}"
    exit 0
  fi
  ditto "$LOCAL_APP" "$STAGE_ROOT/M3.app"
  STAGED_APP="$STAGE_ROOT/M3.app"
else
  TOKEN="$(resolve_token)"

  META_JSON="$STAGE_ROOT/resolved.json"
  M3_INSTALL_TOKEN="$TOKEN" python3 - "$REPO" "$CHANNEL" "$REQUESTED_VERSION" "$TARGET" "$META_JSON" <<'PY'
import json
import os
import sys
import urllib.error
import urllib.request

repo, channel, req_ver, target, out_meta = sys.argv[1:]
token = os.environ.get("M3_INSTALL_TOKEN", "").strip()

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None

def github_headers(accept: str, with_auth: bool = True) -> dict[str, str]:
    headers = {
        "Accept": accept,
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "m3-installer",
    }
    if with_auth and token:
        headers["Authorization"] = f"Bearer {token}"
    return headers

def api_get_json(url: str) -> dict:
    req = urllib.request.Request(url, headers=github_headers("application/vnd.github+json"))
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.loads(resp.read().decode("utf-8"))

def download_bytes(url: str, accept: str = "application/octet-stream", send_auth: bool = True) -> bytes:
    opener = urllib.request.build_opener(NoRedirect())
    req = urllib.request.Request(url, headers=github_headers(accept, with_auth=send_auth))
    try:
        with opener.open(req, timeout=30) as resp:
            return resp.read()
    except urllib.error.HTTPError as err:
        if err.code in (301, 302, 303, 307, 308):
            location = err.headers.get("Location")
            if not location:
                raise
            # Follow signed object URL WITHOUT Authorization header.
            with urllib.request.urlopen(urllib.request.Request(location, headers={"User-Agent": "m3-installer"}), timeout=60) as resp2:
                return resp2.read()
        raise

if not token and not req_ver:
    if channel == "beta":
        manifest_url = f"https://github.com/{repo}/releases/download/beta/latest.json"
    else:
        manifest_url = f"https://github.com/{repo}/releases/latest/download/latest.json"
    try:
        manifest = json.loads(download_bytes(manifest_url, accept="application/json", send_auth=False).decode("utf-8"))
    except urllib.error.HTTPError as err:
        raise SystemExit(f"error: public release manifest fetch {manifest_url} failed with HTTP {err.code}")
    t_info = manifest.get("targets", {}).get(target)
    if not t_info:
        raise SystemExit(f"error: release manifest missing target {target}")
    asset_name = t_info.get("asset") or f"m3-{manifest['version']}-{target}.tar.gz"
    asset_url = t_info.get("url") or f"https://github.com/{repo}/releases/download/{manifest['tag']}/{asset_name}"
    out = {
        "version": manifest["version"],
        "tag": manifest["tag"],
        "channel": manifest.get("channel", channel),
        "asset": asset_name,
        "asset_download_url": asset_url,
        "use_token": False,
        "sha256": t_info["sha256"].lower(),
        "size": int(t_info.get("size") or 0),
    }
else:
    if req_ver:
        tag = req_ver if req_ver.startswith("v") else f"v{req_ver}"
        rel_url = f"https://api.github.com/repos/{repo}/releases/tags/{tag}"
    elif channel == "beta":
        rel_url = f"https://api.github.com/repos/{repo}/releases/tags/beta"
    else:
        rel_url = f"https://api.github.com/repos/{repo}/releases/latest"

    try:
        release = api_get_json(rel_url)
    except urllib.error.HTTPError as err:
        raise SystemExit(f"error: GitHub Release API {rel_url} failed with HTTP {err.code}")

    assets = {a["name"]: a for a in release.get("assets", [])}
    if "latest.json" not in assets:
        raise SystemExit(f"error: release {release.get('tag_name')} missing latest.json")

    manifest = json.loads(download_bytes(assets["latest.json"]["url"]).decode("utf-8"))
    t_info = manifest.get("targets", {}).get(target)
    if not t_info:
        raise SystemExit(f"error: release manifest missing target {target}")

    asset_name = t_info.get("asset") or f"m3-{manifest['version']}-{target}.tar.gz"
    if asset_name not in assets:
        raise SystemExit(f"error: release missing asset {asset_name}")

    out = {
        "version": manifest["version"],
        "tag": manifest["tag"],
        "channel": manifest.get("channel", channel),
        "asset": asset_name,
        "asset_download_url": assets[asset_name]["url"],
        "use_token": bool(token),
        "sha256": t_info["sha256"].lower(),
        "size": int(t_info.get("size") or assets[asset_name].get("size") or 0),
    }
with open(out_meta, "w", encoding="utf-8") as fh:
    json.dump(out, fh)
PY

  INSTALLED_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$META_JSON")"
  ASSET_NAME="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["asset"])' "$META_JSON")"
  EXPECTED_SHA="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["sha256"])' "$META_JSON")"

  if [ "$DRY_RUN" -eq 1 ]; then
    echo "Dry run: version=$INSTALLED_VERSION channel=$CHANNEL asset=$ASSET_NAME sha256=$EXPECTED_SHA target_app=$TARGET_APP cli_link=$CLI_LINK workspace=${WORKSPACE_DIR:-<unchanged>}"
    exit 0
  fi

  ARCHIVE_PATH="$STAGE_ROOT/$ASSET_NAME"
  echo "Downloading $ASSET_NAME (version $INSTALLED_VERSION)..."
  M3_INSTALL_TOKEN="$TOKEN" python3 - "$META_JSON" "$ARCHIVE_PATH" <<'PY'
import hashlib
import json
import os
import sys
import urllib.error
import urllib.request

meta_path, dest_path = sys.argv[1:]
meta = json.load(open(meta_path))
token = os.environ.get("M3_INSTALL_TOKEN", "").strip()

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None

opener = urllib.request.build_opener(NoRedirect())
headers = {
    "Accept": "application/octet-stream",
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "m3-installer",
}
if meta.get("use_token") and token:
    headers["Authorization"] = f"Bearer {token}"

req = urllib.request.Request(meta["asset_download_url"], headers=headers)

def stream_to_file(resp):
    h = hashlib.sha256()
    total = 0
    with open(dest_path, "wb") as out:
        while True:
            chunk = resp.read(65536)
            if not chunk:
                break
            out.write(chunk)
            h.update(chunk)
            total += len(chunk)
    return h.hexdigest(), total

try:
    with opener.open(req, timeout=60) as resp:
        actual_sha, actual_size = stream_to_file(resp)
except urllib.error.HTTPError as err:
    if err.code in (301, 302, 303, 307, 308):
        location = err.headers.get("Location")
        if not location:
            raise
        with urllib.request.urlopen(urllib.request.Request(location, headers={"User-Agent": "m3-installer"}), timeout=300) as resp2:
            actual_sha, actual_size = stream_to_file(resp2)
    else:
        raise

if meta["size"] > 0 and actual_size != meta["size"]:
    raise SystemExit(f"error: size mismatch ({actual_size} != {meta['size']})")
if actual_sha != meta["sha256"]:
    raise SystemExit(f"error: SHA-256 mismatch ({actual_sha} != {meta['sha256']})")
PY

  tar -xzf "$ARCHIVE_PATH" -C "$STAGE_ROOT"
  STAGED_APP="$STAGE_ROOT/M3.app"
  STAGED_BIN="$(resolve_bundle_primary_bin "$STAGED_APP" || true)"
  if [ -z "$STAGED_BIN" ]; then
    echo "error: extracted archive missing M3.app/Contents/MacOS/{m3-nsh|m3-desktop}" >&2
    exit 1
  fi
  smoke_out="$("$STAGED_BIN" -V)"
  if [ "$smoke_out" != "m3 $INSTALLED_VERSION" ]; then
    echo "error: extracted binary verification failed: got '$smoke_out', expected 'm3 $INSTALLED_VERSION'" >&2
    exit 1
  fi
  if [ -x "$STAGED_APP/Contents/MacOS/m3" ]; then
    cli_smoke_out="$("$STAGED_APP/Contents/MacOS/m3" -V)"
    if [ "$cli_smoke_out" != "m3 $INSTALLED_VERSION" ]; then
      echo "error: extracted CLI binary verification failed: got '$cli_smoke_out', expected 'm3 $INSTALLED_VERSION'" >&2
      exit 1
    fi
  fi
fi

# Atomic bundle swap with rollback on failure.
if [ -e "$TARGET_APP" ] || [ -L "$TARGET_APP" ]; then
  if [ -L "$TARGET_APP" ]; then
    rm -f "$TARGET_APP"
  else
    BACKUP_APP="${APP_DIR%/}/.M3.app.backup.$$"
    rm -rf "$BACKUP_APP"
    mv "$TARGET_APP" "$BACKUP_APP"
  fi
fi

if ! mv "$STAGED_APP" "$TARGET_APP"; then
  if [ -n "$BACKUP_APP" ] && [ -e "$BACKUP_APP" ]; then
    mv "$BACKUP_APP" "$TARGET_APP" || true
  fi
  echo "error: failed to install '$TARGET_APP'" >&2
  exit 1
fi
if [ -n "$BACKUP_APP" ] && [ -e "$BACKUP_APP" ]; then
  rm -rf "$BACKUP_APP" || true
fi

mkdir -p "$BIN_DIR"
CLI_TARGET="$(resolve_bundle_cli_bin "$TARGET_APP" || true)"
if [ -z "$CLI_TARGET" ]; then
  echo "error: installed bundle missing CLI binary under '$TARGET_APP/Contents/MacOS/'" >&2
  exit 1
fi
TMP_LINK="${CLI_LINK}.tmp.$$"
ln -s "$CLI_TARGET" "$TMP_LINK"
mv -f "$TMP_LINK" "$CLI_LINK"

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [ -x "$LSREGISTER" ]; then
  "$LSREGISTER" -f "$TARGET_APP" >/dev/null 2>&1 || true
fi

if [ -n "$WORKSPACE_DIR" ]; then
  "$CLI_LINK" workspace set "$WORKSPACE_DIR" >/dev/null
fi
"$CLI_LINK" update channel "$CHANNEL" >/dev/null

echo "Installed M3 v$INSTALLED_VERSION to $TARGET_APP"
echo "CLI symlink: $CLI_LINK"
echo "Channel: $CHANNEL"
if [ -n "$WORKSPACE_DIR" ]; then
  echo "Workspace: $WORKSPACE_DIR"
else
  echo "Workspace: ~/Library/Application Support/M3/workspace (auto-initialized empty on first launch)"
fi
