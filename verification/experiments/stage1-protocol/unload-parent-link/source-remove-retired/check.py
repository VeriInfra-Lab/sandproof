"""Check both complete development projects; never issue public/native acceptance."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

import extract
import inputs

HERE = Path(__file__).resolve().parent
INPUTS = {'correct', 'wrong', 'reads', 'reads_audit', 'retention', 'retention_audit'}
_prepare_spec = importlib.util.spec_from_file_location('source_remove_retired_prepare', HERE / 'prepare.py')
prepare = importlib.util.module_from_spec(_prepare_spec)
_prepare_spec.loader.exec_module(prepare)


def command(argv, cwd, log, timeout):
    started = time.monotonic()
    bounded_stop = False
    with log.open('x') as stream:
        child = subprocess.Popen(argv, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT,
                                 text=True, start_new_session=True)
        try:
            child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            bounded_stop = True
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
    record = dict(argv=[str(a) for a in argv], seconds=time.monotonic() - started,
                  exit_code=child.returncode, bounded_stop=bounded_stop, log_sha256=inputs.digest(log))
    return record


def completed(report, argv, cwd, log, timeout):
    record = command(argv, cwd, log, timeout)
    report['commands'].append(record)
    if record['exit_code'] or record['bounded_stop']:
        raise ValueError('complete development command failed: ' + str(log))


def audit(project, directory, view):
    plan = json.loads((project / 'plan.json').read_bytes())
    result = json.loads((directory / 'result.json').read_bytes())
    expected = set(plan['audits'])
    actual = []
    for item in result.get('audited', []):
        match = re.fullmatch(r'Checked ([\w.]+): \d+ standard logical dependencies', item['message'])
        if not match:
            raise ValueError('an original theorem-audit message is required')
        actual.append(match[1])
    if (result['complete'] is not True or any(result.get(k) for k in
            ('warnings', 'errors', 'own_warnings', 'rejected'))
            or result['source_sha256'] != plan['files']
            or result['physical_files'] != view['physical_files']
            or len(actual) != view['full_audits'] or len(actual) != len(set(actual))
            or set(actual) != expected or not set(view['own_audits']) <= expected):
        raise ValueError('all actual files, full audits and current properties must pass')
    for name, wanted in plan['files'].items():
        if inputs.digest(project / name) != wanted:
            raise ValueError('actual project changed after full diagnostics: ' + name)
    if inputs.digest(directory / 'service.log') != result['service_log_sha256']:
        raise ValueError('the complete original service log changed')
    return dict(physical_files=result['physical_files'], full_audits=len(actual),
                plan=inputs.digest(project / 'plan.json'), result=inputs.digest(directory / 'result.json'),
                service=result['service_log_sha256'])


def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('the original formal machine role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    locations_bytes = args.inputs.read_bytes()
    locations = json.loads(locations_bytes)
    if set(locations) != INPUTS or not all(isinstance(p, str) for p in locations.values()):
        raise ValueError('all six original read-only input locations are required')
    locations = {name: Path(path).resolve(strict=True) for name, path in locations.items()}
    registration = json.loads((HERE / 'registration-candidate.json').read_bytes())
    views = registration['formal_views']
    if [v['variant'] for v in views] != list(extract.VARIANTS):
        raise ValueError('both original and actual wrong views are mandatory')
    args.output.mkdir(mode=0o700)
    private_lock = args.output / 'diagnostics.lock'
    private_lock.touch(mode=0o600)
    report = dict(development_only=True, public_acceptance=False, native_complete=False,
                  complete=False, source=extract.SOURCE, tree=extract.TREE, variants=[], commands=[])
    started = time.monotonic()
    try:
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            report['dependencies'] = inputs.dependencies(args.lake_packages, extract.REPO / 'lake-manifest.json')
            cache = locations['reads']
            for view in views:
                variant = view['variant']
                project = args.output / (variant + '-project')
                options = argparse.Namespace(variant=variant,
                    actual=locations['correct' if variant == 'original' else 'wrong'],
                    reference=locations['correct'], reads=locations['reads'], reads_audit=locations['reads_audit'],
                    retention=locations['retention'], retention_audit=locations['retention_audit'],
                    output=project, lake_packages=args.lake_packages, lean_mcp=args.lean_mcp)
                prepare.prepare(options)
                print(variant, 'complete project materialized', flush=True)
                if (cache / '.lake/build').is_dir():
                    subprocess.run(['cp', '-a', '--reflink=auto', str(cache / '.lake/build'),
                                    str(project / '.lake/build')], check=True)
                plan = json.loads((project / 'plan.json').read_bytes())
                if (plan['root'], len(plan['files']), len(plan['audits'])) != (
                        view['formal_root'], view['physical_files'], view['full_audits']):
                    raise ValueError('the registered complete current project is required')
                completed(report, ['lake', 'build', plan['root']], project,
                    args.output / (variant + '-build.log'), 1800)
                print(variant, 'full build passed; checking every actual file', flush=True)
                diagnosed = args.output / (variant + '-audit')
                completed(report, [
                    sys.executable, str(HERE.parent / 'diagnose.py'), '--project', str(project),
                    '--output', str(diagnosed), '--lock', str(private_lock)], extract.REPO,
                    args.output / (variant + '-diagnostics.log'), 3700)
                result = audit(project, diagnosed, view)
                fresh = args.output / (variant + '-reproduced')
                options.output = fresh
                prepare.prepare(options)
                if (fresh / 'plan.json').read_bytes() != (project / 'plan.json').read_bytes():
                    raise ValueError('the entire final plan must reproduce byte for byte')
                for name in (*plan['files'], *inputs.SUPPORT, 'lakefile.toml'):
                    if (fresh / name).read_bytes() != (project / name).read_bytes():
                        raise ValueError('the full final actual-file closure must reproduce: ' + name)
                result.update(variant=variant, fresh_plan_reproduced=True)
                report['variants'].append(result)
                cache = project
                print(variant, result['physical_files'], 'actual files,', result['full_audits'],
                      'unique audits and fresh reproduction passed', flush=True)
            if inputs.dependencies(args.lake_packages, extract.REPO / 'lake-manifest.json') != report['dependencies']:
                raise ValueError('fixed Lean dependencies changed during the check')
        if args.inputs.read_bytes() != locations_bytes:
            raise ValueError('original source locations changed during the check')
        report['complete'] = True
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        report['seconds'] = time.monotonic() - started
        report['inputs_sha256'] = inputs.digest(args.inputs)
        (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2), flush=True)
    return not report['complete']


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('inputs', 'output', 'lock', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    for name, path in vars(args).items():
        setattr(args, name, path.resolve(strict=name != 'output'))
    raise SystemExit(check(args))
