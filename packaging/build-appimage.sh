#!/bin/bash
# Builds CouchTube-<arch>.AppImage in the repository root. Needs cargo, npm, appimagetool, rsvg-convert.
set -euo pipefail
cd "$(dirname "$0")/.."

(cd frontend && npm ci && npm run build)
# Keep the build machine's home and project paths out of the binary.
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=~ --remap-path-prefix=$PWD=." cargo build --release

APPDIR=target/AppDir
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/couchtube"
install -m755 target/release/couchtube "$APPDIR/usr/bin/couchtube"
cp -r frontend/dist "$APPDIR/usr/share/couchtube/frontend"
mkdir -p "$APPDIR/usr/share/couchtube/config"
cp -r config/mpv "$APPDIR/usr/share/couchtube/config/mpv"
install -m755 packaging/AppRun "$APPDIR/AppRun"
install -m644 packaging/couchtube.desktop "$APPDIR/couchtube.desktop"
rsvg-convert -w 256 -h 256 frontend/public/favicon.svg -o "$APPDIR/couchtube.png"

ARCH="$(uname -m)" appimagetool "$APPDIR" "CouchTube-$(uname -m).AppImage"
