"""Run pinned task recipes and Harbor's oracle/verifier against a local base VM.

This module never builds a template or interprets Dockerfiles. A privileged,
external supervisor must provision and independently audit the local service.
"""

import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shlex
import sys
import time
from urllib.parse import urlsplit

from e2b import AsyncSandbox
from harbor.agents.oracle import OracleAgent
from harbor.environments.capabilities import EnvironmentCapabilities
from harbor.environments.e2b import E2BEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.task.task import Task
from harbor.models.trial.paths import TrialPaths
from harbor.verifier.verifier import Verifier

from benchmarks.artifact_budget import add_artifact_arguments, artifact_limits_from_args
from benchmarks.check_harbor import check_runtime, check_tasks
from benchmarks.check_browser_evidence import audit_browser_evidence
from benchmarks.failure_diagnostics import collect as collect_failure_diagnostics
from benchmarks.oracle_repairs import REPAIRS, prepare_repaired_task
from benchmarks.sdk_environment import SdkEnvironment
from tools.benchmark_tasks import InvalidInput, verify

BASE_FROM = {
    'ubuntu-24.04': ['ubuntu:24.04'],
    'python-3.13.9-bookworm': ['python:3.13-slim-bookworm'],
    'python311slim': ['python:3.11-slim'],
    'python310': ['python:3.10-slim-bookworm'],
    'python311full': ['python:3.11'],
    'debian13': ['debian:13.0-slim'],
    'bullseye': ['debian:bullseye-slim'],
}

RECIPES = Path(__file__).with_name('task-recipes.json')
TASK_BASES = {
    'task-custom-memory-heap-crash': (
        'custom-memory-heap-crash', 'alexgshaw/custom-memory-heap-crash:20251031'),
}
GUEST_HELPERS = {
    'fix-git-source': (Path(__file__).with_name('guest_git_fixture.py'),
                       '/usr/local/lib/sandproof/guest_git_fixture.py'),
    'range-fetch': (Path(__file__).with_name('guest_fetch.py'),
                    '/usr/local/lib/sandproof/guest_fetch.py'),
    'chromium-lifecycle': (Path(__file__).with_name('guest_chromium.py'),
                           '/usr/local/lib/sandproof/guest_chromium.py'),
}


def local_url(value):
    url = urlsplit(value)
    if (url.scheme != 'http' or url.hostname not in ('127.0.0.1', 'localhost', '::1')
            or url.username or url.password or url.path not in ('', '/')
            or url.query or url.fragment):
        raise InvalidInput('only an explicit local HTTP endpoint is accepted')
    return value


def input_path(root, relative):
    path = PurePosixPath(relative)
    if path.is_absolute() or '..' in path.parts:
        raise InvalidInput('recipe input must remain in environment/')
    result = root / path
    if not result.resolve().is_relative_to(root.resolve()) or result.is_symlink():
        raise InvalidInput('recipe input escapes environment/')
    return result


def load_recipes(lock, source):
    recipes = json.loads(RECIPES.read_text())
    entries = {entry['name']: entry for entry in lock['tasks']}
    if set(recipes) != set(entries):
        raise InvalidInput('recipes must account for the entire pinned task set')
    for name, recipe in recipes.items():
        entry = entries[name]
        expected_from = BASE_FROM.get(recipe['base'])
        if recipe['base'] in TASK_BASES:
            task_name, reference = TASK_BASES[recipe['base']]
            if name != task_name or entry.get('upstream_image_reference') != reference:
                raise InvalidInput(name + ': task image cannot be reused for another task or reference')
            expected_from = ['ubuntu:24.04']
        if name == 'financial-document-processor' and recipe['base'] == 'ubuntu-24.04':
            expected_from = ['ubuntu:24.04 as build', 'ubuntu:24.04 AS target']
        if expected_from is None or [x.strip() for x in entry['dockerfile_from_lines']] != expected_from:
            raise InvalidInput(name + ': recipe requires its reviewed fixed base')
        root = source / name / 'environment'
        if hashlib.sha256((root / 'Dockerfile').read_bytes()).hexdigest() != recipe['environment_sha256']:
            raise InvalidInput(name + ': recipe differs from the pinned environment')
        for step in recipe['steps']:
            if 'helper' in step:
                if set(step) != {'helper'} or step['helper'] not in GUEST_HELPERS:
                    raise InvalidInput(name + ': unknown guest helper')
                target = GUEST_HELPERS[step['helper']][1]
            elif 'copy' in step:
                if set(step) != {'copy', 'target'} or not input_path(root, step['copy']).exists():
                    raise InvalidInput(name + ': invalid recipe copy')
                target = step['target']
            else:
                if set(step) - {'run', 'cwd', 'env'} or not isinstance(step['run'], str):
                    raise InvalidInput(name + ': invalid recipe command')
                target = step['cwd']
            if not target.startswith('/') or '..' in PurePosixPath(target).parts:
                raise InvalidInput(name + ': unsafe guest destination')
    return recipes


