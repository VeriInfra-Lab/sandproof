"""Public create/unload/reopen assertions over the original RuntimeServices.

The sibling runtime helper owns deployment, processes, snapshots and physical
observations. HTTP acceptance is never used as proof of physical completion.
This entry does not test Resume/GC and never retries a write with a fresh key.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import sys
import time
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


def uint(value, *, positive=False):
    return type(value) is int and (1 if positive else 0) <= value < 2**64


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


class PublicUnloadScenario:
    """One known owner/window and two saved complete requests, not a client SDK."""
    def __init__(self, base_url, owner_key, foreign_key, deployment, deadline):
        assert type(deadline) in (int, float) and math.isfinite(deadline)
        address = urlsplit(base_url)
        assert address.scheme == 'http' and address.hostname == '127.0.0.1'
        assert address.port and not address.username and not address.password
        assert not address.path and not address.query and not address.fragment
        self.base_url = base_url
        self.owner_key, self.foreign_key = owner_key, foreign_key
        self.deployment, self.deadline = deployment, deadline
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.window = None
        self.session = None
        self.create_request = self.create_acceptance = None
        self.unload_request = self.unload_acceptance = None

    def remaining(self):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('original lifecycle scenario deadline; no completion inferred')
        return remaining

    def call(self, method, path, *, key, body=None):
        # Existing HTTP helper supplies the original X-API-Key header. Failure
        # of a write propagates: a new window/sequence is never allocated here.
        return fetch(self.opener, self.base_url, method, path, key, body,
                     timeout=min(40, self.remaining()))

    def read_json(self, method, path, *, key, body=None, status=200):
        actual, encoded = self.call(method, path, key=key, body=body)
        assert actual == status, f'public {method} returned {actual}, expected {status}'
        value = json.loads(encoded, object_pairs_hook=unique_keys)
        assert type(value) is dict
        return value

    def key(self, sequence):
        return dict(deployment=self.deployment, owner='1', window=self.window, sequence=sequence)

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
        window = self.read_json('GET', PREFIX + '/request-window', key=self.owner_key)
        assert set(window) == {'window'} and uint(window['window'], positive=True)
        self.window = window['window']
        self.create_request = dict(window=self.window, sequence=1, parameters=copy.deepcopy(parameters))
        result = self.read_json('POST', PREFIX + '/create', key=self.owner_key,
                                body=self.create_request, status=202)
        self.create_acceptance = self.acceptance(result, sequence=1, version=1)
        self.session = result['session']
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
        assert set(view) == {'session', 'operation', 'current_version', 'phase', 'unloaded', 'create_refusal'}
        assert view['session'] == self.session and view['operation'] == acceptance['key']
        assert uint(view['current_version'], positive=True)
        assert view['current_version'] == acceptance['accepted_version']
        assert view['create_refusal'] is None
        if view['phase'] != 'unloaded':
            assert view['unloaded'] is None, 'pending phase exposed a completed material result'
        return view

    def wait_phase(self, acceptance, *, waiting, complete):
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
                return view
            assert view['phase'] in waiting, 'unexpected lifecycle phase; no completion inferred'
            time.sleep(min(.1, self.remaining()))

    def wait_running(self):
        return self.wait_phase(self.create_acceptance, waiting={'creating'}, complete='running')

    def assert_owner_isolation(self):
        path = self.path()
        assert self.call('GET', path, key=self.foreign_key)[0] == 404
        assert self.call('GET', path, key=None)[0] == 401
        if self.unload_request is not None:
            assert self.call('POST', path + '/unload', key=self.foreign_key,
                             body=self.unload_request)[0] == 404

    def unload(self, running):
        self.check_view(running, self.create_acceptance)
        assert running['phase'] == 'running' and self.unload_request is None
        self.unload_request = dict(window=self.window, sequence=2,
                                  expected_version=running['current_version'])
        result = self.read_json('POST', self.path() + '/unload', key=self.owner_key,
                                body=self.unload_request, status=202)
        self.unload_acceptance = self.acceptance(result, sequence=2,
            version=running['current_version'] + 1)
        return copy.deepcopy(result)

    def wait_unloaded(self):
        return self.wait_phase(self.unload_acceptance, waiting={'unloading'}, complete='unloaded')

    def assert_unloaded_facts(self, view, independent_facts):
        self.check_view(view, self.unload_acceptance)
        assert view['phase'] == 'unloaded'
        facts = view['unloaded']
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
        create = self.read_json('POST', PREFIX + '/create', key=self.owner_key,
                               body=self.create_request, status=202)
        unload = self.read_json('POST', self.path() + '/unload', key=self.owner_key,
                               body=self.unload_request, status=202)
        assert create == self.create_acceptance and unload == self.unload_acceptance
        assert self.query() == saved
        changed = dict(self.unload_request, expected_version=self.unload_acceptance['accepted_version'])
        assert self.call('POST', self.path() + '/unload', key=self.owner_key, body=changed)[0] == 409
        assert self.query() == saved, 'rejected altered replay changed the saved result'

    def assert_reopened(self, saved):
        assert self.query() == saved, 'manager reopen changed the original complete public result'
        window = self.read_json('GET', PREFIX + '/request-window', key=self.owner_key)
        assert window == {'window': self.window}
        self.assert_owner_isolation()
        self.assert_original_replay(saved)
        self.remaining()

    def run_public_workload(self, running):
        """Use the pinned SDK constructor after an owner-authenticated GET.

        SDK connect renews the session. This original no-create constructor
        follows the existing DK fixture and leaves lifecycle version/time alone.
        """
        import importlib.metadata
        from e2b import Sandbox
        from e2b.connection_config import ConnectionConfig
        from packaging.version import Version
        assert importlib.metadata.version('e2b') == '2.51.0'
        detail = self.read_json('GET', '/sandboxes/' + self.session, key=self.owner_key)
        assert detail['sandboxID'] == self.session and detail['clientID'] == 'tenant-1'
        assert detail['state'] == 'running'
        assert self.call('GET', '/sandboxes/' + self.session, key=self.foreign_key)[0] == 404
        self.data_token = detail['envdAccessToken']
        assert type(self.data_token) is str and self.data_token
        connection = ConnectionConfig(api_url=self.base_url, sandbox_url=self.base_url,
            domain=detail['domain'], api_key=self.owner_key, retries=0,
            request_timeout=min(30, self.remaining()),
            extra_sandbox_headers={'X-Access-Token': self.data_token})
        sandbox = Sandbox(sandbox_id=self.session, sandbox_domain=detail['domain'],
            envd_version=Version(detail['envdVersion']), envd_access_token=self.data_token,
            connection_config=connection)
        marker = 'sandproof-live-unload-v1'
        path = '/tmp/sandproof-unload-marker'
        command = "printf '%s' 'sandproof-live-unload-v1' > /tmp/sandproof-unload-marker && sync && cat /tmp/sandproof-unload-marker"
        result = sandbox.commands.run(command, timeout=min(30, self.remaining()))
        assert result.exit_code == 0 and result.stdout == marker
        assert self.query() == running, 'workload or SDK setup changed the lifecycle version'
        self.remaining()
        return dict(sdk='2.51.0', entry='owner-detail/original-sdk-constructor/public-gateway',
                    path=path, marker_sha256=hashlib.sha256(marker.encode()).hexdigest())

    def assert_data_closed(self):
        import httpx
        assert self.data_token
        with httpx.Client(base_url=self.base_url, trust_env=False,
                          timeout=min(10, self.remaining())) as client:
            response = client.get('/files', params={'path': '/tmp/sandproof-unload-marker'},
                                  headers={'X-Access-Token': self.data_token})
            assert response.status_code == 401, 'original token still authorizes data after Unload'
        assert self.call('GET', '/sandboxes/' + self.session, key=self.owner_key)[0] == 404


def exercise(args):
    """Only the sibling helper can start/restart or inspect original resources."""
    from lifecycle_unload_runtime import prepare
    seconds = args.scenario_timeout
    if type(seconds) not in (int, float) or not math.isfinite(seconds) or not 0 < seconds <= 3600:
        raise ValueError('explicit finite scenario deadline at most one hour required')
    deadline = time.monotonic() + seconds
    runtime = prepare(args)
    root = private_path(runtime.root, directory=True)
    report = dict(schema=1, kind='create-unload-manager-reopen-system-scenario',
                  complete=False, phase='configuration', scope='no Resume/GC or formal proof')
    client = None
    failed = False
    try:
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
        report['running'] = runtime.record_running(client.session, deadline)
        lifetime = report['running']['lifetime']
        assert set(lifetime) == {'started_at_ms', 'end_at_ms', 'max_end_at_ms', 'startup_deadline_ms'}
        report['phase'] = 'public-unload'
        report['unload'] = client.unload(running)
        # This checks new authorization closes at durable Unload acceptance;
        # it does not claim existing holders or physical workers have exited.
        client.assert_data_closed()
        saved = client.wait_unloaded()
        client.assert_owner_isolation()
        report['phase'] = 'original-physical-evidence'
        physical = runtime.unload_evidence(client.session, deadline)
        client.assert_unloaded_facts(saved, physical['facts'])
        assert {key: physical['facts'][key] for key in lifetime} == lifetime
        report['before_restart'] = saved
        report['physical_before_restart'] = physical
        client.assert_original_replay(saved)
        report['phase'] = 'same-manager-config-reopen'
        report['manager_restart'] = runtime.restart_managers(deadline)
        client.assert_reopened(saved)
        client.assert_data_closed()
        physical = runtime.unload_evidence(client.session, deadline)
        client.assert_unloaded_facts(saved, physical['facts'])
        report['physical_after_restart'] = physical
        report['phase'] = 'accepted-scenario'
        report['complete'] = True
    except BaseException as error:
        failed = True
        report['complete'] = False
        # Never record HTTP bodies, SDK exceptions, API keys or data tokens.
        report['failure_type'] = type(error).__name__
        raise
    finally:
        if client is not None:
            report['known_session'] = client.session
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
            report['elapsed_seconds'] = seconds - (deadline - time.monotonic())
            write_once(root / 'lifecycle-unload-report.json', report)
    return report


def main():
    from lifecycle_unload_runtime import add_arguments
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    parser.add_argument('--scenario-timeout', type=float, default=600)
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
