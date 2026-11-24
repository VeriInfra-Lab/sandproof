"""Audit the same complete encoder contract over both actual captures."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import sys

from encoder_inputs import HERE, MODES, ROOT, retention, row_provider
from encoder_prepare import checked, correspondence, materialize
import joint_prepare as parent
from check import dependencies, run


def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('proof-machine role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    args.output.mkdir(mode=0o700)
    report = dict(complete=False, conditional_formal_development=True, native_complete=False, public_acceptance=False, views={})
    rules = {item.name: retention.sha(item) for item in HERE.iterdir() if item.is_file()}
    report['rule_inputs'] = rules
    try:
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
            for name, path in (('charon', args.tools / 'charon/bin/charon'), ('aeneas', args.tools / 'bin/aeneas')):
                if retention.sha(path) != fixed[name]['binary_sha256']:
                    raise ValueError('fixed tool differs: ' + name)
            before = dependencies(args.lake_packages)
            report['dependencies'] = before
            diagnostic_lock = args.output / 'diagnostics.lock'
            diagnostic_lock.touch(mode=0o600)
            row = row_provider(args.output)
            import prepare_source_row_sql_composition as original
            parent_views = {mode: parent.checked(args.parent_captures / mode, mode, row) for mode in parent.MODES}
            reference = original.captured(args.row_reference, 'core')
            parent_matches = parent.correspondence(parent_views, reference)
            row_model = args.output / 'row-model'
            original.materialize('core', args.row_reference, args.row_reference, row_model,
                                 args.lake_packages, args.lean_mcp)
            parent_model = args.output / 'parent-model'
            parent.materialize(parent_model, 'correct', row_model, parent_views['correct'],
                               args.lake_packages, args.lean_mcp, parent_matches)
            views = {mode: checked(args.captures / mode, mode, row) for mode in MODES}
            matches = correspondence(views['correct'], views['zero-row-key'], parent_views['parent'])
            for mode in MODES:
                out = args.output / mode
                out.mkdir(mode=0o700)
                project = out / 'project'
                plan = materialize(project, views[mode], parent_model, mode)
                print(mode, len(plan['files']), 'real Lean files;', len(plan['audits']), 'unique audits', flush=True)
                result = {'build': run(['lake', 'build', plan['root']], project, out / 'build.log')}
                diagnostics = out / 'diagnostics'
                result['diagnostics'] = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                    '--project', project, '--output', diagnostics, '--lock', diagnostic_lock], ROOT, out / 'diagnose.log')
                audit = json.loads((diagnostics / 'result.json').read_bytes())
                found = {entry['message'].split(':', 1)[0].removeprefix('Checked ') for entry in audit['audited']}
                if (audit['complete'] is not True or audit['warnings'] or audit['errors'] or audit['rejected']
                        or found != set(plan['audits']) or audit['physical_files'] != len(plan['files'])):
                    raise ValueError('every actual source file and audit must pass completely')
                repeated = materialize(out / 'reproduced', views[mode], parent_model, mode)
                if repeated != plan:
                    raise ValueError('the entire final closure must reproduce')
                result.update(actual_files=len(plan['files']), unique_audits=len(found), matching_definitions=matches, complete=True)
                report['views'][mode] = result
            if before != dependencies(args.lake_packages):
                raise ValueError('fixed Lean dependencies changed during the proof')
        if rules != {item.name: retention.sha(item) for item in HERE.iterdir() if item.is_file()}:
            raise ValueError('composition inputs changed during the check')
        report['complete'] = True
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        retention.write_json(args.output / 'result.json', report)
    print(json.dumps({k: v for k, v in report.items() if k not in ('dependencies', 'rule_inputs')}, indent=2), flush=True)
    return not report['complete']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('captures', 'parent-captures', 'row-reference', 'output', 'tools', 'lake-packages', 'lean-mcp', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for name, value in vars(args).items():
        setattr(args, name, value.resolve(strict=name != 'output'))
    raise SystemExit(check(args))