class LocalBaseEnvironment(SdkEnvironment):
    def __init__(self, *, recipe, sdk_options, **kwargs):
        self.recipe = recipe
        self._startup_handles = []
        super().__init__(sdk_options=sdk_options, **kwargs)
        self._workdir = recipe['workdir']

    @property
    def capabilities(self):
        # Public IPv4 only; do not inherit E2B's
        # unsupported dynamic policies, hostname allowlists or cloud promises.
        return EnvironmentCapabilities()

    async def start(self, force_build=False):
        if force_build:
            raise InvalidInput('online template builds are unsupported')
        if not self._network_is_public:
            raise InvalidInput('this local runner only accepts the reviewed public IPv4 policy')
        self.create_dispatched = True
        self._sandbox = await AsyncSandbox.create(
            template='base', timeout=86400, allow_internet_access=True,
            metadata={'benchmark': 'terminal-bench-2.0', 'task': self.environment_name},
            envs=self._startup_env(),
            **self.sdk_options,
        )
        self.sandbox_id = self._sandbox.sandbox_id
        info = await self._sandbox.get_info()
        if (info.cpu_count, info.memory_mb) != (self.task_env_config.cpus, self.task_env_config.memory_mb):
            raise InvalidInput('actual SDK resource profile differs from the task')
        identity = await self.exec('cat /etc/os-release', cwd='/', timeout_sec=30)
        if identity.return_code:
            raise InvalidInput('guest OS identity could not be observed')
        if self.recipe['base'] == 'ubuntu-24.04':
            if 'ID=ubuntu\n' not in identity.stdout or 'VERSION_ID="24.04"' not in identity.stdout:
                raise InvalidInput('guest OS differs from recipe base')
        else:
            marker = await self._sandbox.files.read('/etc/sandproof-base-profile')
            if marker.strip() != self.recipe['base']:
                raise InvalidInput('guest fixed profile differs from recipe')
        await self._sandbox.files.write('/etc/resolv.conf', 'nameserver 1.1.1.1\noptions timeout:2 attempts:2\n')
        descriptors = await self.exec(
            'test -e /dev/fd || ln -s /proc/self/fd /dev/fd', cwd='/', timeout_sec=30)
        if descriptors.return_code:
            raise RuntimeError('guest file descriptor directory is unavailable')
        if self.recipe['base'] == 'ubuntu-24.04':
            # The fixed VM base owns sysctl.conf before procps is installed.
            # Preserve its existing settings when dpkg asks about a conffile;
            # DEBIAN_FRONTEND alone does not control that prompt.
            await self._sandbox.files.write(
                '/etc/apt/apt.conf.d/99sandproof-conffiles',
                'Dpkg::Options { "--force-confdef"; "--force-confold"; };\n')
        await self.exec('mkdir -p /logs/agent /logs/verifier /logs/artifacts', cwd='/', timeout_sec=30)
        for step in self.recipe['steps']:
            if 'helper' in step:
                source, target = GUEST_HELPERS[step['helper']]
                await self._sandbox.files.make_dir(str(PurePosixPath(target).parent))
                await self.upload_file(source, target)
            elif 'copy' in step:
                source = input_path(self.environment_dir, step['copy'])
                if source.is_dir():
                    await self.upload_dir(source, step['target'])
                else:
                    await self.upload_file(source, step['target'])
            else:
                result = await self.exec(step['run'], cwd=step['cwd'],
                                         env={'DEBIAN_FRONTEND': 'noninteractive', **step.get('env', {})},
                                         timeout_sec=int(self.task_env_config.build_timeout_sec))
                if result.return_code:
                    raise RuntimeError('initialization command failed')
        if self.recipe.get('start_command'):
            handle = await self._sandbox.commands.run(
                self.recipe['start_command'] + ' > /logs/initializer-service.txt 2>&1',
                background=True, cwd=self._workdir, envs=self._startup_env(), timeout=0, user='root')
            self._startup_handles.append(handle)
            # The only registered long-lived initializer is this task's
            # supervisord. Require its actual PID file and live process.
            ready = await self.exec(
                'for i in $(seq 1 100); do test -s /root/supervisord.pid && '
                'kill -0 $(cat /root/supervisord.pid) && exit 0; sleep 0.1; done; exit 1',
                cwd='/', timeout_sec=15)
            if ready.return_code:
                raise RuntimeError('task startup service did not become ready')
        # Initializers cannot leave private solution/test material behind.
        boundary = await self.exec('test ! -e /solution && test ! -e /tests', cwd='/', timeout_sec=30)
        if boundary.return_code:
            raise InvalidInput('initialization leaked a reserved phase directory')

