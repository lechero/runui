#!/usr/bin/env python3
"""Capture real runui screens for the README and HTML guide.

Requires Python 3.10+, Pillow, tmux, Task and ffmpeg. Everything runui writes lives in a temporary HOME;
the only files kept are screenshots, GIFs and MP4s in docs/assets. Run after `task build`:

    python3 scripts/capture-docs.py
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import json
import os
import re
import select
import shlex
import shutil
import signal
import subprocess
import tempfile
import time
import unicodedata
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "bin/runui"
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


def demo_env(home: Path) -> dict[str, str]:
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / "config"),
               XDG_STATE_HOME=str(home / "state"), PATH=f"{home / 'bin'}:/usr/bin:/bin",
               TERM="xterm-256color", COLORTERM="truecolor")
    env.pop("NO_COLOR", None)
    return env


class Session:
    def __init__(self, socket: Path, home: Path, app: Path):
        self.socket, self.home, self.app = socket, home, app
        self.target = "docs"
        self.environment = {}
        self.unset = []

    def tmux(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return run("tmux", "-S", str(self.socket), "-f", "/dev/null", *args, check=check)

    def start(self) -> None:
        env = demo_env(self.home)
        env.update(self.environment)
        values = [f"{key}={env[key]}" for key in
                  dict.fromkeys(("HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "PATH", "TERM", "COLORTERM", *self.environment))]
        unset = [part for name in self.unset for part in ('-u',name)]
        command = shlex.join(["env", "-u", "NO_COLOR", *unset, *values, str(BIN), "-C", str(self.app)])
        self.tmux("new-session", "-d", "-s", self.target, "-x", str(COLS), "-y", str(ROWS), command)
        self.wait("[2]", 8)

    def capture(self) -> str:
        return self.tmux("capture-pane", "-p", "-e", "-t", self.target).stdout

    def plain(self) -> str:
        return self.tmux("capture-pane", "-p", "-t", self.target).stdout

    def key(self, *keys: str) -> None:
        for key in keys:
            self.tmux("send-keys", "-t", self.target, key)
            time.sleep(0.16)

    def wait(self, needle: str, seconds: float = 5) -> None:
        wait_for(lambda: needle in self.plain(), f"runui never showed {needle!r}", seconds)

    def status(self) -> dict[str, int]:
        result = subprocess.run([str(BIN), "status"], env=demo_env(self.home),
                                capture_output=True, text=True, check=True)
        return {key: int(value) for key, value in (part.split("=") for part in result.stdout.split())}

    def expect_status(self, running: int, completed: int, errored: int) -> None:
        expected = dict(running=running, completed=completed, errored=errored)
        wait_for(lambda: self.status() == expected, f"expected {expected}; got {self.status()}")

    def shot(self, name: str, title: str, rows: int = ROWS) -> Image.Image:
        image = picture(self.capture(), title, rows)
        image.save(OUT / f"{name}.png", optimize=True)
        print(f"docs/assets/{name}.png", flush=True)
        return image

    def close(self) -> None:
        self.tmux("kill-server", check=False)


class Client:
    """Use the actual stdio MCP server; runs are never fabricated."""
    def __init__(self, session: Session):
        self.child = subprocess.Popen([str(BIN), "mcp", "-C", str(session.app)],
                                      env=demo_env(session.home), stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, text=True)
        self.seq = 0
        self.request("initialize", dict(protocolVersion="2025-11-25", capabilities={},
                     clientInfo=dict(name="docs-agent", version="1")))
        self.child.stdin.write(json.dumps(dict(jsonrpc="2.0", method="notifications/initialized")) + "\n")
        self.child.stdin.flush()

    def request(self, method: str, params: dict) -> dict:
        self.seq += 1
        self.child.stdin.write(json.dumps(dict(jsonrpc="2.0", id=self.seq, method=method, params=params)) + "\n")
        self.child.stdin.flush()
        if not select.select([self.child.stdout], [], [], 8)[0]:
            raise RuntimeError(f"MCP request timed out: {method}")
        reply = json.loads(self.child.stdout.readline())
        if reply.get("id") != self.seq or "error" in reply:
            raise RuntimeError(f"MCP request failed: {reply}")
        return reply["result"]

    def tool(self, tool_name: str, **arguments) -> dict:
        result = self.request("tools/call", dict(name=tool_name, arguments=arguments))
        if result.get("isError"):
            raise RuntimeError(f"{tool_name}: {result}")
        return result["structuredContent"]

    def start(self, name: str, command: str, directory: str = "") -> str:
        args = dict(name=name, command=command)
        if directory:
            args["directory"] = directory
        return self.tool("start_run", **args)["id"]

    def ended(self, ident: str, state: str = "exited") -> None:
        wait_for(lambda: self.tool("get_run", id=ident)["state"] == state, f"run {ident} did not become {state}")

    def close(self) -> None:
        if self.child.poll() is None:
            self.child.stdin.close()
            try:
                self.child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def wait_for(check, label: str, seconds: float = 8) -> None:
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if check():
            return
        time.sleep(0.08)
    raise RuntimeError(label)


def stop_demo(home: Path) -> None:
    """Stop only supervisors created inside the disposable home on failure."""
    for run_file in (home / "state/runui/runs").glob("*/run.json"):
        try:
            data = json.loads(run_file.read_text())
            supervisor = data.get("supervisor", 0)
            if data.get("ended") or not isinstance(supervisor, int) or supervisor <= 0:
                continue
            os.kill(supervisor, signal.SIGTERM)
            wait_for(lambda: json.loads(run_file.read_text()).get("ended"), "demo run did not stop", 7)
        except (OSError, ValueError):
            continue


@contextmanager
def demo():
    # A short socket path works on macOS and Linux, regardless of $TMPDIR.
    with tempfile.TemporaryDirectory(prefix="runui-docs-", dir="/tmp") as scratch:
        root = Path(scratch).resolve()
        home = root / "home"
        app = fixture(home)
        (home / "bin").mkdir()
        # Keep mise and unrelated tools/config out of the capture environment.
        (home / "bin/task").symlink_to(shutil.which("task"))
        session = Session(root / "tmux.sock", home, app)
        try:
            yield session
        finally:
            session.close()
            stop_demo(home)


class Tour:
    def __init__(self):
        self.frames: list[Image.Image] = []
        self.durations: list[int] = []

    def frame(self, session: Session, title: str, hold: int = 1100) -> None:
        self.frames.append(picture(session.capture(), title))
        self.durations.append(hold)

    def save(self, name: str) -> None:
        path = OUT / f"{name}.gif"
        self.frames[0].save(path, save_all=True, append_images=self.frames[1:],
                            duration=self.durations, loop=0, optimize=True, disposal=1)
        # The HTML guide offers native play/pause, a poster, and no autoplay.
        run("ffmpeg", "-v", "error", "-y", "-ignore_loop", "1", "-i", str(path),
            "-vf", "scale=1280:-2:flags=lanczos", "-c:v", "libx264", "-preset", "fast",
            "-crf", "24", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an",
            str(OUT / f"{name}.mp4"))
        print(f"docs/assets/{name}.gif + .mp4 ({len(self.frames)} frames)", flush=True)


def capture_tour() -> None:
    tour = Tour()
    with demo() as session:
        session.start()
        session.shot("project", "runui · Commands ready before you run anything")
        tour.frame(session, "1 / Discover tasks, scripts, targets, services and builds")
        session.key("m", "1")
        session.wait("★ preview")
        session.shot("favorites", "runui · Your favorites, per project")
        tour.frame(session, "2 / Star a command with m; find it in Favorites")
        session.key("Enter")
        session.wait("cycle 01")
        tour.frame(session, "3 / Press enter; follow the live output", 650)
        session.wait("cycle 02")
        session.shot("running", "runui · Live output, persistent supervisor")
        tour.frame(session, "The command runs in its own folder", 900)
        session.key("q")
        time.sleep(.3)
        session.start()
        session.wait("cycle 02")
        session.shot("runs", "runui · Reopened: the same run, still going")
        tour.frame(session, "4 / Reopen runui: the same process and output are still here", 1600)
        session.key("2", "]")
        session.shot("scripts", "runui · Root and apps/web package scripts")
        tour.frame(session, "5 / Browse commands from nested project folders")
        session.key("]")
        session.shot("makefile", "runui · Makefile targets")
        session.key("]")
        session.shot("compose", "runui · Compose actions and services")
        session.key("]")
        session.shot("maven", "runui · Maven builds and profiles")
        session.key("[", "[", "[", "/", "w", "e", "b")
        session.wait("/web")
        session.shot("filter", "runui · Filter by name, command or folder")
        tour.frame(session, "6 / Filter with /; every command runs where it belongs")
        session.key("Escape")
        session.tmux("resize-window", "-t", session.target, "-y", "36")
        time.sleep(.2)
        session.key("?")
        session.shot("keys", "runui · Every key, in the app", 36)
        session.key("x", "a")
        session.tmux("resize-window", "-t", session.target, "-y", str(ROWS))
        time.sleep(.2)
        session.shot("add-command", "runui · Save a command of your own")
        session.key("Escape", "3", "s")
        session.wait("stopped after", 9)
        session.shot("stopped", "runui · Stopped, with the output kept")
        tour.frame(session, "7 / Stop with s; the output stays available")
        session.key("4", "]")
        tour.frame(session, "8 / Log keeps finished attempts for your return", 1600)
    tour.save("tour")


def capture_log() -> None:
    tour = Tour()
    with demo() as session:
        other = session.home / "projects/site"
        other.mkdir()
        with Client(session) as client:
            client.start("preview", "sh scripts/preview.sh")
            ok = client.start("checks", "printf 'All checks passed\\n'")
            client.ended(ok)
            bad = client.start("lint", "printf 'Lint failed: fix the unused import\\n'; exit 2")
            client.ended(bad)
            remote = client.start("build", "printf 'Build failed: missing demo config\\n'; exit 7", str(other))
            client.ended(remote)
        session.start()
        session.expect_status(1, 1, 2)
        tour.frame(session, "1 / Runs and tmux count work from every folder")
        session.key("4")
        session.wait("Unread (2)")
        session.expect_status(1, 1, 1)
        session.shot("log", "runui · Log: unread results across projects")
        tour.frame(session, "2 / Open Log; viewing a failure clears only its notice", 1600)
        time.sleep(.8)
        session.expect_status(1, 1, 1)
        tour.frame(session, "The selected output stays; refreshing leaves other results unread")
        session.key("j")
        session.wait("Unread (1)")
        session.expect_status(1, 1, 0)
        tour.frame(session, "3 / View the next failure: its error count disappears")
        session.key("j")
        session.wait("Unread (0)")
        session.expect_status(1, 0, 0)
        tour.frame(session, "4 / Successes clear too; the running process stays counted", 1600)
        session.key("2", "4")
        session.wait("All caught up")
        tour.frame(session, "5 / All caught up: zero finished counters stay hidden")
        session.key("]")
        session.wait("site › build")
        session.shot("log-history", "runui · Log: reviewed history and preserved output")
        tour.frame(session, "6 / Switch to All with ]; every reviewed attempt is still here", 1700)
        assert all((session.home / "state/runui/runs" / ident / "output.log").exists()
                   for ident in (ok, bad, remote))
    tour.save("log-tour")


def capture_agents() -> None:
    tour = Tour()
    with demo() as session:
        session.start()
        tour.frame(session, "1 / Your terminal and agent share the same runs")
        with Client(session) as client:
            commands = client.tool("list_commands")["commands"]
            assert any(command["command"] == "task preview" for command in commands)
            ident = client.start("agent preview", "sh scripts/preview.sh")
            assert any(run["id"] == ident for run in client.tool("list_runs")["runs"])
        # Closing the MCP connection does not stop its supervisor.
        session.wait("agent preview")
        session.key("3", "]", "]")
        session.wait("by docs-agent")
        session.shot("agents", "runui · Agents: started through the real MCP server")
        tour.frame(session, "2 / start_run appears in Agents under the MCP client's name", 1600)
        session.wait("cycle 03")
        tour.frame(session, "3 / Disconnect the client: the process keeps running", 1500)
        with Client(session) as client:
            output = client.tool("read_output", id=ident, lines=20)
            assert "cycle" in output["output"]
            client.tool("stop_run", id=ident)
            client.ended(ident, "stopped")
            session.wait("stopped after")
            tour.frame(session, "4 / read_output and stop_run operate on that same process", 1500)
            check = client.start("agent checks", "printf 'Agent checks passed\\n'")
            client.ended(check)
            assert "Agent checks passed" in client.tool("read_output", id=check)["output"]
        session.expect_status(0, 1, 0)
        session.key("4")
        session.wait("Unread (0)")
        session.expect_status(0, 0, 0)
        tour.frame(session, "5 / Agent completions reach Log; viewing clears their notices", 1500)
        session.key("]")
        session.shot("agent-log", "runui · Agent results, ready to revisit in Log")
        tour.frame(session, "6 / Agent identity and output stay with the finished history", 1700)
    tour.save("agents-tour")


def capture_composer() -> None:
    tour = Tour()
    with demo() as session:
        (session.app / "composer.json").write_text(json.dumps({
            "scripts": {"check": ["@php tests/check.php", "@lint"],
                        "lint": "@php tools/lint.php",
                        "post-install-cmd": "@php tools/setup.php"},
            "scripts-descriptions": {"check": "Run the PHP checks and linter"},
            "scripts-aliases": {"check": ["verify"]},
        }, indent=2))
        session.start()
        session.key("C-p")
        session.tmux("send-keys", "-l", "-t", session.target, "source:composer verify")
        session.wait("aliases: verify")
        session.wait("php:")
        session.expect_status(0, 0, 0)
        session.shot("composer", "runui · Composer: find a canonical script by its alias")
        tour.frame(session, "1 / Search an alias; Composer keeps its arrays and helpers", 2200)
        session.key("C-u")
        session.tmux("send-keys", "-l", "-t", session.target, "source:composer kind:hook")
        session.wait("Lifecycle hook")
        tour.frame(session, "2 / Discover lifecycle hooks explicitly with kind:hook", 2200)
        session.key("M-i")
        session.wait("unavailable or unsupported")
        session.expect_status(0, 0, 0)
        assert not list((session.home / "state/runui/runs").glob("*/run.json"))
        tour.frame(session, "3 / Check tool readiness explicitly; no run or unread notice is created", 2600)
    tour.save("composer-tour")


def prepare_cargo_demo(session: Session) -> None:
    rustup = shutil.which('rustup')
    def installed(name):
        if rustup:
            return Path(subprocess.run([rustup,'which',name],env=dict(os.environ,RUSTUP_AUTO_INSTALL='0'),text=True,capture_output=True,check=True,timeout=10).stdout.strip()).resolve()
        return Path(shutil.which(name)).resolve()
    cargo,rustc = installed('cargo'),installed('rustc')
    app=session.app
    (app/'Cargo.toml').write_text("[workspace]\nresolver='3'\nmembers=['crates/*']\n")
    member=app/'crates/demo'
    (member/'src').mkdir(parents=True)
    (member/'Cargo.toml').write_text("[package]\nname='demo'\nversion='0.1.0'\nedition='2024'\n[features]\ndemo=[]\n[[bin]]\nname='demo'\nrequired-features=['demo']\n")
    (member/'src/main.rs').write_text('fn main(){println!("Cargo demo ready");println!("Runtime cwd: {}",std::env::current_dir().unwrap().display());}\n')
    session.unset=[key for key in os.environ if key.startswith(('CARGO_','RUST','MISE_'))]
    session.environment={'PATH':f"{session.home/'bin'}:{cargo.parent}:/usr/bin:/bin",'RUSTC':str(rustc),'CARGO_HOME':str(session.home/'cargo-home'),'CARGO_TARGET_DIR':str(session.home/'target'),'RUSTUP_AUTO_INSTALL':'0','CARGO_NET_OFFLINE':'true'}
    env=demo_env(session.home);env.update(session.environment)
    for key in session.unset:
        if key not in session.environment:env.pop(key,None)
    subprocess.run([str(cargo),'generate-lockfile','--offline'],cwd=app,env=env,capture_output=True,text=True,check=True,timeout=20)


def capture_cargo() -> None:
    tour = Tour()
    with demo() as session:
        prepare_cargo_demo(session)
        session.start()
        session.key('C-p')
        session.tmux('send-keys','-l','-t',session.target,'source:cargo check')
        session.wait('check (default members)')
        session.shot('cargo','runui · Cargo: default members and explicit workspace actions')
        tour.frame(session,'1 / Cargo defaults stay distinct from explicit all-member actions',2200)
        session.key('C-u')
        session.tmux('send-keys','-l','-t',session.target,'source:cargo run --bin demo')
        session.wait('required features enabled: demo')
        tour.frame(session,'2 / Choose a binary; its required features and runtime cwd are explicit',2400)
        session.key('M-i')
        session.wait('Selected definition matches native Cargo metadata.')
        session.key('C-v','NPage','NPage')
        session.wait('metadata verified')
        session.expect_status(0,0,0)
        assert not list((session.home/'state/runui/runs').glob('*/run.json'))
        tour.frame(session,'3 / Explicit offline metadata uses installed tools and creates no run or unread count',2600)
        session.key('C-v','Enter')
        session.wait('Cargo demo ready',30)
        session.wait('exited',10)
        session.expect_status(0,0,0)
        tour.frame(session,'4 / Run through the real supervisor and keep the native Cargo output',2400)
        assert list((session.home/'state/runui/runs').glob('*/run.json'))
    tour.save('cargo-tour')


def capture_cargo_aliases() -> None:
    tour = Tour()
    with demo() as session:
        prepare_cargo_demo(session)
        config=session.app/'.cargo/config.toml';config.parent.mkdir()
        config.write_text('[alias]\ndemo=["run","--offline","--locked","--package","demo","--bin","demo","--features","demo"]\n')
        global_config=session.home/'cargo-home/config.toml';global_config.parent.mkdir(exist_ok=True)
        global_config.write_text('[alias]\nverify-global="check --workspace --offline --locked"\n')
        session.start()
        session.key('C-p')
        session.tmux('send-keys','-l','-t',session.target,'source:cargo kind:alias')
        session.wait('Cargo alias · project context')
        assert 'verify-global' not in session.plain()
        assert not list((session.home/'state/runui/runs').glob('*/run.json'))
        session.shot('cargo-aliases','runui · Cargo aliases: native expansion, explicit context')
        tour.frame(session,'1 / Find project aliases and inspect their native configuration',2400)
        session.key('M-g','Down')
        session.wait('Cargo alias · global context')
        session.wait('verify-global')
        tour.frame(session,'2 / Opt into global aliases and see where each declaration comes from',2600)
        session.key('M-g')
        session.wait('Cargo alias · project context')
        session.key('Enter')
        session.wait('Cargo demo ready',30)
        session.wait('exited',10)
        session.expect_status(0,0,0)
        tour.frame(session,'3 / Run the selected alias through Cargo and return to its retained output',2400)
    tour.save('cargo-aliases-tour')


def main() -> None:
    global BIN
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=BIN, help="Rust binary to capture (default: bin/runui)")
    parser.add_argument("--only", choices=["composer","cargo","cargo-aliases"], help="Refresh one focused tour")
    args = parser.parse_args()
    BIN = args.binary.expanduser().resolve()
    if not BIN.is_file():
        raise SystemExit("Build first with `task build`, or pass --binary /path/to/runui.")
    for name in ("tmux", "task", "ffmpeg"):
        if shutil.which(name) is None:
            raise SystemExit(f"Install {name} to regenerate the complete media set.")
    OUT.mkdir(parents=True, exist_ok=True)
    if args.only == "composer":
        capture_composer()
        return
    if args.only == "cargo":
        capture_cargo()
        return
    if args.only == "cargo-aliases":
        capture_cargo_aliases()
        return
    capture_tour()
    capture_log()
    capture_agents()
    capture_composer()
    capture_cargo()
    capture_cargo_aliases()
    print("Captured all screenshots, 6 GIFs and 6 videos using isolated real runs.", flush=True)


if __name__ == "__main__":
    main()
