#!/bin/sh
# Builds the vitrail native library (Blitz: HTML + CSS without a browser) for every platform the game ships on, and puts
# it into the Unity package (which the game uses) and the lab project. The editor picks a new build up by itself (WebNative.Refresh).
#   macOS: universal (Apple Silicon + Intel), stripped    Windows: x86_64, cross-compiled with mingw-w64, stripped
# Usage: native/build.sh [mac|win]   (both when no argument)
set -e
. "$HOME/.cargo/env"
cd "$(dirname "$0")/blitz"
PKG="$HOME/projects/vitrail-ui/package/Plugins"
LAB="$HOME/projects/vitrail-ui/unity/Assets/Vitrail/Plugins"
want() { [ -z "$1" ] || [ "$1" = "$2" ]; }
# Write to a temp name, then move: the editor never sees a half-written file.
put() { cp "$1" "$2.tmp" && mv -f "$2.tmp" "$2"; }

if want "$1" mac; then
  cargo build --release -p vitrail --target aarch64-apple-darwin
  cargo build --release -p vitrail --target x86_64-apple-darwin
  lipo -create target/aarch64-apple-darwin/release/libvitrail.dylib target/x86_64-apple-darwin/release/libvitrail.dylib -output target/libvitrail-universal.dylib
  strip -x target/libvitrail-universal.dylib
  put target/libvitrail-universal.dylib "$PKG/macOS/libvitrail.dylib"
  put target/libvitrail-universal.dylib "$LAB/macOS/libvitrail.dylib"
  echo "mac: $(du -h target/libvitrail-universal.dylib | cut -f1) universal"
fi
if want "$1" win; then
  cargo build --release -p vitrail --target x86_64-pc-windows-gnu
  x86_64-w64-mingw32-strip -o target/vitrail-stripped.dll target/x86_64-pc-windows-gnu/release/vitrail.dll
  put target/vitrail-stripped.dll "$PKG/Windows/x86_64/vitrail.dll"
  put target/vitrail-stripped.dll "$LAB/x86_64/vitrail.dll"
  echo "win: $(du -h target/vitrail-stripped.dll | cut -f1)"
fi
