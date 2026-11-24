#!/usr/bin/env bash
set -euo pipefail

# Release management helper for Project-M3.
# Usage:
#   ./scripts/release.sh status
#   ./scripts/release.sh stable <X.Y.Z> [--push]

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT_DIR/apps/m3-nsh"
CORE_DIR="$ROOT_DIR/crates/m3-core"
CLI_DIR="$ROOT_DIR/crates/m3-cli"
cd "$ROOT_DIR"

usage() {
  cat <<'EOF'
Usage:
  scripts/release.sh status
  scripts/release.sh stable <X.Y.Z> [--push]
EOF
}

cmd="${1:-status}"
case "$cmd" in
  status)
    ver="$(python3 -c 'import tomllib; print(tomllib.load(open("apps/m3-nsh/Cargo.toml","rb"))["package"]["version"])')"
    echo "Branch: $(git branch --show-current)"
    echo "HEAD:   $(git rev-parse --short=7 HEAD)"
    echo "Cargo:  $ver"
    echo "Tags:"
    git tag -l --sort=-v:refname | head -n 10 || true
    if command -v gh >/dev/null 2>&1; then
      echo "GitHub Releases:"
      gh release list -L 10 || true
      echo "Recent Workflows:"
      gh run list -L 5 || true
    fi
    ;;
  stable)
    shift
    version="${1:-}"
    if [ -z "$version" ]; then
      usage >&2
      exit 1
    fi
    shift
    do_push=0
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --push) do_push=1; shift ;;
        *) echo "error: unknown flag '$1'" >&2; exit 1 ;;
      esac
    done

    if ! python3 -c 'import re,sys; sys.exit(0 if re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", sys.argv[1]) else 1)' "$version"; then
      echo "error: version must be strict MAJOR.MINOR.PATCH without 'v' prefix, got '$version'" >&2
      exit 1
    fi

    branch="$(git branch --show-current)"
    if [ "$branch" != "main" ]; then
      echo "error: must run from 'main' branch (current: '$branch')" >&2
      exit 1
    fi
    if [ -n "$(git status --porcelain)" ]; then
      echo "error: working tree is not clean; commit or stash changes first" >&2
      exit 1
    fi

    tag="v$version"
    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null 2>&1; then
      echo "error: tag '$tag' already exists" >&2
      exit 1
    fi

    python3 - "$version" "$CORE_DIR/Cargo.toml" "$CLI_DIR/Cargo.toml" "$APP_DIR/Cargo.toml" "$APP_DIR/nsh.json" <<'PY'
import json, pathlib, re, sys
version, *targets = sys.argv[1:]
for raw_path in targets:
    path = pathlib.Path(raw_path)
    if path.name == "nsh.json":
        doc = json.loads(path.read_text(encoding="utf-8"))
        doc["version"] = version
        path.write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")
    else:
        text = path.read_text(encoding="utf-8")
        updated, n = re.subn(r'(?m)^(version\s*=\s*")([^"]+)(")', rf'\g<1>{version}\g<3>', text, count=1)
        if n != 1:
            raise SystemExit(f"failed to update version in {path}")
        path.write_text(updated, encoding="utf-8")
PY

    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$APP_DIR/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$APP_DIR/Info.plist"
    /usr/bin/plutil -lint "$APP_DIR/Info.plist" >/dev/null

    (
      cd "$ROOT_DIR"
      cargo update -p m3-core -p m3-cli -p m3-nsh --offline
      cargo fmt --all -- --check
      cargo clippy --workspace --locked --all-targets -- -D warnings
      cargo test --workspace --locked -j 4
    )

    git add "$CORE_DIR/Cargo.toml" "$CLI_DIR/Cargo.toml" "$APP_DIR/Cargo.toml" "$APP_DIR/nsh.json" "$APP_DIR/Info.plist" "$ROOT_DIR/Cargo.lock"
    if ! git diff --cached --quiet; then
      git commit -m "chore(release): prepare $tag"
    fi
    git tag -a "$tag" -m "M3 $tag"
    echo "Created release tag $tag at $(git rev-parse --short=7 HEAD)"

    if [ "$do_push" -eq 1 ]; then
      git push origin main "$tag"
    else
      echo "Next step: git push origin main $tag"
    fi
    ;;
  -h|--help)
    usage
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
