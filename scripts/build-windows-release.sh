#!/bin/bash
# Build the Windows x64 installer and refresh artifacts/SHA256SUMS.txt.
#
#   scripts/build-windows-release.sh [--skip-build]
#
# On macOS this cross-compiles with cargo-xwin. On Windows (Git Bash/MSYS2) it
# uses the native MSVC toolchain. Both paths feed the same NSIS installer.
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT="$PWD"
VERSION=$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
TARGET=x86_64-pc-windows-msvc
INSTALLER="artifacts/Squid-Setup-$VERSION-win-x64.exe"
NOTES="artifacts/README-Windows-x64.txt"
HOST=$(uname -s)

if [ "$HOST" = "Darwin" ]; then
  export PATH="$HOME/.cargo/bin:$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:/opt/homebrew/opt/llvm/bin:$PATH"
  export RUSTUP_TOOLCHAIN=stable-aarch64-apple-darwin
  export RUSTC="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc"
fi

SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1 ;;
    *) echo "usage: $0 [--skip-build]" >&2; exit 2 ;;
  esac
  shift
done

if [ -z "${SQUID_UPDATE_PUBKEY:-}" ] && [ -n "${SQUID_RELEASE_KEY:-}" ]; then
  SQUID_UPDATE_PUBKEY=$(python3 scripts/release-key.py pubkey "${SQUID_RELEASE_KEY%.ed25519}.pub")
  export SQUID_UPDATE_PUBKEY
fi
if [ -n "${SQUID_UPDATE_PUBKEY:-}" ]; then
  echo "== release key baked in: ${SQUID_UPDATE_PUBKEY:0:12}… =="
else
  echo "== no release key: this build will report the feed as unsigned =="
fi

if [ "$SKIP_BUILD" != "1" ]; then
  echo "== frontend (Windows target: chrome105) =="
  TAURI_ENV_PLATFORM=windows pnpm build

  echo "== rust build ($TARGET) =="
  CROSS_LOG=$(mktemp /tmp/squid-win-build.XXXXXX)
  if [ "$HOST" = "Darwin" ]; then
    if ! (
      cd src-tauri
      cargo xwin build --target "$TARGET" --release \
        --features tauri/custom-protocol >"$CROSS_LOG" 2>&1
    ); then
      sed -n '1,240p' "$CROSS_LOG"
      exit 1
    fi
  else
    if ! (
      cd src-tauri
      cargo build --target "$TARGET" --release \
        --features tauri/custom-protocol >"$CROSS_LOG" 2>&1
    ); then
      sed -n '1,240p' "$CROSS_LOG"
      exit 1
    fi
  fi
  grep -Ev "^warning:|^\s+\||^\s+=|LNK4099|failed to load|^$|^\s+-->|^note:" \
    "$CROSS_LOG" || true
fi

EXE="src-tauri/target/$TARGET/release/squid-desktop.exe"
test -f "$EXE"

echo "== PE machine check (x64) =="
python3 - "$EXE" <<'PY'
import pathlib, struct, sys
exe = pathlib.Path(sys.argv[1]).read_bytes()
machine = struct.unpack_from("<H", exe, struct.unpack_from("<I", exe, 0x3C)[0] + 4)[0]
if machine != 0x8664:
    sys.exit(f"{sys.argv[1]} is PE machine 0x{machine:04x}, expected 0x8664")
print("pe machine ok: 0x8664")
PY

echo "== verify the frontend is embedded =="
python3 - "$EXE" <<'PY'
import pathlib, sys
exe = pathlib.Path(sys.argv[1]).read_bytes()
assets = pathlib.Path("dist/assets")
missing = [p.name for p in assets.iterdir() if p.name.encode() not in exe]
if b"index.html" not in exe:
    missing.append("index.html")
asset_text = "".join(
    p.read_text(encoding="utf-8")
    for p in assets.iterdir()
    if p.suffix in {".js", ".css", ".html"}
)
for marker in ("智能扫描", "扫描完成", "自动检查更新"):
    if marker not in asset_text:
        missing.append(marker)
if missing:
    sys.exit(f"frontend not embedded, missing: {missing}")
print(f"embedded ok: {len(exe):,} byte exe carries the current dist/assets")
PY

echo "== windows notes (x64) =="
STAGE=$(mktemp -d /tmp/squid-windows-release.XXXXXX)
python3 - "$STAGE" "$VERSION" <<'PY'
import pathlib, sys
stage = pathlib.Path(sys.argv[1])
version = sys.argv[2]
text = f"""Squid {version} - Windows x64

系统要求
--------
- Windows 10 或 Windows 11（x64；ARM64 设备通过系统兼容层运行）。
- Microsoft Edge WebView2 Runtime。Windows 10/11 通常已内置；如果 Squid
  启动后窗口空白，请从微软官方页面安装 WebView2 Evergreen Runtime：
  https://developer.microsoft.com/microsoft-edge/webview2/
- 安装与运行都不需要管理员权限。安装位置为
  %LOCALAPPDATA%\\Programs\\Squid。

安装
----
1. 运行 Squid-Setup-{version}-win-x64.exe。
2. 这是未签名构建。若 SmartScreen 显示警告，请选择“更多信息” > “仍要运行”。
3. 卸载请在 Windows“设置 > 应用”中移除 Squid。

更新
----
Squid 使用与 nex_shift 一致的 releases 分支更新通道。更新包会先校验
文件大小与 SHA256，再在退出后运行同一个安装程序。若构建时写入发布公钥，
还会验证清单的 Ed25519 签名。

校验
----
PowerShell：
  Get-FileHash .\\Squid-Setup-{version}-win-x64.exe -Algorithm SHA256
"""
(stage / "README-Windows.txt").write_bytes(text.replace("\n", "\r\n").encode("utf-8"))
PY

echo "== NSIS installer (x64) =="
mkdir -p artifacts
SIZE_KB=$(( ($(stat -f%z "$EXE" 2>/dev/null || stat -c%s "$EXE") + 1023) / 1024 ))
makensis -V2 \
  -DSQUID_EXE="$ROOT/$EXE" \
  -DLICENSE_FILE="$ROOT/LICENSE" \
  -DREADME_FILE="$STAGE/README-Windows.txt" \
  -DICONS_DIR="$ROOT/src-tauri/icons" \
  -DOUT_FILE="$ROOT/$INSTALLER" \
  -DVERSION="$VERSION" \
  -DEST_SIZE_KB="$SIZE_KB" \
  scripts/windows-installer.nsi
mv "$STAGE/README-Windows.txt" "$NOTES"
rm -rf "$STAGE"

echo "== checksums =="
python3 - "$VERSION" <<'PY'
import hashlib, pathlib, sys
version = sys.argv[1]
artifacts = pathlib.Path("artifacts")
preferred = [
    f"Squid-{version}-macOS-universal.dmg",
    f"Squid-Setup-{version}-win-x64.exe",
    "README-Windows-x64.txt",
]
names = [name for name in preferred if (artifacts / name).is_file()]
lines = [
    f"{hashlib.sha256((artifacts / name).read_bytes()).hexdigest()}  {name}"
    for name in names
]
(artifacts / "SHA256SUMS.txt").write_text("".join(line + "\n" for line in lines), encoding="utf-8")
print("\n".join(lines))
PY

echo "== result =="
ls -lh artifacts/