def write_report(path, content, row):
    """Retain local reporting failures without preventing exact VM cleanup."""
    try:
        path.write_text(content)
    except OSError as error:
        row.setdefault('report_errors', {})[path.name] = {
            'error_type': type(error).__name__, 'errno': error.errno,
        }
        # Missing local evidence cannot be advertised as a complete pass.
        # Preserve interrupted/unknown outcomes and the original stage results.
        if row['status'] == 'passed':
            row['status'] = 'failed'
            row['reason'] = 'local_report_write_failed'


def record_artifact_status(environment, row):
    budget = getattr(environment, 'artifact_budget', None)
    if budget is not None:
        row['artifact_budget'] = budget.report()
        row['evidence_complete'] = budget.failure is None
        if budget.failure is not None and row['status'] == 'passed':
            row['status'] = 'failed'
            row['reason'] = 'artifact_evidence_incomplete'


async def run_task(args, entry, recipe, *, environment_factory=None, result_row=None):
    task = Task(args.source / entry['name'])
    paths = TrialPaths(args.output / entry['name'])
    paths.trial_dir.mkdir(mode=0o700)
    for directory in (paths.agent_dir, paths.verifier_dir, paths.artifacts_dir):
        directory.mkdir(mode=0o700)
    recipe = recipe or {}
    factory = environment_factory or LocalBaseEnvironment
    environment = factory(
        recipe=recipe, sdk_options=dict(api_key=os.environ['SANDPROOF_TEST_API_KEY'],
            api_url=args.api_url, sandbox_url=args.api_url, domain='example.invalid',
            retries=0, request_timeout=60),
        environment_dir=task.paths.environment_dir, environment_name=task.name,
        session_id=task.name, trial_paths=paths, task_env_config=task.config.environment,
        persistent_env=recipe.get('env', {}),
        artifact_limits=getattr(args, 'artifact_limits', None),
    )
    oracle_mode = getattr(args, 'oracle_mode', 'original')
    # The enclosing supervisor retains this same row even when cancellation
    # propagates and result.json cannot be published. Disk reports are evidence,
    # not the only channel carrying the original identity and failure to cleanup.
    row = result_row if result_row is not None else {}
    row.update(name=task.name, status='running', stage='initialization',
               oracle_mode=oracle_mode, oracle='not_run', verifier='not_run')
    start = time.monotonic()
    def event(stage):
        row['stage'] = stage
        if stage in ('oracle', 'verifier'):
            row[stage] = 'running'
        print(json.dumps({'task': task.name, 'stage': stage}), flush=True)
    try:
        event('initialization')
        await asyncio.wait_for(environment.start(), task.config.environment.build_timeout_sec)
        event('oracle')
        oracle_task_dir = task.paths.task_dir
        if oracle_mode == 'repaired':
            oracle_task_dir = prepare_repaired_task(
                task.name, task.paths.task_dir, paths.trial_dir / 'repaired-task')
        oracle = OracleAgent(logs_dir=paths.agent_dir, task_dir=oracle_task_dir,
                             trial_paths=paths, agent_timeout_sec=task.config.agent.timeout_sec)
        await asyncio.wait_for(oracle.run(task.instruction, environment, AgentContext()), task.config.agent.timeout_sec)
        row['oracle'] = 'failed' if (paths.agent_dir / 'exit-code.txt').exists() else 'passed'
        record_artifact_status(environment, row)
        event('verifier')
        verifier = Verifier(task=task, trial_paths=paths, environment=environment)
        result = await asyncio.wait_for(verifier.verify(), task.config.verifier.timeout_sec)
        row['rewards'] = result.rewards
        row['verifier'] = 'passed' if result.rewards == {'reward': 1} else 'failed'
        row['status'] = 'passed' if row['oracle'] == row['verifier'] == 'passed' else 'failed'
        record_artifact_status(environment, row)
        if environment_factory is None and task.name == 'filter-js-from-html':
            lifecycle = paths.verifier_dir / 'browser-lifecycle.jsonl'
            row['verification_audit'] = audit_browser_evidence(
                paths.test_stdout_path.read_text() if paths.test_stdout_path.exists() else '',
                lifecycle.read_text() if lifecycle.exists() else '')
            if not row['verification_audit']['complete']:
                row['status'] = 'failed'
                row['reason'] = 'browser verification incomplete; original reward retained'
        event('package_inventory')
        packages = await environment.exec("dpkg-query -W -f='${Package}=${Version}\\n'", cwd='/', timeout_sec=30)
        environment.retain_text(paths.trial_dir / 'packages.txt', packages.stdout or '')
        row['stage'] = 'verifier'
    except (asyncio.CancelledError, KeyboardInterrupt) as error:
        row['status'] = 'interrupted'
        row['error_type'] = type(error).__name__
        row['interrupted'] = True
        if row['stage'] in ('oracle', 'verifier'):
            row[row['stage']] = 'interrupted'
        # Do not turn cancellation into a completed run; finally still records
        # the interruption and attempts cleanup before propagating it.
        raise
    except Exception as error:
        row['status'] = 'timeout' if isinstance(error, TimeoutError) else 'failed'
        row['error_type'] = type(error).__name__
        if row['stage'] in ('oracle', 'verifier'):
            row[row['stage']] = row['status']
        write_report(paths.trial_dir / 'error.txt', str(error), row)
        # Preserve partial diagnostics before destroying the VM. A log failure
        # must neither hide the original result nor prevent cleanup.
        if environment.sandbox_id is not None:
            for remote, target in (
                ('/logs/agent/oracle.txt', paths.agent_dir / 'oracle.txt'),
                ('/logs/verifier/test-stdout.txt', paths.test_stdout_path),
                ('/logs/verifier/reward.txt', paths.reward_text_path),
            ):
                if target.exists():
                    continue
                try:
                    await asyncio.wait_for(environment.download_file(remote, target), 10)
                except Exception as log_error:
                    row.setdefault('diagnostic_errors', {})[remote] = type(log_error).__name__
    finally:
        record_artifact_status(environment, row)
        row['execution_status'] = row['status']
        row['sandbox_id'] = environment.sandbox_id
        row['create_dispatched'] = environment.create_dispatched
        if hasattr(environment, 'build_id'):
            row.update(build_id=environment.build_id, build=environment.build_result,
                       build_log_error=getattr(environment, 'build_log_error', None),
                       template=environment.template, exact_cleanup='not_audited',
                       build_dispatched=environment.build_dispatched,
                       submission_key=environment.submission_key)
            row['runtime_intent'] = getattr(environment, 'runtime_intent', None)
        if environment.create_dispatched and environment.sandbox_id is None:
            row['status'] = 'create_unknown'
        try:
            if (getattr(args, 'failure_diagnostics', False) and environment.sandbox_id is not None
                    and row['status'] == 'failed' and row['verifier'] == 'failed'):
                try:
                    row['failure_diagnostic'] = await collect_failure_diagnostics(
                        environment, task.name, paths.trial_dir)
                except (asyncio.CancelledError, KeyboardInterrupt):
                    row['failure_diagnostic'] = {'status': 'interrupted'}
                    row['interrupted'] = True
                    raise
            if environment.sandbox_id is not None and row['status'] in ('failed', 'timeout'):
                try:
                    observation = await asyncio.wait_for(environment.exec(
                        'cat /proc/sys/vm/overcommit_memory /proc/meminfo\ndf -Pk /\ndf -Pi /',
                        cwd='/', timeout_sec=10), 10)
                    environment.retain_text(paths.trial_dir / 'environment-diagnostics.txt',
                                            (observation.stdout or '') + (observation.stderr or ''))
                except Exception as diagnostic_error:
                    row.setdefault('diagnostic_errors', {})['environment'] = type(diagnostic_error).__name__
            write_report(paths.trial_dir / 'events.json',
                         json.dumps(environment.events, indent=2), row)
        finally:
            # Even diagnostics/serialization errors must reach this path.
            try:
                await environment.stop()
            except (Exception, asyncio.CancelledError, KeyboardInterrupt) as error:
                row['cleanup_error_type'] = type(error).__name__
                row['status'] = 'cleanup_unknown'
                if isinstance(error, (asyncio.CancelledError, KeyboardInterrupt)):
                    row['interrupted'] = True
                    raise
            finally:
                record_artifact_status(environment, row)
                row['kill_confirmed'] = environment.kill_confirmed
                row['seconds'] = round(time.monotonic() - start, 3)
                write_report(paths.trial_dir / 'result.json',
                             json.dumps(row, indent=2) + '\n', row)
                print(json.dumps(row), flush=True)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--registry', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--task', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--oracle-mode', choices=('original', 'repaired'), default='original')
    add_artifact_arguments(parser)
    args = parser.parse_args()
    try:
        args.artifact_limits = artifact_limits_from_args(args)
        local_url(args.api_url)
        runtimes = check_runtime()
        lock = verify(json.loads(args.lock.read_text()), args.registry, args.source)
        check_tasks(lock, args.source)
        recipes = load_recipes(lock, args.source)
        entry = next(t for t in lock['tasks'] if t['name'] == args.task)
        if args.oracle_mode == 'repaired' and args.task not in REPAIRS:
            raise InvalidInput('no reviewed reference-solution repair for ' + args.task)
        if args.task not in recipes:
            raise InvalidInput('task requires an additional reviewed base/initialization recipe')
        args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
        (args.output / 'inputs.json').write_text(json.dumps({'source': lock['source'],
            'runtimes': runtimes, 'task': entry, 'recipe': recipes[args.task],
            'oracle_mode': args.oracle_mode, 'artifact_limits': args.artifact_limits,
            'recipe_file_sha256': hashlib.sha256(RECIPES.read_bytes()).hexdigest()}, indent=2))
        rows = [{'name': t['name'], 'status': 'not_run',
                 'reason': 'recipe_available' if t['name'] in recipes else 'additional_base_or_initializer_required'}
                for t in lock['tasks']]
        row = asyncio.run(run_task(args, entry, recipes[args.task]))
        rows = [row if t['name'] == args.task else t for t in rows]
        (args.output / 'full-set.json').write_text(json.dumps({
            'task_count': len(rows), 'tasks': rows, 'scope': 'Harbor OracleAgent/Verifier via fixed-base adaptation; not upstream template compatibility or full-set acceptance',
            'oracle_mode': args.oracle_mode,
            'source': lock['source'], 'dependencies': 'original task downloads; package inventory observed, remote contents not fully locked',
        }, indent=2) + '\n')
        return 0 if row['status'] == 'passed' else 1
    except (InvalidInput, OSError, ValueError, KeyError, StopIteration) as error:
        print(f'Local benchmark rejected: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
