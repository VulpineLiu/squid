#!/usr/bin/env python3
"""Grow the console underneath a running TUI and check what is left on screen.

Resizing is the one thing a terminal interface is not allowed to get wrong, and
the one thing no unit test can prove. The bug lives between the frame the
renderer compared and the cells the console moved: a diff renderer writes the
cells that changed, so after a resize it happily leaves behind every cell it
still believes in, in coordinates the console no longer agrees to. A legacy
Windows console adds its own twist -- it answers a size question with the size of
its *window* while the buffer behind it is still the size it had when the program
entered the alternate screen. Every column that just came into view is then full
of the previous frame, and every cell written past the end of a real row lands at
the start of the next one. What the user sees is a screen of diagonals.

The test: start the interface small, paint a panel, grow the console while it is
running, then read the screen back through pyte and ask how many columns the
right hand borders sit in. One column means a screen. Two means two frames drawn
on top of each other.

Five facts about Windows OpenSSH were measured on the box, and each one
shaped this script:

* the ConPTY it hands a session ignores the client's window-change requests, so
  this side cannot resize the console it is watching. The grower runs on the box,
  in a process attached to the same console;
* `mode con cols=X lines=Y` fails silently there -- it reports no error and
  changes nothing -- so the grower drives `SetConsoleWindowInfo` itself. Any
  resize test built on `mode con` measured nothing at all;
* and it drives that call on a handle to `CONOUT$`, not on its own stdout or on
  `[Console]::WindowWidth`: entering the alternate screen swapped the *shown*
  buffer, and the process that asked for the resize is still describing the one
  nobody is looking at. A resize done that way reads back unchanged, which is
  the same silent nothing `mode con` produces;
* a window can never be wider than the screen buffer, and a ConPTY reports its
  buffer size as the largest window it will allow. Growing a 120 column console
  to 180 columns therefore means growing the buffer first, or the resize is
  refused and the test watches a size that never changed;
* nothing may be written to that console while the interface is drawing, not even
  by the test itself. A line printed by the harness lands inside the picture, and
  its newline scrolls the console a row -- after which the renderer's diff and
  the cells on the glass are one row apart everywhere, which looks exactly like
  the bug being hunted. So the grower reports into a file, and a second ssh
  connection reads that file once the interface has exited.

The console is resized by its own box on a timer this script starts alongside the
interface, which is also when the emulator is told to grow. Both are derived from
the moment the box announced the interface had started, so the two stay within a
few milliseconds of each other on a LAN.

    python3 scripts/win-tui-resize.py                     # 100x30 -> 180x50
    python3 scripts/win-tui-resize.py --lang zh           # CJK is twice as wide
    python3 scripts/win-tui-resize.py --small 80x24 --big 200x60

Exit code 0 means the interface survived the resize as one picture.
"""

import argparse
import codecs
import fcntl
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import time

try:
    import pyte
except ImportError:
    sys.exit("requires pyte:  python3 -m pip install --user pyte")

HOST = os.environ.get("WIN_HOST", "KANGWJ@192.168.2.92")
KEY = os.environ.get("WIN_KEY", os.path.expanduser("~/.ssh/id_ed25519_windows"))
BIN = os.environ.get("WIN_BIN", r"C:\dev\target\release\squid.exe")
HOME = "C:\\Users\\kangwj"
REMOTE_GROW = HOME + "\\win-tui-grow.ps1"
REMOTE_CLEAN = HOME + "\\win-tui-clean.ps1"


def report_path(run_id):
    """Where this run writes the facts it cannot print, named after this run.

    The name carries the run on purpose. Sharing one path and deleting it first
    looks equivalent and is not: a grower left behind by an interrupted run is
    still parked in WaitForExit, and when its interface finally dies it appends
    its own MARK_QUIT to whatever file is there. The next run then reads a
    report with one line in it and concludes that no resize ever happened,
    which is a fact about a process from ten minutes ago.
    """
    return f"{HOME}\\win-tui-report-{run_id}.txt"


def trace_path(run_id):
    return f"{HOME}\\win-tui-trace-{run_id}.log"

# TIOCSWINSZ, as macOS spells it. `struct winsize` is rows, cols, xpixel, ypixel.
TIOCSWINSZ = 0x40087468


def winsize(columns, rows):
    return struct.pack("HHHH", rows, columns, 0, 0)


def shape(text):
    columns, rows = text.lower().split("x")
    return int(columns), int(rows)


def ssh(*arguments):
    return ["ssh", "-i", KEY, "-o", "BatchMode=yes", HOST] + list(arguments)


