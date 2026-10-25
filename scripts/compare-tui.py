#!/usr/bin/env python3
"""Drives the Go and the Rust runui in a private tmux server, on one home:
the same screens side by side after each key, cell by cell (character,
colours, attributes), and a run started by one found, followed and stopped
by the other.

    RUNUI_COMPARE=dir python3 scripts/compare-tui.py

dir holds runui-go and runui-rs (task compare builds them there). E2E_W and
E2E_H set the window (140x40). The runs it starts are stopped as it goes."""

import os
import re
import shutil
import subprocess
import sys
import time

ROOT = os.environ.get("RUNUI_COMPARE") or os.path.join(os.environ.get("TMPDIR", "/tmp"), "runui-compare")
BASE = os.path.join(ROOT, "tui")
SOCK = os.path.join(BASE, "tmux.sock")
GO = os.path.join(ROOT, "runui-go")
RS = os.path.join(ROOT, "runui-rs")
HOME = os.path.join(BASE, "home")
APP = os.path.join(HOME, "projects", "app")
W, H = int(os.environ.get("E2E_W", 140)), int(os.environ.get("E2E_H", 40))

ENV = f"env HOME={HOME} XDG_CONFIG_HOME={HOME}/config XDG_STATE_HOME={HOME}/state TERM=xterm-256color"


def tmux(*args):
    result = subprocess.run(["tmux", "-S", SOCK, "-f", "/dev/null", *args], capture_output=True, text=True)
    if result.returncode and args[0] != "kill-server":
        raise RuntimeError(f"tmux {' '.join(args)}: {result.stderr.strip()}")
    return result


def start(name, binary, args=""):
    cmd = f"{ENV} {binary} {args}; echo; echo EXIT=$?; sleep 600"
    tmux("new-session", "-d", "-s", name, "-x", str(W), "-y", str(H), cmd)
    time.sleep(0.8)


def screen(name):
    return tmux("capture-pane", "-p", "-t", name).stdout


def keys(name, *ks):
    for k in ks:
        tmux("send-keys", "-t", name, k)
        time.sleep(0.25)


def wait(name, want, secs=10):
    deadline = time.time() + secs
    while time.time() < deadline:
        s = screen(name)
        if want in s:
            return s
        time.sleep(0.2)
    print(f"!! {name} never shows {want!r}:\n{screen(name)}")
    return screen(name)


def expand_tabs(line):
    # tmux keeps the tabs Bubble Tea's renderer moves over blank cells with:
    # the cells are blank either way
    import unicodedata
    out, col = [], 0
    for ch in line:
        if ch == "\t":
            n = 8 - col % 8
            out.append(" " * n)
            col += n
        else:
            out.append(ch)
            col += 2 if unicodedata.east_asian_width(ch) in "WF" else 1
    return "".join(out)


def masked(s):
    # a line Bubble Tea's renderer drew with tabs over blank cells: its
    # blanks as one space, the rest of it as it is
    s = "\n".join(re.sub(r"[ \t]+", " ", l) if "\t" in l else l for l in s.split("\n"))
    # times and pids move on between two captures
    s = re.sub(r"\d+m \d+s|\d+s\b", "<t>", s)
    s = re.sub(r"pid \d+", "pid <p>", s)
    s = re.sub(r"\b\d\d:\d\d\b", "<hh:mm>", s)
    s = re.sub(r"tick \d+", "tick <n>", s)
    return s


problems = 0


def cells(line):
    """a captured line's cells as tmux holds them: (char, fg, bg, attrs), a
    blank's foreground left out (it doesn't show), unless it's reversed or
    underlined"""
    out, fg, bg, attrs, i = [], None, None, set(), 0
    while i < len(line):
        if line[i] == "\x1b":
            m = re.match(r"\x1b\[([0-9;]*)m", line[i:])
            if m:
                ps = [int(p) if p else 0 for p in m.group(1).split(";")] if m.group(1) else [0]
                j = 0
                while j < len(ps):
                    p = ps[j]
                    if p == 0:
                        fg = bg = None
                        attrs = set()
                    elif p in (1, 2, 3, 4, 5, 7, 9):
                        attrs.add(p)
                    elif p == 22:
                        attrs -= {1, 2}
                    elif p == 27:
                        attrs.discard(7)
                    elif p in (38, 48):
                        if ps[j + 1] == 2:
                            v = tuple(ps[j + 2 : j + 5])
                            j += 4
                        else:
                            v = ("256", ps[j + 2])
                            j += 2
                        if p == 38:
                            fg = v
                        else:
                            bg = v
                    elif p == 39:
                        fg = None
                    elif p == 49:
                        bg = None
                    elif 30 <= p <= 37 or 90 <= p <= 97:
                        fg = ("16", p)
                    elif 40 <= p <= 47 or 100 <= p <= 107:
                        bg = ("16", p)
                    j += 1
                i += m.end()
                continue
        ch = line[i]
        i += 1
        if ch == "\t":
            ch = " "
        visible = fg if (ch != " " or attrs & {4, 7, 9}) else None
        out.append((ch, visible, bg, frozenset(attrs - ({1} if ch == " " else set()))))
    return out


