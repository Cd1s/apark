#!/usr/bin/env bash
# package.sh <rust-target> <label>   e.g. package.sh aarch64-apple-darwin macos-arm64
# Produces dist/Apark-<label>.(zip|tar.gz) (desktop + CLI) and dist/apark-cli-<label>.(zip|tar.gz).
set -euo pipefail
target="$1"; label="$2"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
bin="target/$target/release"
rm -rf dist/stage && mkdir -p dist/stage
case "$target" in
  *apple-darwin)
    app="dist/stage/Apark.app/Contents"
    mkdir -p "$app/MacOS" "$app/Resources"
    cp "$bin/apark-desktop" "$bin/apark" "$app/MacOS/"
    sed "s/VERSION/$version/g" packaging/Info.plist > "$app/Info.plist"
    python3 packaging/make_icon.py dist/stage/icon.png 1024
    iconset=dist/stage/Apark.iconset; mkdir -p "$iconset"
    for s in 16 32 128 256 512; do
      sips -z $s $s dist/stage/icon.png --out "$iconset/icon_${s}x${s}.png" >/dev/null
      sips -z $((s*2)) $((s*2)) dist/stage/icon.png --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "$iconset" -o "$app/Resources/Apark.icns"
    codesign --force --deep -s - dist/stage/Apark.app
    (cd dist/stage && ditto -c -k --keepParent Apark.app "../Apark-$label.zip")
    tar -C "$bin" -czf "dist/apark-cli-$label.tar.gz" apark
    ;;
  *windows*)
    mkdir -p dist/stage/Apark
    cp "$bin/apark-desktop.exe" dist/stage/Apark/Apark.exe
    cp "$bin/apark.exe" dist/stage/Apark/apark.exe
    (cd dist/stage && 7z a -tzip "../Apark-$label.zip" Apark >/dev/null)
    (cd "$bin" && 7z a -tzip "$OLDPWD/dist/apark-cli-$label.zip" apark.exe >/dev/null)
    ;;
  *linux*)
    mkdir -p dist/stage/Apark
    cp "$bin/apark-desktop" "$bin/apark" packaging/apark.desktop dist/stage/Apark/
    python3 packaging/make_icon.py dist/stage/Apark/apark.png 256
    tar -C dist/stage -czf "dist/Apark-$label.tar.gz" Apark
    tar -C "$bin" -czf "dist/apark-cli-$label.tar.gz" apark
    ;;
esac
rm -rf dist/stage
ls -la dist
