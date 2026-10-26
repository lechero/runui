#!/usr/bin/env python3
"""Exercise private shell recall, real native formats and insertion in a private TTY.

All history, configuration, runs and shell startup files are disposable. The
fixture never reads personal history or installs tools. Needs fish, Zsh, Bash
4+, tmux and a built Rust runui; captures optionally need Pillow.
"""
import base64
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time

ROOT=Path(__file__).resolve().parents[1]
BIN=Path(os.environ.get('RUNUI_BIN',ROOT/'bin/runui')).resolve()
FISH=Path(os.environ.get('RUNUI_FISH_BIN') or shutil.which('fish')).resolve()
ZSH=Path(os.environ.get('RUNUI_ZSH_BIN') or shutil.which('zsh')).resolve()
BASH=Path(os.environ.get('RUNUI_BASH_BIN') or shutil.which('bash')).resolve()
TMUX=shutil.which('tmux')
CAPTURES=Path(os.environ['RUNUI_RECALL_CAPTURES']) if 'RUNUI_RECALL_CAPTURES' in os.environ else None

def wait(predicate,label):
    until=time.monotonic()+15
    while not predicate():
        if time.monotonic()>=until:
            if 'tmux' in globals():
                pane=tmux('display-message','-p','-t','verify:0.0','#{pane_pid}',check=False).stdout.strip()
                snapshot=subprocess.run(['ps','-A','-o','pid=,ppid=,pgid=,tpgid=,stat=,comm='],text=True,capture_output=True)
                descendants={int(pane)} if pane.isdigit() else set()
                rows=[line.split(None,5) for line in snapshot.stdout.splitlines()]
                for _ in range(8):
                    descendants.update(int(row[0]) for row in rows if len(row)==6 and int(row[1]) in descendants)
                print('Private TTY process states:',[row for row in rows if len(row)==6 and int(row[0]) in descendants],flush=True)
            raise AssertionError(label() if callable(label) else label)
        time.sleep(.08)

