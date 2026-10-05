#!/usr/bin/env python3
"""Trusted, bounded replenishment of five isolated ordinary CI runner roles.

The queue is a demand signal, not a job-to-runner binding. GitHub assigns jobs
by labels; only Actions owns check conclusions. This service never writes them.
"""
import argparse
from collections import Counter
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import time
import uuid

if __package__:
    from .ci_github_app import APIError, GitHubApp, REPOSITORY, Transport
    from .ci_vm import MAX_ROLE_WORKERS, ROOT, ROLES, cleanup_work, valid_run_id, valid_slot
else:
    from ci_github_app import APIError, GitHubApp, REPOSITORY, Transport
    from ci_vm import MAX_ROLE_WORKERS, ROOT, ROLES, cleanup_work, valid_run_id, valid_slot

JOBS = {'rust': 'rust', 'sdk-contract': 'sdk', 'harbor-inputs': 'harbor',
        'pr-title': 'title', 'pr-policy': 'policy'}
PREFIX = '/repos/' + REPOSITORY + '/actions/'
STATE_ROOT = ROOT / 'supervisor'
UNIT_PREFIX = 'sandproof-ci-worker-'


def atomic_json(path, value):
    temporary = path.with_suffix('.tmp')
    with open(temporary, 'w', opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as stream:
        json.dump(value, stream)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    fd = os.open(path.parent, os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def role_for_job(job):
    role = JOBS.get(job.get('name'))
    if role is None or job.get('status') != 'queued':
        return None
    expected = {'self-hosted', 'linux', 'x64', 'sandproof-ci-' + role}
    labels = job.get('labels', [])
    if {str(label).lower() for label in labels} != expected or len(labels) != 4:
        return None
    return role


class State:
    def __init__(self, root=STATE_ROOT):
        self.root = root
        self.path = root / 'state.json'
        self.leases = json.loads(self.path.read_text()) if self.path.exists() else {}
        if not isinstance(self.leases, dict):
            raise ValueError('unsafe supervisor state')
        identities = set()
        for slot, lease in self.leases.items():
            role = lease['role']
            if role not in JOBS.values() or valid_slot(role, slot) != lease.get('slot', role):
                raise ValueError('lease slot mismatch')
            valid_run_id(lease['id'])
            if lease['id'] in identities or lease['name'] != runner_name(role, lease['id']):
                raise ValueError('lease identity mismatch')
            identities.add(lease['id'])
            if lease['phase'] not in ('registering', 'ready', 'launching', 'running', 'cleaning'):
                raise ValueError('unknown lease phase')

    def save(self):
        atomic_json(self.path, self.leases)


def runner_name(role, identity):
    return 'sandproof-ci-' + role + '-' + valid_run_id(identity)


class Runtime:
    def unit(self, identity):
        return UNIT_PREFIX + valid_run_id(identity) + '.service'

    def unit_state(self, identity):
        result = subprocess.run(['systemctl', 'show', self.unit(identity),
            '--property=LoadState,ActiveState,MemoryCurrent'], capture_output=True, text=True, timeout=15)
        fields = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
        if fields.get('LoadState') == 'not-found':
            return False, 0
        if result.returncode or fields.get('ActiveState') not in (
                'active', 'activating', 'deactivating', 'inactive', 'failed'):
            raise ValueError('worker state unknown; refusing replacement')
        memory = fields.get('MemoryCurrent', '0')
        return fields['ActiveState'] not in ('inactive', 'failed'), int(memory) if memory.isdigit() else 0

    def admission(self, role, leases):
        # Pausing admissions does not terminate an already accepted CI task.
        if (STATE_ROOT / 'paused').exists() or (STATE_ROOT / 'exclusive-host').exists():
            return False
        fields = dict(line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
        available = int(fields['MemAvailable'].split()[0]) // 1024
        reserved_not_resident = 0
        for lease in leases.values():
            active, used = self.unit_state(lease['id'])
            if active:
                reserved_not_resident += max(0, ROLES[lease['role']][0] + 256 - used // 1024 ** 2)
        required = 2048 + reserved_not_resident + ROLES[role][0] + 256
        # Sparse disks are charged their full logical budget at admission. This
        # protects free space without promising a disk quota for another service.
        disk = shutil.disk_usage(ROOT).free
        disk_required = (8 + 32 * (len(leases) + 1)) * 1024 ** 3
        return available >= required and disk >= disk_required

    def start(self, lease, jit_path):
        identity = valid_run_id(lease['id'])
        unit = self.unit(identity)
        directory = ROOT / 'runs' / identity
        directory.mkdir(mode=0o711)
        directory.chmod(0o711)
        (directory / 'host.log').touch(mode=0o600)
        # LoadCredential copies ONLY this job's JIT config, not the App key.
        command = ['systemd-run', '--quiet', '--collect', '--service-type=exec', '--unit=' + unit,
            '--property=LoadCredential=jit:' + str(jit_path),
            '--property=KillMode=control-group', '--property=TimeoutStopSec=25',
            '--property=RuntimeMaxSec=1900', '--property=MemoryMax=' + str(ROLES[lease['role']][0] + 512) + 'M',
            '--property=MemorySwapMax=0', '--property=TasksMax=128',
            '--property=NoNewPrivileges=yes', '--property=ProtectSystem=strict',
            '--property=ProtectHome=yes', '--property=PrivateTmp=yes',
            '--property=ReadWritePaths=' + str(ROOT), '--property=UMask=0077',
            '--property=StandardOutput=append:' + str(directory / 'host.log'),
            '--property=StandardError=append:' + str(directory / 'host.log'),
            '/usr/bin/python3', '/opt/sandproof-ci/ci_vm.py', '--role', lease['role'],
            '--slot', valid_slot(lease['role'], lease.get('slot', lease['role'])),
            '--run-id', identity, '--timeout', '1800',
            '--jit-file', '/run/credentials/' + unit + '/jit']
        subprocess.run(command, check=True, capture_output=True, timeout=30)

    def cleanup(self, identity):
        if self.unit_state(identity)[0]:
            raise ValueError('cannot clean an active worker')
        directory = ROOT / 'runs' / valid_run_id(identity)
        if directory.exists():
            cleanup_work(directory)


def prune_diagnostics(root, active_ids, limit_count=40, limit_bytes=128 * 1024 ** 2):
    """Bound completed root-owned diagnostics; never recurse or touch old VM-owned runs."""
    complete = []
    for directory in root.iterdir():
        if not re.fullmatch(r'[0-9a-f]{32}', directory.name) or directory.name in active_ids:
            continue
        details = directory.lstat()
        if not stat.S_ISDIR(details.st_mode) or details.st_uid != 0:
            continue
        entries = list(directory.iterdir())
        if not entries or any(p.name not in {'console.log', 'qemu.log', 'report.json', 'host.log'}
                              or not stat.S_ISREG(p.lstat().st_mode) for p in entries):
            continue
        complete.append((details.st_mtime, directory, entries, sum(p.stat().st_size for p in entries)))
    complete.sort(key=lambda item: (item[0], item[1].name), reverse=True)
    retained = 0
    for index, (_modified, directory, entries, size) in enumerate(complete):
        if index < limit_count and retained + size <= limit_bytes:
            retained += size
            continue
        for entry in entries:
            entry.unlink()
        directory.rmdir()


class Supervisor:
    def __init__(self, api, state, runtime, workflow_id, clock=time.time):
        self.api = api
        self.state = state
        self.runtime = runtime
        self.workflow_id = workflow_id
        self.clock = clock

    def inventory(self):
        runners = []
        for page in range(1, 5):
            result = self.api.request('GET', PREFIX + 'runners?per_page=100&page=' + str(page))
            runners.extend(result['runners'])
            if len(result['runners']) < 100:
                return runners
        raise ValueError('runner inventory exceeds fixed limit')

    def reconcile(self):
        runners = self.inventory()  # A failed read forbids new registrations.
        owned_names = {lease['name'] for lease in self.state.leases.values()}
        # No blind deletion of an unfamiliar runner, even if its name resembles
        # ours. Missing durable ownership is an operator-visible safety stop.
        pattern = r'sandproof-ci-(?:rust|sdk|harbor|title|policy)-[0-9a-f]{32}'
        if any(re.fullmatch(pattern, runner['name']) and runner['name'] not in owned_names for runner in runners):
            raise ValueError('untracked CI runner; operator reconciliation required')
        for slot, lease in list(self.state.leases.items()):
            active, _memory = self.runtime.unit_state(lease['id'])
            if active:
                lease['phase'] = 'running'
                self.state.save()
                continue
            # Intent was durable BEFORE registration. If POST completed but its
            # response was lost, exact generated name recovers ownership.
            matches = [runner for runner in runners if runner['name'] == lease['name']]
            if len(matches) > 1:
                raise ValueError('ambiguous owned runner identity')
            lease['phase'] = 'cleaning'
            self.state.save()
            self.runtime.cleanup(lease['id'])
            if matches:
                try:
                    self.api.request('DELETE', PREFIX + 'runners/' + str(matches[0]['id']))
                except APIError as error:
                    if error.status != 404:
                        raise
            jit_path = self.state.root / (lease['id'] + '.jit')
            if jit_path.exists():
                if jit_path.is_symlink():
                    raise ValueError('unsafe JIT credential entry')
                jit_path.unlink()
            del self.state.leases[slot]
            self.state.save()
        return runners

    def queued_counts(self):
        # The owner moved full Rust checks off PRs into a separate push/manual
        # workflow. Discover only that exact trusted filename; absent until its
        # first deployment is normal, not a reason to block existing PR checks.
        workflow_ids = [self.workflow_id]
        try:
            rust = self.api.request('GET', PREFIX + 'workflows/rust-ci.yml')
        except APIError as error:
            if error.status != 404:
                raise
        else:
            if (type(rust.get('id')) is not int or rust['id'] <= 0
                    or rust.get('path') != '.github/workflows/rust-ci.yml'
                    or rust['id'] == self.workflow_id):
                raise ValueError('Rust workflow identity mismatch')
            workflow_ids.append(rust['id'])
        counts = Counter()
        seen = set()
        for workflow_id in workflow_ids:
            self.count_workflow(workflow_id, counts, seen)
        return counts

    def count_workflow(self, workflow_id, counts, seen):
        for status in ('queued', 'in_progress'):
            for page in range(1, 5):
                result = self.api.request('GET', PREFIX + 'workflows/' + str(workflow_id)
                    + '/runs?status=' + status + '&per_page=100&page=' + str(page))
                runs = result['workflow_runs']
                for run in runs:
                    if run['workflow_id'] != workflow_id:
                        raise ValueError('workflow identity changed')
                    if workflow_id != self.workflow_id and run['event'] not in ('push', 'workflow_dispatch'):
                        continue
                    if run['event'] not in ('pull_request', 'push', 'merge_group', 'workflow_dispatch'):
                        continue
                    for job_page in range(1, 5):
                        jobs = self.api.request('GET', PREFIX + 'runs/' + str(run['id'])
                            + '/attempts/' + str(run['run_attempt']) + '/jobs?per_page=100&page=' + str(job_page))['jobs']
                        for job in jobs:
                            role = role_for_job(job)
                            if role is None:
                                continue
                            if type(job.get('id')) is not int or job['id'] <= 0:
                                raise ValueError('queued job identity missing')
                            identity = (run['id'], run['run_attempt'], job['id'])
                            if identity not in seen:
                                counts[role] += 1
                                seen.add(identity)
                        if len(jobs) < 100:
                            break
                    else:
                        raise ValueError('job inventory exceeds fixed limit')
                if len(runs) < 100:
                    break
            else:
                raise ValueError('workflow inventory exceeds fixed limit')

    def tick(self):
        runners = {runner['name']: runner for runner in self.reconcile()}
        counts = self.queued_counts()
        # Booting/offline/idle owned workers already cover queued demand. Only a
        # positively busy runner frees demand for another worker of that role.
        unassigned = Counter(lease['role'] for lease in self.state.leases.values()
            if runners.get(lease['name'], {}).get('busy') is not True)
        # Offer every role its first slot before filling second slots. Count all
        # durable leases in resource admission, including cleanup-unknown ones.
        for index in range(1, MAX_ROLE_WORKERS + 1):
            for role in JOBS.values():
                slot = role if index == 1 else role + '-' + str(index)
                if (slot in self.state.leases or counts[role] <= unassigned[role]
                        or not self.runtime.admission(role, self.state.leases)):
                    continue
                self.start_worker(role, slot)
                unassigned[role] += 1

    def start_worker(self, role, slot):
        identity = uuid.uuid4().hex
        lease = {'id': identity, 'role': role, 'slot': valid_slot(role, slot),
                 'name': runner_name(role, identity), 'created': self.clock(), 'phase': 'registering'}
        self.state.leases[slot] = lease
        self.state.save()
        result = self.api.request('POST', PREFIX + 'runners/generate-jitconfig',
            {'name': lease['name'], 'runner_group_id': 1,
             'labels': ['self-hosted', 'Linux', 'X64', 'sandproof-ci-' + role], 'work_folder': '_work'})
        if result['runner']['name'] != lease['name']:
            raise ValueError('registered runner identity mismatch')
        lease['runner_id'] = result['runner']['id']
        jit_path = self.state.root / (identity + '.jit')
        with open(jit_path, 'x', opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as stream:
            stream.write(result['encoded_jit_config'])
            stream.flush()
            os.fsync(stream.fileno())
        lease['phase'] = 'ready'
        self.state.save()
        # Persist the identity BEFORE invoking systemd; if acknowledgement
        # is lost, unit_state resolves it without launching a second VM.
        lease['phase'] = 'launching'
        self.state.save()
        self.runtime.start(lease, jit_path)
        lease['phase'] = 'running'
        self.state.save()
        print('CI worker started: ' + slot + ' ' + identity, flush=True)


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, default=Path('/etc/sandproof-ci/supervisor.json'))
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('trusted host supervisor requires root; guest CI never does')
    config = json.loads(args.config.read_text())
    STATE_ROOT.mkdir(mode=0o700, exist_ok=True)
    if STATE_ROOT.is_symlink() or STATE_ROOT.stat().st_uid != 0 or STATE_ROOT.stat().st_mode & 0o077:
        raise ValueError('supervisor state must be root-private')
    lock = (STATE_ROOT / 'supervisor.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    key = Path(os.environ['CREDENTIALS_DIRECTORY']) / 'app-key'
    api = GitHubApp(config['client_id'], key, Transport(config['proxy']), config['installation_id'])
    api.verify_installation()
    workflow = api.request('GET', PREFIX + 'workflows/function-ci.yml')
    if workflow['id'] != config['workflow_id']:
        raise ValueError('fixed workflow identity mismatch')
    supervisor = Supervisor(api, State(), Runtime(), config['workflow_id'])
    stopping = False

    def stop(_signum, _frame):
        nonlocal stopping
        stopping = True  # Dedicated worker units survive manager restart.
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    delay = 15
    while not stopping:
        try:
            supervisor.tick()
            prune_diagnostics(ROOT / 'runs', {v['id'] for v in supervisor.state.leases.values()})
            delay = 15
        except (APIError, ValueError, OSError, subprocess.SubprocessError) as error:
            # No exception detail/response body: transport errors can contain
            # credentials. Unknown cleanup keeps its durable lease occupied.
            status = error.status if isinstance(error, APIError) else type(error).__name__
            print('CI replenishment paused for retry: ' + str(status), flush=True)
            delay = min(300, max(30, delay * 2, getattr(error, 'retry_after', 0)))
        deadline = time.monotonic() + delay
        while not stopping and time.monotonic() < deadline:
            time.sleep(min(1, deadline - time.monotonic()))
    print('CI supervisor stopped; owned workers remain supervised by systemd.', flush=True)


if __name__ == '__main__':
    main()
