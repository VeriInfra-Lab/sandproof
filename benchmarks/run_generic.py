"""Run original Harbor oracle/verifier through the generic self-service chain.

SDK kill acknowledgment is reported separately from external exact-resource
audit. No paid model, cloud E2B call, fixed-base recipe or oracle repair.
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
from tools.benchmark_tasks import InvalidInput, verify


async def run_selected(args, lock):
    selected = {t['name'] for t in lock['tasks']} if args.all else set(args.task)
    entries = {t['name']: t for t in lock['tasks']}
    if not selected or not selected <= entries.keys():
        raise InvalidInput('select task names from the complete locked task set')
    rows = [{'name': t['name'], 'status': 'not_run',
             'reason': 'not_started' if t['name'] in selected else 'not_selected'} for t in lock['tasks']]
    def report():
        (args.output / 'full-set.json').write_text(json.dumps({
            'schema_version': 1, 'source': lock['source'], 'task_count': len(rows), 'tasks': rows,
            'source_kind': args.source_kind, 'oracle_mode': 'original',
            'scope': 'generic templates and original Harbor oracle/verifier; exact host cleanup requires external audit',
            'dependencies': 'output image digest recorded; dynamic build/verifier downloads not fully locked',
        }, indent=2) + '\n')
    report()
    for name in sorted(selected):
        entry = entries[name]
        def factory(*, recipe, **kwargs):
            return GenericEnvironment(entry=entry, source=args.source, source_kind=args.source_kind,
                                      build_url=args.build_url, start_audit=getattr(args, 'start_audit', None), **kwargs)
        row = {'name': name, 'status': 'runner_error', 'oracle': 'not_run', 'verifier': 'not_run',
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
        if getattr(args, 'completion_audit', None) is not None:
            try:
                row.update(await asyncio.to_thread(args.completion_audit, row))
            except Exception as error:
                row.update(status='cleanup_unknown', exact_cleanup='unknown',
                           audit_error_type=type(error).__name__)
            # Persist the external audit separately from the SDK kill receipt.
            rows = [row if r['name'] == name else r for r in rows]
            report()
            result.parent.mkdir(parents=True, exist_ok=True)
            result.write_text(json.dumps(row, indent=2) + '\n')
        rows = [row if r['name'] == name else r for r in rows]
        report()
        build_state = (row.get('build') or {}).get('state')
        if (row['status'] in ('create_unknown', 'cleanup_unknown', 'runner_error')
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
    # Match fixed Harbor's OCI preference by default; the Dockerfile path is
    # an explicit ORIGINAL source alternative, never a rewritten task recipe.
    parser.add_argument('--source-kind', choices=('oci', 'dockerfile'), default='oci')
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
        args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
        (args.output / 'inputs.json').write_text(json.dumps({
            'source': lock['source'], 'runtimes': runtimes, 'source_kind': args.source_kind,
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
