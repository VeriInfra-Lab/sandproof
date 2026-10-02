"""Run original Harbor oracle/verifier through the generic self-service chain.

SDK kill acknowledgment is reported separately from external exact-resource
audit. Original solutions by default; registered repairs require explicit opt-in.
No paid model, cloud E2B call or fixed-base recipe.
"""
import argparse
import asyncio
import json
import os
from pathlib import Path
import sys

from benchmarks.artifact_budget import add_artifact_arguments, artifact_limits_from_args
from benchmarks.check_harbor import check_runtime, check_tasks
from benchmarks.generic_environment import GenericEnvironment
from benchmarks.run_local import local_url, run_task, write_report
from benchmarks.oracle_repairs import REPAIRS
from benchmarks.environment_repairs import PRUNED_SOURCE, SOURCE_KINDS, validate_source_kind
from tools.benchmark_tasks import InvalidInput, verify
from tools.runtime_create_recovery import load_runtime_intent


def validate_oracle_mode(args, selected):
    mode = getattr(args, 'oracle_mode', 'original')
    if mode not in ('original', 'repaired'):
        raise InvalidInput('unknown oracle mode')
    if mode == 'repaired' and set(selected) - REPAIRS.keys():
        raise InvalidInput('repaired mode requires reviewed repairs for every selected task')
    return mode


