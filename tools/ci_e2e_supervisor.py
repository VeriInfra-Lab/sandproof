#!/usr/bin/env python3
"""Trusted host E2E queue. No privileged GitHub runner; no external PR trigger.

Install this reviewed source on the host. Automatically observes only reviewed
protected promotion refs; other candidates require an exact-SHA allowlist. Dedicated
systemd batch units survive manager restarts. Unknown state is not a retry.
"""
import argparse
import base64
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import time
import uuid

from tools.benchmark_ci import (CHECK_NAME, aggregate, authorize, batch_result, check_name, cohort_policy, oracle_modes,
    instantiate_policy, private_path, remove_private_tree, run_identity, selection)
from tools.benchmark_tasks import canonical, digest, verify, pin_for
from tools.ci_github_app import (APIError, CHECK_PERMISSIONS, SOURCE_PERMISSIONS,
                               GitHubApp, REPOSITORY, Transport)
from tools.ci_runner_supervisor import atomic_json
from tools.system_acceptance import json_file, write_once
from tools.check_kvm_host import trusted_path

PREFIX = '/repos/' + REPOSITORY + '/'
REPO = 'https://github.com/' + REPOSITORY + '.git'
UNIT_PREFIX = 'sandproof-e2e-batch-'


def dataset_files(config):
    dataset = config.get('dataset', 'terminal-bench@2.1')  # Recover prior installed policies unchanged.
    pin = pin_for({'dataset': dataset})
    stem = Path(config['checkout']) / 'benchmarks' / ('terminal-bench-' + pin.version)
    registry = Path(config.get('task_registry', str(stem) + '.registry.json'))
    return dataset, Path(str(stem) + '.lock.json'), registry, Path(str(stem) + '-suites.json')


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
    dataset = dataset_files(config)[0]
    expected_check = check_name(dataset)
    if config.get('check_name', expected_check) not in (expected_check, expected_check + ' / diagnostic'):
        raise ValueError('check name differs from the dataset policy')
    if config.get('task_registry'):
        trusted_path(Path(config['task_registry']))
        if config['task_registry'] not in config['host_assets']:
            raise ValueError('external registry fingerprint missing')
    if config.get('prepare_candidate'):
        for name in ('uv', 'cargo'):
            if config[name] not in config['host_assets']:
                raise ValueError('prepared tool fingerprint missing')
        trusted_path(Path(config['toolchain_bin']))
    for name in ('scratch_parent', 'state_root', 'ci_state_root'):
        private_path(config[name], directory=True)


def git(*args, cwd=None, source_client=None, proxy=None):
    env = dict(os.environ, GIT_TERMINAL_PROMPT='0')
    if proxy:
        env.update(HTTPS_PROXY=proxy, HTTP_PROXY=proxy, https_proxy=proxy, http_proxy=proxy)
    if source_client is not None:
        # Short-lived repository-only read token, never in argv, Git config,
        # checkout, stdout or batch environment. No credential helper runs.
        token = source_client.access_token()
        header = 'Authorization: Basic ' + base64.b64encode(('x-access-token:' + token).encode()).decode()
        env.update(GIT_CONFIG_COUNT='1', GIT_CONFIG_KEY_0='http.https://github.com/.extraheader',
                   GIT_CONFIG_VALUE_0=header)
    return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
        '-c', 'core.fsmonitor=false', '-c', 'credential.helper=', *args],
        cwd=cwd, env=env, text=True, stderr=subprocess.PIPE, timeout=300).strip()


def is_ancestor(base, candidate, *, cwd):
    try:
        git('merge-base', '--is-ancestor', base, candidate, cwd=cwd)
        return True
    except subprocess.CalledProcessError as error:
        if error.returncode != 1:
            raise  # Missing objects or corrupt history are not a negative answer.
        return False


def promotion_heads(*, source_client, proxy):
    expected = {'refs/heads/' + name for name in ('integration', 'test', 'main')}
    output = git('ls-remote', REPO, *sorted(expected), source_client=source_client, proxy=proxy)
    refs = {}
    for line in output.splitlines():
        fields = line.split()
        if (len(fields) != 2 or fields[1] not in expected or fields[1] in refs
                or re.fullmatch('[0-9a-f]{40}', fields[0]) is None):
            raise ValueError('protected promotion ref identity unknown')
        refs[fields[1]] = fields[0]
    if set(refs) != expected:
        raise ValueError('protected promotion refs incomplete')
    return {name.removeprefix('refs/heads/'): sha for name, sha in refs.items()}


