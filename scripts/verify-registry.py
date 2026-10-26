#!/usr/bin/env python3
"""Verify registered search using real Git worktrees and private tmux.

Run after task build, or set RUNUI_BIN. Needs Git, tmux, Make, Python and
Pillow. All repositories, configuration and runs are temporary. PNG/ANSI
captures are retained in the printed temporary folder.
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
BIN = Path(os.environ.get('RUNUI_BIN', str(ROOT/'bin/runui'))).resolve()
CAPTURES = Path(tempfile.mkdtemp(prefix='runui-registry-captures-'))
print(f'Registry captures: {CAPTURES}', flush=True)

def wait(test, label, seconds=10):
    until = time.monotonic()+seconds
    while not test():
        if time.monotonic()>until:
            raise AssertionError(label)
        time.sleep(.08)

with tempfile.TemporaryDirectory(prefix='runui-registry-') as temporary:
    home = Path(temporary)
    env = {key:value for key,value in os.environ.items() if not key.startswith('GIT_')}
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(home/'config'), XDG_STATE_HOME=str(home/'state'),
               GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null', GIT_AUTHOR_NAME='Runui demo',
               GIT_AUTHOR_EMAIL='demo@example.invalid', GIT_COMMITTER_NAME='Runui demo',
               GIT_COMMITTER_EMAIL='demo@example.invalid', TERM='xterm-256color', COLORTERM='truecolor', SHELL='/bin/sh')
    env.pop('NO_COLOR',None)
    def run(*args):
        return subprocess.run(args,env=env,cwd=home,text=True,capture_output=True,check=True).stdout
    def git(directory,*args):
        return run('git','-C',str(directory),*args)
    def cli(*args):
        return json.loads(run(str(BIN),*args))

    roots = [home/'one/app',home/'two/app']
    for root in roots:
        root.mkdir(parents=True)
        git(root,'init','-q','-b','main')
        (root/'Makefile').write_text("test: ## Check this checkout\n\t@pwd > chosen.cwd; printf 'REGISTRY_READY\\n'; sleep 0.3\n")
        git(root,'add','Makefile')
        git(root,'commit','-q','-m','Demo commands')
    for branch, directory in [('feature-one',home/'worktrees/a/app'),('feature-two',home/'worktrees/b/app')]:
        directory.parent.mkdir(parents=True)
        git(roots[0],'worktree','add','-q','-b',branch,str(directory),'HEAD')
        roots.append(directory)
    registrations = [cli('projects','add',str(root),'--name','App','--json')['project'] for root in roots]
    missing = home/'missing/app'
    missing.mkdir(parents=True)
    cli('projects','add',str(missing),'--json')
    missing.rename(home/'unavailable')
    page = cli('commands','-C',str(roots[0]),'--scope','registered','--json','source:make test')
    assert len(page['commands']) == 4 and len({entry['id'] for entry in page['commands']}) == 4
    assert {entry['branch'] for entry in page['commands']} == {'main','feature-one','feature-two'}
    assert page['complete'] is False and any('unavailable' in item['message'] for item in page['diagnostics'])

    socket = str(home/'verify.sock')
    def tmux(*args,check=True):
        return subprocess.run(['tmux','-S',socket,'-f','/dev/null',*args],env=env,cwd=home,text=True,capture_output=True,check=check)
    def plain():
        return tmux('capture-pane','-p','-t','verify:0.0').stdout
    def key(*keys):
        for value in keys:
            tmux('send-keys','-t','verify:0.0',value)
            time.sleep(.16)
    def text(value):
        tmux('send-keys','-l','-t','verify:0.0',value)
        time.sleep(.2)
    def shows(value):
        wait(lambda:value in plain(), 'missing '+repr(value)+'\n'+plain())
    def metadata():
        return [json.loads(path.read_text()) for path in (home/'state/runui/runs').glob('*/run.json')]
    spec=importlib.util.spec_from_file_location('capture',ROOT/'scripts/capture-docs.py')
    capture=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(capture)
    def shot(name,cols=120,rows=36):
        screen=tmux('capture-pane','-p','-e','-t','verify:0.0').stdout
        (CAPTURES/(name+'.ansi')).write_text(screen)
        capture.COLS=cols
        capture.picture(screen,'runui · registered projects',rows).save(CAPTURES/(name+'.png'))
    values=[f'{name}={env[name]}' for name in ('HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','TERM','COLORTERM','SHELL')]
    command=shlex.join(['env','-u','NO_COLOR',*values,str(BIN),'-C',str(roots[0])])
    try:
        tmux('new-session','-d','-s','verify','-x','120','-y','36',command)
        tmux('set-option','-sg','escape-time','10')
        shows('[2]')
        key('C-p')
        text('source:make test')
        key('M-s','M-s')
        shows('Commands · Registered')
        shows('feature-one')
        shows('feature-two')
        assert not metadata()
        shot('registered-wide')
        key('F2')
        shows('Discovery diagnostics')
        shows('unavailable')
        shot('registered-diagnostics')
        key('F2')
        key('M-p')
        shows('Registered projects')
        text('feature-two')
        key('Down')
        shot('project-picker')
        tmux('resize-window','-t','verify:0','-x','60','-y','16')
        time.sleep(.3)
        key('C-v')
        shot('project-picker-small',60,16)
        key('C-v')
        tmux('resize-window','-t','verify:0','-x','120','-y','36')
        key('Enter')
        shows('Registered / App')
        shows('feature-two')
        key('Enter')
        shows('REGISTRY_READY')
        chosen=roots[-1]/'chosen.cwd'
        wait(chosen.exists,'selected worktree did not run')
        assert Path(chosen.read_text().strip()) == roots[-1].resolve()
        assert all(not (root/'chosen.cwd').exists() for root in roots[:-1])
        wait(lambda:len(metadata())==1 and all(run.get('ended') for run in metadata()),'first run did not finish')
        chosen.unlink()
        key('C-p','M-p')
        text('feature-two')
        key('Down','C-o')
        shows('Opened App')
        key('Enter')
        wait(lambda:len(metadata())==2 and chosen.exists(),'opening the project did not change the command context')
        assert Path(chosen.read_text().strip()) == roots[-1].resolve()
        assert all(not (root/'chosen.cwd').exists() for root in roots[:-1])
        wait(lambda:all(run.get('ended') for run in metadata()),'second run did not finish')
        key('C-p','M-p')
        text('feature-two')
        key('Down','M-d')
        wait(lambda:len(cli('projects','--json')['projects'])==4,'registration was not forgotten')
        assert (roots[-1]/'Makefile').exists() and len(metadata())==2
        assert all((home/'state/runui/runs'/run['id']/'output.log').exists() for run in metadata())
        print('real TUI: two repositories, two worktrees, duplicate commands, missing checkout, selection cwd, project open, forget preserves data',flush=True)
    except Exception:
        (CAPTURES/'failure.txt').write_text(plain())
        raise
    finally:
        tmux('kill-server',check=False)