def told(text, marker):
    """What the box said after `marker`, or None when it has not said yet.

    A cursor sequence can land between the words even in the safe part of the
    stream, so the pattern tolerates a handful of them. What it will not do is
    guess: nothing here trusts a requested size.
    """
    gap = r"(?:\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\s){0,8}"
    match = re.search(marker + gap + r"(\d{1,4})" + gap + r"x" + gap + r"(\d{1,4})", text)
    return None if not match else (int(match.group(1)), int(match.group(2)))


def read_back(remote_path):
    """A file the box wrote while the picture owned the console, read out of band.

    Nothing can be printed to that console while the interface is drawing, so
    every fact from inside it travels through a file and a second connection.
    """
    try:
        done = subprocess.run(ssh("type", remote_path), capture_output=True, timeout=30)
    except subprocess.TimeoutExpired:
        return ""
    return done.stdout.decode("utf-8", "replace")


def report(path):
    """The grower's own account of the sizes it ended up with."""
    return read_back(path)


def script(name):
    return os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                        "scripts", name)


def upload(local, remote):
    subprocess.run(["scp", "-i", KEY, "-o", "BatchMode=yes", "-q", local,
                    f"{HOST}:{remote}"], check=True)


def clean(reason):
    """End the runs that are still alive on the box, this one included.

    Killing the local ssh does not end the remote PowerShell, and a grower left
    parked in WaitForExit holds the console, the release binary that the next
    build cannot overwrite, and the habit of writing into a report file that no
    longer belongs to it.
    """
    upload(script("win-tui-clean.ps1"), REMOTE_CLEAN)
    done = subprocess.run(
        ["ssh", "-i", KEY, "-o", "BatchMode=yes", HOST,
         "powershell -NoProfile -ExecutionPolicy Bypass -File " + REMOTE_CLEAN],
        capture_output=True, timeout=60)
    said = done.stdout.decode("utf-8", "replace")
    for line in said.splitlines():
        if line.strip():
            print(f"[clean ] {line.strip()} ({reason})")
    return said


def grow_script(args):
    """Upload the grower and return the command that runs it on the box."""
    upload(script("win-tui-grow.ps1"), REMOTE_GROW)
    return [
        "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", REMOTE_GROW,
        str(args.small[0]), str(args.small[1]), str(args.big[0]), str(args.big[1]),
        str(int(args.resize_at)), args.lang or "en", BIN, args.report,
        args.trace_file if args.trace else "",
    ]


def run(args):
    clean("before")
    try:
        return watch(args)
    finally:
        clean("after")


def watch(args):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, TIOCSWINSZ, winsize(*args.small))
    child = subprocess.Popen(
        ssh("-tt", "-o", "ServerAliveInterval=30") + grow_script(args),
        stdin=slave, stdout=slave, stderr=slave, close_fds=True,
    )
    os.close(slave)

    screen = pyte.Screen(*args.small)
    stream = pyte.Stream(screen)
    # One decoder for the whole stream: a UTF-8 sequence split across two reads
    # would otherwise arrive as a replacement character the console never sent.
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    raw, text = [], []

    def feed(chunk):
        raw.append(chunk)
        decoded = decoder.decode(chunk)
        text.append(decoded)
        stream.feed(decoded)

    def said():
        return "".join(text)

    def adopt(size, note):
        """Follow the console rather than lead it, in the emulator as in the pty."""
        fcntl.ioctl(master, TIOCSWINSZ, winsize(*size))
        screen.resize(lines=size[1], columns=size[0])
        print(f"[shape ] {note}: now {size[0]}x{size[1]}")

    began = time.time()
    started, announced = None, None
    grown = None
    problems = []
    # Switch to the Power panel before the resize: the frame that has to survive
    # a resize must not be the frame that was already on screen. A digit opens
    # the sidebar menu, so `enter` closes it again and hands the keys back.
    keys = iter([(args.settle, "5"), (args.settle + 2, "\r")])
    next_key = next(keys, None)
    deadline = began + args.hold

    while time.time() < deadline:
        ready, _, _ = select.select([master], [], [], 0.2)
        if ready:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            feed(chunk)
            if "MARK_QUIT" in text[-1]:
                # The interface exited on its own: whatever it printed on the way
                # out is the finding, so stop rather than press keys into a corpse.
                problems.append("the interface exited by itself before the test asked it to")
                break
        now = time.time() - began
        if next_key and now >= next_key[0]:
            os.write(master, next_key[1].encode())
            next_key = next(keys, None)
        if started is None:
            started = told(said(), "MARK_STARTED")
            if started:
                announced = time.time()
                if started != tuple(args.small):
                    print(f"[shape ] requested {args.small[0]}x{args.small[1]}, "
                          f"the console is {started[0]}x{started[1]}")
                    adopt(started, "announced")
        if grown is None and announced is not None:
            # The grower sleeps this long between announcing and resizing, so the
            # two clocks agree to a few milliseconds on a LAN. Growing the
            # emulator a moment early is the safe direction: a screen wider than
            # the console never wraps a row by mistake, and the interface wipes
            # and redraws the whole area the moment the real resize arrives.
            if time.time() >= announced + max(args.resize_at - args.margin, 0.0):
                grown = args.big
                adopt(grown, "grown")

    painted = [line.rstrip() for line in screen.display]

    # Ask to quit. The alternate screen is invisible through this ConPTY (it does
    # not forward ?1049h/?1049l), so a clean exit is proved by the grower's last
    # line, which it may print only once the console belongs to it again.
    quit_cleanly = False
    try:
        os.write(master, b"q")
    except OSError:
        ending = time.time()
    else:
        ending = time.time() + 6
    while time.time() < ending:
        ready, _, _ = select.select([master], [], [], 0.3)
        if not ready:
            continue
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        feed(chunk)
        if "MARK_QUIT" in said()[-200:]:
            quit_cleanly = True
            break

    child.send_signal(signal.SIGTERM)
    try:
        child.wait(5)
    except subprocess.TimeoutExpired:
        child.kill()
    os.close(master)

    raw_bytes = b"".join(raw)
    if args.out:
        with open(args.out, "wb") as handle:
            handle.write(raw_bytes)
        print(f"[raw   ] {len(raw_bytes)} bytes kept in {args.out}")
    return dict(painted=painted, started=started, grown=grown,
                report_text=report(args.report), trace=trace(args),
                quit_cleanly=quit_cleanly, problems=problems, characters=len(said()))