def select_promotion(refs, floor, ancestor, ready=lambda candidate: True):
    """Forward promotions and legal main→test sync; never task/fork heads."""
    if (set(refs) != {'integration', 'test', 'main'}
            or any(re.fullmatch('[0-9a-f]{40}', value or '') is None for value in (*refs.values(), floor))):
        raise ValueError('invalid protected promotion policy')
    for source, target in (('integration', 'test'), ('test', 'main'), ('main', 'test')):
        candidate, base = refs[source], refs[target]
        if not ancestor(floor, candidate):
            continue  # Versions older than the reviewed gate implementation.
        if candidate == base or ancestor(candidate, base):
            continue  # Already promoted; do not rerun a completed old version.
        if not ancestor(base, candidate):
            continue  # Its gate stays absent; a legal reverse sync can run first.
        if ready(candidate):
            return source, target, candidate, base
    return None


class Checks:
    def __init__(self, api):
        self.api = api

    def ordinary_ready(self, candidate, app_id):
        """Do not pause the shared host before the promotion's ordinary CI."""
        if type(app_id) is not int or app_id <= 0:
            raise ValueError('ordinary CI App identity is not approved')
        found = self.api.request('GET', PREFIX + 'commits/' + candidate + '/check-runs?per_page=100&filter=all')
        if found['total_count'] > 100:
            raise ValueError('check inventory exceeds fixed limit')
        for name in ('sdk-contract', 'harbor-inputs', 'pr-title', 'pr-policy'):
            matches = [r for r in found['check_runs'] if r.get('name') == name
                       and r.get('head_sha') == candidate and r.get('app', {}).get('id') == app_id]
            if not matches:
                return False
            latest = max(matches, key=lambda r: r['id'])
            if latest.get('status') != 'completed' or latest.get('conclusion') != 'success':
                return False
        return True

    def ensure(self, state, save):
        if state.get('check_id') is not None:
            return state['check_id']
        found = self.api.request('GET', PREFIX + 'commits/' + state['candidate'] + '/check-runs?per_page=100&filter=all')
        if found['total_count'] > 100:
            raise ValueError('check inventory exceeds fixed limit')
        matches = [r for r in found['check_runs'] if r.get('external_id') == state['id']
                   and r.get('name') == state.get('check_name', CHECK_NAME) and r.get('head_sha') == state['candidate']
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
        created = self.api.request('POST', PREFIX + 'check-runs', dict(name=state.get('check_name', CHECK_NAME),
            head_sha=state['candidate'], external_id=state['id'], status='in_progress',
            output=dict(title=state.get('dataset', 'terminal-bench@2.1') + ' 验收进行中',
                summary='配置选定 ' + str(len(state['expected'])) + ' 题；范围和配置绑定本次提交。失败和清理未知均不会绿灯。')))
        if created.get('head_sha') != state['candidate'] or created.get('external_id') != state['id']:
            raise ValueError('created check identity mismatch')
        state['check_id'] = created['id']
        save()
        return state['check_id']

    def finish(self, state, result):
        conclusion = 'success' if result['passed'] else 'failure'
        rows = ['固定版本：' + result['dataset'] + '；范围：' + result['scope'] + '。',
                '基线：' + result.get('baseline', 'original tasks') + '；原解答与评分：' + str(result.get('original_oracle_verifier')) + '。',
                '通过数量：' + str(sum(r['status'] == 'passed' for r in result['tasks'])) + '/' + str(len(result['required_tasks'])) + '。',
                '未运行、评分失败、超时及清理未知均按失败处理；没有使用付费模型。']
        rows.extend(r['name'] + ': ' + str(r['status']) + ' (' + r.get('oracle_mode', 'original') + ')' for r in result['tasks'])
        self.api.request('PATCH', PREFIX + 'check-runs/' + str(state['check_id']), dict(
            status='completed', conclusion=conclusion, output=dict(
                title='配置题集验收通过' if result['passed'] else '配置题集验收未通过', summary='\n\n'.join(rows))))


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
        '--property=RuntimeMaxSec=' + str(batch['deadline_seconds']),
        '--property=TimeoutStopSec=900', '--property=KillSignal=SIGINT', '--property=KillMode=process',
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

    def publish_terminal(self):
        if not self.state.get('check_publish_pending'):
            return
        if self.checks is None:
            raise ValueError('original check publication needs its authorized client')
        self.checks.ensure(self.state, self.save)
        # PATCH of this exact check is idempotent. The durable result is never
        # rerun when its completion receipt is lost.
        self.checks.finish(self.state, self.state['result'])
        self.state['check_publish_pending'] = False
        self.save()

    def terminal(self, result, *, phase, reason=None):
        self.state.update(result=result, phase=phase, check_publish_pending=self.checks is not None)
        if reason:
            self.state['reason'] = reason
        self.save()  # Terminal evidence BEFORE API mutation, including failure.
        self.publish_terminal()

    def block(self, reason):
        result = aggregate(self.state['candidate'],
            [b['result'] for b in self.state['batches'] if 'result' in b], self.state['expected'],
            all_tasks=self.state.get('all_tasks'), dataset=self.state.get('dataset', 'terminal-bench@2.1'))
        result['passed'] = False
        self.terminal(result, phase='blocked', reason=reason)

    def begin(self, candidate, base, observed_test=None, observed_promotion=None):
        if observed_promotion is not None and self.config.get('auto_promotions') is not True:
            raise ValueError('automatic promotion policy is not enabled')
        authorize(candidate, approved_commits=self.config['approved_commits'], observed_test=observed_test,
                  observed_promotion=observed_promotion)
        validate_host_inputs(self.config)
        checkout = private_path(self.config['checkout'], directory=True)
        if git('rev-parse', 'HEAD', cwd=checkout) != candidate or git('status', '--porcelain', cwd=checkout):
            raise ValueError('clean operator-prepared exact candidate checkout required')
        # A changed target cannot be covered by a check of an unrelated tree.
        # Strict branch protection is still required for the final merge race.
        git('merge-base', '--is-ancestor', base, candidate, cwd=checkout)
        dataset, lock_path, registry, catalog = dataset_files(self.config)
        lock = verify(json_file(lock_path, limit=8*1024*1024), registry, Path(self.config['task_source']))
        if lock['dataset'] != dataset:
            raise ValueError('dataset configuration differs from the verified lock')
        manifest = json_file(catalog)
        if digest(canonical(manifest)) != self.config['selection_sha256']:
            raise ValueError('selection catalog changed; operator review required')
        batches = selection(lock, manifest, self.config['selection'])
        modes = oracle_modes(lock, manifest)
        template = json_file(Path(self.config['policy_template']))
        for names in batches:
            cohort_policy(template, len(names))  # Before state creation or host pause.
        if self.config['source_kind'] != manifest['source_kind']:
            raise ValueError('environment preparation differs from the frozen subset policy')
        identity = run_identity(candidate, base, self.config, manifest, lock)
        if self.state:
            if self.state.get('check_publish_pending'):
                raise ValueError('original check completion receipt remains unresolved')
            if self.state['id'] != identity and self.state['phase'] != 'done':
                raise ValueError('unresolved original run; cannot admit a replacement')
            if self.state['id'] == identity:
                return  # Never repeat the same candidate/base/config identity.
            archive = self.root / (self.state['id'] + '.json')
            if not archive.exists():
                write_once(archive, self.state)
        self.state = dict(id=identity, candidate=candidate, base=base, phase='waiting', configuration=self.config,
            dataset=dataset,
            promotion=dict(source=observed_promotion[0], target=observed_promotion[1]) if observed_promotion else None,
            batches=[dict(names=names, oracle_mode=modes[names[0]], phase='new') for names in batches],
            expected=[name for names in batches for name in names], check_id=None,
            check_name=self.config.get('check_name', check_name(dataset)),
            all_tasks=[row['name'] for row in lock['tasks']],
            task_inputs={t['name']: dict(files_sha256=t['files_sha256'], requirements=t['requirements'])
                         for t in lock['tasks'] if t['name'] in {n for b in batches for n in b}})
        self.save()

    def prepare(self, batch, index):
        config = self.config
        validate_host_inputs(config)
        checkout = Path(config['checkout'])
        root = private_path(config['scratch_parent'], directory=True) / uuid.uuid4().hex[:8]
        namespaces = [uuid.uuid4().hex[:16], uuid.uuid4().hex[:16]]
        policy = instantiate_policy(json_file(Path(config['policy_template'])), root, namespaces,
                                    job_count=len(batch['names']))
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
        _, lock_path, registry, _ = dataset_files(config)
        args.extend(['--registry', str(registry),
            '--source', config['task_source'], '--lock', str(lock_path),
            '--bounded-policy', str(policy_file), '--run-root', str(root),
            '--namespace', namespaces[0], '--build-namespace', namespaces[1], '--temporarily-forward',
            '--oracle-mode', batch.get('oracle_mode', 'original'), '--source-kind', config.get('source_kind', 'oci')])
        if config.get('docker_hub_mirror'):
            args.extend(['--docker-hub-mirror', config['docker_hub_mirror']])
        for name in batch['names']:
            args.extend(['--task', name])
        batch.update(root=str(root), command=args, checkout=str(checkout),
            deadline_seconds=int(sum(sum(self.state['task_inputs'][name]['requirements'][key]
                for key in ('build_timeout_sec', 'agent_timeout_sec', 'verifier_timeout_sec'))
                for name in batch['names'])) + 7200,
            unit=UNIT_PREFIX + uuid.uuid4().hex + '.service', phase='launching')
        self.save()  # Intent before launching; original identity survives crash.

    def tick(self):
        state = self.state
        if not state:
            return
        if state['phase'] in ('done', 'blocked'):
            self.publish_terminal()
            return
        if self.checks:
            self.checks.ensure(state, self.save)
        lease = HostLease(Path(self.config['ci_state_root']), state['id'])
        if state['phase'] == 'waiting':
            if state.get('promotion'):
                if self.checks is None:
                    raise ValueError('promotion requires authorized GitHub check observation')
                if not self.checks.ordinary_ready(state['candidate'], self.config.get('ordinary_check_app_id')):
                    return
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
                subprocess.run(['systemctl', 'stop', state['prepare_unit']], check=True, capture_output=True, timeout=20)
                lease.release()  # Preparation never dispatched a sandbox.
                result = aggregate(state['candidate'], [], state['expected'], all_tasks=state['all_tasks'],
                                   dataset=state.get('dataset', 'terminal-bench@2.1'))
                self.terminal(result, phase='done', reason='candidate_preparation_failed')
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
                    assets = self.config['host_assets'] | state.get('candidate_artifacts', {})
                    result = batch_result(root, candidate=state['candidate'], names=batch['names'],
                        dataset=state.get('dataset', 'terminal-bench@2.1'), source_kind=self.config['source_kind'],
                        oracle_mode=batch.get('oracle_mode', 'original'),
                        exit_code=int(fields['ExecMainStatus']), full_task_count=len(state['all_tasks']), expected_hashes={
                            name: assets[self.config[key]] for name, key in
                            (('server_sha256', 'server'), ('kernel_sha256', 'kernel'),
                             ('builder_base_sha256', 'rootfs'))})
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
        result = aggregate(state['candidate'], [b['result'] for b in state['batches']], state['expected'],
                           all_tasks=state['all_tasks'], dataset=state.get('dataset', 'terminal-bench@2.1'))
        state['result'] = result
        self.save()
        lease.release()  # All batch units and original resources are gone.
        self.terminal(result, phase='done')


def checkout_candidate(config, candidate, source_client=None):
    repository = private_path(config['repository'], directory=True)
    if git('remote', 'get-url', 'origin', cwd=repository) != REPO:
        raise ValueError('candidate repository origin mismatch')
    root = private_path(config['checkout_parent'], directory=True) / candidate
    if not root.exists():
        git('fetch', '--no-tags', 'origin', candidate, cwd=repository,
            source_client=source_client, proxy=config.get('proxy'))
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
    automatic = parser.add_mutually_exclusive_group()
    automatic.add_argument('--follow-test', action='store_true', help='legacy trusted test-only observer')
    automatic.add_argument('--follow-promotions', action='store_true',
                           help='protected promotions and legal main→test sync only')
    parser.add_argument('--local-only', action='store_true', help='no GitHub check write; not automatic CI acceptance')
    selected = parser.add_mutually_exclusive_group()
    selected.add_argument('--profile', help='reviewed task profile from the frozen catalog')
    selected.add_argument('--task', action='append', help='exact task name; repeat to select a custom subset')
    parser.add_argument('--batch-size', type=int, help='maximum retained tasks per serial batch')
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('trusted root host only; never run as a GitHub PR job')
    config = json_file(private_path(args.config))
    if (args.follow_test or args.follow_promotions) and (args.profile or args.task or args.batch_size is not None):
        parser.error('automatic selection comes only from the protected host policy')
    if args.profile or args.task or args.batch_size is not None:
        config['selection'] = dict(config['selection'])
        # A one-task diagnostic must never satisfy the default promotion gate.
        config['check_name'] = check_name(config.get('dataset', 'terminal-bench@2.1')) + ' / diagnostic'
        if args.profile:
            config['selection'].pop('tasks', None)
            config['selection']['profile'] = args.profile
        if args.task:
            config['selection'].pop('profile', None)
            config['selection']['tasks'] = args.task
        if args.batch_size is not None:
            config['selection']['batch_size'] = args.batch_size
    if args.follow_test and (args.candidate or args.base or not config.get('auto_test')):
        parser.error('follow-test requires the reviewed auto_test policy and no manual candidate')
    if args.follow_promotions and (args.candidate or args.base or args.local_only
            or config.get('auto_promotions') is not True or not config.get('repository')
            or type(config.get('ordinary_check_app_id')) is not int or config['ordinary_check_app_id'] <= 0
            or re.fullmatch('[0-9a-f]{40}', config.get('promotion_floor', '')) is None):
        parser.error('follow-promotions requires an enabled private repository/floor policy and GitHub checks')
    if not (args.follow_test or args.follow_promotions) and (not args.candidate or not args.base):
        parser.error('manual admission requires candidate and base')
    root = private_path(config['state_root'], directory=True)
    lock = (root / 'supervisor.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    checks = None
    source_client = None
    if config.get('repository'):
        source_client = GitHubApp(config['client_id'], Path(os.environ['CREDENTIALS_DIRECTORY']) / 'app-key',
                                 Transport(config['proxy']), config['installation_id'], permissions=SOURCE_PERMISSIONS)
        source_client.verify_installation()
    if not args.local_only:
        api = GitHubApp(config['client_id'], Path(os.environ['CREDENTIALS_DIRECTORY']) / 'app-key',
                        Transport(config['proxy']), config['installation_id'], permissions=CHECK_PERMISSIONS)
        api.verify_installation()
        checks = Checks(api)
    supervisor = Supervisor(config, checks)
    observed_refs = None
    while True:
        try:
            if supervisor.state and supervisor.state.get('check_publish_pending'):
                supervisor.publish_terminal()
            if supervisor.state and supervisor.state['phase'] == 'blocked':
                break
            if not supervisor.state or supervisor.state['phase'] == 'done':
                observed_promotion = None
                refs = {}
                if args.follow_promotions:
                    heads = promotion_heads(source_client=source_client, proxy=config.get('proxy'))
                    repository = private_path(config['repository'], directory=True)
                    if git('remote', 'get-url', 'origin', cwd=repository) != REPO:
                        raise ValueError('promotion repository origin mismatch')
                    if heads != observed_refs:
                        git('fetch', '--no-tags', 'origin', 'refs/heads/integration', 'refs/heads/test',
                            'refs/heads/main', cwd=repository, source_client=source_client, proxy=config.get('proxy'))
                        observed_refs = heads
                    proposal = select_promotion(heads, config['promotion_floor'],
                        lambda parent, child: is_ancestor(parent, child, cwd=repository),
                        lambda candidate: checks.ordinary_ready(candidate, config['ordinary_check_app_id']))
                    if proposal is None:
                        time.sleep(15)
                        continue
                    source, target, candidate, base = proposal
                    observed_promotion = (source, target, candidate)
                else:
                    values = git('ls-remote', REPO, 'refs/heads/test', 'refs/heads/main',
                        source_client=source_client, proxy=config.get('proxy')) if args.follow_test else ''
                    refs = {line.split()[1]: line.split()[0] for line in values.splitlines()}
                    candidate = refs['refs/heads/test'] if args.follow_test else args.candidate
                    base = refs['refs/heads/main'] if args.follow_test else args.base
                authorize(candidate, approved_commits=config['approved_commits'],
                          observed_test=refs.get('refs/heads/test') if config.get('auto_test') else None,
                          observed_promotion=observed_promotion)
                prepared = checkout_candidate(config, candidate, source_client) if config.get('repository') else config
                supervisor.config = prepared
                supervisor.begin(candidate, base, observed_test=refs.get('refs/heads/test') if config.get('auto_test') else None,
                                 observed_promotion=observed_promotion)
            if supervisor.state['phase'] == 'blocked':
                break
            supervisor.tick()
            if not (args.follow_test or args.follow_promotions) and supervisor.state['phase'] == 'done':
                break
        except (APIError, subprocess.SubprocessError, OSError, ValueError) as error:
            print('E2E observation pending: ' + type(error).__name__, flush=True)
        time.sleep(15)
    print(json.dumps({k: supervisor.state.get(k) for k in ('candidate', 'phase', 'reason', 'result')}))
    return 0 if supervisor.state['phase'] == 'done' and supervisor.state['result']['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
