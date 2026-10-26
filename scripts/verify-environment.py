#!/usr/bin/env python3
"""Exercise real mise installed-tool inspection without downloading tools.

Uses the installed mise and Rust binaries in disposable HOME/XDG/mise/runui
directories. Optional PHP/Composer checks use the existing CI PHAR. Native
inspection must not execute project scripts, hooks or environment expressions.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get('RUNUI_BIN', ROOT/'bin/runui')).resolve()
mise_binary = os.environ.get('RUNUI_MISE_BIN') or shutil.which('mise')
assert mise_binary, 'Installed mise 2026.10.19 or newer is required; this fixture installs no tools'
MISE = Path(mise_binary).resolve()
PHAR = os.environ.get('RUNUI_COMPOSER_PHAR')
CAPTURES = os.environ.get('RUNUI_ENV_CAPTURES')
TMUX = shutil.which('tmux')

def installed(name):
    rustup = shutil.which('rustup')
    if rustup:
        result = subprocess.run([rustup, 'which', name], env=dict(os.environ, RUSTUP_AUTO_INSTALL='0'),
                                capture_output=True, text=True, check=True, timeout=10)
        return Path(result.stdout.strip()).resolve()
    return Path(shutil.which(name)).resolve()

CARGO, RUSTC = installed('cargo'), installed('rustc')
VERSION = subprocess.check_output([CARGO, '--version'], text=True).split()[1]

with tempfile.TemporaryDirectory(prefix='runui-env-',dir='/tmp' if CAPTURES else None) as temporary:
    home = Path(temporary).resolve()
    project = home/"workspace 'with spaces' ☕"
    (project/'.git').mkdir(parents=True)
    (project/'src').mkdir()
    (project/'src/lib.rs').write_text('pub fn check() {}\n')
    (project/'build.rs').write_text('fn main(){std::fs::write("build-must-not-run", "built").unwrap();}\n')
    (project/'Cargo.toml').write_text("[package]\nname='environment_fixture'\nversion='0.1.0'\nedition='2024'\n")
    (project/'.cargo').mkdir()
    (project/'.cargo/config.toml').write_text('[alias]\nfixture = "version"\n')
    (project/'composer.json').write_text(json.dumps({'scripts': {'check': '@php never.php'}}))
    (project/'never.php').write_text('<?php file_put_contents("script-must-not-run", "bad");')
    bin_dir = home/'bin'
    bin_dir.mkdir()
    (bin_dir/'mise').symlink_to(MISE)
    # Mise's Rust backend needs only existing installation directories here;
    # it must not enter rustup's install-on-demand path during inspection.
    tool_bin = home/'mise-data/installs/rust'/VERSION/'bin'
    tool_bin.mkdir(parents=True)
    (tool_bin/'cargo').symlink_to(CARGO)
    (tool_bin/'rustc').symlink_to(RUSTC)
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(('MISE_', 'CARGO_', 'RUST', 'XDG_', 'GIT_'))}
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(home/'config'), XDG_DATA_HOME=str(home/'data'),
               XDG_STATE_HOME=str(home/'state'), XDG_CACHE_HOME=str(home/'cache'),
               MISE_DATA_DIR=str(home/'mise-data'), MISE_STATE_DIR=str(home/'mise-state'),
               MISE_CACHE_DIR=str(home/'mise-cache'), MISE_GLOBAL_CONFIG_FILE=str(home/'global.toml'),
               MISE_SYSTEM_CONFIG_FILE=str(home/'system.toml'),
               CARGO_HOME=str(home/'cargo-home'), CARGO_TARGET_DIR=str(home/'target'),
               PATH=os.pathsep.join([str(bin_dir), str(CARGO.parent), '/usr/bin', '/bin']),
               RUSTUP_AUTO_INSTALL='0', NO_COLOR='1', GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')

    def run(*args, check=True):
        result = subprocess.run(list(map(str,args)), cwd=project, env=env, text=True,
                                capture_output=True, timeout=60)
        if check:
            assert result.returncode == 0, (args, result.returncode, result.stdout, result.stderr)
        return result

    def cli(*args):
        return json.loads(run(BIN, *args).stdout)

    def definition(source):
        return next(row for row in cli('commands','--json')['commands']
                    if row['source'] == source and row['cargo_alias'] is None and row['kind'] == 'command')

    def inspect(row):
        return cli('inspect','--native','--json',row['id'])['native_inspection']

    def no_execution():
        for name in ['build-must-not-run','script-must-not-run','hook-must-not-run','env-must-not-run']:
            assert not (project/name).exists(), name
        assert not list((home/'state/runui/runs').glob('*/meta.json'))
        assert not (home/'state/runui/catalog-v1.json').exists()

    def config(version=VERSION):
        (project/'mise.toml').write_text(f'''[tools]
rust={json.dumps(version)}
[env]
PRIVATE_VALUE="secret-must-not-appear"
CARGO_HOME="/intentionally-invalid-project-env"
CHECK="{{{{exec(command='touch env-must-not-run')}}}}"
[hooks]
enter="touch hook-must-not-run"
[settings]
auto_install=true
''')

    def capture(label, expected):
        if not CAPTURES:
            return
        import importlib.util
        import shlex
        spec = importlib.util.spec_from_file_location('capture_docs', ROOT/'scripts/capture-docs.py')
        renderer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(renderer)
        out = Path(CAPTURES)
        out.mkdir(parents=True, exist_ok=True)
        sockets = tempfile.TemporaryDirectory(prefix='rue-',dir='/tmp')
        socket = str(Path(sockets.name)/'tmux.sock')
        display_env=dict(env, SHELL='/bin/sh',TERM='xterm-256color',COLORTERM='truecolor')
        display_env.pop('NO_COLOR',None)
        def tmux(*args, check=True):
            result = subprocess.run([TMUX,'-S',socket,'-f','/dev/null',*args],cwd=project,env=display_env,
                                    text=True,capture_output=True)
            if check:
                assert result.returncode==0,(args,result.stdout,result.stderr)
            return result
        def plain():
            return tmux('capture-pane','-p','-t','verify:0.0').stdout
        def wait_for(needle):
            until = time.monotonic()+15
            while needle not in plain():
                assert time.monotonic()<until, (needle,plain())
                time.sleep(.08)
        try:
            command = shlex.join(['env','TERM=xterm-256color','COLORTERM=truecolor',str(BIN),'-C',str(project)])
            tmux('new-session','-d','-s','verify','-x','120','-y','42',command)
            tmux('set-option','-sg','escape-time','10')
            wait_for('[2]')
            tmux('send-keys','-t','verify:0.0','C-p')
            time.sleep(.15)
            tmux('send-keys','-l','-t','verify:0.0','source:cargo check')
            wait_for('Static Cargo discovery')
            tmux('send-keys','-t','verify:0.0','M-i')
            tmux('send-keys','-t','verify:0.0','C-v')
            wait_for(expected)
            for cols,rows,suffix in [(120,42,'wide'),(70,32,'small')]:
                tmux('resize-window','-t','verify:0','-x',str(cols),'-y',str(rows))
                time.sleep(.25)
                # Full details can scroll at narrow widths; keep the context
                # limitation and native outcome visible in the capture.
                if expected[:35] not in plain():
                    tmux('send-keys','-t','verify:0.0','NPAGE')
                    time.sleep(.2)
                screen = tmux('capture-pane','-p','-e','-t','verify:0.0').stdout
                (out/f'{label}-{suffix}.ansi').write_text(screen)
                renderer.COLS=cols
                renderer.picture(screen,'runui · installed project tools',rows).save(out/f'{label}-{suffix}.png')
        finally:
            tmux('kill-server',check=False)
            sockets.cleanup()
        no_execution()

    run(CARGO, 'generate-lockfile', '--offline')
    lock = (project/'Cargo.lock').read_bytes()
    config()
    entry = definition('cargo')
    assert entry['availability'] == 'project_environment'
    no_execution()
    report = inspect(entry)
    assert report['ready'], report
    assert report['cargo']['definition_matches']
    assert report['environment']['scope'] == 'installed_tools'
    assert report['environment']['launch_environment_verified'] is False
    assert report['environment']['tools'][0]['version'] == VERSION
    assert 'secret-must-not-appear' not in json.dumps(report)
    mise_probes = [probe for probe in report['checked'] if probe['tool'].startswith('mise ')]
    assert len(mise_probes) == 3
    assert all(not p['output']['stdout'] and not p['output']['stderr'] for p in mise_probes)
    assert (project/'Cargo.lock').read_bytes() == lock
    no_execution()
    print('mise: installed Rust selected; safe environment/hooks/compiler/build scripts not evaluated', flush=True)
    capture('runui-environment-installed', 'full launch environment unchecked')

    alias = next(row for row in cli('commands','--json')['commands'] if row['cargo_alias'])
    skipped = inspect(alias)
    assert not skipped['checked'] and 'does not execute Cargo aliases' in skipped['skipped']

    config('99.99.99')
    report = inspect(definition('cargo'))
    assert not report['ready'] and 'not installed' in report['skipped'], report
    assert not (home/'mise-data/installs/rust/99.99.99').exists()
    no_execution()
    print('mise: missing selected tool stays unavailable despite system Cargo; no provisioning', flush=True)
    capture('runui-environment-missing', 'not installed')

    (project/'mise.toml').write_text('[env]\nPRIVATE_VALUE="secret-must-not-appear" bad syntax\n')
    report = inspect(definition('cargo'))
    assert not report['ready'] and 'secret-must-not-appear' not in json.dumps(report), report
    no_execution()
    print('mise: private values withheld from successful and failed native reports', flush=True)

    if PHAR:
        config()
        php = Path(shutil.which('php')).resolve()
        (bin_dir/'php').symlink_to(php)
        shutil.copyfile(PHAR, project/'composer.phar')
        (project/'.runui.json').write_text(json.dumps({'version': 1, 'composer': {'phar': 'composer.phar'}}))
        report = inspect(definition('composer'))
        assert report['ready'], report
        assert [p['tool'] for p in report['checked']][-2:] == ['php', 'composer']
        no_execution()
        print('mise: PHP/Composer version checks use the safe project tool context without scripts/plugins', flush=True)
    else:
        print('mise: real PHP/Composer exercised by the pinned Composer CI jobs', flush=True)
