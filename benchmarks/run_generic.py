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
from benchmarks.run_local import local_url, run_task
from benchmarks.oracle_repairs import REPAIRS
from benchmarks.environment_repairs import PRUNED_SOURCE, SOURCE_KINDS, validate_source_kind
from tools.benchmark_tasks import InvalidInput, verify


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
    def report():
        (args.output / 'full-set.json').write_text(json.dumps({
            'schema_version': 1, 'source': lock['source'], 'task_count': len(rows), 'tasks': rows,
            'source_kind': args.source_kind, 'oracle_mode': mode,
            'original_environment': args.source_kind != PRUNED_SOURCE,
            'failure_diagnostics': getattr(args, 'failure_diagnostics', False),
            'scope': ('generic templates; ' + mode + ' oracle and original verifier; '
                      'exact host cleanup requires external audit'),
            'dependencies': 'output image digest recorded; dynamic build/verifier downloads not fully locked',
        }, indent=2) + '\n')
    report()
    for name in sorted(selected):
        entry = entries[name]
        def factory(*, recipe, **kwargs):
            return GenericEnvironment(entry=entry, source=args.source, source_kind=args.source_kind,
                                      build_url=args.build_url, start_audit=getattr(args, 'start_audit', None), **kwargs)
        row = {'name': name, 'status': 'runner_error', 'oracle': 'not_run', 'verifier': 'not_run',
               'oracle_mode': mode, 'environment_source_kind': args.source_kind,
               'original_environment': args.source_kind != PRUNED_SOURCE,
               'exact_cleanup': 'unknown'}
        try:
            row = await run_task(args, entry, {}, environment_factory=factory)
        except asyncio.CancelledError:
            row.update(status='interrupted', interrupted=True)
            raise
        except Exception as error:
            # An unexpected runner error is not evidence of a pre-dispatch
            # refusal. Keep its identity/evidence and stop the suite for audit.
            row['error_type'] = type(error).__name__
        finally:
            # Cancellation must still retain this trial in the complete report.
            result = args.output / name / 'result.json'
            if result.exists():
                row = json.loads(result.read_text())
            rows = [row if r['name'] == name else r for r in rows]
            report()
            # Also runs while unwinding cancellation: keep the controllers
            # alive long enough to reconcile the original dispatched create.
            if getattr(args, 'completion_audit', None) is not None:
                try:
                    row.update(await asyncio.to_thread(args.completion_audit, row))
                except Exception as error:
                    row.update(status='cleanup_unknown', exact_cleanup='unknown',
                               audit_error_type=type(error).__name__)
                # Persist the external audit separately from the SDK receipt.
                rows = [row if r['name'] == name else r for r in rows]
                report()
                result.parent.mkdir(parents=True, exist_ok=True)
                result.write_text(json.dumps(row, indent=2) + '\n')
        rows = [row if r['name'] == name else r for r in rows]
        report()
        build_state = (row.get('build') or {}).get('state')
        recovered = (row.get('exact_cleanup') == 'passed'
                     and (row.get('runtime_recovery') or {}).get('status') == 'destroyed')
        if ((row['status'] in ('create_unknown', 'cleanup_unknown') and not recovered)
                or row['status'] == 'runner_error'
                or (row.get('build_dispatched') and build_state not in ('Ready', 'Failed'))):
            # Do not create more resources while the previous identity is uncertain.
            for remaining in rows:
                if remaining['name'] in selected and remaining['status'] == 'not_run':
                    remaining['reason'] = 'previous_identity_requires_audit'
            report()
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
