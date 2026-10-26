#!/usr/bin/env python3
"""Compare passive discovery with native Cargo in disposable projects.

Needs an installed Rust toolchain and a built runui (RUNUI_BIN). Uses no
downloaded dependencies, personal Cargo config, runui state or history. Cargo
is explicit, offline and locked; rustup automatic installation is disabled.
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
assert BIN.is_file(), 'Build runui first'

def installed(name):
    if RUSTUP:
        result = subprocess.run([RUSTUP,'which',name],env=dict(os.environ,RUSTUP_AUTO_INSTALL='0'),text=True,capture_output=True,timeout=10,check=True)
        return Path(result.stdout.strip()).resolve()
    return Path(shutil.which(name)).resolve()

CARGO, RUSTC = installed('cargo'), installed('rustc')

with tempfile.TemporaryDirectory(prefix='runui-native-cargo-') as temporary:
    home=Path(temporary).resolve()
    env={key:value for key,value in os.environ.items() if not key.startswith(('CARGO_','RUST','MISE_','GIT_'))}
    env.update(HOME=str(home),XDG_CONFIG_HOME=str(home/'config'),XDG_STATE_HOME=str(home/'state'),
               CARGO_HOME=str(home/'cargo-home'),CARGO_TARGET_DIR=str(home/'target'),RUSTC=str(RUSTC),
               RUSTUP_AUTO_INSTALL='0',PATH=f'{CARGO.parent}:/usr/bin:/bin',NO_COLOR='1',
               GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL='/dev/null')
    project=home/"workspace with 'quotes' and ☕"
    (project/'.git').mkdir(parents=True)

    def write(root,name,text):
        path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(text)

    def package(root,name,extra='',edition="edition='2024'"):
        write(root,'Cargo.toml',f"[package]\nname={json.dumps(name)}\nversion='0.1.0'\n{edition}\n{extra}\n")
        write(root,'src/lib.rs','pub fn check() {}\n')

    def run(*args,cwd=project,check=True):
        result=subprocess.run(list(map(str,args)),cwd=cwd,env=env,text=True,capture_output=True,timeout=90)
        if check: assert result.returncode==0,(args,result.returncode,result.stdout,result.stderr)
        return result

    def cli(*args,cwd=project):return json.loads(run(BIN,*args,cwd=cwd).stdout)

    write(project,'Cargo.toml',"""[workspace]
resolver='3'
members=['members/*','deep/a/b/c/*']
exclude=['members/excluded']
default-members=['members/library']
[workspace.package]
edition='2024'
[workspace.dependencies]
engine={path='implicit/engine'}
""")
    package(project/'members/library','library',"""
[[example]]
name='as_library'
crate-type=['rlib']
""")
    write(project,'members/library/examples/as_library.rs','pub fn example() {}\n')
    write(project,'members/library/examples/demo.rs','fn main() {}\n')
    write(project,'members/library/tests/integration.rs','#[test] fn check() {}\n')
    write(project,'members/library/benches/measure.rs','fn main() {}\n')
    tool=project/'members/tool'
    package(tool,'tool',"""
