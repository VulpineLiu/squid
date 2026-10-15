#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""把 Windows 真机抓回来的原始 ANSI 流还原成画面,再判断它有没有花屏。

用法:
    python3 scripts/tui-frame-check.py /tmp/squid-win-tui.bin
    python3 scripts/tui-frame-check.py --selftest     # 判据自己的回归测试

为什么要脚本而不是让 cargo test 跑:花屏不长在数据里,长在"终端把这一串字节
摆成什么"里。crossterm 发出的是坐标 + 字形,谁收到这些字节谁才知道会不会错位,
而在 macOS/Linux 上没有任何一个终端是 Windows 控制台。所以流程反过来:
真机上的 ConPTY 收到什么,我们就原样存什么(scripts/win-tui-capture.sh),
回到本地用 pyte 这个终端仿真器把它重放成画面,再量画面。

判据本身在 geometry() 里,它只认结构不认内容:边框必须横平竖直。
"""
import re
import sys
import unicodedata

try:
    import pyte
except ImportError:
    sys.exit("需要 pyte: pip3 install pyte")

# 能合法出现在一行之末的只有竖线和拐角。横线(─═-)只配当"这行属于某个框"的线索
# —— 半帧里 ╰──── 刚画了一半,尾巴就是根横线,把它当右边框会冤枉一幅好画面。
BORDER = '│┃|+╭╮╰╯┌┐└┘╔╗╚╝'
ANCHOR = BORDER + '─═-'

# 一行至少要占这么多个格子才算"框的一行"。绘制中间态的残行(ratatui 刚把这行的
# 左边界写下去、正文还没画到)一定很短,rstrip() 之后更是短得离谱;而真正排错了的
# 行只会太长、不会太短 —— 溢出是把行推到行尾之外,不是把它缩短。所以短行一律不看,
# 半帧就再也冤枉不了一幅好画面。
MIN_ROW = 8

# 界面一起步就画二十几行。少于这个数的画面是程序还没跑起来(屏上只有一行标记),
# 拿"屏上没有边框"去判它们,是把等待判成故障。
PAINTED = 8


def cells(ch):
    """这个字符在终端里占几格。

    列号不能数 len():屏上一个汉字占两格,pyte 的 display 却只在串里留一个字符,
    数字符会把带中文的行少算几十列,一幅整齐的界面同样被读成"右边框散落四处"。
    """
    return 2 if unicodedata.east_asian_width(ch) in ('W', 'F') else 1


def column_of_right_border(line):
    """行里最后一个边框字符落在第几列(从 0 数)。"""
    col, edge = 0, -1
    for ch in line:
        if ch in BORDER:
            edge = col
        col += cells(ch)
    return edge


def border_columns(line):
    """行里每一个边框字符落在第几列(从 0 数)。

    只量"最右边那个边框字符"的列,会把并排的两个框读成一幅画:左栏在 31 列收口、
    面板在 140 列收口,一行之上两个收口点都存在,量到的永远是右边那个。要知道这一行
    到底属于哪个框,得把它全部的边框列都记下来。
    """
    col, out = 0, []
    for ch in line:
        if ch in BORDER and (not out or out[-1] != col):
            out.append(col)
        col += cells(ch)
    return out


def geometry(lines, cols):
    """量一帧,返回(问题列表, 给人看的说明, 这一帧值不值得判)。

    花屏只有两种走样:两张画叠在一起(窗口尺寸变了却没重画),或者行写过了行尾、
    被换行挤到下一行开头 —— 而"一列的字符被控制台收了两格价"正是这么坏掉的:
    ratatui 以为这行正好 100 列,终端却按 118 格排版。两种走样都会破坏同一件事:
    所有右边框落在同一列,且没有一行的右边框被挤到行尾之外。
    """
    problems, notes = [], []
    box_rows, strays, torn = [], [], []
    for n, line in enumerate(lines, 1):
        stripped = line.rstrip()
        if not stripped:
            continue
        # 哪些行算边框行:不能拿"行里最右边的边框字符"当右边框。边框在不同价格下
        # 是不同的字符 —— 一格收一笔的终端画 │,两格收一笔的终端退化成 | —— 而 |
        # 同时也是正文里的普通字符(路径、分隔符、页脚)。用 rindex 找它,会把正文
        # 里一个无辜的竖线当成第 76 列的"右边框",整齐的界面被判成花屏。
        # 真正贴着框的行,第一个字符就是边框字符:左边界贴着屏幕最左边。
        if stripped[0] in ANCHOR:
            box_rows.append((n, stripped))
            continue
        if any(ch in stripped for ch in '│┃|'):
            strays.append(n)
        # 一行正文压在边框行上的样子很好认:不是边框行,却以一长串横线收尾 ——
        # 那些横线是被 Clear 挖剩下的边框。toast 曾经画在卡片底边框那一行,于是
        # 屏上出现"功能菜单 — 上下键选择，回车返回 ----------╯"这种半是句子半是
        # 框的东西,用户看到的正是"重叠"。自己起手的边框行不算(上面已经 continue):
        # 带标题的 ╭  squid  ────╮ 本来就是这个样子。
        if stripped[-1] in BORDER and re.search(r'[-─═]{8,}', stripped):
            torn.append((n, stripped))

    edges, ragged, stubs = {}, [], 0
    for n, stripped in box_rows:
        width = sum(cells(ch) for ch in stripped)
        if width < MIN_ROW:
            stubs += 1
        elif stripped[-1] in BORDER:
            edges.setdefault(column_of_right_border(stripped), []).append(n)
        elif width >= cols:
            # 已经写到屏幕最右边,尾巴上却没有边框 —— 边框被挤到行尾之外了。
            ragged.append((n, stripped))
        else:
            stubs += 1

    if not edges and not ragged and not torn:
        return [], [], False   # 整帧没有可判的边框行:多半是绘制中间态
    if torn:
        for n, stripped in torn[:2]:
            notes.append(f"[frame ]   第 {n} 行正文压在边框上: {stripped[:26]!r}…{stripped[-12:]!r}")
        problems.append(f"有 {len(torn)} 行正文压在边框行上、把边框截成了一段横线 —— 重叠")

    hist = ", ".join(f"第{c + 1}列x{len(v)}" for c, v in sorted(edges.items())) or "无"
    judged = sum(len(v) for v in edges.values()) + len(ragged)
    notes.append(f"[frame ] 边框行 {judged} 行(跳过未画完 {stubs}) · 右边框所在列: {hist} · "
                 f"含竖线的正文行 {len(strays)} 行(不计入)")
    if ragged:
        for n, stripped in ragged[:2]:
            notes.append(f"[frame ]   第 {n} 行右边界缺失: ...{stripped[-20:]!r}")
        problems.append(f"有 {len(ragged)} 行左边框在、右边框却被挤没了 —— 行写过了行尾")
    # 右边框不止一列,有两种解释,只有一种是病。
    #
    # 界面天生就是好几个框并排画的:左栏收在第 31 列,面板一直画到第 140 列。
    # 于是"左栏比面板长"的那些行,行尾落在第 31 列 —— 这是合法布局里最常见的一种,
    # 而按"所有右边框必须落在同一列"去判,它每一拍都是花屏。
    # 合法的真正特征不是宽度相等,而是"截短":短行的每一个边框列,宽行上也都有。
    # 左栏行 {0,30} 的两条边,面板行 {0,30,31,139} 里都找得到 —— 它看起来就是那幅
    # 宽画被从右边切掉一截的样子,而切掉一截正是 ratatui 少画一个框时的行为。
    # 真叠帧坏在另一件事上:窄框(70 列)接在宽框(100 列)下面时,它的右边框落在
    # 第 69 列,而宽框那一行从来没有第 69 列。这一列是凭空冒出来的,只能来自另一幅画
    # —— 尺寸变了却没重画,新旧两版叠在同一屏上,就是用户说的"重叠"。
    widest = max(edges, default=-1)
    frame = set()
    for _, stripped in box_rows:
        if column_of_right_border(stripped) == widest:
            frame.update(border_columns(stripped))
    strangers = [n for n, stripped in box_rows
                 if any(c not in frame for c in border_columns(stripped))]
    # edges 为空 = 没有任何一行画出了右边框,那正是 ragged 在管的事,
    # 这里再插一句"叠帧"只是把同一个毛病换名再报一遍。
    if edges and strangers:
        for n in strangers[:2]:
            line = dict(box_rows)[n]
            extra = [c for c in border_columns(line) if c not in frame]
            notes.append(f"[frame ]   第 {n} 行有宽框上不存在边框列 {extra}: {line[:26]!r}…")
        problems.append(f"有 {len(strangers)} 行的边框列在最宽那一行上根本不存在"
                        " —— 屏上有两幅不同尺寸的画叠着")
    if edges:
        if widest + 1 > cols:
            problems.append(f"右边框在第 {widest + 1} 列,超出 {cols} 列")
        else:
            notes.append(f"[frame ] 画面宽 {widest + 1} 列(窗口 {cols} 列)")
    return problems, notes, True


def analyze(raw, echo=True):
    """重放整条流,量每一个画稳的画面,返回问题列表(空 = 没花屏)。"""
    reported = re.search(r'MARK_STARTED\s+(\d{1,4})\s*x\s*(\d{1,4})', raw)
    if reported:
        cols, rows = int(reported.group(1)), int(reported.group(2))
    else:
        cols, rows = 120, 30
        print("[size] !!! 流里没有 MARK_STARTED,退回 120x30 仿真(结论不可信)")
    print(f"[size] 控制台实际 {cols}x{rows}")
    # 代码页要印出来:同一个二进制在 65001 和 936 下可以是两幅画 —— 代码页决定
    # "没被问到"的字符按几格算,而用户那个窗口就站在 936 上。一帧是在哪个代码页上
    # 抓的,结论就只对那个代码页成立,所以这件事必须写在报告的第一行。
    codepage = re.search(r'MARK_CODEPAGE\s+(\d+)', raw)
    print(f"[size] 代码页 {codepage.group(1) if codepage else '未知(旧版抓帧脚本)'}")

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

    # 只对"画稳了"的画面下判据。crossterm 一格一格补 diff,采样切在中间就是半张画:
    # 半张画当然缺右边界,拿它判花屏是自己冤枉自己。半张画和整张画的区别不用猜 ——
    # 同一个画面被采到第二次,说明这次绘制已经结束、终端停在那儿了。
    step = 64
    frames = [tuple(render(raw[:cut])) for cut in range(step, len(raw) + step, step)]
    seen, stable = {}, []
    for frame in frames:
        if sum(1 for l in frame if l.strip()) < PAINTED:
            continue
        count = seen.get(frame, 0)
        seen[frame] = count + 1
        if count == 1:
            stable.append(frame)

    # 退出时 TUI 会擦掉整屏(crossterm 的 Clear + LeaveAlternateScreen),所以"最后
    # 一帧"必然是空的。给人看的那一帧取画稳的画面里内容最多的那个。
    best = list(max(stable, key=lambda f: sum(1 for l in f if l.strip()), default=()))
    if echo:
        print("\n".join(best))
        print()

    problems = []
    # 制表符没解码出来 = 字体/代码页问题;U+FFFD = 编码链路某一环把字节弄丢了。
    bad = [ch for ch in raw if ch == '\ufffd']
    if bad:
        at = raw.index('\ufffd')
        where = ('流中间 = 控制台画不出这个字形' if at < len(raw) - 24
                 else '流末尾 = ssh 断开切掉半帧')
        print(f"[decode] 第一个 U+FFFD 在偏移 {at}/{len(raw)}:{where}")
    screen_text = "".join(best)
    fbad = screen_text.count('\ufffd')
    print(f"[decode] {len(raw)} 字符 · 原始流 U+FFFD {len(bad)} 个 · 屏上 U+FFFD {fbad} 个 · "
          f"制表符 {'出现' if any(c in screen_text for c in '─│┌┐└┘') else '未出现'}")
    for n, line in enumerate(best, 1):
        if '\ufffd' in line:
            problems.append(f"第 {n} 行有画不出来的字形: {line!r}")
    cjk = [ch for ch in raw if 'CJK' in unicodedata.name(ch, '')]
    print(f"[decode] CJK 字符 {len(cjk)} 个")
    if 'panicked' in raw.lower():
        problems.append("输出里有 panic 字样")
    # 抓帧的壳子坏在程序之前。win-tui-run.ps1 一旦有语法错误,PowerShell 报完自己
    # 的错就退出,屏上一个边框都不会有 —— 那会被下面读成"界面没跑起来",再往下就是
    # 有人上真机去找一个根本不存在的排版 bug。这一条把案发现场直接摆出来:
    # 换行符或 BOM 被谁改坏过一次,就够了两次这种空跑的调试轮。
    parse = re.search(r'CategoryInfo\s*:\s*\(.*?\)[^\r\n]*', raw)
    if parse:
        problems.append("抓帧脚本自己没跑起来,远端 PowerShell 报的是它自己的错: "
                        f"{parse.group(0).strip()!r} —— 界面根本没被启动")
    if 'MARK_QUIT' not in raw:
        problems.append("没有 MARK_QUIT —— 界面没退,或根本没跑起来")

    checked, failures = 0, []
    for frame in stable:
        probs, notes, judgeable = geometry(list(frame), cols)
        if not judgeable:
            continue
        checked += 1
        if echo and (checked == 1 or probs and not failures):
            for note in notes:
                print(note)
        if probs:
            failures.append((checked, probs))
    if echo and checked:
        last = geometry(list(stable[-1]), cols)[1] if stable else []
        for note in last:
            print(note)
    print(f"[frame ] 采样 {len(frames)} 次 · 画稳的界面 {checked} 个全部量过 · "
          f"其中 {len(failures)} 个不合格")
    for index, probs in failures[:3]:
        for problem in probs:
            print(f"[frame ]   第 {index} 个画面: {problem}")
    problems.extend(p for _, probs in failures for p in probs)
    if not checked:
        problems.append("屏上从来没有一个画稳、又画出了边框的界面可以判 "
                        f"(至少 {PAINTED} 行、边框行不短于 {MIN_ROW} 格)")
    return problems


def selftest():
    """判据自己也要能被证明会抓花屏 —— 否则它只是在给坏消息开门。"""
    def card(width, height, fill='  尚未扫描', footer=()):
        top = '╭' + '─' * (width - 2) + '╮'
        mid = '│' + fill + ' ' * (width - 2 - sum(cells(c) for c in fill)) + '│'
        bottom = '╰' + '─' * (width - 2) + '╯'
        return [top] + [mid] * height + [bottom] + list(footer)

    def side_by_side(left, right, left_rows, right_rows):
        """左栏和面板并排,左栏比面板长 —— 用户那个 140 列窗口就是这个样子。"""
        top = '╭' + '─' * (left - 2) + '╮' + '╭' + '─' * (right - 2) + '╮'
        mid = ('│' + ' ' * (left - 2) + '│' + '│' + ' ' * (right - 2) + '│')
        bottom = '╰' + '─' * (left - 2) + '╯' + '╰' + '─' * (right - 2) + '╯'
        only = '│' + ' ' * (left - 2) + '│'   # 面板已经结束、只剩左栏的那些行
        return ([top] + [mid] * right_rows + [bottom]
                + [only] * (left_rows - right_rows))

    def stream(*screens, size='100x30'):
        """把若干画面写成一条流;每个画面写两遍,好让采样认出"画稳了"。"""
        out = [f'MARK_STARTED {size}\r\n']
        for screen in screens:
            body = '\x1b[H' + '\r\n'.join(screen)
            out.append(body * 2)
        out.append('\x1b[2J\r\nMARK_QUIT\r\n')
        return "".join(out)

    good = card(100, 10, footer=[' 语言 · zh（环境变量 SQUID_LANG）'] * 4)
    cases = [
        ("整齐的界面", stream(good), False),
        # 中文行:len() 只有 78 个字符,占格却是 100 —— 数 len() 的判据会误报。
        ("中文把 len 骗窄了", stream(card(100, 10, fill='  12.58 MB 移入回收站后才真正释放 删除方式')), False),
        # 两张画叠在一起:同一屏上出现了两种宽度的框。
        ("窄框和宽框叠着", stream(card(100, 10) + card(70, 6)), True),
        # 一列字符被收了两格价:行被推到行尾之外,右边框整个不见了。
        ("右边框被挤出行尾", stream(['│' + 'x' * 99] * 12), True),
        ("屏上根本没画框", stream(['  没有边框的一行'] * 12), True),
        # toast 画在卡片底边框那一行上留下的痕迹:句子 + 剩下的半截边框。
        ("正文压在边框上", stream(['╭' + '─' * 98 + '╮'] * 2
                                  + [' 功能菜单 — 上下键选择，回车返回   ' + '-' * 60 + '+']
                                  + ['│' + ' ' * 98 + '│'] * 8, ), True),
        # 左栏 31 列 + 面板到 140 列并排,左栏比面板长四行。
        # 这一条是判据自己欠的账:窗口一超过 104 列,"所有右边框必须同列"这句
        # 就不成立了 —— 左栏那些比面板长的行,行尾当然落在第 31 列。
        # 一幅整齐的界面因此每拍都被判成花屏。判据错了不该让真机再跑一遍,
        # 所以把它写成回归:以后谁再把"并排"判成"叠帧",这里就红。
        ("左栏比面板长", stream(side_by_side(31, 109, 14, 18), size='140x35'), False),
        # 而"窄框接在宽框下面"在 140 列窗口里同样得报:第 69 列是凭空冒出来的。
        ("140 列上窄框叠宽框", stream(card(100, 8) + card(70, 6), size='140x35'), True),
        # 抓帧脚本自己炸了:远端 PowerShell 吐出一段它自己的报错,界面上一个字都没画。
        # 这种流必须把"壳子坏了"单独说出来。否则"花屏"的罪名会扣在一个根本没启动的
        # 界面上 —— 我们真的被这个假故障骗着去改过一版绘制代码。
        ("抓帧脚本自己没跑起来",
         'MARK_STARTED 140x35\r\n'
         'CategoryInfo : (:) [], ParentContainsErrorRecordException\r\n'
         'FullyQualifiedErrorId : PositionalParameterNotFound\r\n', True),
    ]
    failed = 0
    for name, data, want_problems in cases:
        problems = analyze(data, echo=False)
        ok = bool(problems) == want_problems
        failed += 0 if ok else 1
        verdict = '按预期' if ok else '判错了'
        want = '该报问题' if want_problems else '该安静'
        print(f"[selftest] {name}: {verdict}(期望{want})"
              + (f" -> {problems}" if problems else ""))
    print(f"[selftest] {'RESULT: OK' if not failed else f'RESULT: {failed} 个用例判错了'}")
    return 1 if failed else 0


if __name__ == '__main__':
    if '--selftest' in sys.argv:
        sys.exit(selftest())
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    found = analyze(open(sys.argv[1], 'rb').read().decode('utf-8', 'replace'))
    print()
    for problem in found:
        print(f"[FAIL] {problem}")
    print(f"RESULT: {'FAIL' if found else 'OK'}")
    sys.exit(1 if found else 0)
