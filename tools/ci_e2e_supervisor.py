#!/usr/bin/env python3
"""Trusted host E2E queue. No privileged GitHub runner; no external PR trigger.

Install this reviewed source on the host. Automatically observes the trusted
test ref; other candidates require a root-owned exact-SHA allowlist. Dedicated
systemd batch units survive manager restarts. Unknown state is not a retry.
"""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import time
import uuid

from tools.benchmark_ci import (CHECK_NAME, aggregate, authorize, batch_result,
    instantiate_policy, private_path, remove_private_tree, run_identity, subset)
from tools.benchmark_tasks import canonical, digest, verify
from tools.ci_github_app import APIError, CHECK_PERMISSIONS, GitHubApp, REPOSITORY, Transport
from tools.ci_runner_supervisor import atomic_json
from tools.system_acceptance import json_file, write_once
from tools.check_kvm_host import trusted_path

PREFIX = '/repos/' + REPOSITORY + '/'
REPO = 'https://github.com/' + REPOSITORY + '.git'
UNIT_PREFIX = 'sandproof-e2e-batch-'


def validate_host_inputs(config):
    for flag in ('policy_template', 'budget_profile'):
        private_path(config[flag])
    for path, expected in config['host_assets'].items():
        artifact = trusted_path(Path(path))
        from tools.system_acceptance import digest as file_digest
        if file_digest(artifact) != expected:
            raise ValueError('approved host artifact changed')
    required = {config[k] for k in ('kernel', 'rootfs', 'firecracker', 'jailer', 'proxy_binary',
                                    'policy_template', 'budget_profile')}
    if not required <= set(config['host_assets']):
        raise ValueError('host artifact fingerprints incomplete')
    if config.get('source_kind', 'oci') not in ('oci', 'dockerfile'):
        raise ValueError('original environment source required')
    if config.get('prepare_candidate'):
        for name in ('uv', 'cargo'):
            if config[name] not in config['host_assets']:
                raise ValueError('prepared tool fingerprint missing')
        trusted_path(Path(config['toolchain_bin']))
    for name in ('scratch_parent', 'state_root', 'ci_state_root'):
        private_path(config[name], directory=True)


def git(*args, cwd=None):
    return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
        '-c', 'core.fsmonitor=false', *args], cwd=cwd, text=True, stderr=subprocess.PIPE, timeout=300).strip()


class Checks:
    def __init__(self, api):
        self.api = api

    def ensure(self, state, save):
        if state.get('check_id') is not None:
            return state['check_id']
        found = self.api.request('GET', PREFIX + 'commits/' + state['candidate'] + '/check-runs?per_page=100&filter=all')
        if found['total_count'] > 100:
            raise ValueError('check inventory exceeds fixed limit')
        matches = [r for r in found['check_runs'] if r.get('external_id') == state['id']
                   and r.get('name') == CHECK_NAME and r.get('head_sha') == state['candidate']
                   and r.get('app', {}).get('id') == self.api.app_id]
        if len(matches) > 1:
            raise ValueError('duplicate check identity')
        if matches:
            state['check_id'] = matches[0]['id']
            save()
            return state['check_id']
        if state.get('check_post_pending'):
            raise ValueError('check creation reply unknown; no blind repeat')
        state['check_post_pending'] = True
        save()  # Intent BEFORE the API mutation.
        created = self.api.request('POST', PREFIX + 'check-runs', dict(name=CHECK_NAME,
            head_sha=state['candidate'], external_id=state['id'], status='in_progress',
            output=dict(title='固定 2.1 八题验收进行中',
                summary='只验收预选八题；不代表全部 89 题通过。失败和清理未知均不会绿灯。')))
        if created.get('head_sha') != state['candidate'] or created.get('external_id') != state['id']:
            raise ValueError('created check identity mismatch')
        state['check_id'] = created['id']
        save()
        return state['check_id']

    def finish(self, state, result):
        conclusion = 'success' if result['passed'] else 'failure'
        rows = ['固定版本：Terminal-Bench 2.1；范围：8 道回归题，不是完整 89 题。',
                '通过数量：' + str(sum(r['status'] == 'passed' for r in result['tasks'])) + '/8。',
                '未运行、评分失败、超时及清理未知均按失败处理；没有使用付费模型。']
        rows.extend(r['name'] + ': ' + str(r['status']) for r in result['tasks'])
        self.api.request('PATCH', PREFIX + 'check-runs/' + str(state['check_id']), dict(
            status='completed', conclusion=conclusion, output=dict(
                title='八题验收通过' if result['passed'] else '八题验收未通过', summary='\n\n'.join(rows))))


