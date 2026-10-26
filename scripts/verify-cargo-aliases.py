#!/usr/bin/env python3
"""Compare effective aliases and supervised arguments with installed Cargo.

All configuration, plugins, registry, runs and usage are disposable. Nothing
is downloaded. A real Cargo subcommand fixture records cwd and literal argv.
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
RUSTUP = shutil.which('rustup')
if RUSTUP:
    CARGO = Path(subprocess.check_output([RUSTUP, 'which', 'cargo'], env=dict(os.environ, RUSTUP_AUTO_INSTALL='0'), text=True).strip()).resolve()
else:
    CARGO = Path(shutil.which('cargo')).resolve()

def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)

with tempfile.TemporaryDirectory(prefix='runui-native-aliases-') as temporary:
    home = Path(temporary).resolve()
    root = home/"projects/aliases 'and spaces'"
    (root/'.git').mkdir(parents=True)
    write(root/'Cargo.toml', "[package]\nname='alias_fixture'\nversion='0.1.0'\nedition='2024'\n")
    write(root/'src/lib.rs', 'pub fn check() {}\n')
    write(root/'Makefile', 'check:\n\ttrue\n')
    plugin = home/'bin/cargo-runui-echo'
    write(plugin, '#!/usr/bin/env python3\nimport json,os,sys\nfrom pathlib import Path\nPath(os.environ["ALIAS_MARKER"]).write_text("ran")\nprint("ALIAS_JSON "+json.dumps({"cwd":os.getcwd(),"args":sys.argv[1:]}))\n')
    plugin.chmod(0o700)
    cargo_home = home/'cargo-home'
    write(cargo_home/'config.toml', '[alias]\narray=["runui-echo","from global"]\ntext="runui-echo global text"\nglobal-only="runui-echo global-only"\n')
    write(home/'projects/.cargo/config.toml', '[alias]\narray=["from parent"]\nparent-only="runui-echo parent-only"\n')
    write(root/'.cargo/shared config.toml', '[alias]\narray=["from include"]\n')
    local_text = 'include=["shared config.toml",{path="optional.toml",optional=true}]\n[alias]\narray=["from local"]\ntext="runui-echo local text"\nrecursive="array from-recursive"\n'
    write(root/'.cargo/config', local_text)
    write(root/'.cargo/config.toml', '[alias]\nignored="runui-echo ignored"\n')
    marker = home/'alias-ran'
    env = {key:value for key,value in os.environ.items() if not key.startswith(('CARGO_', 'RUST', 'MISE_', 'GIT_'))}
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(home/'config'), XDG_STATE_HOME=str(home/'state'),
               CARGO_HOME=str(cargo_home), RUSTUP_AUTO_INSTALL='0', CARGO_NET_OFFLINE='true',
               PATH=f'{home/"bin"}:{CARGO.parent}:/usr/bin:/bin', ALIAS_MARKER=str(marker),
               CARGO_ALIAS_TEXT='runui-echo env text', CARGO_ALIAS_ARRAY='from-env "quoted words"\tline\nbreak',
               CARGO_ALIAS_ENV_ONLY='runui-echo env-only', NO_COLOR='1',
               GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')

    def run(*args, cwd=root, check=True):
        result = subprocess.run(list(map(str, args)), cwd=cwd, env=env, text=True, capture_output=True, timeout=30)
        if check: assert result.returncode==0,(args,result.returncode,result.stdout,result.stderr)
        return result
    def cli(*args, cwd=root): return json.loads(run(BIN,*args,cwd=cwd).stdout)
    def page(global_aliases=False, cwd=root, scope='project'):
        options=['--cargo-global-aliases'] if global_aliases else []
        return cli('commands','--json','--scope',scope,*options,'source:cargo kind:alias',cwd=cwd)
    def by_name(page): return {entry['name']:entry for entry in page['commands']}
    def output_json(text): return json.loads(next(line.removeprefix('ALIAS_JSON ') for line in text.splitlines() if line.startswith('ALIAS_JSON ')))
    def metadata(): return list((home/'state/runui/runs').glob('*/run.json'))
    def ended(started):
        output=Path(started['output_file']);until=time.monotonic()+30
        while True:
            state=json.loads((output.parent/'run.json').read_text())
            if state.get('ended'):
                text=output.read_text();assert state.get('exit')==0,(state,text)
                return output_json(text),state
            assert time.monotonic()<until,state
            time.sleep(.03)

    environment_words=['from-env','"quoted','words"','line','break']
    passive=page();local=by_name(passive)
    assert passive['complete'],passive['diagnostics']
    assert set(local)=={'array','text','recursive'},local
    assert local['array']['cargo_alias']['value']=={'type':'array','value':['runui-echo','from global','from parent','from include','from local',*environment_words]}
    assert local['text']['cargo_alias']['value']=={'type':'string','value':'runui-echo env text'}
    assert len(local['array']['cargo_alias']['origins'])==5
    assert all(entry['origin']=='declared' and entry['recipe']['separator'] is False for entry in local.values())
    assert not marker.exists() and not metadata()
    assert not (root/'Cargo.lock').exists()
    enabled=by_name(page(True))
    assert set(enabled)==set(local)|{'global-only','parent-only','env-only'}
    assert enabled['env-only']['entry_file'] is None and enabled['env-only']['defining_file'] is None
    assert all(enabled[name]['id']==entry['id'] and enabled[name]['fingerprint']==entry['fingerprint'] for name,entry in local.items())
    unchanged=local['array']['fingerprint']
    write(root/'.cargo/config.toml', '[alias]\nignored="changed ignored config"\n')
    assert by_name(page())['array']['fingerprint']==unchanged
    inspected=cli('inspect','--native','--json',local['array']['id'])['native_inspection']
    assert not inspected['checked'] and 'does not execute Cargo aliases' in inspected['skipped']
    assert not marker.exists() and not metadata()

    arguments=['--literal','space here',"'\"$();`touch NEVER`",'', 'line\nbreak','☕']
    native=output_json(run(CARGO,'recursive',*arguments).stdout)
    marker.unlink()
    assert native['args']==['runui-echo','from global','from parent','from include','from local',*environment_words,'from-recursive',*arguments],native
    selected=local['recursive']
    observed,state=ended(cli('run','--json','--fingerprint',selected['fingerprint'],selected['id'],'--',*arguments))
    assert observed==native and state['dir']==str(root),(observed,native,state)
    assert not (root/'NEVER').exists()
    before=len(metadata())
    rejected=run(BIN,'run','--json',enabled['env-only']['id'],check=False)
    assert rejected.returncode!=0 and len(metadata())==before

    requests=[
        {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'alias-native-check','version':'1'}}},
        {'jsonrpc':'2.0','method':'notifications/initialized'},
        {'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'list_commands','arguments':{'query':'source:cargo kind:alias'}}},
        {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'list_commands','arguments':{'query':'source:cargo kind:alias','cargo_global_aliases':True}}},
        {'jsonrpc':'2.0','id':4,'method':'tools/call','params':{'name':'start_run','arguments':{'id':enabled['env-only']['id'],'cargo_global_aliases':True,'inputs':{'args':['from MCP','literal ;$']}}}},
    ]
    process=subprocess.run([str(BIN),'mcp'],cwd=root,env=env,text=True,input=''.join(json.dumps(request)+'\n' for request in requests),capture_output=True,timeout=30,check=True)
    replies={reply['id']:reply for reply in map(json.loads,process.stdout.splitlines())}
    for ident, expected in [(2,local),(3,enabled)]:
        found=by_name(replies[ident]['result']['structuredContent'])
        assert {name:entry['id'] for name,entry in found.items()}=={name:entry['id'] for name,entry in expected.items()}
        assert all(found[name]['fingerprint']==entry['fingerprint'] for name,entry in expected.items())
    observed,state=ended(replies[4]['result']['structuredContent'])
    assert state['agent']=='alias-native-check'
    assert observed['args']==['runui-echo','env-only','from MCP','literal ;$']
    marker.unlink()

    runtime=root/'runtime cwd'
    write(runtime/'.cargo/config.toml', '[alias]\ntext="runui-echo invocation-specific"\n')
    member=by_name(page(cwd=runtime))['text']
    assert member['directory']==str(runtime)
    # An environment string outranks the closer string too.
    assert member['cargo_alias']['value']['value']=='runui-echo env text'
    del env['CARGO_ALIAS_TEXT']
    member=by_name(page(cwd=runtime))['text']
    assert member['cargo_alias']['value']['value']=='runui-echo invocation-specific'
    assert by_name(page(cwd=runtime,scope='workspace'))['text']['id']==member['id']
    observed=output_json(run(CARGO,'text',cwd=runtime).stdout)
    actual,_state=ended(cli('run','--json',member['id'],cwd=runtime))
    assert actual==observed and actual['cwd']==str(runtime)

    registration=cli('projects','add',str(root),'--json')['project']
    elsewhere=home/'elsewhere';write(elsewhere/'Makefile','all:\n\ttrue\n')
    registered=by_name(page(cwd=elsewhere,scope='registered'))['text']
    assert registered['directory']==str(root) and registered['cargo_alias']['value']['value']=='runui-echo local text'
    actual,state=ended(cli('run','--json','--scope','registered','--project',registration['id'],registered['id'],cwd=elsewhere))
    assert actual['cwd']==str(root) and state['dir']==str(root)

    stale=by_name(page())['array']
    write(cargo_home/'config.toml','[alias]\narray=["runui-echo","changed global"]\n')
    before=len(metadata())
    failed=run(BIN,'run','--json','--fingerprint',stale['fingerprint'],stale['id'],check=False)
    assert failed.returncode!=0 and len(metadata())==before
    assert 'definition changed' in failed.stderr
    write(root/'.cargo/config',local_text+'loop="loop"\nshell="!echo never"\n')
    invalid=page();invalid_aliases=by_name(invalid)
    assert not invalid['complete'] and invalid_aliases['loop']['cargo_alias']['disabled_reason']
    assert 'does not interpret' in invalid_aliases['shell']['cargo_alias']['diagnostics'][0]
    before=len(metadata())
    failed=run(BIN,'run','--json',invalid_aliases['loop']['id'],check=False)
    assert failed.returncode!=0 and len(metadata())==before
    assert not (root/'Cargo.lock').exists()
    write(cargo_home/'config.toml','[alias]\narray="runui-echo conflicting type"\n')
    mixed=page()
    assert not mixed['complete'] and any('mixes string and array' in item['message'] for item in mixed['diagnostics'])
    assert any(entry['source']=='make' for entry in cli('commands','--json')['commands'])
    native_error=run(CARGO,'text',check=False)
    assert native_error.returncode!=0 and 'failed to merge' in native_error.stderr
    write(cargo_home/'config.toml','[alias]\narray=["runui-echo","global"]\n')
    write(root/'.cargo/config',local_text)
    before=by_name(page())['array']['fingerprint']
    write(root/'.cargo/optional.toml','[alias]\narray=["from optional"]\n')
    optional=by_name(page())['array']
    assert optional['fingerprint']!=before
    assert optional['cargo_alias']['value']['value']==output_json(run(CARGO,'array').stdout)['args']
    for includes in ['["cycle-a.toml"]','["shared config.toml","shared config.toml"]']:
        write(root/'.cargo/config','include='+includes+'\n[alias]\ntext="runui-echo text"\n')
        write(root/'.cargo/cycle-a.toml','include=["cycle-b.toml"]\n')
        write(root/'.cargo/cycle-b.toml','include=["cycle-a.toml"]\n')
        errors=page()
        assert not errors['complete'] and any('include cycle' in item['message'] for item in errors['diagnostics'])
        native_error=run(CARGO,'text',check=False)
        assert native_error.returncode!=0 and 'cycle' in native_error.stderr

    # CARGO_HOME can be relative and also be the probed local .cargo folder;
    # native Cargo applies that file only once.
    duplicate=home/'projects/duplicate-home'
    write(home/'projects/.cargo/config.toml','')
    write(duplicate/'Cargo.toml',"[package]\nname='duplicate'\nversion='0.1.0'\n")
    write(duplicate/'src/lib.rs','pub fn check() {}\n')
    write(duplicate/'.cargo/config.toml','[alias]\ndedup=["runui-echo","once"]\n')
    env['CARGO_HOME']='.cargo'
    env.pop('CARGO_ALIAS_ARRAY',None)
    dedup=by_name(page(cwd=duplicate))['dedup']['cargo_alias']
    assert len(dedup['origins'])==1
    assert dedup['value']['value']==output_json(run(CARGO,'dedup',cwd=duplicate).stdout)['args']==['runui-echo','once']
    internal_home=home/'projects/home-inside-checkout'
    write(internal_home/'Cargo.toml',"[package]\nname='internal_home'\nversion='0.1.0'\n")
    write(internal_home/'src/lib.rs','pub fn check() {}\n')
    write(internal_home/'cache/cargo-home/config.toml','[alias]\nhome-only="version"\n')
    env['CARGO_HOME']='cache/cargo-home'
    assert 'home-only' not in by_name(page(cwd=internal_home))
    assert by_name(page(True,cwd=internal_home))['home-only']['cargo_alias']['global']
    links=home/'projects/linked-config'
    write(links/'Cargo.toml',"[package]\nname='links'\nversion='0.1.0'\n")
    write(links/'src/lib.rs','pub fn check() {}\n')
    write(home/'shared/base.toml','include=["included.toml"]\n')
    write(home/'shared/included.toml','[alias]\nlinked="runui-echo wrong-side"\n')
    write(links/'.cargo/included.toml','[alias]\nlinked="runui-echo invocation-side"\n')
    (links/'.cargo/config.toml').symlink_to(home/'shared/base.toml')
    env['CARGO_HOME']=str(home/'empty-cargo-home')
    linked=by_name(page(cwd=links))['linked']
    assert linked['cargo_alias']['value']['value']=='runui-echo invocation-side'
    assert not linked['cargo_alias']['global']
    assert output_json(run(CARGO,'linked',cwd=links).stdout)['args']==['runui-echo','invocation-side']
    write(links/'.cargo/included.toml','[alias]\nthrough-version="version"\nversion="through-version"\n')
    builtin=by_name(page(cwd=links))
    assert builtin['through-version']['cargo_alias']['disabled_reason'] is None
    assert builtin['version']['cargo_alias']['disabled_reason']
    assert run(CARGO,'through-version',cwd=links).stdout.startswith('cargo 1.')
    print('Native Cargo alias checks passed: precedence/typed merges/includes/env, project/global visibility, no passive execution, literal supervised CLI/MCP arguments and attribution, invocation/registered cwd, stale definitions and disabled cycles.')
