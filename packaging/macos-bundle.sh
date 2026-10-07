#!/usr/bin/env bash
# Assemble dist/Apark.app from the SwiftUI build plus the universal CLI, then zip it.
#   packaging/macos-bundle.sh <swift-binary> <cli-binary>
set -euo pipefail
swift_bin="$1"; cli_bin="$2"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
app=dist/Apark.app/Contents
rm -rf dist/Apark.app && mkdir -p "$app/MacOS" "$app/Resources"
cp "$swift_bin" "$app/MacOS/Apark"
cp "$cli_bin" "$app/MacOS/apark-cli"
sed "s/VERSION/$version/g" packaging/Info.plist > "$app/Info.plist"
iconset="$(mktemp -d)/Apark.iconset"; mkdir -p "$iconset"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon-1024.png --out "$iconset/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon-1024.png --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Resources/Apark.icns"
codesign --force --deep -s - dist/Apark.app
(cd dist && ditto -c -k --keepParent Apark.app Apark-macos-universal.zip)
tar -C "$(dirname "$cli_bin")" -czf dist/apark-cli-macos-universal.tar.gz "$(basename "$cli_bin")"
ls -la dist
