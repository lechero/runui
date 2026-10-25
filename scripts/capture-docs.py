#!/usr/bin/env python3
"""Capture real runui screens for the README and HTML guide.

Requires tmux and Pillow. Everything runui writes lives in a temporary HOME;
the only files kept are the images in docs/assets. Run after `task build`:

    python3 scripts/capture-docs.py
"""

from __future__ import annotations

import json
import os
import re
import signal
import subprocess
import tempfile
import time
import unicodedata
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target/release/runui"
OUT = ROOT / "docs/assets"
COLS, ROWS = 142, 28
CELL_W, CELL_H = 11, 23
PAD, BAR = 24, 48
BG = "#0b1220"
FG = "#e5e7eb"
FONT = "/System/Library/Fonts/Menlo.ttc"
if not Path(FONT).exists():
    FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"

SGR = re.compile(r"\x1b\[([0-9;]*)m")


def run(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(args, text=True, capture_output=True)
    if check and result.returncode:
        raise RuntimeError(f"{' '.join(args[:4])}: {result.stderr.strip()}")
    return result


def xterm_color(n: int) -> tuple[int, int, int]:
    base = [
        0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0,
        0x808080, 0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
    ]
    if n < 16:
        v = base[n]
        return (v >> 16, (v >> 8) & 255, v & 255)
    if n < 232:
        n -= 16
        steps = (0, 95, 135, 175, 215, 255)
        return (steps[n // 36], steps[(n // 6) % 6], steps[n % 6])
    shade = 8 + (n - 232) * 10
    return (shade, shade, shade)


def styled_cells(line: str) -> list[tuple[str, tuple[int, int, int] | None, tuple[int, int, int] | None, bool]]:
    cells = []
    fg = bg = None
    bold = reverse = False
    i = 0
    while i < len(line):
        match = SGR.match(line, i)
        if match:
            params = [int(p or 0) for p in match.group(1).split(";")]
            j = 0
            while j < len(params):
                p = params[j]
                if p == 0:
                    fg = bg = None
                    bold = reverse = False
                elif p == 1:
                    bold = True
                elif p == 22:
                    bold = False
                elif p == 7:
                    reverse = True
                elif p == 27:
                    reverse = False
                elif p in (38, 48) and j + 2 < len(params):
                    color = None
                    if params[j + 1] == 5:
                        color = xterm_color(params[j + 2])
                        j += 2
                    elif params[j + 1] == 2 and j + 4 < len(params):
                        color = tuple(params[j + 2:j + 5])
                        j += 4
                    if p == 38:
                        fg = color
                    else:
                        bg = color
                elif p == 39:
                    fg = None
                elif p == 49:
                    bg = None
                elif 30 <= p <= 37:
                    fg = xterm_color(p - 30)
                elif 90 <= p <= 97:
                    fg = xterm_color(p - 90 + 8)
                elif 40 <= p <= 47:
                    bg = xterm_color(p - 40)
                elif 100 <= p <= 107:
                    bg = xterm_color(p - 100 + 8)
                j += 1
            i = match.end()
            continue
        ch = line[i]
        i += 1
        if ch == "\t":
            count = 8 - len(cells) % 8
            cells.extend((" ", fg, bg, bold) for _ in range(count))
        elif unicodedata.combining(ch) and cells:
            prev = cells[-1]
            cells[-1] = (prev[0] + ch, prev[1], prev[2], prev[3])
        else:
            cells.append((ch, bg if reverse else fg, fg if reverse else bg, bold))
            if unicodedata.east_asian_width(ch) in "WF":
                cells.append(("", bg if reverse else fg, fg if reverse else bg, bold))
    return cells


def picture(screen: str, title: str, rows: int = ROWS) -> Image.Image:
    width = COLS * CELL_W + PAD * 2
    height = rows * CELL_H + BAR + PAD
    image = Image.new("RGB", (width, height), "#111827")
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((5, 5, width - 6, height - 6), radius=15, fill="#161e2e", outline="#344155", width=2)
    draw.rounded_rectangle((PAD - 1, BAR - 1, width - PAD, height - PAD + 1), radius=6, fill=BG)
    for n, color in enumerate(("#fb7185", "#fbbf24", "#4ade80")):
        x = PAD + 14 + n * 20
        draw.ellipse((x, 20, x + 10, 30), fill=color)
    ui_font = ImageFont.truetype(FONT, 14)
    font = ImageFont.truetype(FONT, 17)
    bold_font = ImageFont.truetype(FONT, 17, index=1) if FONT.endswith(".ttc") else font
    draw.text((width // 2, 17), title, fill="#aebcd1", font=ui_font, anchor="mt")
    top = BAR + 5
    for row, line in enumerate(screen.splitlines()[:rows]):
        for col, (ch, fg, bg, bold) in enumerate(styled_cells(line)[:COLS]):
            x, y = PAD + col * CELL_W, top + row * CELL_H
            if bg is not None:
                draw.rectangle((x, y - 1, x + CELL_W, y + CELL_H - 1), fill=bg)
            if ch and ch != " ":
                draw.text((x, y), ch, fill=fg or FG, font=bold_font if bold else font)
    return image


def fixture(home: Path) -> Path:
    app = home / "projects/atlas"
    (app / ".git").mkdir(parents=True)
    (app / "apps/web").mkdir(parents=True)
    (app / "scripts").mkdir()
    (app / "Taskfile.yml").write_text("""version: '3'
tasks:
  preview:
    desc: Watch the workspace
    cmds:
      - sh scripts/preview.sh
  build:
    desc: Build all packages
    cmds:
      - echo 'Build complete'
  check:
    desc: Check formatting and tests
    cmds:
      - echo 'Checks passed'
""")
    (app / "scripts/preview.sh").write_text("""#!/bin/sh
printf '\\033[1;36mAtlas workspace\\033[0m\\n'
printf 'Watching packages for changes...\\n'
i=1
while :; do
  printf 'cycle %02d  checked 4 packages  ✓\\n' "$i"
  i=$((i + 1))
  sleep 1
done
""")
    (app / "package.json").write_text(json.dumps({
        "name": "atlas", "private": True, "packageManager": "pnpm@10.0.0",
        "scripts": {"dev": "pnpm --filter web dev", "build": "pnpm -r build", "lint": "eslint ."},
    }, indent=2) + "\n")
    (app / "apps/web/package.json").write_text(json.dumps({
        "name": "@atlas/web", "scripts": {"dev": "vite --host", "build": "vite build", "test": "vitest run"},
    }, indent=2) + "\n")
    (app / "Makefile").write_text("""test: ## Run the test suite
\t@echo tests passed

clean: ## Remove generated files
\t@echo cleaned
""")
    (app / "compose.yaml").write_text("""services:
  db:
    image: postgres:17
  cache:
    image: redis:7
""")
    (app / "pom.xml").write_text("""<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion><groupId>dev.atlas</groupId>
  <artifactId>tools</artifactId><version>1.0.0</version>
  <profiles><profile><id>release</id></profile></profiles>
</project>
""")
    return app


class Session:
    def __init__(self, socket: Path, home: Path, app: Path):
        self.socket = socket
        self.home = home
        self.app = app
        self.target = "docs"

    def tmux(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return run("tmux", "-S", str(self.socket), "-f", "/dev/null", *args, check=check)

    def start(self) -> None:
        env = f"env -u NO_COLOR HOME={self.home} XDG_CONFIG_HOME={self.home}/config XDG_STATE_HOME={self.home}/state TERM=xterm-256color COLORTERM=truecolor"
        command = f"{env} {BIN} -C {self.app}; sleep 30"
        self.tmux("new-session", "-d", "-s", self.target, "-x", str(COLS), "-y", str(ROWS), command)
        self.wait("[2]", 8)

    def capture(self) -> str:
        return self.tmux("capture-pane", "-p", "-e", "-t", self.target).stdout

    def plain(self) -> str:
        return self.tmux("capture-pane", "-p", "-t", self.target).stdout

    def key(self, *keys: str) -> None:
        for key in keys:
            self.tmux("send-keys", "-t", self.target, key)
            time.sleep(0.18)

    def wait(self, needle: str, seconds: float = 5) -> None:
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if needle in self.plain():
                return
            time.sleep(0.1)
        raise RuntimeError(f"runui never showed {needle!r}:\n{self.plain()}")

    def shot(self, name: str, title: str, rows: int = ROWS) -> Image.Image:
        image = picture(self.capture(), title, rows)
        image.save(OUT / f"{name}.png", optimize=True)
        print(f"docs/assets/{name}.png")
        return image

    def close(self) -> None:
        self.tmux("kill-server", check=False)


def stop_demo(home: Path) -> None:
    """Stop only supervisors created inside the disposable home on failure."""
    for run_file in (home / "state/runui/runs").glob("*/run.json"):
        try:
            run_data = json.loads(run_file.read_text())
        except (OSError, ValueError):
            continue
        if run_data.get("ended"):
            continue
        supervisor = run_data.get("supervisor", 0)
        if not isinstance(supervisor, int) or supervisor <= 0:
            continue
        try:
            os.kill(supervisor, signal.SIGTERM)
        except ProcessLookupError:
            continue
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline:
            try:
                if json.loads(run_file.read_text()).get("ended"):
                    break
            except (OSError, ValueError):
                break
            time.sleep(0.1)


def main() -> None:
    if not BIN.exists():
        raise SystemExit("Build first with `task build`.")
    OUT.mkdir(parents=True, exist_ok=True)
    # Unix socket paths have a small limit on macOS; $TMPDIR is too deep.
    with tempfile.TemporaryDirectory(prefix="runui-docs-", dir="/private/tmp") as scratch:
        home = Path(scratch) / "home"
        app = fixture(home)
        session = Session(Path(scratch) / "tmux.sock", home, app)
        frames: list[Image.Image] = []
        try:
            session.start()
            session.shot("project", "runui  ·  ~/projects/atlas")
            session.key("m")  # star the Taskfile's first task
            session.key("1")
            session.wait("★ preview")
            session.shot("favorites", "runui  ·  Favorites")
            frames.append(picture(session.capture(), "runui  ·  Favorites"))
            session.key("Enter")
            session.wait("cycle 02", 8)
            session.shot("running", "runui  ·  Live output")
            frames.append(picture(session.capture(), "runui  ·  Live output"))
            session.key("3")
            session.shot("runs", "runui  ·  Runs")
            frames.append(picture(session.capture(), "runui  ·  Runs"))
            session.key("2", "]")
            session.shot("scripts", "runui  ·  package.json scripts")
            session.key("]")
            session.shot("makefile", "runui  ·  Makefile targets")
            session.key("]")
            session.shot("compose", "runui  ·  Compose services")
            session.key("]")
            session.shot("maven", "runui  ·  Maven builds")
            session.key("[", "[", "[")
            session.key("/", "w", "e", "b")
            session.wait("/web")
            session.shot("filter", "runui  ·  Filter commands")
            frames.append(picture(session.capture(), "runui  ·  Filter"))
            session.key("Escape")
            session.tmux("resize-window", "-t", session.target, "-y", "36")
            time.sleep(0.2)
            session.key("?")
            session.shot("keys", "runui  ·  All keys", 36)
            session.key("x", "a")
            session.tmux("resize-window", "-t", session.target, "-y", str(ROWS))
            time.sleep(0.2)
            session.shot("add-command", "runui  ·  Add your own command")
            session.key("Escape", "3", "s")
            session.wait("stopped after", 9)
            session.shot("stopped", "runui  ·  Stopped run")
            frames.append(picture(session.capture(), "runui  ·  Stopped"))
            session.key("q")
        finally:
            session.close()
            stop_demo(home)
        frames[0].save(
            OUT / "tour.gif", save_all=True, append_images=frames[1:],
            duration=[1300, 1900, 1300, 1500, 1900], loop=0, optimize=True,
        )
        print("docs/assets/tour.gif")


if __name__ == "__main__":
    main()
