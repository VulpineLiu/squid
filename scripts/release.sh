#!/usr/bin/env bash
# 打一个可发布的 squid 包：macOS 通用二进制 + tar.gz + sha256。
#
#   ./scripts/release.sh                 # arm64 + x86_64,产物在 dist/
#   ARCHES="aarch64-apple-darwin" ./scripts/release.sh   # 只出当前芯片的包
#
# 为什么不是 .dmg:TUI/CLI 没有 .app bundle,拖进 Applications 的那套东西
# (图标、LaunchServices、Gatekeeper 授权)对它毫无意义,反而多一次「已损坏,
# 无法打开」的隔离属性摩擦。一个 tar.gz + 一行 curl 安装脚本对终端程序更诚实,
# 也是 cargo-binstall / Homebrew 期望的形状。
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
NAME=squid
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
ARCHES=${ARCHES:-"aarch64-apple-darwin x86_64-apple-darwin"}
DIST="$ROOT/dist"
SLUG="$NAME-$VERSION-universal-apple-darwin"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

command -v cargo >/dev/null || die "cargo 不在 PATH 里(source ~/.cargo/env)"
[ "$(uname)" = Darwin ] || die "通用二进制只在 macOS 上打"

BINARIES=()
for target in $ARCHES; do
  if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    say "rustup target add $target"
    rustup target add "$target"
  fi
  say "cargo build --release --target $target"
  cargo build --release --target "$target" -p squid-cli
  BINARIES+=("target/$target/release/$NAME")
done

mkdir -p "$DIST"
BIN="$DIST/$NAME"
if [ "${#BINARIES[@]}" -gt 1 ]; then
  say "lipo -create (${BINARIES[*]})"
  lipo -create "${BINARIES[@]}" -output "$BIN"
else
  cp "${BINARIES[0]}" "$BIN"
  SLUG="$NAME-$VERSION-$(basename "$(dirname "$(dirname "${BINARIES[0]}")")")"
fi
chmod 0755 "$BIN"

# arm64 要求所有可执行文件至少有一个签名。ad-hoc 签名对本机足够;要发给别人
# 需要 Developer ID + 公证(notarytool),见 docs 里的说明。
say "codesign --sign - (ad-hoc)"
codesign --force --sign - --timestamp=none "$BIN" >/dev/null

say "squid --version: $("$BIN" --version)"
lipo -archs "$BIN" | sed 's/^/    architectures: /'

TARBALL="$SLUG.tar.gz"
STAGING="$(mktemp -d)"
mkdir -p "$STAGING/$SLUG"
cp "$BIN" "$STAGING/$SLUG/"
cp LICENSE README.md "$STAGING/$SLUG/" 2>/dev/null || true
tar -C "$STAGING" -czf "$DIST/$TARBALL" "$SLUG"
rm -rf "$STAGING"

( cd "$DIST" && shasum -a 256 "$TARBALL" > "$TARBALL.sha256" )
say "产物"
ls -lh "$DIST/$TARBALL" "$DIST/$TARBALL.sha256"
cat "$DIST/$TARBALL.sha256"
printf '\n把这个 sha256 填进 packaging/homebrew/squid.rb,然后:\n'
printf '  gh release create v%s dist/%s dist/%s.sha256\n' "$VERSION" "$TARBALL" "$TARBALL"