def squash(cs):
    """a line Bubble Tea's renderer drew with tabs over blank cells, which
    tmux keeps: its blank runs as one blank"""
    out = []
    for c in cs:
        if out and c[0] == " " and out[-1] == c:
            continue
        out.append(c)
    return out


def same_cells(label, name_go="go", name_rs="rs"):
    global problems
    g = tmux("capture-pane", "-p", "-e", "-t", name_go).stdout.split("\n")
    r = tmux("capture-pane", "-p", "-e", "-t", name_rs).stdout.split("\n")
    bad = []
    for i, (x, y) in enumerate(zip(g, r)):
        cx, cy = cells(x), cells(y)
        if "\t" in x:
            cx, cy = squash(cx), squash(cy)
        if cx != cy:
            bad.append(i)
    if not bad:
        print(f"ok   {label} (cells)")
        return
    problems += 1
    print(f"DIFF {label} (cells): lines {bad}")


def same(label, a, b):
    global problems
    ga, rb = masked(a).split("\n"), masked(b).split("\n")
    tabbed = [i for i, l in enumerate(a.split("\n")) if "\t" in l]
    rb = [re.sub(r"[ \t]+", " ", l) if i in tabbed else l for i, l in enumerate(rb)]
    if ga == rb:
        print(f"ok   {label}")
        return
    problems += 1
    print(f"DIFF {label}")
    al, bl = masked(a).split("\n"), masked(b).split("\n")
    for i, (x, y) in enumerate(zip(al, bl)):
        if "\t" in a.split("\n")[i]:
            y = re.sub(r"[ \t]+", " ", y)
        if x != y:
            print(f"  line {i}\n    go: {x!r}\n    rs: {y!r}")


def same_live(label, a_name="go", b_name="rs", tries=6):
    """same, for screens following a run's output, which moves on between
    two captures: looked at again until they agree, a few times"""
    for _ in range(tries - 1):
        a, b = screen(a_name), screen(b_name)
        if masked(a) == masked(b):
            break
        time.sleep(0.15)
    same(label, screen(a_name), screen(b_name))


def check(label, cond, s=""):
    global problems
    if cond:
        print(f"ok   {label}")
    else:
        problems += 1
        print(f"FAIL {label}\n{s}")


shutil.rmtree(BASE, ignore_errors=True)
os.makedirs(APP)
with open(os.path.join(APP, "package.json"), "w") as f:
    f.write('{"scripts": {"serve": "echo serving; i=0; while true; do echo tick $i; i=$((i+1)); sleep 1; done", '
            '"build": "echo built; exit 3", "lint": "echo lint"}}')
with open(os.path.join(APP, "Makefile"), "w") as f:
    f.write("test: ## Run the tests\n\t@echo testing\n")
with open(os.path.join(APP, "compose.yaml"), "w") as f:
    f.write("services:\n  db: {image: postgres:16}\n")
tmux("kill-server")

# 1. the first screen, and every key's
start("go", GO, f"-C {APP}")
start("rs", RS, f"-C {APP}")
same("first screen", screen("go"), screen("rs"))
same_cells("first screen")
for k in ["?", "x", "]", "]", "[", "2", "j", "j", "/", "t", "Enter", "Escape", "1", "3", "Tab", "0", "+", "_", "a"]:
    keys("go", k)
    keys("rs", k)
    same(f"after {k}", screen("go"), screen("rs"))
    same_cells(f"after {k}")
for k in ["e", "c", "h", "o", " ", "h", "i", "Tab", "g", "r", "e", "e", "t", "Enter"]:
    keys("go", k)
form_screen = screen("go")
keys("go", "q")
time.sleep(0.5)
tmux("kill-server")
favorites_path = os.path.join(HOME, "config", "runui", "favorites.json")
if not os.path.exists(favorites_path):
    check("the form adds a command of your own (Go)", False, form_screen)
    sys.exit(1)
check("the form adds a command of your own (Go)", '"command": "echo hi"' in open(favorites_path).read())

# 2. the favorites the Go build kept, as the Rust build shows them
start("go", GO, f"-C {APP}")
start("rs", RS, f"-C {APP}")
same("favorites written by Go", screen("go"), screen("rs"))
keys("go", "q")
keys("rs", "q")
time.sleep(0.5)
tmux("kill-server")