with tempfile.TemporaryDirectory(prefix='runui-recall-',dir='/tmp') as temporary:
    home=Path(temporary).resolve();project=home/"project 'quoted' ☕";project.mkdir()
    (project/'Makefile').write_text('check:\n\t@printf PROJECT_OK\\n\n')
    (project/'.git').mkdir()
    bin_dir=home/'bin';bin_dir.mkdir()
    for name,path in [('runui',BIN),('fish',FISH),('zsh',ZSH),('bash',BASH)]: (bin_dir/name).symlink_to(path)
    env={k:v for k,v in os.environ.items() if not k.startswith(('XDG_','MISE_','CARGO_','RUST','fish_','GIT_','HIST','BASH_FUNC_')) and k not in ('BASH_ENV','ENV','SHELLOPTS','BASHOPTS')}
    env.update(HOME=str(home),XDG_DATA_HOME=str(home/'data'),XDG_CONFIG_HOME=str(home/'config'),
               XDG_STATE_HOME=str(home/'state'),XDG_CACHE_HOME=str(home/'cache'),
               PATH=os.pathsep.join([str(bin_dir),'/usr/bin','/bin']),SHELL=str(FISH),
               TERM='xterm-256color',COLORTERM='truecolor',HISTFILE=str(home/'.bash_history'),
               ZDOTDIR=str(home),fish_history='fish',GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL='/dev/null')
    env.pop('NO_COLOR',None)
    fish_file=home/'data/fish/fish_history';fish_file.parent.mkdir(parents=True)
    marker=project/'HISTORY_MUST_NOT_RUN'
    commands=['echo PRIVATE_RECALL_SENTINEL', 'begin\n printf "%s\\n" "Unicode ☕"\nend',
              'echo literal\\path $(touch HISTORY_MUST_NOT_RUN)', '-leading option', 'printf tail\n\n',
              'begin\n printf "%s\\n" SHELL_REPLAY_READY\n pwd\nend']
    def fish_bytes(items):
        return ''.join('- cmd: '+s.replace('\\','\\\\').replace('\n','\\n')+'\n  when: '+str(1700000000+i)+'\n  paths:\n    - /not/the/cwd\n' for i,s in enumerate(items)).encode()
    original=fish_bytes(commands);fish_file.write_bytes(original)

    def run(*args,check=True):
        result=subprocess.run(list(map(str,args)),cwd=project,env=env,capture_output=True,timeout=30)
        if check: assert result.returncode==0,(args,result.returncode,result.stdout,result.stderr)
        return result
    def cli(*args):return json.loads(run(BIN,*args).stdout)
    def no_side_effects():
        assert not marker.exists()
        assert not list((home/'state/runui/runs').glob('*/run.json'))
        assert not (home/'state/runui/runs/catalog-v1.json').exists()
        assert not (home/'config/runui/favorites.json').exists()

    native=run(FISH,'--no-config','-c','builtin history search --null --max 20').stdout.split(b'\0')[:-1]
    assert native==[s.encode() for s in reversed(commands)],native
    page=cli('find','--mode','shell','--shell','fish','--json')
    assert [r['command'] for r in page['history']]==list(reversed(commands))
    assert all(r['directory'] is None and r['exit_code'] is None for r in page['history'])
    assert fish_file.read_bytes()==original
    assert cli('commands','--json','PRIVATE_RECALL_SENTINEL')['commands']==[]
    assert run(BIN,'find','--mode','shell','--json',check=False).returncode==2
    assert run(BIN,'find','--mode','shell','--shell','fish','--scope','workspace',check=False).returncode==2
    disabled=subprocess.run([str(BIN),'find','--mode','shell','--shell','fish','--json'],cwd=project,
                            env=dict(env,fish_history=''),capture_output=True,timeout=10)
    disabled_page=json.loads(disabled.stdout)
    assert disabled.returncode==0 and disabled_page['history']==[] and disabled_page['bytes_read']==0
    assert any('disabled/private' in note for note in disabled_page['diagnostics'])
    named=fish_file.with_name('private-fixture_history');named.write_bytes(fish_bytes(['echo NAMED_SESSION']))
    selected=subprocess.run([str(BIN),'find','--mode','shell','--shell','fish','--json'],cwd=project,
                            env=dict(env,fish_history='private-fixture'),capture_output=True,timeout=10)
    assert selected.returncode==0 and [row['command'] for row in json.loads(selected.stdout)['history']]==['echo NAMED_SESSION']
    for size in [1,500]:
        many=home/'many-fish';many.write_bytes(fish_bytes(['echo candidate '+str(i) for i in range(520)]))
        page=cli('find','--mode','shell','--shell','fish','--history-file',many,'--json','--limit',size)
        assert len(page['history'])==size and page['next_offset']==size,page
    no_side_effects()
    print('private recall: native fish, exact multiline/Unicode/escapes, unknown cwd, pagination, no catalog or state import',flush=True)

    zsh_commands=['echo Unicode ☕ café', 'printf "%s\\n" "literal\\path"','begin\n echo middle\nend','echo terminal\\',': literal prefix','printf tail\n\n']
    zsh_file=home/'.zsh_history'
    script='HISTFILE=$TARGET_HISTORY; HISTSIZE=100; SAVEHIST=100; setopt EXTENDED_HISTORY; for c in "$@"; do print -sr -- "$c"; done; fc -W "$HISTFILE"'
    result=subprocess.run([str(ZSH),'-f','-i','-c',script,'zsh',*zsh_commands],cwd=project,
                          env=dict(env,TARGET_HISTORY=str(zsh_file)),capture_output=True,timeout=20)
    assert result.returncode==0,result.stderr
    before=zsh_file.read_bytes()
    page=cli('find','--mode','shell','--shell','zsh','--history-file',zsh_file,'--json')
    assert [r['command'] for r in page['history']]==list(reversed(zsh_commands)),page
    assert zsh_file.read_bytes()==before
    bash_file=home/'.bash_history';bash_commands=['echo Unicode ☕ café',"printf 'first\nsecond'",'echo $(touch HISTORY_MUST_NOT_RUN)']
    bash_file.write_text(''.join('#'+str(1700000100+i)+'\n'+s+'\n' for i,s in enumerate(bash_commands)))
    for i,command in enumerate(bash_commands):
        native=run(BASH,'--noprofile','--norc','-c','HISTTIMEFORMAT="%s "; history -c; history -r "$HISTFILE"; history -p "!'+str(i+1)+'"').stdout
        assert native==command.encode()+b'\n',(native,command)
    page=cli('find','--mode','shell','--shell','bash','--history-file',bash_file,'--json')
    assert [r['command'] for r in page['history']]==list(reversed(bash_commands)),page
    no_side_effects()
    print('private recall: real Zsh extended/metafied/multiline/trailing-backslash format and native timestamped Bash',flush=True)

    # MCP has no path into the local recall module, even after a local query.
    client=subprocess.Popen([str(BIN),'mcp','-C',str(project)],env=env,cwd=project,stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
    def request(i,method,params):
        client.stdin.write(json.dumps({'jsonrpc':'2.0','id':i,'method':method,'params':params})+'\n');client.stdin.flush()
        return json.loads(client.stdout.readline())
    request(1,'initialize',{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'recall-fixture','version':'1'}})
    client.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n');client.stdin.flush()
    reply=request(2,'tools/call',{'name':'list_commands','arguments':{'query':'PRIVATE_RECALL_SENTINEL'}})
    assert reply.get('result',{}).get('structuredContent',{}).get('commands')==[],reply
    client.stdin.close();client.wait(timeout=10);no_side_effects()

    socket=str(home/'tmux.sock')
    def tmux(*args,check=True):
        result=subprocess.run([TMUX,'-S',socket,'-f','/dev/null',*args],env=dict(env,SHELL='/bin/sh'),cwd=project,text=True,capture_output=True)
        if check:assert result.returncode==0,(args,result.stdout,result.stderr)
        return result
    def screen():return tmux('capture-pane','-p','-t','verify:0.0').stdout
    def key(*keys):
        for value in keys:tmux('send-keys','-t','verify:0.0',value);time.sleep(.14)
    def text(value):tmux('send-keys','-l','-t','verify:0.0',value);time.sleep(.15)
    def shows(value):wait(lambda:value in screen(),lambda:value+'\n'+screen())
    def shot(name):
        if not CAPTURES:return
        spec=importlib.util.spec_from_file_location('capture_docs',ROOT/'scripts/capture-docs.py');renderer=importlib.util.module_from_spec(spec);spec.loader.exec_module(renderer)
        CAPTURES.mkdir(parents=True,exist_ok=True);renderer.COLS=120
        ansi=tmux('capture-pane','-p','-e','-t','verify:0.0').stdout
        (CAPTURES/(name+'.ansi')).write_text(ansi);renderer.picture(ansi,'runui · private shell recall',36).save(CAPTURES/(name+'.png'))
    def start(command):tmux('new-session','-d','-s','verify','-x','120','-y','36',command);tmux('set-option','-sg','escape-time','10')
    try:
        # Stdout contains only the exact selected payload, including trailing LF.
        result_file=home/'selection0';status_file=home/'picker-status'
        command=shlex.join([str(BIN),'find','--mode','shell','--shell','fish','--pick','--print0','--query','printf tail'])+' > '+shlex.quote(str(result_file))+'; printf "%s" "$?" > '+shlex.quote(str(status_file))+'; sleep 30'
        start(command);shows('printf tail');key('Enter');wait(status_file.exists,'picker did not finish')
        assert result_file.read_bytes()==b'printf tail\n\n\0',result_file.read_bytes()
        assert status_file.read_text()=='0';no_side_effects();tmux('kill-session','-t','verify')
        cancel_out=home/'cancel-output';cancel_status=home/'cancel-status'
        command=shlex.join([str(BIN),'find','--mode','shell','--shell','fish','--pick','--print0'])+' > '+shlex.quote(str(cancel_out))+'; printf "%s" "$?" > '+shlex.quote(str(cancel_status))+'; sleep 30'
        start(command);shows('Shell history');key('Escape');wait(cancel_status.exists,'cancel did not finish')
        assert cancel_out.read_bytes()==b'' and cancel_status.read_text()=='130';no_side_effects();tmux('kill-session','-t','verify')
        print('shell picker: stdout is exact NUL transport; multiline/trailing newlines survive; cancel returns 130 and no payload',flush=True)

        # A real fish binding inserts without executing. A second binding saves
        # the editable buffer as bytes so trailing newlines can be checked.
        fish_init=home/'fish-init.fish';buffer=home/'fish-buffer'
        fish_init.write_text('source '+shlex.quote(str(ROOT/'shell/runui.fish'))+'\n'+
            "bind \\cx\\ch runui-history\nfunction capture_buffer\n commandline --current-buffer > "+shlex.quote(str(buffer))+"\nend\nbind \\cx\\cb capture_buffer\n")
        start(shlex.join([str(FISH),'--no-config','--interactive','--init-command','source '+shlex.quote(str(fish_init))]))
        key('C-x','C-h');shows('Shell history');text('printf tail');key('Enter','C-x','C-b');wait(buffer.exists,'fish did not capture inserted buffer')
        assert buffer.read_bytes()==b'printf tail\n\n\n',buffer.read_bytes()
        key('C-c');buffer.unlink();text('echo KEEP_EDITING_☕');key('C-x','C-h');shows('Shell history');key('Escape','C-x','C-b')
        wait(buffer.exists,'fish did not preserve the original buffer on cancel')
        assert buffer.read_bytes()=='echo KEEP_EDITING_☕\n'.encode(),buffer.read_bytes()
        key('C-c');buffer.unlink();key('C-x','C-h');shows('Shell history');text('Unicode');key('Enter');text('!');key('C-x','C-b')
        wait(buffer.exists,'fish did not capture the Unicode cursor position')
        assert buffer.read_bytes()==(commands[1]+'!\n').encode(),buffer.read_bytes()
        no_side_effects();tmux('kill-session','-t','verify')
        print('fish widget: literal multiline/Unicode insertion, cursor at end, cancellation preserves original buffer',flush=True)

        for dialect,program,source in [('zsh',ZSH,zsh_file),('bash',BASH,bash_file)]:
            if dialect=='bash':
                source.write_text(source.read_text()+'#1700000200\nprintf tail\n\n\n')
                assert int(run(BASH,'--version').stdout.decode().split('version ')[1].split('.')[0])>=4,'Bash widget needs 4+'
            before=source.read_bytes();buffer=home/(dialect+'-buffer')
            start(shlex.join(['env','HISTFILE='+str(source),str(program),*(['-f','-i'] if dialect=='zsh' else ['--noprofile','--norc','-i'])]))
            if dialect=='zsh':
                setup='HISTFILE='+shlex.quote(str(source))+'; bindkey -e; source '+shlex.quote(str(ROOT/'shell/runui.zsh'))+'; bindkey '+shlex.quote('^X^H')+' runui-history-widget; capture_buffer() { print -rn -- "$BUFFER" > '+shlex.quote(str(buffer))+'; }; zle -N capture_buffer; bindkey '+shlex.quote('^X^B')+' capture_buffer'
            else:
                setup='HISTFILE='+shlex.quote(str(source))+'; source '+shlex.quote(str(ROOT/'shell/runui.bash'))+'; bind -x '+shlex.quote('"\\C-x\\C-h":_runui_history_widget')+'; capture_buffer() { printf "%s" "$READLINE_LINE" > '+shlex.quote(str(buffer))+'; }; bind -x '+shlex.quote('"\\C-x\\C-b":capture_buffer')
            ready=home/(dialect+'-ready')
            setup+='; printf READY > '+shlex.quote(str(ready))
            text(setup);key('Enter');wait(ready.exists,dialect+' initialization did not finish');key('C-x','C-h')
            shows('Shell history')
            text('printf tail');key('Enter');wait(lambda:'Shell history' not in screen(),lambda:dialect+' picker did not return\n'+screen());key('C-x','C-b');wait(buffer.exists,lambda:dialect+' did not capture inserted buffer\n'+screen())
            assert buffer.read_bytes()==b'printf tail\n\n',(dialect,buffer.read_bytes())
            key('C-c');buffer.unlink();text('echo KEEP_EDITING_☕');key('C-x','C-h');shows('Shell history');key('Escape','C-x','C-b')
            wait(buffer.exists,dialect+' did not preserve the original buffer on cancel')
            assert buffer.read_bytes()=='echo KEEP_EDITING_☕'.encode(),(dialect,buffer.read_bytes())
            key('C-c');buffer.unlink();key('C-x','C-h');shows('Shell history');text('Unicode');key('Enter')
            wait(lambda:'Shell history' not in screen(),dialect+' Unicode picker did not return');text('!');key('C-x','C-b')
            wait(buffer.exists,dialect+' did not capture the Unicode cursor position')
            assert buffer.read_bytes()=='echo Unicode ☕ café!'.encode(),(dialect,buffer.read_bytes())
            no_side_effects();assert source.read_bytes()==before
            key('C-c');text('unset HISTFILE');key('Enter');tmux('kill-session','-t','verify')
            assert source.read_bytes()==before
            print(dialect+' widget: literal multiline/Unicode insertion, cursor at end, cancellation preserves original buffer',flush=True)

        start(shlex.join([str(BIN),'-C',str(project)]));shows('[2]');key('C-p','Tab','Tab');shows('Shell history is off')
        assert not marker.exists();key('M-h');shows('SHELL_REPLAY_READY');shot('shell-recall')
        key('C-u');text('SHELL_REPLAY_READY');key('M-r');shows('original cwd unknown');shot('shell-replay-cwd')
        no_side_effects();key('Enter');shows('SHELL_REPLAY_READY');wait(lambda:len(list((home/'state/runui/runs').glob('*/run.json')))==1,'explicit replay did not create exactly one run')
        metadata=json.loads(next((home/'state/runui/runs').glob('*/run.json')).read_text());assert metadata['shell'][0].endswith('/fish'),metadata
        assert metadata['dir']==str(project),metadata
        output=next((home/'state/runui/runs').glob('*/output.log'))
        wait(lambda:'SHELL_REPLAY_READY' in output.read_text(),'native fish output missing')
        wait(lambda:str(project) in output.read_text(),'native fish did not execute in the chosen cwd')
        shot('shell-replayed');assert not marker.exists();assert fish_file.read_bytes()==original
        print('TUI: session opt-in, copy-only selection, explicit cwd, real fish supervisor and retained output',flush=True)
    finally:tmux('kill-server',check=False)