default-run='tool'
[features]
gated=[]
[[bin]]
name='tool'
path='src/main.rs'
required-features=['gated']
[dependencies]
engine.workspace=true
""",edition='edition.workspace=true')
    source=r'''fn main() {
    for (label,value) in std::iter::once(("CWD_HEX",std::env::current_dir().unwrap().to_string_lossy().into_owned()))
        .chain(std::env::args().skip(1).map(|arg|("ARG_HEX",arg))) {
        println!("{} {}",label,value.as_bytes().iter().map(|b|format!("{:02x}",b)).collect::<String>());
    }
    #[cfg(feature="gated")] println!("FEATURE_ENABLED");
}
'''
    write(tool,'src/main.rs',source)
    write(tool,'src/bin/admin.rs','fn main() {}\n')
    write(tool,'build.rs','fn main(){std::fs::write(std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("native-build.marker"),"built").unwrap();}\n')
    package(project/'implicit/engine','engine')
    package(project/'deep/a/b/c/deep','deep')
    package(project/'members/excluded','excluded')
    package(project/'members/.hidden','hidden')
    package(project/'members/macros','macros','[lib]\nproc-macro=true')
    write(project,'members/macros/src/lib.rs','extern crate proc_macro;\n')
    invocation=tool/'runtime cwd'
    invocation.mkdir(parents=True)

    # No native call precedes the passive-discovery assertions.
    page=cli('commands','--scope','workspace','--json','source:cargo',cwd=invocation)
    # Cargo targets can be deeper than the other providers' incidental scan.
    # Keep that whole-snapshot limitation visible and prove Cargo separately
    # against every native member/target below, rather than suppressing it.
    assert all(item['source'] is None and item['message']=='incidental folders below depth 3 were not inspected'
               for item in page['diagnostics']),page['diagnostics']
    commands=page['commands']
    assert not (project/'Cargo.lock').exists()
    assert not (home/'target').exists()
    assert not (tool/'native-build.marker').exists()
    assert all(entry['origin']=='generated' for entry in commands)
    assert all(entry['directory']==str(invocation) for entry in commands), 'Cargo lost invocation cwd'
    passive_binary=next(entry for entry in commands if entry['name']=='run --bin tool (tool)')
    missing_lock=cli('inspect','--native','--json',passive_binary['id'],cwd=invocation)['native_inspection']
    assert not missing_lock['ready'] and not missing_lock['checked']
    assert 'existing Cargo.lock' in missing_lock['skipped']
    assert not (project/'Cargo.lock').exists()

    # Only the disposable fixture receives a lockfile. --no-deps metadata
    # builds nothing and invokes no build scripts or procedural macros.
    run(CARGO,'generate-lockfile','--offline')
    metadata=json.loads(run(CARGO,'metadata','--no-deps','--offline','--locked','--format-version','1').stdout)
    assert not (tool/'native-build.marker').exists()
    refreshed=cli('commands','--scope','workspace','--json','source:cargo',cwd=invocation)['commands']
    assert {entry['id'] for entry in refreshed}=={entry['id'] for entry in commands}
    commands=refreshed
    members={package['manifest_path']:package for package in metadata['packages'] if package['id'] in metadata['workspace_members']}
    discovered={entry['entry_file'] for entry in commands if entry['cargo']['package']}
    assert discovered==set(members),(discovered,set(members))
    opaque={package['id']:package['manifest_path'] for package in members.values()}
    defaults={str(Path(opaque[ident]).parent.relative_to(project)) for ident in metadata['workspace_default_members']}
    root_command=next(entry for entry in commands if entry['cargo']['scope']=='default_members')
    assert set(root_command['cargo']['default_member_paths'])==defaults
    assert '--workspace' not in root_command['recipe']['argv']
    assert all('--all-features' not in entry['recipe']['argv'] for entry in commands)

    kind_map={'binary':'bin','library':'lib','example':'example','test':'test','benchmark':'bench'}
    for manifest,package_data in members.items():
        actual={}
        for entry in commands:
            target=entry.get('cargo',{}).get('target')
            if entry['entry_file']!=manifest or not target:continue
            key=(kind_map[target['kind']],target['name'])
            actual[key]=target
        expected={}
        for target in package_data['targets']:
            kinds=target['kind']
            if kinds==['custom-build']:continue
            kind=kinds[0] if kinds[0] in ('bin','example','test','bench') else 'lib'
            expected[(kind,target['name'])]=target
        assert set(actual)==set(expected),(manifest,set(actual),set(expected))
        for key,target in expected.items():
            ours=actual[key]
            assert ours['source_path']==target['src_path'],(key,ours,target)
            assert ours['crate_types']==target['crate_types'],(key,ours,target)
            assert ours['required_features']==target.get('required-features',[]),(key,ours,target)
            runnable=key[0]=='bin' or (key[0]=='example' and target['crate_types']==['bin'])
            assert ours['runnable']==runnable,(key,ours,target)

    local=cli('commands','--json','source:cargo',cwd=invocation)['commands']
    scoped={entry['id']:entry for entry in commands if entry['entry_file']==str(tool/'Cargo.toml')}
    assert {entry['id'] for entry in local}==set(scoped)
    assert all(entry['fingerprint']==scoped[entry['id']]['fingerprint'] for entry in local)
    binary=next(entry for entry in local if entry['name']=='run --bin tool (tool)')
    assert binary['cargo']['target']['default_run']
    assert binary['recipe']['argv'][-2:]==['--features','gated']
    checked=cli('inspect','--native','--json',binary['id'],cwd=invocation)['native_inspection']
    assert checked['ready'],checked
    assert checked['cargo']['definition_matches'] and checked['cargo']['package']=='tool'
    assert checked['cargo']['targets'] and checked['cargo']['available_features']
    assert not (tool/'native-build.marker').exists()
    assert not list((home/'state/runui/runs').glob('*/run.json'))
    assert not (home/'state/runui/runs/catalog-v1.json').exists()
    # Metadata checks must not execute compiler overrides or wrappers. Cargo
    # owns those during a deliberate build; inspection uses installed rustc.
    trap=home/'compiler trap'
    trap.write_text('#!/bin/sh\ntouch '+str(home/'COMPILER_RAN')+'\nexit 7\n')
    trap.chmod(0o700)
    config=invocation/'.cargo/config.toml'
    write(invocation,'.cargo/config.toml',
          '[build]\nrustc='+json.dumps(str(trap))+'\nrustc-wrapper='+json.dumps(str(trap))+
          '\nrustc-workspace-wrapper='+json.dumps(str(trap))+'\n[env]\nRUSTC={value='+json.dumps(str(trap))+',force=true}\nRUSTUP_AUTO_INSTALL={value="1",force=true}\n')
    guarded=cli('inspect','--native','--json',binary['id'],cwd=invocation)['native_inspection']
    assert guarded['ready'],guarded
    assert not (home/'COMPILER_RAN').exists()
    config.unlink()
    config.parent.rmdir()
    # Script shims and unavailable rustup toolchains fall back before Cargo
    # can build, install, or create an unread run. Each uses its own PATH/home.
    scripts=home/'script-shims'
    scripts.mkdir()
    (scripts/'cargo').write_text('#!/bin/sh\ntouch '+str(home/'SHIM_RAN')+'\n')
    (scripts/'cargo').chmod(0o700)
    variant=dict(env,PATH=f'{scripts}:{env["PATH"]}')
    skipped=json.loads(subprocess.run([str(BIN),'inspect','--native','--json',binary['id']],cwd=invocation,env=variant,text=True,capture_output=True,timeout=30,check=True).stdout)['native_inspection']
    assert not skipped['ready'] and 'script shim' in skipped['skipped']
    assert not (home/'SHIM_RAN').exists()
    if RUSTUP:
        proxies=home/'rustup-proxies'
        proxies.mkdir()
        rustup_binary=Path(RUSTUP).resolve()
        with rustup_binary.open('rb') as stream:script_proxy=stream.read(2)==b'#!'
        for name in ['cargo','rustc','rustup']:(proxies/name).symlink_to(rustup_binary)
        if CARGO.parent.parent.parent.name=='toolchains':
            variant=dict(env,PATH=f'{proxies}:/usr/bin:/bin',RUSTUP_HOME=str(CARGO.parent.parent.parent.parent),RUSTUP_TOOLCHAIN=CARGO.parent.parent.name)
            located=json.loads(subprocess.run([str(BIN),'inspect','--native','--json',binary['id']],cwd=invocation,env=variant,text=True,capture_output=True,timeout=30,check=True).stdout)['native_inspection']
            if script_proxy:
                # Some managed macOS runners expose rustup via a script.
                # Product inspection must reject it without executing it.
                assert not located['ready'] and not located['checked'] and 'script shim' in located['skipped'],located
            else:
                assert located['ready'],located
                assert [probe['tool'] for probe in located['checked'][:2]]==['rustup which cargo','rustup which rustc']
        variant=dict(env,PATH=f'{proxies}:/usr/bin:/bin',RUSTUP_HOME=str(home/'missing-rustup'),RUSTUP_TOOLCHAIN='runui-never-install')
        skipped=json.loads(subprocess.run([str(BIN),'inspect','--native','--json',binary['id']],cwd=invocation,env=variant,text=True,capture_output=True,timeout=30,check=True).stdout)['native_inspection']
        assert not skipped['ready'],skipped
        if script_proxy:
            assert not skipped['checked'] and 'script shim' in skipped['skipped'],skipped
        else:
            assert 'not installed' in skipped['skipped'],skipped
            assert skipped['checked'][0]['tool']=='rustup which cargo'
        assert not (home/'missing-rustup/toolchains').exists()
    binary=next(entry for entry in cli('commands','--json','source:cargo',cwd=invocation)['commands'] if entry['name']=='run --bin tool (tool)')
    arguments=['--literal-flag','space here',"'\"$();`touch NEVER`",'', 'line\nbreak','☕']
    started=cli('run','--json','--fingerprint',binary['fingerprint'],binary['id'],'--',*arguments,cwd=invocation)

    def ended(started):
        output=Path(started['output_file']);until=time.monotonic()+60
        while True:
            state=json.loads((output.parent/'run.json').read_text())
            if state.get('ended'):
                text=output.read_text();assert state.get('exit')==0,(state,text);return text,state
            assert time.monotonic()<until,('run did not finish',state)
            time.sleep(.04)

    text,state=ended(started)
    assert state['dir']==str(invocation)
    actual_args=[bytes.fromhex(line.removeprefix('ARG_HEX ')).decode() for line in text.splitlines() if line.startswith('ARG_HEX ')]
    assert actual_args==arguments,(actual_args,arguments,text)
    assert 'CWD_HEX '+str(invocation).encode().hex() in text
    assert 'FEATURE_ENABLED' in text
    assert (tool/'native-build.marker').exists()
    assert not (invocation/'NEVER').exists()

    requests=[
        {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'cargo-native-check','version':'1'}}},
        {'jsonrpc':'2.0','method':'notifications/initialized'},
        {'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'list_commands','arguments':{'scope':'workspace','query':'source:cargo dir:members/tool'}}},
        {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'start_run','arguments':{'scope':'workspace','id':binary['id'],'fingerprint':binary['fingerprint'],'inputs':{'args':['from MCP']}}}},
    ]
    process=subprocess.run([str(BIN),'mcp'],cwd=invocation,env=env,text=True,input=''.join(json.dumps(request)+'\n' for request in requests),capture_output=True,timeout=30,check=True)
    replies={reply['id']:reply for reply in map(json.loads,process.stdout.splitlines())}
    catalog=replies[2]['result']['structuredContent']['commands']
    selected=next(entry for entry in catalog if entry['id']==binary['id'])
    assert selected['fingerprint']==binary['fingerprint']
    assert selected['directory']==str(invocation)
    text,state=ended(replies[3]['result']['structuredContent'])
    assert state['agent']=='cargo-native-check'
    assert 'ARG_HEX '+b'from MCP'.hex() in text

    # Older editions disable inferred targets when targets are explicit.
    legacy=home/'legacy'
    package(legacy,'legacy',"[[example]]\nname='manual'",edition="edition='2015'")
    write(legacy,'src/main.rs','fn main() {}\n')
    write(legacy,'examples/manual.rs','fn main() {}\n')
    write(legacy,'tests/implicit.rs','#[test] fn check() {}\n')
    native=json.loads(run(CARGO,'metadata','--no-deps','--offline','--format-version','1',cwd=legacy).stdout)
    ours=cli('commands','--json','source:cargo',cwd=legacy)['commands']
    ours={(entry['cargo']['target']['kind'],entry['cargo']['target']['name']) for entry in ours if entry['cargo']['target']}
    expected={(('library' if target['kind'][0]=='lib' else {'bin':'binary','example':'example','test':'test','bench':'benchmark'}[target['kind'][0]]),target['name']) for target in native['packages'][0]['targets']}
    assert ours==expected,(ours,expected)
    dual=home/'dual-kind'
    write(dual,'Cargo.toml',"[package]\nname='dual'\nedition='2024'\n[lib]\npath='src/main.rs'\n")
    write(dual,'src/main.rs','fn main() {}\n')
    native=json.loads(run(CARGO,'metadata','--no-deps','--offline','--format-version','1',cwd=dual).stdout)
    ours=cli('commands','--json','source:cargo',cwd=dual)['commands']
    targets=[entry['cargo']['target'] for entry in ours if entry['cargo']['target']]
    assert {target['kind'] for target in targets}=={'library','binary'}
    assert len(targets)==len(native['packages'][0]['targets'])
    ambiguous=home/'ambiguous'
    package(ambiguous,'ambiguous')
    write(ambiguous,'src/bin/duplicate.rs','fn main() {}\n')
    write(ambiguous,'src/bin/duplicate/main.rs','fn main() {}\n')
    invalid=subprocess.run([str(CARGO),'metadata','--no-deps','--offline','--format-version','1'],cwd=ambiguous,env=env,text=True,capture_output=True,timeout=30)
    assert invalid.returncode!=0 and 'duplicate binary name' in invalid.stderr
    ours=cli('commands','--json','source:cargo',cwd=ambiguous)
    assert not ours['complete'] and any('multiple inferred binary' in item['message'] for item in ours['diagnostics'])
    assert not any(entry['name'].startswith('run') for entry in ours['commands'])
    nested=home/'polyglot'
    write(nested,'Makefile','all:\n\ttrue\n')
    write(nested,'rust/workspace/Cargo.toml',"[workspace]\nmembers=['deep/a/b/c/member']\n")
    package(nested/'rust/workspace/deep/a/b/c/member','nested')
    nested_entries=cli('commands','--json','source:cargo',cwd=nested)['commands']
    assert {entry['cargo']['package'] for entry in nested_entries if entry['cargo']['package']}=={'nested'}
    native=json.loads(run(CARGO,'metadata','--no-deps','--offline','--format-version','1',cwd=nested/'rust/workspace').stdout)
    assert {entry['entry_file'] for entry in nested_entries if entry['cargo']['package']}=={package['manifest_path'] for package in native['packages']}
    print('Native Cargo checks passed: passive/no-build discovery, explicit bounded metadata and static fallback, installed rustup tools, no compiler wrappers or script shims, membership/defaults, target kinds/features, invocation cwd, literal supervised args, shared CLI/MCP scope IDs and attribution, and edition inference.')