def trace(args):
    """What the interface itself measured, when it was asked to keep a log."""
    return read_back(args.trace_file) if args.trace else ""


def told_offband(text, marker):
    for line in text.splitlines():
        if line.startswith(marker):
            return line
    return None


def window_of(line):
    """The window half of `MARK_RESIZED 180x50 in 180x9001`."""
    match = re.search(r"(\d{1,4})x(\d{1,4})", line or "")
    return None if not match else (int(match.group(1)), int(match.group(2)))


def buffer_of(line):
    """The buffer half of the same line, after the word `in`.

    Worth reading separately because the two halves are the whole story of a
    legacy console: a window bigger than its buffer is a window with columns
    that have no cells, and a buffer bigger than its window is a window that
    can slide away from the picture being painted into it.
    """
    match = re.search(r"\bin\s*(\d{1,4})x(\d{1,4})", line or "")
    return None if not match else (int(match.group(1)), int(match.group(2)))


def check(found, args):
    problems = list(found["problems"])
    painted = found["painted"]

    # What the console says it did, from the grower's file rather than from the
    # picture: an interface can only draw in the space it was given, so a resize
    # that the console refused has to be told apart from one it survived.
    account = found["report_text"]
    facts = [line for line in account.splitlines() if line.strip()]
    for line in facts:
        print(f"[report] {line}")
    if not facts:
        problems.append("the grower left no report — nothing on the box was measured")
    # The file names itself, so a report from a run that never ended cannot be
    # mistaken for this one's. Everything below depends on that line being here.
    if told_offband(account, "MARK_ENTER") is None:
        problems.append("the report does not name this run — it is not this run's report")
    if told_offband(account, "MARK_RUNNING") is None:
        problems.append("the interface was never started on the box")
    # Two seconds after the resize, the console's own final word is the only
    # size worth judging the picture against. Measured on the 18363 box: a
    # ConPTY created by sshd keeps the window it was created with — asking for
    # 180x50 grows the buffer and leaves the window where it was, so a test that
    # demanded the requested shape would be failing the console for an
    # ownership fact instead of the drawing fact. What has to hold either way is
    # that the buffer ends up the size of the window, because that is the one
    # shape in which the cells being painted are the cells being shown.
    still = told_offband(account, "MARK_STILL")
    if still is None:
        problems.append("the grower never looked at the console a second time")
    final = window_of(still) or args.big
    if window_of(still) and window_of(still) != args.big:
        print(f"[note  ] the console kept its window at {final[0]}x{final[1]} "
              f"instead of the requested {args.big[0]}x{args.big[1]} — this console is "
              "owned by something that pins its window. Judging the picture "
              "against what the console actually says.")
    held = buffer_of(still)
    if held and final and held != final:
        problems.append(
            f"the console still holds a {held[0]}x{held[1]} buffer behind a "
            f"{final[0]}x{final[1]} window — the window can slide away from that, "
            "which is a frame painted where nobody is looking"
        )
    said = told_offband(account, "MARK_STARTED")
    if said is None:
        problems.append("the console never reported started — the grower did not run")
    elif window_of(said) != args.small:
        problems.append(
            f"the console is {said} before the interface ran, not "
            f"{args.small[0]}x{args.small[1]} — the picture proves nothing about that shape"
        )
    if told_offband(account, "MARK_RESIZED") is None:
        problems.append("the console never looked at itself after the resize")
    refused = told_offband(account, "SIZE_FAILED")
    if refused:
        problems.append(f"the console refused a size: {refused}")

    # The interface's own numbers. It is the only witness that knows which buffer
    # it was drawing on, so when the two sides disagree this is the line that
    # says who is wrong.
    for line in found["trace"].splitlines():
        if line.strip():
            print(f"[trace ] {line}")

    edges = {}
    for line in painted:
        if line.strip() and "\u2502" in line:
            edges.setdefault(line.rindex("\u2502"), []).append(line)
    histogram = {column: len(rows) for column, rows in edges.items()}
    drawn = sum(1 for line in painted if line.strip())
    print(f"[screen] {drawn} rows drawn - right borders: "
          + (", ".join(f"col {c} x{n}" for c, n in sorted(histogram.items())) or "none"))
    if not histogram:
        problems.append("the screen holds no boxes at all")
    elif len(histogram) > 1:
        best = max(histogram.values())
        stray = sum(histogram.values()) - best
        problems.append(
            f"{stray} of {sum(histogram.values())} bordered rows end somewhere other than the "
            f"column the other {best} do - two screens drawn on top of each other"
        )
        for column, rows in sorted(edges.items()):
            print(f"[stray ] col {column}: " + " | ".join(row[:70] for row in rows[:3]))
    else:
        column, rows = next(iter(edges.items()))
        if column + 1 != final[0]:
            problems.append(
                f"the picture is {column + 1} columns wide in a {final[0]} column window - "
                "the resize was never drawn into"
            )
        if len(rows) < final[1] * 3 // 5:
            problems.append(
                f"only {len(rows)} rows reach the right border in a {final[1]} row window - "
                "the redraw covered part of the screen"
            )

    if not found["quit_cleanly"]:
        problems.append("the interface did not answer `q` - the shell prompt never came back")
    else:
        quit_line = (told_offband(account, "MARK_QUIT") or "").split()
        if quit_line and quit_line[-1] != "0":
            problems.append("the interface left the console with " + " ".join(quit_line))
    rows = [n for n, line in enumerate(painted, 1) if "\ufffd" in line]
    if rows:
        problems.append(f"screen rows {rows} carry glyphs the console could not draw")

    print()
    print("\n[screen] ".join(["[screen]"] + painted))
    print()
    for problem in problems:
        print(f"[FAIL] {problem}")
    print(f"RESULT: {'FAIL' if problems else 'OK'}")
    return 1 if problems else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--small", type=shape, default=(100, 30), metavar="COLSxROWS")
    parser.add_argument("--big", type=shape, default=(180, 50), metavar="COLSxROWS")
    parser.add_argument("--lang", choices=["en", "zh"])
    parser.add_argument("--resize-at", type=float, default=10.0,
                        help="seconds of running before the box grows its own console")
    parser.add_argument("--margin", type=float, default=1.0,
                        help="seconds before that moment to grow the emulator")
    parser.add_argument("--hold", type=float, default=25.0,
                        help="seconds to watch the interface before pressing q")
    parser.add_argument("--settle", type=float, default=6.0,
                        help="seconds before the panel is switched")
    parser.add_argument("--out", default="/tmp/squid-win-resize.bin")
    parser.add_argument("--no-trace", dest="trace", action="store_false",
                        help="do not ask the interface to log what its console claimed")
    parser.set_defaults(trace=True)
    args = parser.parse_args()
    # One name per run: see report_path(). A minute of history stays readable and
    # a leftover from yesterday cannot answer for today.
    args.run_id = time.strftime("%Y%m%d-%H%M%S")
    args.report = report_path(args.run_id)
    args.trace_file = trace_path(args.run_id)
    print(f"[run   ] {args.run_id} - report {args.report}")
    return check(run(args), args)


if __name__ == "__main__":
    sys.exit(main())
