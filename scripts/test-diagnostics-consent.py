#!/usr/bin/env python3
"""Process-level consent regression tests. Disposable preferences, loopback only, no hardware."""
import atexit
import argparse
import http.server
import json
import os
import pathlib
import plistlib
import shutil
import subprocess
import tempfile
import threading
import time
import uuid
import urllib.parse

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', required=True, type=pathlib.Path)
parser.add_argument('--field-test', action='store_true', help='Verify a debug field-test binary excludes all statistics')
args = parser.parse_args()
records = []
faults = {}
lock = threading.Lock()
arrivals = {route: threading.Event() for route in ['profile-token', 'telemetry']}

class Server(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        route=urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query).get('route',[''])[0]
        arrived=route in arrivals and arrivals[route].wait(8)
        self.send_response(200 if arrived else 504)
        self.end_headers()
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        route = self.path.rsplit('/', 1)[-1]
        with lock:
            records.append((route, body))
            fault = faults.pop(route, None)
            arrivals[route].set()
        if fault == 'delay': time.sleep(.6)
        status = 503 if fault == 'fail' else 200
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(b'{' if fault == 'malformed' else json.dumps({'access_token':'test-token'}).encode())

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Server)
threading.Thread(target=server.serve_forever, daemon=True).start()
base = f'http://127.0.0.1:{server.server_port}/api/v1'
root = pathlib.Path(tempfile.mkdtemp(prefix='luxmini-consent-tests-'))
bundles = []
def cleanup():
    for _, identifier in bundles:
        subprocess.run(['defaults', 'delete', identifier], capture_output=True)
    server.shutdown()
    shutil.rmtree(root, ignore_errors=True)
atexit.register(cleanup)
def action(kind, **kw): return dict(kind=kind, **kw)
def run(actions, bundle=None, expected_code=0, **config):
    if bundle is None:
        identifier = 'com.bastiencantet.luxmini.metrics-test.' + uuid.uuid4().hex
        bundle = root / (identifier + '.app')
        macos = bundle / 'Contents/MacOS'
        macos.mkdir(parents=True)
        shutil.copy2(args.binary, macos / 'ConsentTest')
        with (bundle / 'Contents/Info.plist').open('wb') as f:
            plistlib.dump(dict(CFBundleIdentifier=identifier, CFBundleExecutable='ConsentTest', CFBundleName='LuxMini Test', CFBundlePackageType='APPL'), f)
        bundles.append((bundle, identifier))
    identifier = bundle.name[:-4]
    path = root / 'config.json'
    settings = dict(bundle_id=identifier, base_url=base, actions=actions)
    settings.update(config)
    path.write_text(json.dumps(settings))
    env = dict(os.environ, LUXMINI_DIAGNOSTICS_TEST_CONFIG=str(path))
    result = subprocess.run([str(bundle/'Contents/MacOS/ConsentTest')], env=env, text=True, capture_output=True, timeout=30)
    assert result.returncode == expected_code, result.stderr
    if expected_code:
        assert "detected Mac model" not in result.stderr
        return result.stderr, bundle
    data = json.loads(result.stdout.strip().splitlines()[-1])
    assert data['hardware_started'] is False and data['transport'] == 'loopback'
    return data, bundle

def activity(): return [body for route, body in records if route == 'telemetry' and body['event']=='app_activity']
def erasures(): return [body for route, body in records if route == 'telemetry' and body['event']=='forget_installation']
def reset():
    records.clear(); faults.clear()
    for event in arrivals.values(): event.clear()
def accepted(existing=False): return [action('offer', accepted=True, existing=existing), action('start'), action('drain')]
def check(name, fn):
    reset()
    fn()
    print('PASS', name, flush=True)

if args.field_test:
    data,_=run([action('offer',accepted=True),action('choose',accepted=True),action('start'),action('activity'),action('drain'),action('snapshot')])
    assert data['prompts']==0 and not records and not data['snapshots'][0]['has_identity'] and not data['snapshots'][0]['enabled']
    print('PASS Field-test build: no prompt, identity or network even after explicit fixture acceptance')
    raise SystemExit(0)

def no_choice():
    data,_ = run([action('start'),action('drain'),action('snapshot')])
    assert not records and not data['snapshots'][0]['has_identity']
check('No decision: no identity and no network', no_choice)

def refusal():
    data,bundle=run([action('offer'),action('start'),action('drain'),action('offer'),action('snapshot')])
    assert data['prompts']==1 and not records and data['snapshots'][0]['decided']
    data,_=run([action('offer',accepted=True),action('start'),action('drain'),action('snapshot')],bundle)
    assert data['prompts']==0 and not records and not data['snapshots'][0]['enabled']
check('Refusal persists across process restart', refusal)

def initial(existing=False):
    data,_=run(accepted(existing)+[action('snapshot')])
    events=activity()
    assert len(events)==1 and events[0]['outcome']==('existing_install' if existing else 'new_install')
    identifier=uuid.UUID(events[0]['installation_id'])
    assert identifier.version==4 and str(identifier)==events[0]['installation_id']
    assert set(events[0])=={'event','outcome','model_id','macos_major','installation_id','consent_version','app_version'}
    assert data['snapshots'][0]['last_version']=='0.4.3'
check('New installation: UUID v4 and strict payload', initial)
check('Existing installation is counted separately', lambda:initial(True))

def legacy():
    data,_=run([action('seed',key='diagnostics.enabled',value=True),action('seed',key='diagnostics.consent_version',value=0),action('start'),action('drain'),action('snapshot'),action('offer'),action('snapshot')])
    assert not records and data['prompts']==1 and not data['snapshots'][0]['enabled']
