#!/usr/bin/env python3
"""Drive the squid TUI through a real pty and assert on what the screen shows.

The unit tests in `squid-tui` can prove that `handle_key` mutates the right
field; they cannot prove that the user *sees* it. Every bug this script guards
against lived in that gap: a key that moved a cursor no renderer drew, an
interface that kept showing a finished job's progress bar, a palette that asked
a terminal for 24-bit colour it cannot parse, and an `esc` that was swallowed
by the very level it was supposed to leave.

Scenarios:

    menu        the interface *opens* on the six functions; the keyboard model
    mono        the same model with NO_COLOR set (the cursor inverts instead)
    colors      the truecolour probe, the 256-colour fallback, the grey cursor
                bar that palette buys instead of an accent, and `SQUID_TRUECOLOR`
    escape      `esc` walks back exactly one level, in every level there is
    uninstall   two apps back to back - the second must not be refused
    all         every scenario (default)

Requires `pyte` (`python3 -m pip install --user pyte`). Run it against a built
binary:

    cargo build --release
    python3 scripts/tui_smoke.py all target/release/squid

The app scenarios make their own throwaway bundles in `~/Applications` and
delete them again; they never touch a real application.
"""

import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import fcntl
import termios

try:
    import pyte
except ImportError:  # pragma: no cover - the message is the point
    sys.exit("pyte is required: python3 -m pip install --user pyte")

KEYS = {
    "Enter": "\r", "Esc": "\x1b", "Tab": "\t", "BS": "\x7f", "Space": " ",
    "Up": "\x1b[A", "Down": "\x1b[B", "Right": "\x1b[C", "Left": "\x1b[D",
    "SUp": "\x1b[1;2A", "SDown": "\x1b[1;2B",
    "MUp": "\x1b[1;3A", "MDown": "\x1b[1;3B", "MLeft": "\x1b[1;3D", "MRight": "\x1b[1;3C",
    "CUp": "\x1b[1;5A", "CDown": "\x1b[1;5B",
    "PgDn": "\x1b[6~", "PgUp": "\x1b[5~", "Home": "\x1b[H", "End": "\x1b[F",
}

CONFIG = 'delete_mode = "permanent"\nlanguage = "zh"\n'

class TUI:
    """A squid process attached to a pty, plus the screen it has painted."""

    def __init__(self, argv, env=None, cols=112, rows=32, term="xterm-256color"):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        e = dict(os.environ)
        # A CI shell often exports NO_COLOR; colour, and the colourless cursor
        # that replaces it, are what some of these scenarios are about.
        for key in ("NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE", "FORCE_COLOR"):
            e.pop(key, None)
        e.update({"TERM": term, "LANG": "zh_CN.UTF-8", "COLORTERM": "", "TERM_PROGRAM": "ghostty"})
        if env:
            e.update(env)
        self.proc = subprocess.Popen(argv, stdin=slave, stdout=slave, stderr=slave,
                                     env=e, close_fds=True)
        os.close(slave)
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.raw = bytearray()
        self.cols, self.rows = cols, rows
        self.alive = True

    def pump(self, until=0.0):
        end = time.time() + until
        while True:
            r, _, _ = select.select([self.master], [], [], 0.05)
            if r:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    self.alive = False
                    return
                if not data:
                    self.alive = False
                    return
                self.raw += data
                self.stream.feed(data)
            if time.time() >= end:
                return

    def send(self, *tokens, delay=0.12):
        for tok in tokens:
            os.write(self.master, KEYS.get(tok, tok).encode())
            self.pump(delay)

    def text(self):
        return "\n".join(line.rstrip() for line in self.screen.display)

    def show(self, label=""):
        print(f"\n===== {label} =====")
        print(self.text())
        print("===== end =====")

    def wait(self, pattern, timeout=25.0, label=""):
        rx = re.compile(pattern)
        end = time.time() + timeout
        while time.time() < end:
            self.pump(0.2)
            if rx.search(self.text()):
                return True
        print(f"[MISS] pattern {pattern!r} after {timeout}s {label}")
        self.show(f"miss {pattern}")
        return False

    def refute(self, pattern, window=1.5, label=""):
        """Assert the pattern does NOT appear during `window` seconds."""
        rx = re.compile(pattern)
        end = time.time() + window
        while time.time() < end:
            self.pump(0.2)
            m = rx.search(self.text())
            if m:
                print(f"[FAIL] forbidden pattern {pattern!r} matched {m.group(0)!r} {label}")
                self.show(f"forbidden {pattern}")
                return False
        return True

    def snapshot(self):
        """(text, style) per row. Style is what a cursor paints, not the glyphs."""
        texts, styles = [], []
        for y in range(self.rows):
            line = self.screen.buffer[y]
            cells = [line[x] for x in range(self.cols)]
            texts.append("".join(c.data or " " for c in cells).rstrip())
            styles.append(tuple((c.bg, c.fg, c.bold, c.reverse) for c in cells))
        return texts, styles

    def moved_rows(self, before, after):
        """Rows that changed colour while their text stayed put — the cursor."""
        bt, bs = before
        at, as_ = after
        return [y for y in range(self.rows) if bt[y] == at[y] and bs[y] != as_[y]]

    def quit(self):
        try:
            self.send("q", delay=0.4)
        except Exception:
            pass
        self.proc.terminate()
        try:
            self.proc.wait(3)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        os.close(self.master)