# 3. a run started by the Rust build, followed and stopped by the Go build
start("rs", RS, f"-C {APP}")
keys("rs", "2")
s = screen("rs")
keys("rs", "Enter")  # dev? the first script: serve
wait("rs", "tick 2")
rs_running = screen("rs")
check("the Rust build runs it", "● running for" in rs_running, rs_running)
keys("rs", "q")
after = wait("rs", "EXIT=")
check("quitting says what keeps running", "keeps running in ~/projects/app" in after, after)
tmux("kill-server")
start("go", GO, f"-C {APP}")
start("rs", RS, f"-C {APP}")
keys("go", "3")
keys("rs", "3")
go_found = wait("go", "tick ")
check("the Go build finds the Rust build's run", "● running for" in go_found and "[3]─Here" in go_found, go_found)
time.sleep(1.2)
same_live("a run going, in both")
keys("go", "s")
stopped = wait("go", "stopped after")
check("the Go build stops it", "■ stopped after" in stopped, stopped)
time.sleep(0.6)
# the status line says what the Go build did itself: stopped it
same("a run stopped, in both", "\n".join(screen("go").split("\n")[:-3]), "\n".join(screen("rs").split("\n")[:-3]))
keys("go", "q")
keys("rs", "q")
time.sleep(0.5)
tmux("kill-server")

# 4. a run started by the Go build, stopped by the Rust build, and one that fails
start("go", GO, f"-C {APP}")
keys("go", "2", "Enter")
wait("go", "tick 1")
keys("go", "q")
time.sleep(0.6)
tmux("kill-server")
start("rs", RS, f"-C {APP}")
keys("rs", "3")
found = wait("rs", "tick ")
check("the Rust build finds the Go build's run", "● running for" in found, found)
keys("rs", "s")
stopped = wait("rs", "stopped after")
check("the Rust build stops it", "■ stopped after" in stopped, stopped)
keys("rs", "2", "j", "Enter")
failed = wait("rs", "exited 3")
check("a command that fails shows how", "✗ exited 3" in failed and "built" in failed, failed)
keys("rs", "q")
time.sleep(0.5)
tmux("kill-server")

# 6. the mouse, each build in a home of its own: a click on a row, on the
# pane, a right click, the wheel over the pane and over a panel
tmux("kill-server")
MHOME = os.path.join(BASE, "mouse")
MAPP = os.path.join(MHOME, "projects", "app")
os.makedirs(MAPP)
with open(os.path.join(MAPP, "package.json"), "w") as f:
    f.write("{" + ", ".join(f'"s{i}": "echo line {i}; for n in $(seq 1 80); do echo out $n; done"' for i in range(12)).join(['"scripts": {', '}']) + "}")
for name, b in (("go", GO), ("rs", RS)):
    env = f"env HOME={MHOME} XDG_CONFIG_HOME={MHOME}/config-{name} XDG_STATE_HOME={MHOME}/state-{name} TERM=xterm-256color"
    tmux("new-session", "-d", "-s", name, "-x", str(W), "-y", str(H), f"{env} {b} -C {MAPP}; sleep 600")
time.sleep(1.0)


def mouse(b, x, y):
    for n in ("go", "rs"):
        tmux("send-keys", "-t", n, "-l", f"\x1b[<{b};{x};{y}M")
        if b < 64:
            tmux("send-keys", "-t", n, "-l", f"\x1b[<{b};{x};{y}m")
    time.sleep(0.5)


keys("go", "2")
keys("rs", "2")
for label, (b, x, y), then in [
    ("click a row", (0, 10, 6), ["Enter", "3"]),
    ("click the pane", (0, W // 2, H // 2), []),
    ("the wheel up over the pane", (64, W // 2, H // 2), []),
    ("the wheel down over the pane", (65, W // 2, H // 2), []),
    ("the wheel over a panel", (65, 10, H - 6), []),
    ("click the favorites", (0, 10, 2), []),
    ("a right click", (2, 10, 6), []),
]:
    mouse(b, x, y)
    for k in then:
        keys("go", k)
        keys("rs", k)
        time.sleep(0.6)
    same(f"mouse: {label}", screen("go"), screen("rs"))
for n in ("go", "rs"):
    keys(n, "q")
time.sleep(0.5)
tmux("kill-server")

# 5. ps and list on what both did, and nothing left running
for r in subprocess.run(f"{ENV} {RS} ps", shell=True, capture_output=True, text=True).stdout.split("\n"):
    if r.startswith("●"):
        print(f"FAIL left running: {r}")
        problems += 1
ps = lambda b: subprocess.run(f"{ENV} {b} ps", shell=True, capture_output=True, text=True).stdout
ls = lambda b: subprocess.run(f"{ENV} {b} list -C {APP}", shell=True, capture_output=True, text=True).stdout
same("ps", ps(GO), ps(RS))
same("list", ls(GO), ls(RS))
print(ps(RS))
print(f"{problems} problems")
sys.exit(1 if problems else 0)
