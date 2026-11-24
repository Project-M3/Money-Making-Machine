#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
root_dir="$(cd ../.. && pwd)"

version="${M3_BUILD_VERSION:-$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml","rb"))["package"]["version"])')}"
(
  cd "$root_dir"
  cargo build --release --locked -p m3-nsh -p m3-cli -j 4
)
target_dir="$(cd "$root_dir" && cargo metadata --no-deps --format-version 1 | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"

bundle="$PWD/artifacts/M3.app/Contents"
mkdir -p "$bundle/MacOS" "$bundle/Resources/nsh"

cp Info.plist "$bundle/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$bundle/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$bundle/Info.plist"
/usr/bin/plutil -lint "$bundle/Info.plist" >/dev/null

cp "$target_dir/release/m3-nsh" "$bundle/MacOS/m3-nsh"
chmod 755 "$bundle/MacOS/m3-nsh"
cp "$target_dir/release/m3" "$bundle/MacOS/m3"
chmod 755 "$bundle/MacOS/m3"
rm -f "$bundle/MacOS/m3-desktop" "$bundle/MacOS/m3-paper"

if [ -f "assets/AppIcon.icns" ]; then
    cp assets/AppIcon.icns "$bundle/Resources/AppIcon.icns"
fi

python3 - "$PWD/nsh.json" "$bundle/Resources/nsh/app.json" "$version" <<'PY'
import json, pathlib, sys
src, dst, version = sys.argv[1:]
doc = json.loads(pathlib.Path(src).read_text(encoding="utf-8"))
doc["version"] = version
pathlib.Path(dst).write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")
PY

codesign --force --deep --sign - "$PWD/artifacts/M3.app" >/dev/null 2>&1 || true
touch "$PWD/artifacts/M3.app"
echo "Built $PWD/artifacts/M3.app (version $version, from $target_dir/release/{m3-nsh,m3})"