@contextmanager
def admission_guard(root):
    with open(root / 'admission.lock', 'a',
              opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        yield


class HostLease:
    def __init__(self, root, identity):
        self.root, self.identity = private_path(root, directory=True), identity

    def pause(self):
        with admission_guard(self.root):
            marker = self.root / 'paused'
            if marker.exists():
                if json_file(marker) != {'e2e': self.identity}:
                    raise ValueError('host paused by another operator')
            else:
                write_once(marker, {'e2e': self.identity})

    def exclusive(self):
        with admission_guard(self.root):
            if json_file(self.root / 'paused') != {'e2e': self.identity}:
                raise ValueError('pause identity changed')
            if json_file(self.root / 'state.json'):
                return False  # Includes cleanup-unknown leases; wait, never cancel.
            units = subprocess.check_output(['systemctl', 'list-units', '--all', '--plain', '--no-legend',
                '--state=active,activating,deactivating', 'sandproof-ci-worker-*.service'], text=True, timeout=20)
            if units.strip():
                return False
            marker = self.root / 'exclusive-host'
            if marker.exists():
                if json_file(marker) != {'e2e': self.identity}:
                    raise ValueError('another exclusive host owner')
            else:
                write_once(marker, {'e2e': self.identity})
            return True

    def release(self):
        with admission_guard(self.root):
            for name in ('exclusive-host', 'paused'):
                marker = self.root / name
                if marker.exists():
                    if json_file(marker) != {'e2e': self.identity}:
                        raise ValueError('host marker belongs to another owner')
                    marker.unlink()


def unit_state(name):
    if re.fullmatch(r'sandproof-e2e-(?:batch|prepare)-[0-9a-f]{32}\.service', name) is None:
        raise ValueError('unit identity invalid')
    response = subprocess.run(['systemctl', 'show', name,
        '--property=LoadState,ActiveState,ExecMainCode,ExecMainStatus'], capture_output=True, text=True, timeout=20)
    fields = dict(line.split('=', 1) for line in response.stdout.splitlines() if '=' in line)
    if response.returncode or fields.get('LoadState') not in ('loaded', 'not-found'):
        raise ValueError('batch unit state unknown')
    return fields


def launch(batch):
    # No management credential, caller environment or arbitrary command string
    # is passed. A systemd-owned batch continues if this manager is restarted.
    subprocess.run(['systemd-run', '--unit=' + batch['unit'], '--property=Type=exec',
        '--property=RemainAfterExit=yes', '--property=UMask=0077',
        '--property=WorkingDirectory=' + batch['checkout'],
        '--property=StandardOutput=journal', '--property=StandardError=journal',
        '--property=LogRateLimitIntervalSec=60', '--property=LogRateLimitBurst=100',
        '--setenv=PYTHONPATH=' + batch['checkout'], *batch['command']], check=True, capture_output=True, timeout=30)


class Supervisor:
    def __init__(self, config, checks=None):
        self.config = config
        self.root = private_path(config['state_root'], directory=True)
        self.path = self.root / 'state.json'
        self.state = json_file(self.path, limit=4 * 1024 * 1024) if self.path.exists() else None
        if self.state and self.state['phase'] not in ('done', 'blocked'):
            # Never use a new configuration or checkout for an in-flight run.
            self.config = self.state['configuration']
        self.checks = checks

    def save(self):
        atomic_json(self.path, self.state)

    def block(self, reason):
        self.state.update(phase='blocked', reason=reason)
        self.save()
        if self.checks:
            result = aggregate(self.state['candidate'],
                [b['result'] for b in self.state['batches'] if 'result' in b], self.state['expected'])
            result['passed'] = False
            self.checks.finish(self.state, result)

    def begin(self, candidate, base, observed_test=None):
        authorize(candidate, approved_commits=self.config['approved_commits'], observed_test=observed_test)
        validate_host_inputs(self.config)
        checkout = private_path(self.config['checkout'], directory=True)
        if git('rev-parse', 'HEAD', cwd=checkout) != candidate or git('status', '--porcelain', cwd=checkout):
            raise ValueError('clean operator-prepared exact candidate checkout required')
        lock = verify(json_file(checkout / 'benchmarks/terminal-bench-2.1.lock.json', limit=8*1024*1024),
            checkout / 'benchmarks/terminal-bench-2.1.registry.json', Path(self.config['task_source']))
        manifest = json_file(checkout / 'benchmarks/terminal-bench-2.1-subset.json')
        if digest(canonical(manifest)) != self.config['subset_sha256']:
            raise ValueError('selection changed; operator must review the new subset')
        batches = subset(lock, manifest)
        identity = run_identity(candidate, base, self.config, manifest, lock)
        if self.state:
            if self.state['id'] != identity and self.state['phase'] != 'done':
                raise ValueError('unresolved original run; cannot admit a replacement')
            if self.state['id'] == identity:
                return  # Never repeat the same candidate/base/config identity.
            archive = self.root / (self.state['id'] + '.json')
            if not archive.exists():
                write_once(archive, self.state)
        self.state = dict(id=identity, candidate=candidate, base=base, phase='waiting', configuration=self.config,
            batches=[dict(names=names, phase='new') for names in batches],
            expected=[name for names in batches for name in names], check_id=None)
        self.save()

    def prepare(self, batch, index):
        config = self.config
        validate_host_inputs(config)
        checkout = Path(config['checkout'])
        root = private_path(config['scratch_parent'], directory=True) / uuid.uuid4().hex[:8]
        namespaces = [uuid.uuid4().hex[:16], uuid.uuid4().hex[:16]]
        policy = instantiate_policy(json_file(Path(config['policy_template'])), root, namespaces)
        if self.state.get('candidate_artifacts'):
            from tools.system_acceptance import digest as file_digest
            for path, expected in self.state['candidate_artifacts'].items():
                if file_digest(trusted_path(Path(path))) != expected:
                    raise ValueError('prepared candidate artifact changed')
            helper = policy['public']['bounded_builds']['helper']
            helper.update(path=config['window_helper'], sha256=self.state['candidate_artifacts'][config['window_helper']])
        policy_file = self.root / (self.state['id'] + '-' + str(index) + '-policy.json')
        write_once(policy_file, policy)
        args = [str(checkout / 'benchmarks/.venv/bin/python'), str(checkout / 'tests/system/harbor_generic_chain.py')]
        for flag in ('server', 'firecracker', 'jailer', 'kernel', 'rootfs', 'tools',
                     'scratch_parent', 'cgroup_parent', 'build_cgroup_parent', 'service_cgroup_parent',
                     'uid_base', 'budget_profile', 'proxy_binary', 'proxy_sha256'):
            args.extend(['--' + flag.replace('_', '-'), str(config[flag])])
        args.extend(['--registry', str(checkout / 'benchmarks/terminal-bench-2.1.registry.json'),
            '--source', config['task_source'], '--lock', str(checkout / 'benchmarks/terminal-bench-2.1.lock.json'),
            '--bounded-policy', str(policy_file), '--run-root', str(root),
            '--namespace', namespaces[0], '--build-namespace', namespaces[1], '--temporarily-forward',
            '--oracle-mode', 'original', '--source-kind', config.get('source_kind', 'oci')])
        if config.get('docker_hub_mirror'):
            args.extend(['--docker-hub-mirror', config['docker_hub_mirror']])
        for name in batch['names']:
            args.extend(['--task', name])
        batch.update(root=str(root), command=args, checkout=str(checkout),
            unit=UNIT_PREFIX + uuid.uuid4().hex + '.service', phase='launching')
        self.save()  # Intent before launching; original identity survives crash.

    def tick(self):
        state = self.state
        if not state or state['phase'] in ('done', 'blocked'):
            return
        if self.checks:
            self.checks.ensure(state, self.save)
        lease = HostLease(Path(self.config['ci_state_root']), state['id'])
        if state['phase'] == 'waiting':
            lease.pause()
            if not lease.exclusive():
                return
            state['phase'] = 'preparing' if self.config.get('prepare_candidate') else 'running'
            self.save()
        if state['phase'] == 'preparing':
            if 'prepare_unit' not in state:
                state['prepare_unit'] = 'sandproof-e2e-prepare-' + uuid.uuid4().hex + '.service'
                preparation = dict(self.config, candidate=state['candidate'])
                prepare_path = self.root / (state['id'] + '-prepare.json')
                write_once(prepare_path, preparation)
                self.save()
                subprocess.run(['systemd-run', '--unit=' + state['prepare_unit'],
                    '--property=Type=exec', '--property=RemainAfterExit=yes', '--property=UMask=0077',
                    '--property=MemoryMax=3G', '--property=CPUQuota=200%',
                    '--property=WorkingDirectory=' + self.config['control_checkout'],
                    '--setenv=PYTHONPATH=' + self.config['control_checkout'],
                    '/usr/bin/python3', '-m', 'tools.ci_e2e_prepare', '--config', str(prepare_path)],
                    check=True, capture_output=True, timeout=30)
                return
            fields = unit_state(state['prepare_unit'])
            if fields.get('ActiveState') in ('activating', 'deactivating') or (
                    fields.get('ActiveState') == 'active' and fields.get('ExecMainCode') == '0'):
                return
            if fields.get('ExecMainCode') != '1':
                self.block('candidate_preparation_identity_unknown')
                return
            if fields.get('ExecMainStatus') != '0':
                lease.release()  # Preparation never dispatched a sandbox.
                result = aggregate(state['candidate'], [], state['expected'])
                state.update(result=result, phase='done', reason='candidate_preparation_failed')
                if self.checks:
                    self.checks.finish(state, result)
                self.save()
                return
            subprocess.run(['systemctl', 'stop', state['prepare_unit']], check=True, capture_output=True, timeout=20)
            from tools.system_acceptance import digest as file_digest
            state['candidate_artifacts'] = {self.config[key]: file_digest(trusted_path(Path(self.config[key])))
                                           for key in ('server', 'window_helper')}
            state['phase'] = 'running'
            self.save()
        for index, batch in enumerate(state['batches']):
            if batch['phase'] == 'done':
                continue
            if batch['phase'] == 'new':
                self.prepare(batch, index)
                launch(batch)
                batch['phase'] = 'running'
                self.save()
                return
            fields = unit_state(batch['unit'])
            if fields.get('ActiveState') in ('activating', 'deactivating') or (
                    fields.get('ActiveState') == 'active' and fields.get('ExecMainCode') == '0'):
                return
            # A launching unit absent after lost reply is UNKNOWN, not a repeat.
            if fields.get('ExecMainCode') != '1':
                self.block('batch_identity_or_termination_unknown')
                return
            root = Path(batch['root'])
            try:
                if batch['phase'] != 'reclaiming':
                    result = batch_result(root, candidate=state['candidate'], names=batch['names'],
                                          exit_code=int(fields['ExecMainStatus']))
                    batch['result'] = result
                    batch['root_identity'] = [root.stat().st_dev, root.stat().st_ino]
                    batch['phase'] = 'reclaiming'
                    self.save()  # Evidence BEFORE reclamation; restart resumes original cleanup.
                if root.exists():
                    remove_private_tree(root, batch['root_identity'])
                subprocess.run(['systemctl', 'stop', batch['unit']], check=True, capture_output=True, timeout=20)
                batch['phase'] = 'done'
                self.save()
            except (OSError, ValueError):
                self.block('batch_cleanup_unknown')
                return
            return
        result = aggregate(state['candidate'], [b['result'] for b in state['batches']], state['expected'])
        state['result'] = result
        self.save()
        lease.release()  # All batch units and original resources are gone.
        if self.checks:
            self.checks.finish(state, result)
        state['phase'] = 'done'
        self.save()


def checkout_candidate(config, candidate):
    repository = private_path(config['repository'], directory=True)
    if git('remote', 'get-url', 'origin', cwd=repository) != REPO:
        raise ValueError('candidate repository origin mismatch')
    root = private_path(config['checkout_parent'], directory=True) / candidate
    if not root.exists():
        git('fetch', '--no-tags', 'origin', candidate, cwd=repository)
        git('worktree', 'add', '--detach', str(root), candidate, cwd=repository)
        root.chmod(0o700)
    if git('rev-parse', 'HEAD', cwd=root) != candidate or git('status', '--porcelain', cwd=root):
        raise ValueError('immutable candidate worktree mismatch')
    return dict(config, checkout=str(root))


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--candidate')
    parser.add_argument('--base')
    parser.add_argument('--follow-test', action='store_true', help='trusted test ref only; not arbitrary PR heads')
    parser.add_argument('--local-only', action='store_true', help='no GitHub check write; not automatic CI acceptance')
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('trusted root host only; never run as a GitHub PR job')
    config = json_file(private_path(args.config))
    if args.follow_test and (args.candidate or args.base or not config.get('auto_test')):
        parser.error('follow-test requires the reviewed auto_test policy and no manual candidate')
    if not args.follow_test and (not args.candidate or not args.base):
        parser.error('manual admission requires candidate and base')
    root = private_path(config['state_root'], directory=True)
    lock = (root / 'supervisor.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    checks = None
    if not args.local_only:
        api = GitHubApp(config['client_id'], Path(os.environ['CREDENTIALS_DIRECTORY']) / 'app-key',
                        Transport(config['proxy']), config['installation_id'], permissions=CHECK_PERMISSIONS)
        api.verify_installation()
        checks = Checks(api)
    supervisor = Supervisor(config, checks)
    while True:
        try:
            if not supervisor.state or supervisor.state['phase'] == 'done':
                refs = git('ls-remote', REPO, 'refs/heads/test', 'refs/heads/main')
                refs = {line.split()[1]: line.split()[0] for line in refs.splitlines()}
                candidate = refs['refs/heads/test'] if args.follow_test else args.candidate
                base = refs['refs/heads/main'] if args.follow_test else args.base
                authorize(candidate, approved_commits=config['approved_commits'],
                          observed_test=refs['refs/heads/test'] if config.get('auto_test') else None)
                prepared = checkout_candidate(config, candidate) if config.get('repository') else config
                supervisor.config = prepared
                supervisor.begin(candidate, base, observed_test=refs['refs/heads/test'] if config.get('auto_test') else None)
            if supervisor.state['phase'] == 'blocked':
                break
            supervisor.tick()
            if not args.follow_test and supervisor.state['phase'] == 'done':
                break
        except (APIError, subprocess.SubprocessError, OSError, ValueError) as error:
            print('E2E observation pending: ' + type(error).__name__, flush=True)
        time.sleep(15)
    print(json.dumps({k: supervisor.state.get(k) for k in ('candidate', 'phase', 'reason', 'result')}))
    return 0 if supervisor.state['phase'] == 'done' and supervisor.state['result']['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
