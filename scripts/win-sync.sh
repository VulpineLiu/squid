#!/usr/bin/env bash
# 把当前 HEAD 同步到局域网里的 Windows 真机(不走 git 认证,直接 tarball)。
#
#   ./scripts/win-sync.sh                 # 同步 + 在远端跑 check(win-check.ps1)
#   ./scripts/win-sync.sh --no-verify     # 只同步,不跑校验
#
# 依赖: scripts/win-check.ps1 也会一起上传,所以远端永远是最新的那份。
set -euo pipefail

cd "$(dirname "$0")/.."

WIN_HOST="${WIN_HOST:-KANGWJ@192.168.2.92}"
WIN_KEY="${WIN_KEY:-$HOME/.ssh/id_ed25519_windows}"
WIN_DIR="${WIN_DIR:-C:/dev/nex_squid}"
REMOTE_TMP='C:\Users\kangwj\nex_squid.tgz'

ssh_run() { ssh -i "$WIN_KEY" -o BatchMode=yes "$WIN_HOST" "$1"; }
scp_to()  { scp -i "$WIN_KEY" -o BatchMode=yes -q "$1" "$WIN_HOST:$2"; }

TARBALL="$(mktemp -t nexsquid).tgz"
trap 'rm -f "$TARBALL"' EXIT

printf '\033[1;36m==>\033[0m 打包 HEAD (%s)\n' "$(git rev-parse --short HEAD)"
git archive --format=tar HEAD | gzip > "$TARBALL"
scp_to "$TARBALL" "$REMOTE_TMP"

printf '\033[1;36m==>\033[0m 解压到 %s\n' "$WIN_DIR"
# 只覆盖源码:target/ 和 .cargo/ 不在 tarball 里,远端构建缓存得以保留。
ssh_run "mkdir C:\dev\nex_squid 2>nul & tar -xzf $REMOTE_TMP -C $WIN_DIR && echo extracted"

scp_to scripts/win-check.ps1 'C:\Users\kangwj\win-check.ps1'
scp_to scripts/win-hwcheck.ps1 'C:\Users\kangwj\win-hwcheck.ps1'

if [ "${1:-}" = "--no-verify" ]; then exit 0; fi

printf '\033[1;36m==>\033[0m 远端校验 (test / clippy / build)\n'
# chcp 65001:否则中文输出在 GBK 控制台下是乱码,日志没法看。
ssh_run "powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\kangwj\win-check.ps1"
