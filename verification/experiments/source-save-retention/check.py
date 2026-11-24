"""Check the full conditional proof closure; public acceptance remains separate."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys

import inputs as v
from prepare import ROOTS, materialize


def run(argv, project, log):
    with log.open('xb') as stream:
        result = subprocess.run(list(map(str, argv)), cwd=project,
                                stdout=stream, stderr=subprocess.STDOUT, timeout=3700)
    if result.returncode != 0:
        raise ValueError('the complete proof command failed; see its preserved log')
    return dict(exit_code=result.returncode, log_sha256=v.sha(log))


def dependencies(packages):
    values = {}
    for name in sorted(packages.iterdir()):
        if not name.is_dir():
            continue
        files = {}
        for directory, children, entries in os.walk(name):
            children[:] = sorted(child for child in children if child not in ('.git', '.lake'))
            for entry in sorted(entries):
                path = Path(directory) / entry
                if entry != '.git' and path.is_file():
                    files[str(path.relative_to(name))] = v.sha(path)
        values[name.name] = files
    return values


def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('proof-machine role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    args.output.mkdir(mode=0o700)
    report = dict(complete=False, conditional_formal_development=True, native_complete=False,
                  public_registration_complete=False, public_acceptance=False, views={})
    try:
        rules = {p.name: v.sha(p) for p in v.HERE.iterdir() if p.is_file()}
        report['rule_inputs'] = rules
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fixed = json.loads((v.ROOT / 'verification/foundations-environment.json').read_bytes())
            for name, path in (('charon', args.tools / 'charon/bin/charon'), ('aeneas', args.tools / 'bin/aeneas')):
                if v.sha(path) != fixed[name]['binary_sha256']:
                    raise ValueError('the original fixed extraction tool changed: ' + name)
            before = dependencies(args.lake_packages)
            report['dependencies'] = before
            diagnostic_lock = args.output / 'diagnostics.lock'
            diagnostic_lock.touch(mode=0o600)
            for mode, root in ROOTS.items():
                output = args.output / mode
                output.mkdir(mode=0o700)
                options = argparse.Namespace(**vars(args))
                options.mode, options.output = mode, output / 'project'
                materialize(options)
                plan = json.loads((options.output / 'plan.json').read_bytes())
                result = {'build': run(['lake', 'build', root], options.output, output / 'build.log')}
                diagnostics = output / 'diagnostics'
                result['diagnostics'] = run([sys.executable, v.ROOT / v.OLD_PACKAGE / 'diagnose.py',
                    '--project', options.output, '--output', diagnostics, '--lock', diagnostic_lock],
                    v.ROOT, output / 'diagnose.log')
                audit = json.loads((diagnostics / 'result.json').read_bytes())
                found = {item['message'].split(':', 1)[0].removeprefix('Checked ') for item in audit['audited']}
                if (audit['complete'] is not True or audit['warnings'] or audit['errors'] or audit['rejected']
                        or found != set(plan['audits']) or audit['physical_files'] != len(plan['files'])):
                    raise ValueError('every actual source file and every dependency audit must pass completely')
                options.output = output / 'reproduced'
                materialize(options)
                if (options.output / 'plan.json').read_bytes() != (output / 'project/plan.json').read_bytes():
                    raise ValueError('the complete final proof closure must reproduce byte for byte')
                result.update(complete=True, actual_files=len(plan['files']), unique_audits=len(found),
                              matching_parent_declarations=plan['matching_parent_declarations'])
                report['views'][mode] = result
            if before != dependencies(args.lake_packages):
                raise ValueError('the actual fixed Lean dependencies changed during the check')
        if rules != {p.name: v.sha(p) for p in v.HERE.iterdir() if p.is_file()}:
            raise ValueError('the complete composition rules changed during the check')
        report['complete'] = True
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        v.write_json(args.output / 'result.json', report)
    print(json.dumps({key: value for key, value in report.items() if key != 'dependencies'}, indent=2), flush=True)
    return not report['complete']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('captures', 'output', 'tools', 'lake-packages', 'lean-mcp', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for name, value in vars(args).items():
        setattr(args, name, value.resolve(strict=name != 'output'))
    raise SystemExit(check(args))
