#!/bin/bash
# Build the universal macOS DMG and copy it to the versioned release name.
#
#   scripts/build-macos-release.sh [--skip-build]
#
# Produces artifacts/Squid-<version>-macOS-universal.dmg.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != "Darwin" ]; then
  echo "macOS release must be built on macOS." >&2
  exit 1
fi

export PATH="$HOME/.cargo/bin:$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
export RUSTUP_TOOLCHAIN=stable-aarch64-apple-darwin

if [ -z "${SQUID_UPDATE_PUBKEY:-}" ] && [ -n "${SQUID_RELEASE_KEY:-}" ]; then
  SQUID_UPDATE_PUBKEY=$(python3 scripts/release-key.py pubkey "${SQUID_RELEASE_KEY%.ed25519}.pub")
  export SQUID_UPDATE_PUBKEY
fi

SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1 ;;
    *) echo "usage: $0 [--skip-build]" >&2; exit 2 ;;
  esac
  shift
done

VERSION=$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
BUNDLE_DIR="src-tauri/target/universal-apple-darwin/release/bundle"
APP="$BUNDLE_DIR/macos/Squid.app"
OUTPUT="artifacts/Squid-$VERSION-macOS-universal.dmg"

rustup target add x86_64-apple-darwin aarch64-apple-darwin

if [ "$SKIP_BUILD" != "1" ]; then
  if [ -n "${SQUID_UPDATE_PUBKEY:-}" ]; then
    echo "== release key baked in: ${SQUID_UPDATE_PUBKEY:0:12}… =="
  else
    echo "== no release key: this build will report the feed as unsigned =="
  fi
  pnpm tauri build --target universal-apple-darwin --bundles app,dmg
fi

test -d "$APP"
DMG=$(find "$BUNDLE_DIR/dmg" -maxdepth 1 -type f -name '*.dmg' -print -quit)
test -n "$DMG"

codesign --verify --deep --strict "$APP"
hdiutil verify "$DMG" >/dev/null
mkdir -p artifacts
cp -f "$DMG" "$OUTPUT"
hdiutil verify "$OUTPUT" >/dev/null

echo "== macOS release =="
ls -lh "$OUTPUT"
