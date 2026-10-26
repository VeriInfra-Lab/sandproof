"""Public create/unload/reopen assertions over the original RuntimeServices.

The sibling runtime helper owns deployment, processes, snapshots and physical
observations. HTTP acceptance is never used as proof of physical completion.
This entry checks repeated Resume/GC and never retries a write with a fresh key.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
from concurrent.futures import ThreadPoolExecutor
import copy
import hashlib
import json
import math
import shlex
from pathlib import Path
import sys
import time
import threading
import traceback
import urllib.request
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path, write_once
from server_process_crash import fetch

PREFIX = '/sandproof/lifecycle'
FACT_FIELDS = frozenset(('source_incarnation', 'capture', 'manifest_digest',
    'material_bytes', 'started_at_ms', 'end_at_ms', 'max_end_at_ms',
    'startup_deadline_ms'))


# Generated inside the original guest process, never provided in a command,
# environment variable, disk marker or reusable input. Capture must preserve it.
RAM_WITNESS_SERVER = r'''
import json, os, secrets, socket, urllib.request
opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
def metadata_hash():
    request=urllib.request.Request('http://169.254.169.254/latest/api/token',method='PUT',
        headers={'x-metadata-token-ttl-seconds':'60'})
    with opener.open(request,timeout=1) as response:
        token=response.read(8193)
        if response.status!=200 or not 0<len(token)<=8192:
            raise RuntimeError('bounded original MMDS token required')
    request=urllib.request.Request('http://169.254.169.254/',
        headers={'x-metadata-token':token.decode('ascii'),'accept':'application/json'})
    with opener.open(request,timeout=1) as response:
        payload=response.read(8193)
        if response.status!=200 or not 0<len(payload)<=8192:
            raise RuntimeError('bounded original MMDS required')
    value=json.loads(payload)['accessTokenHash']
    if not isinstance(value,str) or len(value)!=128 or any(c not in '0123456789abcdef' for c in value):
        raise RuntimeError('original MMDS credential digest required')
    return value
previous_metadata_hash=metadata_hash()
nonce=secrets.token_hex(32)
activation_nonce=secrets.token_hex(32)
metadata_generation=0
counter=0
pid=os.getpid()
start_ticks=int(open('/proc/self/stat').read().rsplit(')',1)[1].split()[19])
listener=socket.socket(socket.AF_UNIX)
listener.bind('/tmp/sandproof-ram-witness.sock')
listener.listen(1)
listener.settimeout(.25)
while True:
    try:
        current_metadata_hash=metadata_hash()
    except (OSError,ValueError,KeyError):
        current_metadata_hash=None
    if current_metadata_hash is not None and current_metadata_hash!=previous_metadata_hash:
        # Only RAM holds both the credential hash and this fresh random value.
        # This records metadata observation in RAM. Clone RNG reseeding can
        # race guest execution: this nonce does not prove a distinct VMM.
        # E7 separately binds recovery to the same host candidate identity.
        activation_nonce=secrets.token_hex(32)
        metadata_generation+=1
        previous_metadata_hash=current_metadata_hash
    try:
        connection,_=listener.accept()
    except socket.timeout:
        continue
    with connection:
        connection.settimeout(3)
        command=connection.recv(64)
        if command==b'advance':
            counter+=1
            with open('/tmp/sandproof-unload-marker','w') as marker:
                marker.write('disk-counter:'+str(counter))
                marker.flush()
                os.fsync(marker.fileno())
        elif command!=b'read':
            raise RuntimeError('unknown witness command')
        connection.sendall(json.dumps(dict(nonce=nonce,activation_nonce=activation_nonce,
            metadata_generation=metadata_generation,counter=counter,pid=pid,
            start_time_ticks=start_ticks)).encode())
'''
RAM_WITNESS_READY = r'''
import os,sys,time
pid=int(sys.argv[1])
while not os.path.exists('/tmp/sandproof-ram-witness.sock'):
    os.kill(pid,0)
    time.sleep(.01)
'''
RAM_WITNESS_CLIENT = r'''
import json,socket,sys
with socket.socket(socket.AF_UNIX) as connection:
    connection.settimeout(3)
    connection.connect('/tmp/sandproof-ram-witness.sock')
    connection.sendall(sys.argv[1].encode())
    data=bytearray()
    while True:
        part=connection.recv(4096)
        if not part: break
        data.extend(part)
        if len(data)>8192: raise RuntimeError('bounded original witness required')
print(data.decode())
'''


def uint(value, *, positive=False):
    return type(value) is int and (1 if positive else 0) <= value < 2**64


def wait_witness_generation(read, baseline, deadline):
    """Only an unchanged old generation may wait for MMDS observation.

    Each iteration performs a read, never a guest mutation or request replay.
    The original total scenario deadline also bounds this observation barrier.
    """
    while True:
        if time.monotonic() >= deadline:
            raise TimeoutError('original RAM witness metadata observation deadline')
        observed=read()
        if time.monotonic() >= deadline:
            raise TimeoutError('original RAM witness read exceeded scenario deadline')
        if (type(observed) is not dict or set(observed)!={
                'nonce','activation_nonce','metadata_generation','counter','pid','start_time_ticks'}
                or any(type(observed[key]) is not str or len(observed[key])!=64
                       or any(c not in '0123456789abcdef' for c in observed[key])
                       for key in ('nonce','activation_nonce'))
                or any(not uint(observed[key]) for key in ('counter','metadata_generation'))
                or any(not uint(observed[key],positive=True) for key in ('pid','start_time_ticks'))
                or any(observed[key]!=baseline[key] for key in ('nonce','counter','pid','start_time_ticks'))):
            raise ValueError('original RAM-only witness identity/state changed')
        generation=observed['metadata_generation']
        if generation==baseline['metadata_generation']:
            if observed['activation_nonce']!=baseline['activation_nonce']:
                raise ValueError('old metadata generation changed its RAM nonce')
            continue
        if (generation!=baseline['metadata_generation']+1
                or observed['activation_nonce']==baseline['activation_nonce']):
            raise ValueError('metadata generation regressed, skipped or reused its RAM nonce')
        return observed


def check_center_configuration(services, owner_key, foreign_key):
    """Check final fixture consistency without fixing or increasing budgets."""
    center = services.center_config
    assert len(services.host_configs) == 1, 'this scenario requires one original host'
    assert owner_key != foreign_key and owner_key and foreign_key
    assert {row['tenant'] for row in center['credentials']} == {1, 2}
    assert next(row['secret'] for row in center['credentials'] if row['tenant'] == 1) == owner_key
    assert next(row['secret'] for row in center['credentials'] if row['tenant'] == 2) == foreign_key
    assert {row['tenant'] for row in center['tenants']} == {'1', '2'}
    assert isinstance(center['center_id'], str) and center['center_id']
    assert isinstance(center['deployment_id'], str) and center['deployment_id']
    assert len(center['nodes']) == 1
    node = center['nodes'][0]['registration']['identity']
    host_inbox = services.host_configs[0]['inbox']
    assert node == host_inbox['node']
    assert host_inbox['center'] == dict(deployment=center['deployment_id'], center=center['center_id'])
    assert node['deployment_id'] == center['deployment_id']
    assert uint(node['node_generation'], positive=True)
    catalog = center['templates']['templates']
    registrations = center['template_registrations']
    assert len(catalog) == len(registrations) == 2, 'two explicit private owner templates required'
    assert {row['owner'] for row in catalog} == {1, 2}
    assert len({row['template_id'] for row in catalog}) == 2
    assert {row['template_id'] for row in catalog} == {row['template_id'] for row in registrations}
    assert len({row['revision'] for row in registrations}) == 2
    registry = center['template_registry']
    assert uint(registry['max_templates_global'], positive=True)
    assert registry['max_templates_global'] >= len(registrations), 'no implicit quota increase'
    assert registry['max_templates_per_tenant'] == 1
    return dict(deployment=center['deployment_id'], center=center['center_id'],
        node=copy.deepcopy(node), owners=['1', '2'], templates=len(catalog))


class PublicStatusMismatch(AssertionError):
    """Keep this response's fixed status even if another worker finishes later."""
    def __init__(self, actual, expected):
        super().__init__('public response status differs')
        self.actual=actual
        self.expected=expected


