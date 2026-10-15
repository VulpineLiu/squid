#!/usr/bin/env bash
# 在 Windows 真机上把 TUI 抓成一帧纯文本,验证 ConPTY 上的绘制结果。
#
#   ./scripts/win-tui-capture.sh              # 抓 state 界面
#   ./scripts/win-tui-capture.sh --lang zh    # 中文界面(验证 CJK 宽度)
#   TUI_KEYS="s q" ./scripts/win-tui-capture.sh   # 先扫描再退出(验证有数据的排版)
#
# 为什么不能用 scripts/tui_smoke.py:那个脚本自己 forkpty(),是 POSIX 专属。
# 真机上没有 pty 可以 fork,只能反过来:让 sshd 用 CreatePseudoConsole 分配一个
# ConPTY(Windows OpenSSH 在 1809+ 上对 -tt 就是这么做的),crossterm 因此走真正的
# Windows 控制台后端,而不是退化到没验证过的 ANSI 假设。原始 ANSI 落盘后拿回本地,
# 用 pyte 这个终端仿真器解码成文本 —— 看到的才是用户会看到的那一帧。
set -uo pipefail

cd "$(dirname "$0")/.."

WIN_HOST="${WIN_HOST:-KANGWJ@192.168.2.92}"
WIN_KEY="${WIN_KEY:-$HOME/.ssh/id_ed25519_windows}"
BIN="${WIN_BIN:-C:\dev\target\release\squid.exe}"
OUT="${TUI_OUT:-/tmp/squid-win-tui.bin}"
COLS="${TUI_COLS:-100}"
ROWS="${TUI_ROWS:-30}"
DELAY="${TUI_DELAY:-8}"

# 尺寸:ConPTY 的窗口大小由 pty 请求决定,ssh 会把它自己的 winsize 传过去,
# 所以这里用 stty 定一个已知的尺寸,pyte 才能按同样的宽度重排。
printf '\033[1;36m==>\033[0m 抓取 %s (%sx%s, %ss)\n' "$BIN" "$COLS" "$ROWS" "$DELAY"
# 按键序列:默认只按 q 退出。想验证"有数据时"的排版,用 TUI_KEYS="s q"
# 先触发扫描再退出 —— 空面板和有数据的面板排的是两种版。
TUI_KEYS="${TUI_KEYS:-q}"
TUI_HOLD="${TUI_HOLD:-6}"
{ sleep "$DELAY"; for key in $TUI_KEYS; do printf '%s' "$key"; sleep "$TUI_HOLD"; done; sleep 2; } |
  ssh -tt -i "$WIN_KEY" -o BatchMode=yes "$WIN_HOST" \
      "chcp 65001 >nul & mode con cols=$COLS lines=$ROWS >nul 2>&1& $BIN state $* 2>&1" \
      > "$OUT" 2>&1
rc=$?
printf '\033[1;36m==>\033[0m 原始 ANSI 已存 %s (%s 字节, ssh rc=%s)\n' "$OUT" "$(wc -c < "$OUT" | tr -d ' ')" "$rc"

python3 - "$OUT" "$COLS" "$ROWS" <<'PYEOF'
import sys, unicodedata
raw = open(sys.argv[1], 'rb').read().decode('utf-8', 'replace')
cols, rows = int(sys.argv[2]), int(sys.argv[3])
try:
    import pyte
except ImportError:
    sys.exit("需要 pyte: pip3 install pyte")

# 退出时 TUI 会擦掉整屏(crossterm 的 Clear + LeaveAlternateScreen),所以
# "最后一帧"必然是空的。逐块喂给仿真器,留下内容最多的那一帧 —— 那才是用户
# 按 q 之前看到的界面。
def render(data):
    screen = pyte.Screen(cols, rows)
    pyte.Stream(screen).feed(data)
    try:
        lines = [l.rstrip() for l in screen.display]
    except IndexError:
        # 这一帧正好喂到被截断的半个字符上,pyte 量宽度时会越界。跳过它。
        return []
    while lines and not lines[-1]:
        lines.pop()
    return lines

best = []
step = 64
for cut in range(step, len(raw) + step, step):
    lines = render(raw[:cut])
    filled = sum(1 for l in lines if l.strip())
    if filled >= sum(1 for l in best if l.strip()):
        best = lines
print("\n".join(best))
print()
# 制表符没解码出来 = 字体/代码页问题;U+FFFD = 编码链路某一环把字节弄丢了。
bad = [ch for ch in raw if ch == '\ufffd']
if bad:
    at = raw.index('\ufffd')
    where = '流中间 = 控制台画不出这个字形' if at < len(raw) - 24 else '流末尾 = ssh 断开切掉半帧'
    print(f"[decode] 第一个 U+FFFD 在偏移 {at}/{len(raw)}:{where}")
frame = "".join(best)
fbad = frame.count('\ufffd')
print(f"[decode] {len(raw)} 字符 · 原始流 U+FFFD {len(bad)} 个 · 屏上 U+FFFD {fbad} 个 · 制表符 {'出现' if any(c in frame for c in '─│┌┐└┘') else '未出现'}")
if fbad:
    for n, l in enumerate(best, 1):
        if '\ufffd' in l:
            print(f"[decode] !!! 第 {n} 行有画不出来的字形: {l!r}")
cjk = [ch for ch in raw if 'CJK' in unicodedata.name(ch, '')]
print(f"[decode] CJK 字符 {len(cjk)} 个")
if 'panicked' in raw.lower():
    print("[decode] !!! 输出里有 panic 字样")
PYEOF
