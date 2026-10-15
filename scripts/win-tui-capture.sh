#!/usr/bin/env bash
# 在 Windows 真机上把 TUI 抓成一帧纯文本,验证 ConPTY 上的绘制结果。
#
#   ./scripts/win-tui-capture.sh                  # 抓 state 界面
#   ./scripts/win-tui-capture.sh --lang zh        # 中文界面(验证 CJK 宽度)
#   TUI_KEYS="5   q" ./scripts/win-tui-capture.sh # 先切面板再退出
#   TUI_COLS=140 TUI_ROWS=45 ./scripts/win-tui-capture.sh
#   TUI_CELLS=2 ./scripts/win-tui-capture.sh  # 强制按"一列字符收两格"画(ASCII 边框)
#   TUI_REUSE=1 TUI_OUT=/tmp/x.bin ./scripts/win-tui-capture.sh   # 只重算旧帧,不连真机
#
# 为什么不能用 scripts/tui_smoke.py:那个脚本自己 forkpty(),是 POSIX 专属。
# 真机上没有 pty 可以 fork,只能反过来:让 sshd 用 CreatePseudoConsole 分配一个
# ConPTY(Windows OpenSSH 在 1809+ 上对 -tt 就是这么做的),crossterm 因此走真正的
# Windows 控制台后端,而不是退化到没验证过的 ANSI 假设。原始 ANSI 落盘后拿回本地,
# 用 pyte 这个终端仿真器解码成文本 —— 看到的才是用户会看到的那一帧。
#
# 为什么尺寸要由远端自己报:实测 ConPTY 不理 ssh 客户端的 window-change,
# `mode con cols=X lines=Y` 也会静默失败(返回 0,什么都没改)。所以尺寸由
# scripts/win-tui-run.ps1 用 [Console]::WindowWidth 改(SetConsoleWindowInfo,真的有效),
# 再把改完的真实尺寸打印回来。pyte 按这个事实仿真,而不是按我们的请求。
set -uo pipefail

cd "$(dirname "$0")/.."

WIN_HOST="${WIN_HOST:-KANGWJ@192.168.2.92}"
WIN_KEY="${WIN_KEY:-$HOME/.ssh/id_ed25519_windows}"
BIN="${WIN_BIN:-C:\dev\target\release\squid.exe}"
RUNNER='C:\Users\kangwj\win-tui-run.ps1'
OUT="${TUI_OUT:-/tmp/squid-win-tui.bin}"
COLS="${TUI_COLS:-100}"
ROWS="${TUI_ROWS:-30}"
DELAY="${TUI_DELAY:-8}"
# TUI_CELLS=2 让程序按两格的价画(边框退成 |-+),=1 按一格。远端是 ConPTY,
# 探针永远量到 1,所以想看 ASCII 那一版只能靠 SQUID_SYMBOL_CELLS 强制。
CELLS="${TUI_CELLS:-}"
SETUP=""
if [ -n "$CELLS" ]; then SETUP="set SQUID_SYMBOL_CELLS=$CELLS&& "; fi

# TUI_REUSE=1:跳过抓帧,直接重新分析 $OUT 里已有的原始 ANSI 流。
# 改判据的时候用 —— 判据写错不该逼着真机再跑一遍。
if [ "${TUI_REUSE:-}" = 1 ]; then
  [ -s "$OUT" ] || { echo "TUI_REUSE=1 但 $OUT 不存在/为空"; exit 1; }
  printf '\033[1;36m==>\033[0m 复用 %s (%s 字节),不连真机\n' "$OUT" "$(wc -c < "$OUT" | tr -d ' ')"
else
printf '\033[1;36m==>\033[0m 抓取 %s (请求 %sx%s, %ss)\n' "$BIN" "$COLS" "$ROWS" "$DELAY"
scp -i "$WIN_KEY" -o BatchMode=yes -q scripts/win-tui-run.ps1 "$WIN_HOST:$RUNNER" || exit 1
# 按键序列:默认只按 q 退出。想验证"有数据时"的排版,先按键进入面板再退出 ——
# 空面板和有数据的面板排的是两种版。
TUI_KEYS="${TUI_KEYS:-q}"
TUI_HOLD="${TUI_HOLD:-6}"
{ sleep "$DELAY"; for key in $TUI_KEYS; do printf '%s' "$key"; sleep "$TUI_HOLD"; done; sleep 2; } |
  ssh -tt -i "$WIN_KEY" -o BatchMode=yes "$WIN_HOST" \
      "${SETUP}powershell" -NoProfile -ExecutionPolicy Bypass -File "$RUNNER" \
      "$COLS" "$ROWS" "$BIN" state "$@" \
      > "$OUT" 2>&1
rc=$?
printf '\033[1;36m==>\033[0m 原始 ANSI 已存 %s (%s 字节, ssh rc=%s)\n' "$OUT" "$(wc -c < "$OUT" | tr -d ' ')" "$rc"
fi

python3 scripts/tui-frame-check.py "$OUT"