def failure_details(error, client, phase):
    """Finite diagnostics only; never exception text, locals or HTTP content."""
    status=(dict(expected=error.expected,actual=error.actual)
        if isinstance(error,PublicStatusMismatch)
        else copy.deepcopy(client.last_http_status) if client is not None else None)
    return dict(
        failure_step=client.failure_step if client is not None else phase,
        last_http_status=status,
        failure_stack=[dict(file=Path(frame.filename).name[:128],
                            function=frame.name[:96], line=frame.lineno)
                       for frame in traceback.extract_tb(error.__traceback__, limit=16)])


class PublicUnloadScenario:
    """One known owner/window and two saved complete requests, not a client SDK."""
    def __init__(self, base_url, owner_key, foreign_key, deployment, deadline, *, owner='1'):
        assert type(deadline) in (int, float) and math.isfinite(deadline)
        address = urlsplit(base_url)
        assert address.scheme == 'http' and address.hostname == '127.0.0.1'
        assert address.port and not address.username and not address.password
        assert not address.path and not address.query and not address.fragment
        self.base_url = base_url
        self.owner_key, self.foreign_key = owner_key, foreign_key
        self.deployment, self.deadline = deployment, deadline
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.owner = owner
        self.sequence = 0
        self.current_acceptance = None
        self.witness = None
        self.previous_tokens = []
        self.timings = []
        self.window = None
        self.session = None
        self.create_request = self.create_acceptance = None
        self.unload_request = self.unload_acceptance = None
        self.failure_step = 'client-initialization'
        self.last_http_status = None
        self._http_expected = None

    def step(self, name, expected=None):
        self.failure_step = name
        self._http_expected = expected

    def remaining(self):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('original lifecycle scenario deadline; no completion inferred')
        return remaining

    def operation_sent(self, operation):
        self.timings.append(dict(operation=operation,owner=self.owner,
            sent_monotonic=time.monotonic(),accepted_monotonic=None,completed_monotonic=None))

    def operation_accepted(self):
        sample=self.timings[-1]
        sample['accepted_monotonic']=time.monotonic()
        sample['session']=self.session
        sample['acceptance']=copy.deepcopy(self.current_acceptance)

    def operation_completed(self, phase, *, view=None, acceptance=None):
        if not self.timings:
            return
        sample=self.timings[-1]
        if sample['operation']=='unload':
            if (sample['accepted_monotonic'] is None or sample['completed_monotonic'] is not None
                    or view is None or acceptance is None
                    or acceptance!=sample['acceptance'] or acceptance!=self.current_acceptance
                    or acceptance!=self.unload_acceptance):
                return
            self.check_view(view,acceptance)
            outcome=view['unload_outcome']
            if phase==view['phase']=='unloaded' and outcome['state']=='unloaded':
                completed='unloaded'
            elif (phase==view['phase']=='running' and outcome is not None
                    and outcome['state']=='original_continued'):
                completed='original_continued'
            else:
                return
            sample['completed_monotonic']=time.monotonic()
            sample['completion_phase']=completed
            return
        expected={'create':'running','resume':'running','unload':'unloaded','destroy':'cleaned'}
        if (sample['accepted_monotonic'] is not None and sample['completed_monotonic'] is None
                and (expected[sample['operation']]==phase
                    or (sample['operation']=='resume' and phase=='state_lost'))):
            sample['completed_monotonic']=time.monotonic()
            sample['completion_phase']=phase

    def operation_evidence_completed(self):
        sample=self.timings[-1]
        assert sample['completed_monotonic'] is not None
        assert 'evidence_monotonic' not in sample
        sample['evidence_monotonic']=time.monotonic()

    def timing_report(self):
        result=[]
        for sample in self.timings:
            sent,accepted,completed=(sample[key+'_monotonic'] for key in ('sent','accepted','completed'))
            result.append(dict(operation=sample['operation'],owner=sample['owner'],
                session=sample.get('session',self.session),acceptance=sample.get('acceptance'),
                send_to_accept_seconds=None if accepted is None else accepted-sent,
                accept_to_public_completion_seconds=None if completed is None else completed-accepted,
                send_to_public_completion_seconds=None if completed is None else completed-sent,
                send_to_evidence_completion_seconds=None if 'evidence_monotonic' not in sample
                    else sample['evidence_monotonic']-sent,
                public_completion_to_evidence_seconds=None if 'evidence_monotonic' not in sample
                    else sample['evidence_monotonic']-completed,
                completion_condition=sample.get('completion_phase',
                    {'create':'running','resume':'running','unload':'unloaded',
                    'destroy':'cleaned'}[sample['operation']]),
                complete=completed is not None))
        return result

    def call(self, method, path, *, key, body=None):
        # Existing HTTP helper supplies the original X-API-Key header. Failure
        # of a write propagates: a new window/sequence is never allocated here.
        result = fetch(self.opener, self.base_url, method, path, key, body,
                       timeout=min(40, self.remaining()))
        self.last_http_status = dict(expected=self._http_expected, actual=result[0])
        return result

    def read_json(self, method, path, *, key, body=None, status=200):
        self._http_expected = status
        actual, encoded = self.call(method, path, key=key, body=body)
        if actual!=status:
            raise PublicStatusMismatch(actual,status)
        value = json.loads(encoded, object_pairs_hook=unique_keys)
        assert type(value) is dict
        return value

    def key(self, sequence):
        return dict(deployment=self.deployment, owner=self.owner, window=self.window, sequence=sequence)

    def acceptance(self, value, *, sequence, version):
        assert set(value) == {'session', 'key', 'accepted_version'}
        assert value['key'] == self.key(sequence), 'acceptance changed the full original operation key'
        assert type(value['session']) is str and value['session']
        assert uint(value['accepted_version'], positive=True)
        assert value['accepted_version'] == version
        if self.session is not None:
            assert value['session'] == self.session
        return value

    def create(self, parameters):
        assert self.create_request is None, 'one original create only'
        self.step('request-window', 200)
        window = self.read_json('GET', PREFIX + '/request-window', key=self.owner_key)
        assert set(window) == {'window'} and uint(window['window'], positive=True)
        self.window = window['window']
        self.create_request = dict(window=self.window, sequence=1, parameters=copy.deepcopy(parameters))
        self.step('create-acceptance', 202)
        self.operation_sent('create')
        result = self.read_json('POST', PREFIX + '/create', key=self.owner_key,
                                body=self.create_request, status=202)
        self.create_acceptance = self.acceptance(result, sequence=1, version=1)
        self.session = result['session']
        self.current_acceptance = self.create_acceptance
        self.sequence = 1
        self.operation_accepted()
        return copy.deepcopy(result)

    def path(self):
        assert self.session is not None
        # Session IDs use the protocol's bounded ASCII identifier alphabet.
        assert 0 < len(self.session) <= 128
        assert all(char.isascii() and (char.isalnum() or char in '-_.') for char in self.session)
        return PREFIX + '/sessions/' + self.session

    def query(self):
        return self.read_json('GET', self.path(), key=self.owner_key)

    def check_view(self, view, acceptance):
        assert set(view) == {'session', 'operation', 'current_version', 'phase', 'unload_outcome', 'create_refusal'}
        assert view['session'] == self.session and view['operation'] == acceptance['key']
        assert uint(view['current_version'], positive=True)
        assert view['current_version'] == acceptance['accepted_version']
        assert view['create_refusal'] is None
        outcome=view['unload_outcome']
        if view['phase']=='unloading':
            assert outcome=={'state':'pending'}, 'pending unload result required'
        elif view['phase']=='unloaded':
            assert type(outcome) is dict and set(outcome)=={'state','facts'}
            assert outcome['state']=='unloaded'
        elif outcome is not None:
            # A current loss does not rewrite a failed Unload's historical
            # successful continuation. The same result must survive replay.
            assert view['phase'] in ('running','state_lost')
            assert type(outcome) is dict and set(outcome)=={'state','facts'}
            assert outcome['state']=='original_continued', 'running failed-Unload result required'
        return view

    def wait_phase(self, acceptance, *, waiting, complete):
        self.step('wait-' + complete, 200)
        while True:
            # A transient unavailable authority is never converted into a
            # successful fact. Authentication/not-found/conflict errors fail.
            status, encoded = self.call('GET', self.path(), key=self.owner_key)
            if status == 503:
                time.sleep(min(.1, self.remaining()))
                continue
            assert status == 200, f'public query failed with {status}'
            view = json.loads(encoded, object_pairs_hook=unique_keys)
            assert type(view) is dict
            self.check_view(view, acceptance)
            if view['phase'] == complete:
                self.remaining()
                self.operation_completed(complete,view=view,acceptance=acceptance)
                return view
            assert view['phase'] in waiting, 'unexpected lifecycle phase; no completion inferred'
            time.sleep(min(.1, self.remaining()))

    def wait_running(self):
        return self.wait_phase(self.current_acceptance, waiting={'creating','resuming'}, complete='running')

    def assert_owner_isolation(self):
        path = self.path()
        self.step('foreign-lifecycle-query', 404)
        assert self.call('GET', path, key=self.foreign_key)[0] == 404
        self.step('unauthenticated-lifecycle-query', 401)
        assert self.call('GET', path, key=None)[0] == 401
        if self.unload_request is not None:
            self.step('foreign-unload', 404)
            assert self.call('POST', path + '/unload', key=self.foreign_key,
                             body=self.unload_request)[0] == 404

    def unload(self, running, *, concurrent_duplicate=False):
        self.step('unload-input')
        self.check_view(running, self.current_acceptance)
        assert running['phase'] == 'running'
        self.sequence += 1
        self.unload_request = dict(window=self.window, sequence=self.sequence,
                                  expected_version=running['current_version'])
        self.step('unload-acceptance', 202)
        self.operation_sent('unload')
        if concurrent_duplicate:
            # Two workers rendezvous immediately before the original HTTP
            # call. This schedules duplicate delivery, not two business keys.
            barrier=threading.Barrier(3)
            def send():
                barrier.wait(timeout=min(40,self.remaining()))
                return self.read_json('POST',self.path()+'/unload',key=self.owner_key,
                    body=copy.deepcopy(self.unload_request),status=202)
            with ThreadPoolExecutor(max_workers=2,thread_name_prefix='same-unload-key') as executor:
                pending=[executor.submit(send) for _ in range(2)]
                barrier.wait(timeout=min(40,self.remaining()))
                replies=[future.result(timeout=self.remaining()) for future in pending]
            self.remaining()
            assert replies[0]==replies[1], 'concurrent original unload replies differ'
            result=replies[0]
            self.concurrent_unload_replies=copy.deepcopy(replies)
        else:
            result = self.read_json('POST', self.path() + '/unload', key=self.owner_key,
                                    body=self.unload_request, status=202)
        self.unload_acceptance = self.acceptance(result, sequence=self.sequence,
            version=running['current_version'] + 1)
        self.current_acceptance = self.unload_acceptance
        self.operation_accepted()
        return copy.deepcopy(result)

    def resume(self, unloaded):
        self.check_view(unloaded,self.current_acceptance)
        assert unloaded['phase']=='unloaded'
        self.sequence += 1
        self.resume_request=dict(window=self.window,sequence=self.sequence,
            expected_version=unloaded['current_version'])
        self.step('resume-acceptance',202)
        self.operation_sent('resume')
        result=self.read_json('POST',self.path()+'/resume',key=self.owner_key,
            body=self.resume_request,status=202)
        self.current_acceptance=self.acceptance(result,sequence=self.sequence,
            version=unloaded['current_version']+1)
        self.operation_accepted()
        return copy.deepcopy(self.current_acceptance)

    def destroy(self, current):
        self.check_view(current,self.current_acceptance)
        self.sequence += 1
        self.destroy_request=dict(window=self.window,sequence=self.sequence,
            expected_version=current['current_version'])
        self.step('destroy-acceptance',202)
        self.operation_sent('destroy')
        result=self.read_json('POST',self.path()+'/destroy',key=self.owner_key,
            body=self.destroy_request,status=202)
        self.current_acceptance=self.acceptance(result,sequence=self.sequence,
            version=current['current_version']+1)
        self.operation_accepted()
        return copy.deepcopy(self.current_acceptance)

    def wait_cleaned(self):
        return self.wait_phase(self.current_acceptance,waiting={'terminating'},complete='cleaned')

    def wait_unloaded(self):
        return self.wait_phase(self.unload_acceptance, waiting={'unloading'}, complete='unloaded')

    def assert_unloaded_facts(self, view, independent_facts):
        self.step('unloaded-facts')
        self.check_view(view, self.unload_acceptance)
        assert view['phase'] == 'unloaded'
        facts = view['unload_outcome']['facts']
        assert type(facts) is dict and set(facts) == FACT_FIELDS
        assert type(independent_facts) is dict and set(independent_facts) == FACT_FIELDS
        assert facts == independent_facts, 'public result differs from original journal/material/lifetime evidence'
        assert all(uint(facts[name], positive=True) for name in (
            'source_incarnation', 'capture', 'material_bytes'))
        assert all(uint(facts[name]) for name in (
            'started_at_ms', 'end_at_ms', 'max_end_at_ms', 'startup_deadline_ms'))
        assert facts['started_at_ms'] <= facts['startup_deadline_ms']
        assert facts['started_at_ms'] < facts['end_at_ms'] <= facts['max_end_at_ms']
        digest = facts['manifest_digest']
        assert type(digest) is list and len(digest) == 32
        assert all(type(byte) is int and 0 <= byte <= 255 for byte in digest)

    def assert_original_replay(self, saved):
        self.step('replay-create', 202)
        create = self.read_json('POST', PREFIX + '/create', key=self.owner_key,
                               body=self.create_request, status=202)
        self.step('replay-unload', 202)
        unload = self.read_json('POST', self.path() + '/unload', key=self.owner_key,
                               body=self.unload_request, status=202)
        assert create == self.create_acceptance and unload == self.unload_acceptance
        self.step('replay-query', 200)
        assert self.query() == saved
        changed = dict(self.unload_request, expected_version=self.unload_acceptance['accepted_version'])
        self.step('altered-unload-replay', 409)
        assert self.call('POST', self.path() + '/unload', key=self.owner_key, body=changed)[0] == 409
        self.step('altered-replay-query', 200)
        assert self.query() == saved, 'rejected altered replay changed the saved result'

    def assert_reopened(self, saved):
        self.step('reopened-query', 200)
        assert self.query() == saved, 'manager reopen changed the original complete public result'
        self.step('request-window', 200)
        window = self.read_json('GET', PREFIX + '/request-window', key=self.owner_key)
        assert window == {'window': self.window}
        self.assert_owner_isolation()
        self.assert_original_replay(saved)
        self.remaining()

    def sdk(self):
        """Original no-create SDK constructor; does not renew the lifetime."""
        import importlib.metadata
        from e2b import Sandbox
        from e2b.connection_config import ConnectionConfig
        from packaging.version import Version
        assert importlib.metadata.version('e2b') == '2.51.0'
        self.step('owner-sdk-detail', 200)
        detail=self.read_json('GET','/sandboxes/'+self.session,key=self.owner_key)
        assert detail['sandboxID']==self.session and detail['clientID']=='tenant-'+self.owner
        assert detail['state']=='running'
        token=detail['envdAccessToken']
        assert type(token) is str and token
        if hasattr(self,'data_token') and token!=self.data_token:
            self.previous_tokens.append(self.data_token)
        self.data_token=token
        connection=ConnectionConfig(api_url=self.base_url,sandbox_url=self.base_url,
            domain=detail['domain'],api_key=self.owner_key,retries=0,
            request_timeout=min(30,self.remaining()),
            extra_sandbox_headers={'X-Access-Token':token})
        return Sandbox(sandbox_id=self.session,sandbox_domain=detail['domain'],
            envd_version=Version(detail['envdVersion']),envd_access_token=token,
            connection_config=connection)

    def witness_command(self,sandbox,command):
        self.step('ram-witness-'+command)
        result=sandbox.commands.run('python3 -c '+shlex.quote(RAM_WITNESS_CLIENT)+' '+command,
            timeout=min(30,self.remaining()))
        assert result.exit_code==0
        value=json.loads(result.stdout,object_pairs_hook=unique_keys)
        assert set(value)=={'nonce','activation_nonce','metadata_generation','counter','pid','start_time_ticks'}
        assert type(value['nonce']) is str and len(value['nonce'])==64
        assert all(c in '0123456789abcdef' for c in value['nonce'])
        assert all(uint(value[key],positive=True) for key in ('pid','start_time_ticks'))
        assert uint(value['counter']) and uint(value['metadata_generation'])
        assert type(value['activation_nonce']) is str and len(value['activation_nonce'])==64
        assert all(c in '0123456789abcdef' for c in value['activation_nonce'])
        return value

    def run_public_workload(self,running):
        sandbox=self.sdk()
        self.step('start-original-ram-process')
        process=sandbox.commands.run('python3 -c '+shlex.quote(RAM_WITNESS_SERVER),
            background=True,timeout=0)
        # SDK command completion is not a startup barrier. A separate real
        # command waits for the socket while checking the original guest PID.
        self.step('ram-process-ready')
        ready=sandbox.commands.run('python3 -c '+shlex.quote(RAM_WITNESS_READY)+' '+str(process.pid),
            timeout=min(30,self.remaining()))
        assert ready.exit_code==0
        self.witness=self.witness_command(sandbox,'advance')
        assert self.witness['pid']==process.pid and self.witness['counter']==1
        disk=sandbox.files.read('/tmp/sandproof-unload-marker')
        assert disk=='disk-counter:1'
        assert self.query()==running
        return dict(sdk='2.51.0',entry='owner-detail/original-sdk-constructor/public-gateway',
            path='/tmp/sandproof-unload-marker',ram_nonce_sha256=hashlib.sha256(
                self.witness['nonce'].encode()).hexdigest(),guest_pid=process.pid,
            guest_start_time_ticks=self.witness['start_time_ticks'],counter=1)

    def assert_resumed_workload(self,running,gate_receipt=None,physical_running=None):
        previous=copy.deepcopy(self.witness)
        old_token=self.data_token
        sandbox=self.sdk()
        assert self.data_token!=old_token,'resumed execution reused old authorization'
        self.assert_previous_tokens_closed()
        observed=wait_witness_generation(lambda:self.witness_command(sandbox,'read'),
            previous,self.deadline)
        if gate_receipt is not None:
            from lifecycle_unload_runtime import assert_same_restore_candidate
            assert_same_restore_candidate(physical_running,gate_receipt)
            assert gate_receipt['guest_pid']==observed['pid']
            assert gate_receipt['guest_start_time_ticks']==observed['start_time_ticks']
            assert gate_receipt['activation_nonce_sha256']==hashlib.sha256(
                observed['activation_nonce'].encode()).hexdigest(), 'recovered RAM state differs from the sealed gate observation'
        assert sandbox.files.read('/tmp/sandproof-unload-marker')=='disk-counter:'+str(previous['counter'])
        current=self.witness_command(sandbox,'advance')
        assert {key:current[key] for key in ('nonce','pid','start_time_ticks')}=={
            key:previous[key] for key in ('nonce','pid','start_time_ticks')}
        assert current['counter']==previous['counter']+1
        assert current['metadata_generation']==observed['metadata_generation']
        assert current['activation_nonce']==observed['activation_nonce']
        assert sandbox.files.read('/tmp/sandproof-unload-marker')=='disk-counter:'+str(current['counter'])
        self.witness=current
        assert self.query()==running
        return dict(ram_nonce_sha256=hashlib.sha256(current['nonce'].encode()).hexdigest(),
            guest_pid=current['pid'],guest_start_time_ticks=current['start_time_ticks'],
            counter=current['counter'],metadata_generation=current['metadata_generation'],
            activation_nonce_sha256=hashlib.sha256(current['activation_nonce'].encode()).hexdigest(),
            new_authorization=True,old_authorizations_rejected=True)

    def assert_previous_tokens_closed(self):
        import httpx
        with httpx.Client(base_url=self.base_url,trust_env=False,timeout=min(10,self.remaining())) as client:
            for token in self.previous_tokens:
                response=client.get('/files',params={'path':'/tmp/sandproof-unload-marker'},
                    headers={'X-Access-Token':token})
                assert response.status_code==401,'a prior execution token regained authority'

    def assert_continued_workload(self,running,lifetime):
        self.step('original-continued-workload')
        self.check_view(running,self.unload_acceptance)
        assert running['phase']=='running'
        facts=running['unload_outcome']['facts']
        assert set(facts)==set(lifetime)|{'observed_at_ms'}
        assert {key:facts[key] for key in lifetime}==lifetime, 'failed Unload changed original lifetime'
        assert uint(facts['observed_at_ms'])
        assert facts['started_at_ms']<=facts['observed_at_ms']<facts['end_at_ms']
        previous=copy.deepcopy(self.witness)
        token=self.data_token
        sandbox=self.sdk()
        assert self.data_token==token, 'original continuation changed execution authorization'
        observed=self.witness_command(sandbox,'read')
        assert observed==previous, 'failed Unload changed the continued RAM or activation state'
        assert sandbox.files.read('/tmp/sandproof-unload-marker')=='disk-counter:'+str(previous['counter'])
        current=self.witness_command(sandbox,'advance')
        assert {key:current[key] for key in current if key!='counter'}=={
            key:previous[key] for key in previous if key!='counter'}
        assert current['counter']==previous['counter']+1
        assert sandbox.files.read('/tmp/sandproof-unload-marker')=='disk-counter:'+str(current['counter'])
        self.assert_previous_tokens_closed()
        self.witness=current
        assert self.query()==running
        return dict(same_ram_and_activation=True,same_guest_process=True,
            same_authorization=True,same_lifetime=True,counter=current['counter'])

    def assert_data_closed(self):
        import httpx
        self.step('old-data-token', 401)
        assert self.data_token
        with httpx.Client(base_url=self.base_url, trust_env=False,
                          timeout=min(10, self.remaining())) as client:
            response = client.get('/files', params={'path': '/tmp/sandproof-unload-marker'},
                                  headers={'X-Access-Token': self.data_token})
            self.last_http_status = dict(expected=401, actual=response.status_code)
            assert response.status_code == 401, 'original token still authorizes data after Unload'
        self.step('old-sdk-detail', 404)
        assert self.call('GET', '/sandboxes/' + self.session, key=self.owner_key)[0] == 404


