#!/usr/bin/env python3
"""Native Composer contracts, entirely in a temporary HOME and project.

RUNUI_BIN, RUNUI_PHP and RUNUI_COMPOSER_PHAR select existing executables;
this script never installs tools or project dependencies. CI supplies pinned
official PHARs. No personal Composer config, favorites or runs are read.
"""

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get('RUNUI_BIN', ROOT / 'bin/runui')).resolve()
PHP = shutil.which(os.environ.get('RUNUI_PHP', 'php'))
PHAR = Path(os.environ['RUNUI_COMPOSER_PHAR']).resolve()
assert PHP and BIN.is_file() and PHAR.is_file(), 'Build runui and supply existing PHP and Composer PHAR'

with tempfile.TemporaryDirectory(prefix='runui-native-composer-') as temporary:
    home = Path(temporary).resolve()
    project = home / "project with 'quotes' and ☕"
    project.mkdir()
    (project / '.git').mkdir()
    tools = home / 'tools'
    tools.mkdir()
    (tools / 'php').symlink_to(Path(PHP).resolve())
    launcher = tools / 'composer'
    launcher.write_text('#!/bin/sh\nexec ' + shlex.quote(PHP) + ' ' + shlex.quote(str(PHAR)) + ' "$@"\n')
    launcher.chmod(0o700)
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home/'config'),
               XDG_STATE_HOME=str(home/'state'), COMPOSER_HOME=str(home/'composer-home'),
               COMPOSER_CACHE_DIR=str(home/'composer-cache'), COMPOSER_DISABLE_NETWORK='1',
               COMPOSER_ALLOW_SUPERUSER='1', COMPOSER_NO_INTERACTION='1',
               COMPOSER=str(home/'wrong-ambient-manifest.json'), PATH=f'{tools}:/usr/bin:/bin',
               NO_COLOR='1', GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
    # Do not inherit user flags, auth, script skipping or toolchain state.
    for key in list(env):
        if key.startswith(('MISE_', 'GIT_')) or key in ('COMPOSER_AUTH', 'COMPOSER_PROCESS_TIMEOUT', 'COMPOSER_SKIP_SCRIPTS', 'COMPOSER_VENDOR_DIR'):
            env.pop(key)
    env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')

    def run(*args, cwd=project, **kwargs):
        result = subprocess.run(list(map(str,args)), env=env, cwd=cwd, text=True,
                                capture_output=True, timeout=45, **kwargs)
        assert result.returncode == 0, (args, result.returncode, result.stdout, result.stderr)
        return result.stdout

    def cli(*args, cwd=project):
        return json.loads(run(BIN,*args,cwd=cwd))

    def write(directory, name, text):
        path = directory/name
        path.parent.mkdir(parents=True,exist_ok=True)
        path.write_text(text)

    echo = r'''<?php
file_put_contents(__DIR__.'/trace.jsonl', json_encode(array('kind'=>$argv[1], 'args'=>array_slice($argv,2), 'cwd'=>getcwd(), 'manifest'=>getenv('COMPOSER')), JSON_UNESCAPED_UNICODE)."\n", FILE_APPEND);
'''
    write(project,'echo.php',echo)
    write(project,'Scripts.php',r'''<?php
namespace Fixture;
class Scripts {
    public static function check(\Composer\Script\Event $event) {
        file_put_contents(__DIR__.'/trace.jsonl', json_encode(array('kind'=>'callback','args'=>$event->getArguments(),'cwd'=>getcwd(),'manifest'=>getenv('COMPOSER')), JSON_UNESCAPED_UNICODE)."\n",FILE_APPEND);
    }
}
''')
    write(project,'autoload-marker.php',"<?php file_put_contents(__DIR__.'/autoload.executed','loaded');\n")
    odd_name = "-odd'$(touch NEVER)☕"
    manifest = {
        'name':'runui/disposable-composer-check', 'version':'1.0.0',
        'autoload':{'classmap':['Scripts.php'],'files':['autoload-marker.php']},
        'scripts':{'leaf':'@php echo.php leaf',
                   'check':['@leaf','Fixture\\Scripts::check','@composer --version'],
                   odd_name:'@php echo.php odd', 'pre-deploy':'@php echo.php custom',
                   'post-autoload-dump':'@php echo.php hook'},
        'scripts-descriptions':{'check':'Array, callback and native helpers'},
        'scripts-aliases':{'check':['verify']}, 'config':{'process-timeout':17}}
    write(project,'composer.json',json.dumps(manifest))
    write(project,'vendor/ignored/composer.json',json.dumps({'scripts':{'never':'@php missing.php'}}))
    write(project,'packages/api/composer.json',json.dumps({'scripts':{'api':'@php echo.php nested'}}))
    write(project/'packages/api','echo.php',echo)

    page = cli('commands','--json','source:composer')
    assert page['complete'], page['diagnostics']
    entries = page['commands']
    assert not any(entry['kind']=='hook' or entry['name']=='never' for entry in entries)
    check = next(entry for entry in entries if entry['name']=='check')
    assert check['aliases']==['verify']
    assert all(tool['availability']=='on_path' for tool in check['requirements'])
    alias = cli('find','--json','source:composer verify')['commands']
    assert [entry['id'] for entry in alias]==[check['id']]
    hooks = cli('find','--json','source:composer kind:hook')['commands']
    assert [entry['name'] for entry in hooks]==['post-autoload-dump']
    assert not (project/'trace.jsonl').exists()
    passive = cli('inspect','--json',check['id'])
    assert 'native_inspection' not in passive
    report = cli('inspect','--native','--json',check['id'])['native_inspection']
    assert report['ready'], report
    assert not (project/'trace.jsonl').exists()
    assert not (project/'autoload.executed').exists(), 'native inspection loaded project autoload files'
    assert not list((home/'state/runui/runs').glob('*/run.json')), 'inspection created an unread run'

    # Generate only our own autoloader, with no dependency download or hooks.
    configured = dict(env,COMPOSER=str(project/'composer.json'))
    dumped = subprocess.run([PHP,str(PHAR),'--no-plugins','--no-scripts','dump-autoload'],cwd=project,
                            env=configured,text=True,capture_output=True,timeout=30)
    assert dumped.returncode==0,(dumped.stdout,dumped.stderr)
    assert not (project/'trace.jsonl').exists(), 'autoload generation ran the hook'

    def ended(started, expected=0):
        output = Path(started['output_file'])
        until = time.monotonic()+30
        while True:
            metadata = json.loads((output.parent/'run.json').read_text())
            if metadata.get('ended'):
                text = output.read_text()
                assert metadata.get('exit')==expected,(metadata,text)
                return text,metadata
            assert time.monotonic()<until,('run did not end',metadata)
            time.sleep(.03)

    def launch(entry,args=(),timeout=None):
        command = ['run','--json','--fingerprint',entry['fingerprint']]
        if timeout is not None: command += ['--timeout',str(timeout)]
        started = cli(*command,entry['id'],'--',*args)
        text,metadata = ended(started)
        return started,text,metadata

    arguments = ['--literal-flag','space here',"'\"$();`touch NEVER`",'', 'line\nbreak','☕']
    _,text,_ = launch(check,arguments,timeout=0)
    assert 'Composer version' in text
    trace = [json.loads(line) for line in (project/'trace.jsonl').read_text().splitlines()]
    assert [event['kind'] for event in trace]==['leaf','callback'],trace
    assert all(event['args']==arguments and event['cwd']==str(project) for event in trace),trace
    assert all(event['manifest']==str(project/'composer.json') for event in trace)
    assert not (project/'NEVER').exists()
    odd = next(entry for entry in entries if entry['name']==odd_name)
    launch(odd,arguments)
    assert json.loads((project/'trace.jsonl').read_text().splitlines()[-1])['args']==arguments
    nested = next(entry for entry in entries if entry['name']=='api')
    launch(nested)
    assert json.loads((project/'packages/api/trace.jsonl').read_text())['cwd']==str(project/'packages/api')

    # A selected local PHAR and alternate manifest never inherit COMPOSER.
    alternate = project/'alternative'
    alternate.mkdir()
    write(alternate,'echo.php',echo)
    write(alternate,'manifests/composer.dev.json',json.dumps({'scripts':{'alternate':'@php echo.php alternate'}}))
    write(alternate,'composer.json',json.dumps({'scripts':{'wrong':'@php missing.php'}}))
    (alternate/'tools').mkdir()
    shutil.copyfile(PHAR,alternate/'tools/composer.phar')
    write(alternate,'.runui.json',json.dumps({'version':1,'composer':{'manifest':'manifests/composer.dev.json','phar':'tools/composer.phar'}}))
    alt = cli('commands','--json','source:composer',cwd=alternate)['commands']
    assert [entry['name'] for entry in alt]==['alternate'],alt
    started = cli('run','--json',alt[0]['id'],'--','custom arg',cwd=alternate)
    ended(started)
    event = json.loads((alternate/'trace.jsonl').read_text())
    assert event['cwd']==str(alternate) and event['args']==['custom arg'],event
    assert event['manifest']==str(alternate/'manifests/composer.dev.json'),event

    # MCP agrees on canonical IDs and kind:hook; launch keeps attribution.
    requests = [
        {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'composer-native-check','version':'1'}}},
        {'jsonrpc':'2.0','method':'notifications/initialized'},
        {'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'list_commands','arguments':{'query':'source:composer verify'}}},
        {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'list_commands','arguments':{'query':'source:composer kind:hook'}}},
        {'jsonrpc':'2.0','id':4,'method':'tools/call','params':{'name':'start_run','arguments':{'id':check['id'],'fingerprint':check['fingerprint'],'inputs':{'args':['from MCP'],'timeout_seconds':0}}}}]
    replies = run(BIN,'mcp',input=''.join(json.dumps(request)+'\n' for request in requests))
    replies = {reply['id']:reply for reply in map(json.loads,replies.splitlines())}
    assert replies[2]['result']['structuredContent']['commands'][0]['id']==check['id']
    assert replies[3]['result']['structuredContent']['commands'][0]['catalog_kind']=='hook'
    started = replies[4]['result']['structuredContent']
    _,metadata = ended(started)
    assert metadata['agent']=='composer-native-check'
    assert not (project/'NEVER').exists()
    print('Native Composer checks passed: passive discovery, versions, arrays/callbacks/helpers, aliases/hooks, literal inputs, nested cwd, alternate manifest/PHAR, CLI/MCP attribution.')