check('Legacy aggregate consent cannot authorize linked statistics',legacy)

def daily():
    data,bundle=run(accepted()+[action('activity') for _ in range(20)]+[action('drain'),action('clock',day=20001),action('activity'),action('drain'),action('clock',day=19999),action('activity'),action('drain')])
    assert [e['outcome'] for e in activity()]==['new_install','active']
    run([action('start'),action('drain')],bundle,day=20001)
    assert len(activity())==2
check('Daily deduplication, clock rollback and restart',daily)

def versions():
    run(accepted()+[action('clock',version='0.4.4'),action('activity'),action('drain'),action('clock',version='0.4.2'),action('activity'),action('drain')])
    events=activity()
    assert [e['outcome'] for e in events]==['new_install','update','update']
    assert events[1]['previous_version']=='0.4.3' and events[2]['previous_version']=='0.4.4'
check('Upgrade and downgrade preserve previous version',versions)

def retry(route, fault):
    faults[route]=fault
    data,_=run(accepted()+[action('snapshot'),action('activity'),action('drain'),action('snapshot')])
    assert data['snapshots'][0]['last_version'] is None
    assert data['snapshots'][1]['last_version']=='0.4.3'
    assert len(activity())==(2 if route=='telemetry' else 1)
check('Token service failure is retriable',lambda:retry('profile-token','fail'))
check('Malformed token response fails closed',lambda:retry('profile-token','malformed'))
check('Failed telemetry never acknowledges local activity',lambda:retry('telemetry','fail'))

def withdrawal(route):
    faults[route]='delay'
    data,_=run([action('offer',accepted=True),action('start'),action('wait_request',key=route),action('disable'),action('drain'),action('snapshot')])
    assert not data['snapshots'][0]['has_identity'] and not data['snapshots'][0]['enabled']
    assert len(erasures())==1 and len(activity())==(1 if route=='telemetry' else 0)
    assert records[-1][1]['event']=='forget_installation'
check('Withdrawal while token is in flight cancels activity',lambda:withdrawal('profile-token'))
check('Withdrawal while POST is in flight orders erasure last',lambda:withdrawal('telemetry'))

def reaccept():
    run(accepted()+[action('disable'),action('drain'),action('choose',accepted=True),action('start'),action('drain')])
    assert len(activity())==2 and len(erasures())==1
    assert activity()[0]['installation_id']!=activity()[1]['installation_id']
    assert erasures()[0]['installation_id']==activity()[0]['installation_id']
check('Reacceptance creates a different installation identity',reaccept)

def offline_erasure():
    _,bundle=run(accepted())
    faults['telemetry']='fail'
    data,_=run([action('disable'),action('drain'),action('snapshot')],bundle)
    assert data['snapshots'][0]['pending_deletions']==1
    before=len(activity())
    data,_=run([action('start'),action('drain'),action('snapshot')],bundle)
    assert data['snapshots'][0]['pending_deletions']==0 and len(activity())==before
    assert len(erasures())==2
check('Offline erasure survives restart without re-enabling collection',offline_erasure)

def corrupt():
    data,_=run([action('seed',key='diagnostics.product_state',value='{broken'),action('seed',key='diagnostics.pending_deletions',value='{broken'),action('start'),action('drain'),action('snapshot')])
    assert not records and not data['snapshots'][0]['has_identity']
check('Corrupt preferences fail closed',corrupt)

def unrecognized_consent():
    for consent in [-1, 2, 999]:
        data,_=run([action('seed',key='diagnostics.enabled',value=True),action('seed',key='diagnostics.consent_version',value=consent),action('start'),action('drain'),action('snapshot')])
        assert not records and not data['snapshots'][0]['enabled']
check('Unknown and future consent versions fail closed',unrecognized_consent)

def stable_identity():
    _,bundle=run(accepted())
    first=activity()[0]['installation_id']
    data,_=run([action('offer'),action('start'),action('drain')],bundle,day=20001)
    assert data['prompts']==0 and len(activity())==2 and activity()[1]['installation_id']==first
    data,_=run([action('offer'),action('start'),action('drain')],bundle,day=20001,version='0.4.4')
    assert data['prompts']==0 and activity()[-1]['outcome']=='update'
    assert activity()[-1]['installation_id']==first
check('Consented identity is stable across restart',stable_identity)

def reaccept_in_flight():
    faults['profile-token']='delay'
    run([action('offer',accepted=True),action('start'),action('wait_request',key='profile-token'),action('disable'),action('choose',accepted=True),action('start'),action('drain')])
    assert len(activity())==1 and len(erasures())>=1
    assert all(e['installation_id']==erasures()[0]['installation_id'] for e in erasures())
    assert activity()[0]['installation_id']!=erasures()[0]['installation_id']
check('Rapid reacceptance cannot authorize an old in-flight identity',reaccept_in_flight)

def remote_guard():
    message,_=run([action('choose',accepted=True)],expected_code=2,base_url='https://api.luxmini.bastiencantet.com/api/v1')
    assert 'loopback-only' in message and not records
check('Harness rejects production transport before any action',remote_guard)

def bundle_guard():
    message,_=run([action('choose',accepted=True)],expected_code=2,bundle_id='com.bastiencantet.luxmini')
    assert 'disposable test bundle' in message and not records
check('Harness rejects production preference domains',bundle_guard)

print('20 process scenarios passed; production endpoints and hardware were never started.')