async def run_selected(args, lock):
    selected = {t['name'] for t in lock['tasks']} if args.all else set(args.task)
    entries = {t['name']: t for t in lock['tasks']}
    if not selected or not selected <= entries.keys():
        raise InvalidInput('select task names from the complete locked task set')
    mode = validate_oracle_mode(args, selected)
    validate_source_kind(args.source_kind, [entries[name] for name in selected])
    rows = [{'name': t['name'], 'status': 'not_run',
             'reason': 'not_started' if t['name'] in selected else 'not_selected'} for t in lock['tasks']]
    def report(current=None):
        text = json.dumps({
            'schema_version': 1, 'source': lock['source'], 'task_count': len(rows), 'tasks': rows,
            'source_kind': args.source_kind, 'oracle_mode': mode,
            'original_environment': args.source_kind != PRUNED_SOURCE,
            'failure_diagnostics': getattr(args, 'failure_diagnostics', False),
            'scope': ('generic templates; ' + mode + ' oracle and original verifier; '
                      'exact host cleanup requires external audit'),
            'dependencies': 'output image digest recorded; dynamic build/verifier downloads not fully locked',
        }, indent=2) + '\n'
        if current is None:
            # Before dispatch there is nothing to reconcile if publication fails.
            (args.output / 'full-set.json').write_text(text)
        else:
            write_report(args.output / 'full-set.json', text, current)
    report()
    for name in sorted(selected):
        entry = entries[name]
        environment = None
        def factory(*, recipe, **kwargs):
            nonlocal environment
            environment = GenericEnvironment(entry=entry, source=args.source, source_kind=args.source_kind,
                build_url=args.build_url, start_audit=getattr(args, 'start_audit', None), **kwargs)
            return environment
        row = {'name': name, 'status': 'runner_error', 'oracle': 'not_run', 'verifier': 'not_run',
               'oracle_mode': mode, 'environment_source_kind': args.source_kind,
               'original_environment': args.source_kind != PRUNED_SOURCE,
               'exact_cleanup': 'unknown'}
        try:
            row = await run_task(args, entry, {}, environment_factory=factory, result_row=row)
        except asyncio.CancelledError:
            if row['status'] == 'runner_error':
                row['status'] = 'interrupted'
            row.setdefault('execution_status', 'interrupted')
            row['interrupted'] = True
            raise
        except Exception as error:
            # An unexpected runner error is not evidence of a pre-dispatch
            # refusal. Keep its identity/evidence and stop the suite for audit.
            row.setdefault('error_type', type(error).__name__)
            row['runner_error_type'] = type(error).__name__
            if row['status'] == 'passed':
                row.update(status='failed', reason='runner_finalization_failed')
        finally:
            # Use the live row, including on cancellation. A partial, stale or
            # unreadable result.json must never replace it or prevent cleanup.
            result = args.output / name / 'result.json'
            audit_cancelled = None
            if environment is not None and environment.create_dispatched:
                row.setdefault('create_dispatched', True)
                row.setdefault('sandbox_id', environment.sandbox_id)
                row.setdefault('kill_confirmed', environment.kill_confirmed)
                row.setdefault('build_dispatched', environment.build_dispatched)
                row.setdefault('build_id', environment.build_id)
                row.setdefault('build', environment.build_result)
                row.setdefault('template', environment.template)
                if not row.get('runtime_intent'):
                    try:
                        row['runtime_intent'] = load_runtime_intent(
                            result.parent, task=name, runtime_attempt=environment.runtime_attempt,
                            build_id=environment.build_id, template=environment.template)
                    except (OSError, ValueError, TypeError) as error:
                        row.setdefault('execution_status', row['status'])
                        row.update(status='cleanup_unknown', exact_cleanup='unknown',
                                   intent_error_type=type(error).__name__)
            # Also runs while unwinding cancellation: keep the controllers
            # alive long enough to reconcile the original dispatched create.
            if getattr(args, 'completion_audit', None) is not None:
                audit = asyncio.create_task(asyncio.to_thread(args.completion_audit, row))
                try:
                    while not audit.done():
                        try:
                            await asyncio.shield(audit)
                        except asyncio.CancelledError as error:
                            # The callback owns the same reconciliation, not a
                            # replacement create. Do not stop its controllers
                            # while the worker thread still needs them.
                            audit_cancelled = error
                            row.setdefault('execution_status', row['status'])
                            row['interrupted'] = True
                            if row['status'] == 'passed':
                                row['status'] = 'interrupted'
                    row.update(audit.result())
                except Exception as error:
                    row.setdefault('execution_status', row['status'])
                    row.update(status='cleanup_unknown', exact_cleanup='unknown',
                               audit_error_type=type(error).__name__)
            # Only after reconciliation attempt may reporting fail. Keep those
            # errors in the same live row and do not dispatch a later task.
            rows = [row if r['name'] == name else r for r in rows]
            report(row)
            try:
                result.parent.mkdir(parents=True, exist_ok=True)
            except OSError as error:
                row.setdefault('report_errors', {})['trial_directory'] = {
                    'error_type': type(error).__name__, 'errno': error.errno}
                if row['status'] == 'passed':
                    row['status'] = 'failed'
            write_report(result, json.dumps(row, indent=2) + '\n', row)
            report(row)
            if audit_cancelled is not None:
                raise audit_cancelled
        rows = [row if r['name'] == name else r for r in rows]
        build_state = (row.get('build') or {}).get('state')
        recovered = (row.get('exact_cleanup') == 'passed'
                     and (row.get('runtime_recovery') or {}).get('status') == 'destroyed')
        if (row.get('report_errors') or row.get('intent_error_type') or row.get('runner_error_type')
                or (row['status'] in ('create_unknown', 'cleanup_unknown') and not recovered)
                or row['status'] == 'runner_error'
                or (row.get('build_dispatched') and build_state not in ('Ready', 'Failed'))):
            # Do not create more resources while the previous identity is uncertain.
            for remaining in rows:
                if remaining['name'] in selected and remaining['status'] == 'not_run':
                    remaining['reason'] = 'previous_identity_requires_audit'
            report(row)
            break
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('registry', 'source', 'lock', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--build-url', required=True)
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument('--task', action='append')
    selection.add_argument('--all', action='store_true')
    # Original sources remain the default. A reviewed source variant requires
    # explicit selection and is never reported as the original environment.
    parser.add_argument('--source-kind', choices=SOURCE_KINDS, default='oci')
    parser.add_argument('--oracle-mode', choices=('original', 'repaired'), default='original')
    parser.add_argument('--failure-diagnostics', action='store_true',
                        help='after an original failed score, run registered guest diagnostics separately')
    add_artifact_arguments(parser)
    args = parser.parse_args()
    try:
        args.artifact_limits = artifact_limits_from_args(args)
        local_url(args.api_url)
        local_url(args.build_url)
        if not os.environ.get('SANDPROOF_TEST_API_KEY'):
            raise InvalidInput('SANDPROOF_TEST_API_KEY required')
        runtimes = check_runtime()
        lock = verify(json.loads(args.lock.read_text()), args.registry, args.source)
        check_tasks(lock, args.source)
        validate_oracle_mode(args, {t['name'] for t in lock['tasks']} if args.all else args.task)
        selected = {t['name'] for t in lock['tasks']} if args.all else set(args.task)
        validate_source_kind(args.source_kind, [t for t in lock['tasks'] if t['name'] in selected])
        args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
        (args.output / 'inputs.json').write_text(json.dumps({
            'source': lock['source'], 'runtimes': runtimes, 'source_kind': args.source_kind,
            'oracle_mode': args.oracle_mode,
            'original_environment': args.source_kind != PRUNED_SOURCE,
            'failure_diagnostics': args.failure_diagnostics,
            'artifact_limits': args.artifact_limits,
            'task_files': {t['name']: t['files_sha256'] for t in lock['tasks']},
            'original_harbor_template_preference': 'public OCI when docker_image is set',
        }, indent=2) + '\n')
        rows = asyncio.run(run_selected(args, lock))
        ran = [r for r in rows if r['status'] != 'not_run']
        # Zero means execution checks only. The report explicitly does not
        # claim an external resource audit or all-set acceptance.
        return 0 if ran and all(r['status'] == 'passed' for r in ran) else 1
    except (InvalidInput, OSError, ValueError, KeyError) as error:
        print(f'Generic benchmark rejected: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
