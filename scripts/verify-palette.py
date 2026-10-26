#!/usr/bin/env python3
"""Real palette smoke test in private tmux servers and a temporary HOME.

Run after task build, or set RUNUI_BIN. Needs tmux, make, Python and Pillow.
Captures are retained in a temporary folder printed by this script.
"""

import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get('RUNUI_BIN', str(ROOT / 'bin/runui'))).resolve()
CAPTURES = Path(tempfile.mkdtemp(prefix='runui-palette-captures-'))
print(f'Palette captures: {CAPTURES}', flush=True)

def wait(test, label, seconds=8):
    until = time.monotonic() + seconds
    while not test():
        if time.monotonic() > until:
            raise AssertionError(label)
        time.sleep(.08)

with tempfile.TemporaryDirectory(prefix='runui-palette-') as temporary:
    home = Path(temporary)
    app = home / 'project #(touch SHOULD_NOT_EXIST)'
    (app / '.git').mkdir(parents=True)
    (app / '.tasks').mkdir()
    (app / 'Taskfile.yml').write_text("version: '3'\nincludes:\n  docs: .tasks/shared.yml\n  web: .tasks/shared.yml\n")
    (app / '.tasks/shared.yml').write_text("version: '3'\ntasks:\n  build:\n    desc: Build documentation\n    cmds: [echo docs]\n")
    (app / 'Cargo.toml').write_text("[workspace]\nmembers=['crates/*']\n")
    (app / 'crates/engine/src').mkdir(parents=True)
    (app / 'crates/engine/Cargo.toml').write_text("[package]\nname='engine'\nedition='2024'\n[features]\ngated=[]\n[[bin]]\nname='worker'\npath='src/main.rs'\nrequired-features=['gated']\n")
    (app / 'crates/engine/src/main.rs').write_text("fn main() {}\n")
    (app / 'package.json').write_text('{"scripts":{"dev":"echo dev"},"workspaces":["packages/**"]}')
    (app / 'composer.json').write_text(json.dumps({'scripts':{'check':'@php never.php','post-install-cmd':'@php never.php'},'scripts-aliases':{'check':['verify']},'scripts-descriptions':{'check':'Run the PHP checks with native Composer'}}))
    web = app / 'packages/web'
    web.mkdir(parents=True)
    (web / 'package.json').write_text('{"scripts":{"web":"true"}}')
    (app / 'Makefile').write_text("check: ## Check palette launches\n\t@printf 'PALETTE_READY\\n'; sleep 1\nhandoff:\n\t@printf 'TERMINAL_READY\\n' > handoff.marker\n")
    launcher = home / 'launcher'
    launcher.write_text('#!/bin/sh\nprintf "%s\\0%s\\0" "$1" "$2" >> "$HOME/handoffs"\n')
    launcher.chmod(0o700)
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home/'config'),
               XDG_STATE_HOME=str(home/'state'), TERM='xterm-256color',
               COLORTERM='truecolor', SHELL='/bin/sh', EDITOR='/usr/bin/true', RUNUI_TERMINAL=str(launcher))
    env.pop('NO_COLOR', None)
    spec = importlib.util.spec_from_file_location('capture', ROOT/'scripts/capture-docs.py')
    renderer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(renderer)

    for native in (False, True):
        socket = str(home / ('native.sock' if native else 'custom.sock'))
        if native:
            env.pop('RUNUI_TERMINAL')
        def tmux(*args, check=True):
            return subprocess.run(['tmux','-S',socket,'-f','/dev/null',*args],
                                  text=True,capture_output=True,check=check,env=env,cwd=home)
        def plain():
            return tmux('capture-pane','-p','-t','verify:0.0').stdout
        def key(*keys):
            for value in keys:
                tmux('send-keys','-t','verify:0.0',value)
                time.sleep(.16)
        def text(value):
            tmux('send-keys','-l','-t','verify:0.0',value)
            time.sleep(.2)
        def shows(needle):
            wait(lambda: needle in plain(), needle + '\n' + plain())
        def metadata():
            return list((home/'state/runui/runs').glob('*/run.json'))
        def shot(name, cols, rows):
            screen = tmux('capture-pane','-p','-e','-t','verify:0.0').stdout
            (CAPTURES / (name+'.ansi')).write_text(screen)
            renderer.COLS = cols
            renderer.picture(screen, 'runui · shared command palette', rows).save(CAPTURES / (name+'.png'))
        values = [f'{name}={env[name]}' for name in ('HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','TERM','COLORTERM','SHELL','EDITOR')]
        if not native:
            values.append('RUNUI_TERMINAL='+str(launcher))
        command = shlex.join(['env','-u','NO_COLOR',*values,str(BIN),'-C',str(web if native else app)])
        try:
            tmux('new-session','-d','-s','verify','-x','120','-y','36',command)
            tmux('set-option','-sg','escape-time','10')
            shows('[2]')
            if not native:
                key('C-p')
                text('source:composer verify')
                shows('aliases: verify')
                shows('composer:')
                shows('php:')
                assert 'post-install-cmd' not in plain()
                assert not metadata()
                shot('runui-composer-wide',120,36)
                key('C-u')
                text('source:composer kind:hook')
                shows('post-install-cmd')
                shows('Lifecycle hook')
                shot('runui-composer-hooks',120,36)
                key('M-i')
                wait(lambda: any(message in plain() for message in ('wrapper remains unverified', 'version verified', 'unavailable or unsupported')), 'explicit native Composer check did not complete')
                assert not metadata(), 'native inspection created an unread run'
                shot('runui-composer-native',120,36)
                key('Escape','C-p')
                text('source:cargo dir:crates/engine run --bin')
                # Query edits preserve the previously selected ID when it is
                # still a fuzzy match. Inspect the highest ranked run target.
                key('PPage')
                shows('required features enabled: gated')
                shows('manifest folder:')
                assert not metadata()
                assert not (app/'Cargo.lock').exists() and not (app/'target').exists()
                shot('runui-cargo-wide',120,36)
                key('M-i')
                wait(lambda: any(message in plain() for message in ('existing Cargo.lock','wrapper remains unverified')), 'Cargo native fallback did not complete')
                assert not metadata(), 'Cargo inspection created a run'
                assert not (app/'Cargo.lock').exists()
                tmux('resize-window','-t','verify:0','-x','60','-y','16')
                key('C-v')
                shows('cwd:')
                shot('runui-cargo-small',60,16)
                tmux('resize-window','-t','verify:0','-x','120','-y','36')
                key('C-v','Escape')
                key('/')
                text('docs:')
                key('Enter')
            key('C-p')
            shows('Commands · Project')
            text('source:make handoff' if native else 'source:make check')
            if native:
                shows('No matches')
                key('M-s')
                shows('Commands · Workspace')
            shows('$ make handoff' if native else '$ make check')
            if native:
                shot('runui-workspace-wide',120,36)
                key('C-t')
                wait(lambda: (app/'handoff.marker').exists(), 'native terminal command did not run')
                assert 'TERMINAL_READY' in (app/'handoff.marker').read_text()
                assert len(tmux('list-windows','-t','verify','-F','#{window_id}').stdout.splitlines()) == 2
                assert not (home/'SHOULD_NOT_EXIST').exists()
                print('native tmux handoff: exact cwd, literal format-like directory, second window', flush=True)
                continue
            assert not metadata()
            key('M-m')
            shows('Saved as a favorite')
            assert (home/'config/runui/favorites.json').exists()
            key('C-o')
            wait(lambda: (home/'handoffs').exists(), 'source handoff absent')
            key('C-t')
            wait(lambda: len((home/'handoffs').read_bytes().split(b'\0')) >= 5, 'terminal handoff absent')
            handoffs = (home/'handoffs').read_bytes().split(b'\0')
            assert handoffs[0].decode() == str(app.resolve())
            assert handoffs[1].decode().startswith('/usr/bin/true ')
            assert handoffs[2] == handoffs[0] and handoffs[3] == b'make check'
            shot('runui-palette-wide',120,36)
            tmux('resize-window','-t','verify:0','-x','60','-y','16')
            time.sleep(.4)
            key('C-v')
            shows('cwd:')
            shot('runui-palette-small',60,16)
            tmux('resize-window','-t','verify:0','-x','120','-y','36')
            key('C-v','Escape')
            wait(lambda: 'Commands · Project' not in plain(), 'palette did not close')
            assert 'docs:build' in plain() and 'web:build' not in plain(), plain()
            key('C-p')
            text('source:make check')
            shows('$ make check')
            key('Enter')
            shows('PALETTE_READY')
            wait(lambda: len(metadata()) == 1, 'launch not persisted')
            time.sleep(1.3)
            key('C-p')
            shows('Commands · Project')
            key('Tab')
            shows('Recent invocations')
            text('source:make check')
            shows('$ make check')
            shows('1 launches')
            key('M-u')
            shows('Current definition and default inputs')
            key('Escape')
            wait(lambda: 'Commands · Project' not in plain(), 'palette did not close')
            state=json.loads((home/'state/runui/runs/catalog-v1.json').read_text())
            assert len(state['recent']) == 1
            print('real TUI: palette, favorite, source/terminal handoff, wide/narrow, supervised launch, recent/current',flush=True)
        except Exception:
            (CAPTURES / 'failure.txt').write_text(plain())
            raise
        finally:
            tmux('kill-server',check=False)