def exercise(args):
    """Only the sibling helper can start/restart or inspect original resources."""
    from lifecycle_unload_runtime import prepare
    restore_fault=args.scenario in ('restore-activation-loss','restore-candidate-exit')
    continued_exit=args.scenario in ('continued-after-capture-exit','continued-before-admission-exit')
    capture_stages = {
        'capture-freeze-receipt': 'freeze-receipt',
        'capture-copy-partial': 'copy-partial',
        'capture-seal-receipt': 'seal-receipt',
        'capture-stop-receipt': 'stop-receipt',
    }
    capture_stage = capture_stages.get(args.scenario)
    prepare_destroy = args.scenario == 'restore-prepare-destroy'
    prepare_fault = args.scenario in ('restore-prepare-receipt', 'restore-prepare-destroy')
    data_drain = args.scenario == 'data-drain'
    if args.scenario!='normal' and not (restore_fault or continued_exit or capture_stage or prepare_fault or data_drain):
        raise ValueError('one explicit existing lifecycle scenario required')
    if args.scenario!='normal' and (type(args.activation_gate_timeout_ms) is not int
            or not 1000<=args.activation_gate_timeout_ms<=120000):
        raise ValueError('bounded original activation gate timeout required before preparing services')
    seconds = args.scenario_timeout
    if type(seconds) not in (int, float) or not math.isfinite(seconds) or not 0 < seconds <= 3600:
        raise ValueError('explicit finite scenario deadline at most one hour required')
    deadline = time.monotonic() + seconds
    runtime = prepare(args)
    root = private_path(runtime.root, directory=True)
    report = dict(schema=1, kind='create-unload-reopen-resume-destroy-system-scenario',
                  scenario=args.scenario,
                  complete=False, phase='configuration', scope='normal lifecycle and physical materials; not complete E1-E16 or formal proof')
    client = control = None
    capture_probe = data_probe = None
    terminal_destroy_done = False
    failed = False
    try:
        report['measurement_identity']=runtime.measurement_identity()
        report['registration'] = check_center_configuration(
            runtime.services, runtime.owner_key, runtime.foreign_key)
        report['phase'] = 'original-services-start'
        runtime.start(deadline)
        client = PublicUnloadScenario(runtime.services.sdk_url, runtime.owner_key,
            runtime.foreign_key, report['registration']['deployment'], deadline)
        report['phase'] = 'public-create'
        report['create'] = client.create(runtime.create_parameters())
        running = client.wait_running()
        client.assert_owner_isolation()
        report['phase'] = 'public-workload'
        report['workload'] = client.run_public_workload(running)
        client.step('running-native-evidence')
        report['running'] = runtime.record_running(client.session, deadline)
        client.operation_evidence_completed()
        lifetime = report['running']['lifetime']
        assert set(lifetime) == {'started_at_ms', 'end_at_ms', 'max_end_at_ms', 'startup_deadline_ms'}
        if data_drain:
            from lifecycle_unload_data_drain import InflightDataDrain
            gate = runtime.arm_data_drain(deadline)
            data_probe = InflightDataDrain(client, report['running'],
                runtime.services.native_pools[0], runtime.services.host_configs[0],
                gate, runtime.lease.identity)
            report['data_drain_begin'] = data_probe.begin()
        report['phase'] = 'public-unload'
        report['unload'] = client.unload(running,concurrent_duplicate=True)
        report['concurrent_unload']=dict(requests=2,distinct_business_keys=1,
            same_acceptance=client.concurrent_unload_replies[0]==client.concurrent_unload_replies[1],
            acceptance=copy.deepcopy(client.unload_acceptance),
            send_control='two HTTP workers and coordinator rendezvous on one barrier; no sleep')
        # This checks new authorization closes at durable Unload acceptance;
        # it does not claim existing holders or physical workers have exited.
        if data_probe is not None:
            report['phase'] = 'original-inflight-drain-held'
            report['data_drain_held'] = data_probe.wait_held(client.unload_acceptance)
            data_probe.release()
        client.assert_data_closed()
        saved = client.wait_unloaded()
        client.assert_owner_isolation()
        report['phase'] = 'original-physical-evidence'
        client.step('unload-physical-evidence')
        physical = runtime.unload_evidence(client.session, deadline, client.unload_acceptance['key'])
        client.assert_unloaded_facts(saved, physical['facts'])
        client.operation_evidence_completed()
        assert {key: physical['facts'][key] for key in lifetime} == lifetime
        report['before_restart'] = saved
        report['physical_before_restart'] = physical
        client.assert_original_replay(saved)
        report['concurrent_unload'].update(terminal=copy.deepcopy(saved),
            original_capture_responsibilities=physical['original_capture_responsibilities'],
            altered_same_key_status=409,altered_same_key_state_unchanged=True,
            window_expiry='not exercised: no controlled production clock in this real scenario')
        report['phase'] = 'same-manager-config-reopen'
        client.step('restart-managers')
        restart_started=time.monotonic()
        report['manager_restart'] = runtime.restart_managers(deadline,
            original_unload=(client.unload_acceptance,saved))
        report['concurrent_unload']['original_business_operation']=report['manager_restart']['original_unload_operation']
        restart_ready=time.monotonic()
        client.assert_reopened(saved)
        restart_query_completed=time.monotonic()
        report['manager_restart_timing']=dict(
            restart_to_managers_ready_seconds=restart_ready-restart_started,
            ready_to_original_unloaded_query_seconds=restart_query_completed-restart_ready,
            restart_to_original_unloaded_query_seconds=restart_query_completed-restart_started,
            completion_condition='same accepted Unloaded result from original public query')
        client.assert_data_closed()
        client.step('unload-physical-evidence')
        physical = runtime.unload_evidence(client.session, deadline, client.unload_acceptance['key'])
        client.assert_unloaded_facts(saved, physical['facts'])
        report['physical_after_restart'] = physical
        report['phase']='actual-slot-reuse'
        control=PublicUnloadScenario(runtime.services.sdk_url,runtime.foreign_key,
            runtime.owner_key,report['registration']['deployment'],deadline,owner='2')
        control.create(runtime.create_parameters(owner=2))
        control_running=control.wait_running()
        control.assert_owner_isolation()
        control_physical=runtime.record_running(control.session,deadline)
        control.operation_evidence_completed()
        assert control.session!=client.session
        assert control_physical['context']['position']==physical['original_position']['position'], \
            'another session did not actually reuse the old running slot'
        report['slot_reuse']=dict(occupant_session=control.session,
            position=control_physical['context']['position'])
        report['cycles']=[]
        lost=None
        for cycle in (1,2):
            report['phase']='public-resume-'+str(cycle)
            gate_receipt=None
            if cycle==1 and restore_fault:
                report['phase']='arm-original-restore-activation-gate'
                runtime.arm_restore_gate(client.session,deadline,args.activation_gate_timeout_ms)
            if cycle == 1 and prepare_fault:
                from lifecycle_unload_capture_faults import CaptureHandoffProbe
                capture_probe = CaptureHandoffProbe(runtime, client, report['running'],
                    'restore-prepare-destroy' if prepare_destroy else 'restore-prepare-receipt')
                report['capture_handoff_arm'] = capture_probe.arm(deadline, args.activation_gate_timeout_ms)
            accepted=client.resume(saved)
            if cycle == 1 and prepare_fault:
                report['capture_handoff_observation'] = capture_probe.after_acceptance(accepted, deadline)
                if prepare_destroy:
                    report['phase'] = 'hold-exact-pre-marker-loaded-process'
                    report['prepare_destroy_pause'] = capture_probe.pause_original_prepare(deadline)
                    report['phase'] = 'public-destroy-while-original-resume-is-unknown'
                    pending = client.query()
                    client.check_view(pending, accepted)
                    assert pending['phase'] == 'resuming', 'pre-marker Prepare unexpectedly became Running'
                    client.assert_data_closed(); client.assert_previous_tokens_closed()
                    report['destroy'] = client.destroy(pending)
                    report['phase'] = 'original-host-terminal-fence-before-process-continue'
                    def observe_original_peer():
                        assert control.query() == control_running, 'other owner lost its original Running observation'
                        return dict(session=control.session, original_public_running_query_completed=True,
                                    scope='one existing peer query while original loaded process is stopped')
                    report['prepare_destroy_recovery'] = capture_probe.recover_after_destroy(
                        report['destroy'], deadline, observe_peer=observe_original_peer)
                    report['phase'] = 'original-prepared-and-stop-receipts-before-gc'
                    report['prepare_destroy_terminal'] = capture_probe.assert_terminal_before_gc(deadline)
                    report['phase'] = 'public-destroy-cleaned-and-physical-gc'
                    client.assert_data_closed(); client.assert_previous_tokens_closed()
                    client.wait_cleaned()
                    report['final_resources'] = runtime.destroy_evidence(client.session, deadline)
                    client.operation_evidence_completed()
                    assert control.query() == control_running
                    report['slot_reuse']['occupant_after_prepare_destroy'] = runtime.confirm_running(
                        control.session, control_physical, deadline)
                    control.destroy(control_running); control.wait_cleaned()
                    report['slot_reuse']['occupant_cleanup'] = runtime.destroy_evidence(control.session, deadline)
                    control.operation_evidence_completed()
                    report['cycles'].append(dict(cycle=cycle, resume=accepted,
                        destroyed_before_public_running=True, original_prepare_then_stop_observed=True))
                    terminal_destroy_done = True
                    break
                report['capture_handoff_recovery'] = capture_probe.finish(deadline)
            if cycle==1 and restore_fault:
                report['phase']='original-restore-activation-gate-read'
                gate_receipt=runtime.wait_resume_gate(client.session,accepted,
                    copy.deepcopy(client.witness),deadline)
                report['activation_gate_observation']=gate_receipt
                if args.scenario=='restore-candidate-exit':
                    report['phase']='original-restore-candidate-pidfd-exit'
                    report['activation_gate_recovery']=runtime.exit_resume_gate(gate_receipt,accepted,deadline)
                    report['phase']='public-original-state-lost'
                    lost=client.wait_phase(accepted,waiting={'resuming'},complete='state_lost')
                    client.assert_data_closed()
                    client.assert_previous_tokens_closed()
                    client.assert_owner_isolation()
                    report['lost_candidate']=runtime.lost_candidate_evidence(gate_receipt,accepted,deadline)
                    client.operation_evidence_completed()
                    client.step('original-lost-resume-replay',202)
                    replay=client.read_json('POST',client.path()+'/resume',key=client.owner_key,
                        body=client.resume_request,status=202)
                    assert replay==accepted, 'original Resume replay created another acceptance'
                    assert client.query()==lost, 'original Resume replay revived lost state'
                    report['phase']='lost-state-same-manager-config-reopen'
                    report['lost_manager_restart']=runtime.restart_managers(deadline)
                    assert client.query()==lost, 'manager restart changed original StateLost result'
                    report['lost_candidate_after_restart']=runtime.lost_candidate_evidence(gate_receipt,accepted,deadline)
                    client.assert_data_closed()
                    client.assert_previous_tokens_closed()
                    assert control.query()==control_running
                    report['slot_reuse']['occupant_after_loss']=runtime.confirm_running(
                        control.session,control_physical,deadline)
                    assert report['activation_gate_recovery']['candidate_position']!=control_physical['context']['position']
                    control.destroy(control_running)
                    control.wait_cleaned()
                    report['slot_reuse']['occupant_cleanup']=runtime.destroy_evidence(control.session,deadline)
                    control.operation_evidence_completed()
                    report['cycles'].append(dict(cycle=cycle,resume=accepted,state_lost=lost,
                        original_replay_same_acceptance=True,original_state_preserved_after_restart=True))
                    break
                report['phase']='original-activation-manager-lost-receipt'
                report['activation_gate_recovery']=runtime.recover_resume_gate(gate_receipt,deadline)
            resumed=client.wait_running()
            physical_running=runtime.record_running(client.session,deadline,
                resume_acceptance=accepted,gate_receipt=gate_receipt)
            continuity=client.assert_resumed_workload(resumed,gate_receipt=gate_receipt,
                physical_running=physical_running)
            if cycle == 1 and prepare_fault:
                report['capture_handoff_same_candidate'] = capture_probe.assert_restored_candidate(physical_running)
            if cycle == 1 and data_probe is not None:
                report['data_drain_resumed'] = data_probe.assert_resumed()
            client.operation_evidence_completed()
            client.assert_owner_isolation()
            report['cycles'].append(dict(cycle=cycle,resume=accepted,
                running=physical_running,continuity=continuity))
            if cycle==1 and continued_exit:
                assert physical_running['context']['position']!=control_physical['context']['position'], \
                    'original session did not restore into a different actual slot'
                report['slot_reuse']['restored_position']=physical_running['context']['position']
                if args.scenario=='continued-before-admission-exit':
                    report['phase']='second-owner-original-unload-resume'
                    report['control_workload']=control.run_public_workload(control_running)
                    control.unload(control_running)
                    control_saved=control.wait_unloaded()
                    control_material=runtime.unload_evidence(control.session,deadline,control.unload_acceptance['key'])
                    control.assert_unloaded_facts(control_saved,control_material['facts'])
                    control.operation_evidence_completed()
                    control_resume=control.resume(control_saved)
                    control_running=control.wait_running()
                    control_physical=runtime.record_running(control.session,deadline,resume_acceptance=control_resume)
                    report['control_continuity']=control.assert_resumed_workload(control_running,
                        physical_running=control_physical)
                    control.operation_evidence_completed()
                report['phase']='arm-original-full-no-request'
                runtime.arm_capture_cancel_gate(client.session,deadline,args.activation_gate_timeout_ms)
                continued_acceptance=client.unload(resumed)
                report['phase']='cancel-original-full-release'
                report['capture_cancel']=runtime.cancel_original_full(continued_acceptance,deadline)
                report['phase']='public-after-capture-original-continued'
                continued=client.wait_phase(continued_acceptance,waiting={'unloading'},complete='running')
                report['continued_workload']=client.assert_continued_workload(continued,lifetime)
                stage='after_capture'
                after=runtime.continued_evidence(continued_acceptance,continued,stage,deadline)
                report['after_capture_continued']=after
                client.operation_evidence_completed()
                if args.scenario=='continued-before-admission-exit':
                    report['phase']='natural-before-admission-material-refusal'
                    reservation=runtime.capture_cancel_gate['capture_record']['capture']['reserved_material_bytes']
                    assert after['source_records']==3
                    assert after['held_material_bytes']==3*reservation
                    assert after['held_material_bytes']<=after['max_material_bytes']<after['held_material_bytes']+reservation
                    report['natural_material_pressure']=dict(held_bytes=after['held_material_bytes'],
                        next_reservation_bytes=reservation,max_material_bytes=after['max_material_bytes'])
                    continued_acceptance=client.unload(continued)
                    continued=client.wait_phase(continued_acceptance,waiting={'unloading'},complete='running')
                    report['before_admission_workload']=client.assert_continued_workload(continued,lifetime)
                    stage='before_admission'
                    report['before_admission_continued']=runtime.continued_evidence(
                        continued_acceptance,continued,stage,deadline)
                    client.operation_evidence_completed()
                client.assert_original_replay(continued)
                report['phase']='original-continued-candidate-pidfd-exit'
                report['continued_candidate_exit']=runtime.exit_continued_candidate(
                    continued_acceptance,continued,stage,deadline)
                report['phase']='public-continued-state-lost'
                lost=client.wait_phase(continued_acceptance,waiting={'running'},complete='state_lost')
                assert lost['unload_outcome']==continued['unload_outcome'], 'loss rewrote historical continued result'
                client.assert_data_closed();client.assert_previous_tokens_closed();client.assert_owner_isolation()
                report['continued_lost']=runtime.continued_lost_evidence(continued_acceptance,lost,stage,deadline)
                client.assert_original_replay(lost)
                report['phase']='continued-loss-same-manager-config-reopen'
                report['continued_lost_manager_restart']=runtime.restart_managers(deadline)
                client.assert_reopened(lost)
                report['continued_lost_after_restart']=runtime.continued_lost_evidence(
                    continued_acceptance,lost,stage,deadline)
                client.assert_data_closed();client.assert_previous_tokens_closed()
                assert control.query()==control_running
                report['slot_reuse']['occupant_after_loss']=runtime.confirm_running(control.session,control_physical,deadline)
                control.destroy(control_running);control.wait_cleaned()
                report['slot_reuse']['occupant_cleanup']=runtime.destroy_evidence(control.session,deadline)
                control.operation_evidence_completed()
                report['cycles'][-1]['continued_state_lost']=lost
                report['cycles'][-1]['continued_stage']=stage
                break
            if cycle==1:
                assert physical_running['context']['position']!=control_physical['context']['position'], \
                    'original session did not restore into a different actual slot'
                assert control.query()==control_running
                report['slot_reuse']['occupant_after_resume']=runtime.confirm_running(
                    control.session,control_physical,deadline)
                report['slot_reuse']['restored_position']=physical_running['context']['position']
                control.destroy(control_running)
                control.wait_cleaned()
                report['slot_reuse']['occupant_cleanup']=runtime.destroy_evidence(control.session,deadline)
                control.operation_evidence_completed()
                if capture_stage is not None:
                    from lifecycle_unload_capture_faults import CaptureHandoffProbe
                    gate = (Path(runtime.quota['deployment_control']) / 'capture-copy-gate'
                            if capture_stage == 'copy-partial' else None)
                    capture_probe = CaptureHandoffProbe(runtime, client, physical_running,
                        capture_stage, gate)
                    report['capture_handoff_arm'] = capture_probe.arm(deadline, args.activation_gate_timeout_ms)
                report['phase']='second-public-unload'
                accepted_unload = client.unload(resumed)
                if capture_stage is not None:
                    report['phase'] = 'original-capture-handoff-observation'
                    report['capture_handoff_observation'] = capture_probe.after_acceptance(accepted_unload, deadline)
                    report['capture_handoff_recovery'] = capture_probe.finish(deadline)
                    if capture_stage == 'copy-partial':
                        resumed = client.wait_phase(accepted_unload, waiting={'unloading'}, complete='running')
                        report['copy_partial_continued'] = client.assert_continued_workload(resumed, lifetime)
                        report['copy_partial_same_candidate'] = runtime.confirm_running(
                            client.session, physical_running, deadline)
                        client.operation_evidence_completed()
                        client.assert_original_replay(resumed)
                        report['cycles'][-1]['continued_after_copy_error'] = copy.deepcopy(resumed)
                        break
                client.assert_data_closed()
                saved=client.wait_unloaded()
                physical=runtime.unload_evidence(client.session,deadline,client.unload_acceptance['key'])
                client.assert_unloaded_facts(saved,physical['facts'])
                client.operation_evidence_completed()
                report['cycles'][-1]['unloaded']=copy.deepcopy(saved)
                report['cycles'][-1]['physical_unloaded']=physical
                client.assert_original_replay(saved)
        if not terminal_destroy_done:
            report['phase']='public-destroy-and-physical-gc'
            report['destroy']=client.destroy(lost if lost is not None else resumed)
            client.assert_data_closed()
            client.wait_cleaned()
            report['final_resources']=runtime.destroy_evidence(client.session,deadline)
            client.operation_evidence_completed()
        # Quota settlement and historical refusal-frontier retirement use the
        # original owner-provided persisted accounting oracle; they are never
        # inferred solely from filesystem absence or an HTTP acceptance.
        report['phase']='quota-settlement'
        settlement_started=time.monotonic()
        report['settlement']=runtime.settlement_evidence(deadline)
        settlement_completed=time.monotonic()
        report['settlement_timing']=dict(check_seconds=settlement_completed-settlement_started,
            final_destroy_send_to_settled_seconds=settlement_completed-client.timings[-1]['sent_monotonic'],
            completion_condition='original physical materials/VM absence plus zero resources, material charge and backing pins')
        report['acceptance_cases']={
            'E1':'normal RAM/disk/process continuity and actual slot reuse observed',
            'E2':'same-key barrier duplicate and altered replay observed; expired window not exercised',
            'E12':'two owners, two sessions and two complete restore cycles observed',
            'E14':'old execution tokens rejected and new token accepted',
            'other_cases':'not claimed by this normal scenario',
        }
        if args.scenario=='restore-activation-loss':
            report['acceptance_cases']['E7']='gate-sealed RAM state and same host VMM/netns survived original manager exit86 and replay'
        if args.scenario=='restore-candidate-exit':
            report['acceptance_cases']={
                'E7':'latest RAM gate, exact original pidfd exit, original StateLost replay and manager reopen, no replacement candidate observed, explicit Destroy/GC',
                'E12':'other owner/session remained on the same live candidate across original candidate exit',
                'E14':'prior and gate activation credentials rejected after StateLost; same credential registration observed',
                'other_cases':'one death cycle only; no normal two-cycle or full E1-E16 claim',
            }
        if continued_exit:
            report['acceptance_cases']={
                'E7':'failed Unload continued same restored VM, then exact pidfd exit and StateLost with immutable continued history; replay/reopen and Destroy/GC',
                'E12':'other owner remained on its original current live candidate across continued candidate loss',
                'E14':'old execution credentials rejected after loss; continued execution kept its original credentials and lifetime',
                'qualification':stage,
                'other_cases':'one selected continued death branch; no full fault matrix or formal proof claim',
            }
        if capture_stage == 'copy-partial':
            report['acceptance_cases'] = {
                'E3': 'actual partial Memory copy returned an error; original RAM, disk, authorization and lifetime continued, then explicit Destroy/GC',
                'E12': 'two owners and actual slot reuse observed before the copy fault',
                'E14': 'old tokens rejected after the first Resume; original continued authorization preserved',
                'other_cases': 'one restore cycle and failed second Unload; no second Unloaded or full fault matrix claim',
            }
        elif capture_stage or prepare_fault:
            case = {'freeze-receipt': 'E3', 'seal-receipt': 'E4',
                    'stop-receipt': 'E5', 'restore-prepare-receipt': 'E6'}[
                        capture_stage or 'restore-prepare-receipt']
            report['acceptance_cases'][case] = 'one exact original ' + (capture_stage or 'restore-prepare-receipt') + ' boundary recovered through original product workflow'
        if prepare_destroy:
            report['acceptance_cases'] = {
                'E6': 'original pre-marker LoadPaused stayed unknown until actual public Destroy reached Host and terminal fence; same candidate was observed, stopped and collected without another Prepare or Activate',
                'E12': 'other owner kept its original live candidate across the first owner terminal handoff, then explicit cleanup',
                'E14': 'original data access and prior execution tokens remained closed during unknown Resume and Destroy',
                'other_cases': 'one pre-marker cancellation branch; no resumed Running, second Resume, full matrix or formal proof claim',
            }
        if data_probe is not None:
            report['data_drain_gate_cleanup'] = data_probe.cleanup_after_success(runtime)
        if capture_probe is not None:
            report['capture_handoff_gate_cleanup'] = capture_probe.cleanup()
        report['phase'] = 'accepted-scenario'
        report['complete'] = True
    except BaseException as error:
        failed = True
        report['complete'] = False
        # Never record HTTP bodies, SDK exceptions, API keys or data tokens.
        report['failure_type'] = type(error).__name__
        report.update(failure_details(error, client, report['phase']))
        if prepare_destroy and capture_probe is not None:
            report['prepare_destroy_failure_state'] = capture_probe.terminal_failure_state()
        try:
            report['activation_gate_failure_action']=runtime.fail_resume_gate()
        except BaseException as gate_error:
            report['activation_gate_failure_action']='unknown-preserved-original-private-gate'
            report['activation_gate_failure_type']=type(gate_error).__name__
        raise
    finally:
        if client is not None:
            report['known_session'] = client.session
        # Preserve both exact public identities on failure; an uncertain
        # request is never replaced by a new key or a broad cleanup action.
        report['known_sessions']=[dict(owner=value.owner,session=value.session,
            acceptance=value.current_acceptance) for value in (client,control)
            if value is not None and value.session is not None]
        report['operation_timings']=[sample for value in (client,control) if value is not None
            for sample in value.timing_report()]
        report['operation_timing_summary']={}
        for operation in ('create','unload','resume','destroy'):
            values=sorted(sample['send_to_public_completion_seconds']
                for sample in report['operation_timings']
                if sample['operation']==operation and sample['complete'])
            report['operation_timing_summary'][operation]=dict(samples=len(values),
                p50_seconds=values[math.ceil(.5*len(values))-1] if values else None,
                p95_seconds=values[math.ceil(.95*len(values))-1] if values else None,
                method='nearest rank over completed public operations only; observational small sample')
        report['timing_scope']=dict(clock='host monotonic seconds',
            public_completion='strict original Running/Unloaded/StateLost/Cleaned query; not HTTP 202',
            evidence_completion='existing workload/physical evidence checks, including their observation overhead',
            internal_stages={name:'unknown:not separately instrumented' for name in
                ('freeze','capture','copy','sync','stop','load','activate','ready')},
            cache_condition='unknown:not controlled as cold or warm',
            cpu_rss_disk_peak='unknown:not sampled by this timing instrumentation',
            e16_complete=False)
        try:
            # Runtime finish preserves exact resources/material on uncertainty;
            # it is not permission for broad cleanup or untested GC.
            runtime.finish(report)
        except BaseException as error:
            report['complete'] = False
            report['finish_failure_type'] = type(error).__name__
            if not failed:
                raise
        finally:
            if capture_probe is not None:
                capture_probe.close()
            if data_probe is not None:
                data_probe.close()
            runtime.close_restore_gate()
            runtime.close_capture_cancel_gate()
            report['elapsed_seconds'] = seconds - (deadline - time.monotonic())
            write_once(root / 'lifecycle-unload-report.json', report)
    return report


def main():
    from lifecycle_unload_runtime import add_arguments
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    parser.add_argument('--scenario-timeout', type=float, default=600)
    parser.add_argument('--scenario',choices=('normal','restore-activation-loss','restore-candidate-exit',
        'continued-after-capture-exit','continued-before-admission-exit',
        'data-drain','capture-freeze-receipt','capture-copy-partial','capture-seal-receipt',
        'capture-stop-receipt','restore-prepare-receipt','restore-prepare-destroy'),default='normal')
    parser.add_argument('--activation-gate-timeout-ms',type=int,default=60000)
    args = parser.parse_args()
    try:
        report = exercise(args)
    except Exception as error:
        print('lifecycle_unload_scenario: ' + type(error).__name__, file=sys.stderr)
        return 1
    print(json.dumps(dict(complete=report['complete'], kind=report['kind'],
                          phase=report['phase']), sort_keys=True))
    return 0 if report['complete'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
