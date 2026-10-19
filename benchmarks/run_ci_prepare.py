"""One-time preparation using the existing guest-only builder and exact audit.

No oracle or test runs here. A Ready build is not a passing benchmark result.
The completed manifest is published only after every original build is reclaimed.
"""
import asyncio
from functools import partial
import json
import os
from pathlib import Path

from harbor.models.task.task import Task
from harbor.models.trial.paths import TrialPaths

from benchmarks.ci_fixtures import ci_task_names
from benchmarks.generic_environment import GenericEnvironment
from benchmarks.prepare_ci_fixtures import (
    archive_ci_context, make_ci_fixture_manifest, publish_ready_fixture,
)
from tools.benchmark_tasks import InvalidInput


def validate_preparation_timeout(value=None):
    value = 900 if value is None else value
    if type(value) is not int or not 1 <= value <= 900:
        raise InvalidInput('CI preparation timeout must be an integer from 1 to 900 seconds')
    return value


async def prepare_selected(args, lock):
    owner_uid = args.ready_artifacts_owner_uid
    if type(owner_uid) is not int or not 0 <= owner_uid <= 2**32 - 2:
        raise InvalidInput('explicit approved Ready registry owner required')
    selected = set(args.task)
    if (lock['dataset'] != 'terminal-bench@2.0' or args.all
            or selected != set(ci_task_names(lock))
            or args.source_kind != 'dockerfile' or args.oracle_mode != 'original'):
        raise InvalidInput('one-time CI preparation requires exactly its two original tasks')
    timeout = validate_preparation_timeout(getattr(args, 'preparation_timeout_secs', None))
    inputs = json.loads(args.ci_build_inputs.read_text())
    if not isinstance(inputs, dict) or set(inputs) != selected:
        raise InvalidInput('explicit original OCI digest required for both CI fixtures')
    destination = args.prepare_ci_fixtures
    if (not destination.is_absolute() or destination.resolve() != destination
            or destination.exists() or destination.is_symlink()):
        raise InvalidInput('fresh canonical private fixture destination required')
    destination.mkdir(mode=0o700)
    rows, items = [], []
    for entry in (t for t in lock['tasks'] if t['name'] in selected):
        task = Task(args.source / entry['source_path'])
        paths = TrialPaths(args.output / entry['name'])
        paths.trial_dir.mkdir(mode=0o700)
        row = dict(name=entry['name'], status='failed', stage='fixture_preparation',
                   oracle='not_run', verifier='not_run', original_environment=False,
                   environment_source_kind='ci-preparation', exact_cleanup='unknown',
                   create_dispatched=False, preparation_timeout_secs=timeout,
                   original_build_timeout_secs=entry['requirements']['build_timeout_sec'],
                   original_build_deadline_applied=False)
        preparation_entry = entry | {'requirements': entry['requirements'] | {'build_timeout_sec': timeout}}
        env = GenericEnvironment(entry=preparation_entry, source=args.source, source_kind='dockerfile',
            build_url=args.build_url, sdk_options=dict(api_key=os.environ['SANDPROOF_TEST_API_KEY']),
            environment_dir=task.paths.environment_dir, environment_name=entry['name'],
            session_id=entry['name'], trial_paths=paths, task_env_config=task.config.environment,
            artifact_limits=args.artifact_limits,
            context_archiver=partial(archive_ci_context, uv_path=args.ci_uv,
                                     base_digest=inputs[entry['name']]))
        item = None
        try:
            await env.prepare_template()
            item = await asyncio.to_thread(publish_ready_fixture, entry, env.template,
                args.ready_artifacts_root / env.build_id / 'rootfs.ext4', args.kernel,
                destination / entry['name'], inputs[entry['name']], ready_owner_uid=owner_uid)
            row['status'] = 'prepared'
        except Exception as error:
            row['error_type'] = type(error).__name__
        finally:
            row.update(build_dispatched=env.build_dispatched, build_id=env.build_id,
                       build=env.build_result, template=env.template)
            # Preserve uncertain build identity; the shared trusted audit waits
            # for its existing reservation, never submits replacement work.
            audit = asyncio.create_task(asyncio.to_thread(args.completion_audit, row))
            cancelled = None
            try:
                while not audit.done():
                    try:
                        await asyncio.shield(audit)
                    except asyncio.CancelledError as error:
                        cancelled = error
                row.update(audit.result())
            except Exception as error:
                row.update(status='cleanup_unknown', exact_cleanup='unknown',
                           audit_error_type=type(error).__name__)
            rows.append(row)
            (paths.trial_dir / 'result.json').write_text(json.dumps(row, indent=2) + '\n')
            (args.output / 'full-set.json').write_text(json.dumps({
                'dataset': lock['dataset'], 'tasks': rows, 'scope': 'fixture preparation only',
                'preparation_timeout_secs': timeout, 'original_build_deadline_applied': False,
                'benchmark_passed': False, 'original_oracle': False, 'original_verifier': False,
                'original_environment': False}, indent=2) + '\n')
            if cancelled:
                raise cancelled
        if row['status'] != 'prepared' or row['exact_cleanup'] != 'passed':
            break
        items.append(item)
    if len(items) == len(selected):
        manifest = make_ci_fixture_manifest(items, lock)
        path = destination / 'manifest.json'
        # The fresh private directory is not an active fixture. Publishing its
        # final manifest is the commit point, after all cleanup has succeeded.
        with path.open('x') as output:
            json.dump(manifest, output, indent=2)
            output.write('\n')
            output.flush()
            os.fsync(output.fileno())
        path.chmod(0o400)
        directory = os.open(destination, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    return rows