def sandbox():
    """Keep a smoke run out of the user's real settings and ledger."""
    cfg = tempfile.mkdtemp(prefix="squid-smoke-cfg-")
    state = tempfile.mkdtemp(prefix="squid-smoke-state-")
    open(os.path.join(cfg, "config.toml"), "w").write(CONFIG)
    return {"SQUID_CONFIG_DIR": cfg, "SQUID_STATE_DIR": state}


def make_app(name, bundle_id, size_kb=64):
    """A throwaway bundle with the leftovers squid is expected to find."""
    home = os.path.expanduser("~")
    app = os.path.join(home, "Applications", name + ".app", "Contents")
    os.makedirs(os.path.join(app, "MacOS"), exist_ok=True)
    with open(os.path.join(app, "Info.plist"), "w") as f:
        f.write(f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{bundle_id}</string>
<key>CFBundleName</key><string>{name}</string>
<key>CFBundleExecutable</key><string>{name}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
</dict></plist>
""")
    with open(os.path.join(app, "MacOS", name), "w") as f:
        f.write("#!/bin/sh\nexit 0\n")
    os.chmod(os.path.join(app, "MacOS", name), 0o755)
    for folder in ("Application Support", "Caches"):
        path = os.path.join(home, "Library", folder, bundle_id)
        os.makedirs(path, exist_ok=True)
        with open(os.path.join(path, "blob"), "wb") as f:
            f.write(os.urandom(size_kb * 1024))
    open(os.path.join(home, "Library", "Preferences", bundle_id + ".plist"), "w").write("{}")



# ------------------------------------------------------------- what is on screen

def rgb(colour):
    """pyte reports a cell colour as hex text, a palette name, or `default`."""
    if colour and re.fullmatch(r"[0-9a-f]{6}", colour):
        return tuple(int(colour[i:i + 2], 16) for i in (0, 2, 4))
    return None


def is_grey(colour, floor=0):
    """A colour with no hue at all — the whole point of the 256-colour cursor."""
    values = rgb(colour)
    return bool(values) and values[0] == values[1] == values[2] and values[0] >= floor


def is_accent(colour):
    """Blue enough to be the accent bar, and not the slightly-blue card."""
    values = rgb(colour)
    return bool(values) and values[2] > values[0] + 0x20


def say(text):
    """A pattern for a label, tolerant of how the interface lays CJK out.

    A wide glyph is padded to land on an even column, so a label in a fixed
    width cell can reach the screen as 「卸 载」 rather than 「卸载」. Matching
    the literal string would make the scenario depend on where on the line the
    label happens to start.
    """
    return r"\s*".join(re.escape(ch) for ch in text)


def app_rows(texts, needle):
    """The screen rows that list an application, ignoring the filter echo."""
    return [y for y, text in enumerate(texts) if needle in text and "筛" not in text]


def wears_cursor(t, row, mode):
    """Does this row wear the highlight in `mode`?

    A data row always wears *some* background — the list alternates so a wide
    row can be followed across the card — so "lit" is never "has a colour". It
    is the accent bar in a 24-bit terminal, the *light* grey bar in a 256-colour
    one (the card and its band are dark greys), and reverse video where there is
    no palette at all. Those three are the three palettes `squid` picks between.
    """
    _, styles = t.snapshot()
    look = styles[row]
    if mode == "mono":
        return any(style[3] for style in look)
    backgrounds = [style[0] for style in look]
    if mode == "truecolor":
        return any(is_accent(colour) for colour in backgrounds)
    return any(is_grey(colour, 0x80) for colour in backgrounds)


def row_bgs(t, needle):
    """The backgrounds painted on the data row naming `needle`."""
    texts, styles = t.snapshot()
    for y in app_rows(texts, needle):
        return set(style[0] for style in styles[y] if style[0] != "default")
    return None


def wait_gone(t, pattern, timeout=300):
    """Wait for a phrase to leave the screen — a scan finishing on its own.

    The disk map is a real walk of a real home folder: measured on a 201 GB /
    2.1M-inode volume it takes 166s, so a two-minute budget fails on a busy
    machine while the interface is doing nothing wrong.
    """
    rx = re.compile(pattern)
    end = time.time() + timeout
    while time.time() < end:
        t.pump(0.3)
        if not rx.search(t.text()):
            return True
    print(f"[MISS] {pattern!r} still on screen after {timeout}s")
    return False


def fixture_apps():
    """Two throwaway bundles: a list that is never empty, whatever else is installed."""
    fixtures = [("SquidSmokeA", "com.squid.smoke-alpha"),
                ("SquidSmokeB", "com.squid.smoke-beta")]
    for name, bundle in fixtures:
        make_app(name, bundle)
    return fixtures


def remove_apps(fixtures):
    home = os.path.expanduser("~")
    for name, bundle in fixtures:
        shutil.rmtree(os.path.join(home, "Applications", name + ".app"), ignore_errors=True)
        for folder in ("Application Support", "Caches"):
            shutil.rmtree(os.path.join(home, "Library", folder, bundle), ignore_errors=True)
        try:
            os.remove(os.path.join(home, "Library", "Preferences", bundle + ".plist"))
        except OSError:
            pass


def list_apps(t, needle="SquidSmoke"):
    """Open the app list and filter it down to the fixtures."""
    t.send("2")
    if not t.wait(say("个应用") + "|applications found", 90, "app list"):
        return False
    t.send("f")
    t.pump(0.2)
    t.send(*(["BS"] * 24), delay=0.02)
    t.send(needle, delay=0.3)
    t.send("Enter")
    t.pump(1.0)
    return t.wait(needle, 10, "the filtered list")


# ------------------------------------------------------------------- scenarios

def scenario_menu(binary, extra=None, term="xterm-256color", mode="truecolor", dump=False):
    """The keyboard model, judged on the screen the user is actually looking at.

    The first assertion is all of complaint 1: no key has been pressed when it
    runs, so it fails the moment the interface opens anywhere but on the six
    functions. The last assertion is the other half of the same complaint — the
    plain arrows must belong to the list again once the choosing is done.
    """
    env = sandbox()
    env.update(extra or {})
    fixtures = fixture_apps()
    t = TUI([binary, "state"], env=env, term=term)
    ok = True
    try:
        t.wait("squid", 15, "startup")

        # 1. nothing typed at all: the arrows already point at the functions
        ok &= t.wait(r"\[1\]", 8, "the interface opens on the function menu")
        ok &= t.wait(say("选择功能") + "|up and down to choose", 8, "the hint bar says so too")
        if dump:
            t.show("startup focus")

        # 2. the plain arrows walk the six functions and clamp at the ends
        t.send("Down")
        ok &= t.wait(r"\[2\]", 8, "arrow down -> row 2")
        ok &= t.wait(say("卸载"), 8, "panel switched to the uninstall list")
        t.send("Down")
        ok &= t.wait(r"\[3\]", 8, "arrow down -> row 3")
        t.send("Up")
        t.send("Up")
        ok &= t.wait(r"\[1\]", 8, "arrow up walks back")
        t.send("Up")
        ok &= t.refute(r"\[0\]|\[7\]", 1.0, "clamped at the ends")

        # 3. enter hands the keys to the list, esc hands them back
        t.send("Enter", delay=0.4)
        ok &= t.refute(r"\[1\]", 1.0, "the badge is gone after enter")
        t.send("Esc", delay=0.4)
        ok &= t.wait(r"\[1\]", 8, "esc hands the arrows back to the menu")

        # 4. option/control + arrows switch functions from anywhere
        t.send("MDown", delay=0.5)
        ok &= t.wait(say("卸载"), 8, "alt+down switched function")
        t.send("MUp", delay=0.5)
        ok &= t.wait(say("清理"), 8, "alt+up switched back")
        t.send("CDown", delay=0.5)
        ok &= t.wait(say("卸载"), 8, "ctrl+down switched too")

        # 5. and the plain arrows still scroll the list
        ok &= list_apps(t)
        t.send("Home")
        t.pump(0.6)
        before = t.snapshot()
        rows = app_rows(before[0], "SquidSmoke")
        if len(rows) != 2:
            print(f"[FAIL] expected two app rows, found {rows}")
            t.show("list rows")
            ok = False
        else:
            top, bottom = rows
            started = wears_cursor(t, top, mode)
            t.send("Down")
            t.pump(0.8)
            moved = t.moved_rows(before, t.snapshot())
            walked = wears_cursor(t, bottom, mode) and not wears_cursor(t, top, mode)
            print(f"[{'OK' if started and walked else 'FAIL'}] the arrow walked the "
                  f"cursor ({mode}): row {top} -> row {bottom}, repainted {moved}")
            ok = started and walked and ok
            if dump:
                t.show("plain arrows still scroll")
    finally:
        t.quit()
        remove_apps(fixtures)
    print(f"[{'OK' if ok else 'FAIL'}] menu/arrows scenario ({term}, {mode})")
    return ok


def scenario_mono(binary):
    """`NO_COLOR` is a request, not a joke: the cursor has to survive without it."""
    return scenario_menu(binary, extra={"NO_COLOR": "1"}, mode="mono")


def scenario_colors(binary, dump=False):
    """Complaint 2 and 3: the probe at start-up, and what each palette draws.

    The assertions are on the bytes the interface *sends*, because that is the
    difference the user can see: a terminal that is handed `48;2;r;g;b` without
    understanding it prints the numbers as text.
    """
    cases = [
        ("Apple Terminal", {"COLORTERM": "", "TERM_PROGRAM": "Apple_Terminal"}, "256"),
        ("iTerm2", {"COLORTERM": "truecolor", "TERM_PROGRAM": "iTerm.app"}, "truecolor"),
        ("bare ssh", {"COLORTERM": "truecolor", "TERM_PROGRAM": ""}, "truecolor"),
        ("TERM says nothing", {"COLORTERM": "", "TERM_PROGRAM": ""}, "256"),
        ("SQUID_TRUECOLOR=never", {"COLORTERM": "truecolor", "SQUID_TRUECOLOR": "never"}, "256"),
        ("NO_COLOR", {"NO_COLOR": "1"}, "mono"),
    ]
    fixtures = fixture_apps()
    ok = True
    try:
        for label, extra, wanted in cases:
            env = sandbox()
            env.update(extra)
            t = TUI([binary, "state"], env=env)
            try:
                t.wait("squid", 15, "startup")
                ok &= list_apps(t)
                t.send("Home")
                t.pump(0.6)
                sent_truecolour = b"48;2;" in t.raw or b"38;2;" in t.raw
                sent_indexed = b"48;5;" in t.raw or b"38;5;" in t.raw
                t.send("?")
                t.pump(0.8)
                probe = t.wait(say("色彩") + "|Colour", 8, "the help overlay names the palette")
                stated = {"256": r"256 colors", "truecolor": r"truecolor",
                          "mono": r"no colour"}[wanted]
                named = bool(re.search(stated, t.text()))
                t.send("Esc")
                t.pump(0.4)

                rows = app_rows(t.snapshot()[0], "SquidSmoke")
                cursor = wears_cursor(t, rows[0], wanted) if rows else False
                # the cursor sits on the first row: Home was just pressed
                bar = row_bgs(t, "SquidSmokeA") or set()

                checks = {
                    "truecolor": sent_truecolour and not sent_indexed,
                    "256": sent_indexed and not sent_truecolour,
                    "mono": not sent_truecolour and not sent_indexed,
                }[wanted]
                good = checks and probe and named and cursor
                print(f"[{'OK' if good else 'FAIL'}] {label:22s} -> {wanted:11s} "
                      f"truecolour_bytes={sent_truecolour} indexed={sent_indexed} "
                      f"overlay={named} cursor={cursor}")
                if wanted == "256":
                    greys = [c for c in bar if is_grey(c, 0x80)]
                    loud = [c for c in bar if is_accent(c)]
                    print(f"       cursor bar: grey={greys} accent={loud}")
                    good = good and bool(greys) and not loud
                if wanted == "truecolor":
                    accents = [c for c in bar if is_accent(c)]
                    print(f"       cursor bar: accent={accents}")
                    good = good and bool(accents)
                if wanted == "mono":
                    print(f"       cursor bar: no backgrounds at all = {not bar}")
                    good = good and not bar
                if not good and dump:
                    t.show(label)
                ok = ok and good
            finally:
                t.quit()
    finally:
        remove_apps(fixtures)
    print(f"[{'OK' if ok else 'FAIL'}] colour probe scenario")
    return ok


def scenario_escape(binary, dump=False):
    """Complaint 4: every level a key walks into, `esc` walks out of again.

    The levels are the ones that used to answer nothing: the second level of
    the app list (the leftover plan), the confirmation that a delete is what
    the user really wants, the largest-files list and the folder drill of the
    disk map, and — the case that used to hang for a whole disk walk — `esc`
    while a job is still running.
    """
    fixtures = fixture_apps()
    t = TUI([binary, "state"], env=sandbox())
    ok = True
    try:
        t.wait("squid", 15, "startup")
        assert_start = t.wait(r"\[1\]", 8, "startup focus")
        ok &= assert_start

        # level 0 — the help overlay is a level too
        t.send("?")
        t.wait(say("快捷键") + "|shortcuts", 8, "help opened")
        t.send("Esc")
        ok &= t.refute(say("快捷键") + "|shortcuts", 1.5, "esc closes the help")

        # level 1 — the list of applications
        ok &= list_apps(t)
        t.send("Esc")
        ok &= t.wait(r"\[2\]", 8, "esc leaves the list for the menu")
        t.send("Enter")  # back to the list, cursor on the first app
        t.pump(0.4)

        # level 2 — the leftovers of one application
        t.send("Enter")
        ok &= t.wait(r"com\.squid\.smoke-alpha", 30, "the plan opened")
        t.send("Esc")
        ok &= t.refute(r"com\.squid\.smoke-alpha", 2.5,
                       "esc leaves the plan, and the plan stays left")
        ok &= t.wait(r"SquidSmokeA", 8, "the list is back")

        # level 1 again, then level 2, then level 3 — the confirmation
        t.send("Enter")
        t.send("Enter")
        ok &= t.wait(r"com\.squid\.smoke-alpha", 30, "the plan opened again")
        t.send("d")
        ok &= t.wait(say("回车 / y 确认") + "|enter / y confirm", 15, "the confirmation opened")
        t.send("Esc")
        ok &= t.refute(say("回车 / y 确认") + "|enter / y confirm", 1.5, "esc cancels the delete")
        ok &= t.wait(r"com\.squid\.smoke-alpha", 8, "and lands back in the plan, not on the menu")
        t.send("Esc")
        ok &= t.refute(r"com\.squid\.smoke-alpha", 2.0, "esc leaves the plan again")

        # level 2 of the disk map — a folder, then the files inside it. The
        # scan is still running the whole time: a busy panel used to eat `esc`.
        t.send("Esc", "4")
        ok &= t.wait(r"\[4\]", 8, "the disk map was chosen")
        t.send("Enter")  # hand the keys back to the panel; the walk carries on
        t.pump(1.0)
        t.send("Esc")
        ok &= t.wait(r"\[4\]", 8, "esc leaves the disk map even while it is scanning")

        # the largest-files list is the other third level; it needs the map
        # first, so it is checked where the data already is: two esc in a row
        # must be two levels, not one and a shrug.
        t.send("Enter")
        if wait_gone(t, say("正在计算磁盘占用") + "|Mapping your disk"):
            t.send("f")
            if t.wait(say("最大的文件") + "|Largest files", 90, "the largest-files list opened"):
                t.send("Esc")
                left = t.refute(say("最大的文件") + "|Largest files", 4.0,
                                "esc leaves the largest-files list, walking or not")
                ok = left and ok
            else:
                print("[info] the largest-files list never filled in; esc still cancelled it")
                ok = t.refute(say("最大的文件") + "|Largest files", 2.0,
                              "a cancelled scan must not open the list it was asked to drop") and ok
        else:
            print("[FAIL] the disk map never filled in")
            ok = False
    finally:
        if dump:
            t.show("escape scenario")
        t.quit()
        remove_apps(fixtures)
    print(f"[{'OK' if ok else 'FAIL'}] esc walks back scenario")
    return ok


def scenario_uninstall(binary):
    """Two apps in a row. The classic failure is the second one answering
    "the previous task is still running" — a background disk map used to hold
    the whole interface hostage — or the first one's gauge never going away."""
    home = os.path.expanduser("~")
    fixtures = [("SquidSmokeA", "com.squid.smoke-alpha"), ("SquidSmokeB", "com.squid.smoke-beta")]
    for name, bundle in fixtures:
        make_app(name, bundle)
    t = TUI([binary, "state"], env=sandbox())
    ok = True
    try:
        t.wait("squid", 15, "startup")
        t.send("4")  # look at the disk map first, so the post-delete refresh fires
        t.wait("磁盘|storage", 15, "storage panel")
        t.send("2")
        t.wait(say("个应用") + "|applications found", 90, "app list")
        for name, bundle in fixtures:
            t.send("f")
            t.pump(0.2)
            t.send(*(["BS"] * 24), delay=0.02)
            t.send(name, delay=0.05)
            t.send("Enter")
            t.wait(name, 20, f"filter {name}")
            t.send("Enter")  # build the leftover plan
            t.pump(3.0)
            t.send("d")
            if not t.wait(r"立即|确认|卸载|Uninstall", 15, f"confirm dialog {name}"):
                ok = False
                break
            t.send("Enter")
            hit = t.wait(r"已删除|项被跳过|items removed|skipped", 90, f"delete feedback {name}")
            refused = not t.refute(r"还在进行|still running", 1.0, "the second delete must not be refused")
            stale = not t.refute(r"delete · ", 3.0, "a finished job must not keep its gauge")
            gone = not os.path.exists(os.path.join(home, "Applications", name + ".app"))
            good = hit and not refused and not stale and gone
            print(f"[{'OK' if good else 'FAIL'}] uninstall {name}: "
                  f"feedback={hit} refused={refused} stale_gauge={stale} removed={gone}")
            ok = ok and good
    finally:
        t.quit()
        for name, bundle in fixtures:
            shutil.rmtree(os.path.join(home, "Applications", name + ".app"), ignore_errors=True)
            for folder in ("Application Support", "Caches"):
                shutil.rmtree(os.path.join(home, "Library", folder, bundle), ignore_errors=True)
            try:
                os.remove(os.path.join(home, "Library", "Preferences", bundle + ".plist"))
            except OSError:
                pass
    print(f"[{'OK' if ok else 'FAIL'}] back-to-back uninstall scenario")
    return ok




def main(argv):
    known = {"menu": lambda b: scenario_menu(b),
             "mono": scenario_mono,
             "colors": scenario_colors,
             "escape": scenario_escape,
             "uninstall": scenario_uninstall}
    wanted, binary = [], "target/release/squid"
    for arg in argv[1:]:
        if arg in known:
            wanted.append(arg)
        elif arg in ("all", "-a"):
            wanted.extend(known)
        elif arg.startswith("-"):
            sys.exit(f"unknown option: {arg} (scenarios: {', '.join(known)})")
        else:
            binary = arg
    wanted = wanted or list(known)
    if not os.path.exists(binary):
        sys.exit(f"no such binary: {binary} (cargo build --release first)")
    results = {}
    for name in wanted:
        results[name] = known[name](binary)
    for name, ok in results.items():
        print(f"{'PASS' if ok else 'FAIL'}  {name}")
    return 0 if all(results.values()) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
