#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

version="${M3_BUILD_VERSION:-$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml","rb"))["package"]["version"])')}"
cargo build --release --locked -j 4
target_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"
bundle="$PWD/artifacts/M3.app/Contents"
mkdir -p "$bundle/MacOS" "$bundle/Resources"
cp Info.plist "$bundle/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$bundle/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$bundle/Info.plist"
cp "$target_dir/release/m3-desktop" "$bundle/MacOS/m3-desktop"
chmod 755 "$bundle/MacOS/m3-desktop"
if [ -f "assets/AppIcon.icns" ]; then
    cp assets/AppIcon.icns "$bundle/Resources/AppIcon.icns"
fi
rm -f "$bundle/MacOS/m3-paper"
/usr/bin/plutil -lint "$bundle/Info.plist" >/dev/null
codesign --force --deep --sign - "$PWD/artifacts/M3.app" >/dev/null 2>&1 || true
touch "$PWD/artifacts/M3.app"
echo "Built $PWD/artifacts/M3.app (version $version, from $target_dir/release/m3-desktop)"
