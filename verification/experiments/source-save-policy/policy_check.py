"""Audit both original save outcomes with the real validated maintenance reader."""

import argparse
import fcntl
from hashlib import sha256
import json
import os
from pathlib import Path
import re
import sys
import tarfile
import tempfile

from policy_inputs import (CAPACITY, CAPACITY_PROVIDER, HERE, MODES, NAMESPACE, POLICY_PROVIDER,
                           POLICY_SOURCE, ROOT, SOURCE_FILES, configuration, extend_configuration,
                           retention, row_provider, stage)
from policy_capture import admit
from policy_prepare import declarations, materialize, read_exact
from capacity_check import verify_artifacts
from check import dependencies, run

CAPACITY_NAMESPACE = 'SandproofSaveCapacityJoin'
POLICY_NAMESPACE = 'SandproofSourcePolicyRead'
POLICY_DRIVER = retention.OLD_PACKAGE / 'source-policy-read/extract.py'
ARRAY_ORDER = 'Array.Insts.CoreCmpPartialOrdArray'
MUTATION = 'controller.checkpoint.config.read_using'


def checked_diagnostics(plan, report):
    found = {entry['message'].split(':', 1)[0].removeprefix('Checked ') for entry in report['audited']}
    if (report['complete'] is not True or report['errors'] or report['warnings'] or report['rejected']
            or len(plan['files']) != report['physical_files'] or set(plan['audits']) != found):
        raise ValueError('the complete Lean diagnostics and audit names must match the project')


def checked_capacity(output):
    receipt = json.loads((output / 'result.json').read_bytes())
    rules = {p.name: retention.sha(p) for p in CAPACITY.iterdir() if p.is_file()}
    for name, digest in rules.items():
        original = retention.pinned(CAPACITY_PROVIDER, CAPACITY.relative_to(ROOT) / name)
        if sha256(original).hexdigest() != digest:
            raise ValueError('the #1036 parent rules changed: ' + name)
    if (receipt['complete'] is not True or receipt['rule_inputs'] != rules
            or any(view['complete'] is not True for view in receipt['views'].values())
            or set(receipt['views']) != {'correct', 'keep-one-position'}):
        raise ValueError('the complete exact #1036 parent closure is required')
    root = output / 'correct'
    project = root / 'project'
    checked_diagnostics(json.loads((project / 'plan.json').read_bytes()),
                        json.loads((root / 'diagnostics/result.json').read_bytes()))
    return project


def checked_policy(output, audit, capture, row):
    receipt = json.loads((capture / 'result.json').read_bytes())
    driver = retention.pinned(POLICY_PROVIDER, POLICY_DRIVER)
    if (receipt['complete'] is not True or receipt['variant'] != 'original' or receipt['source'] != POLICY_SOURCE
            or receipt['driver'] != sha256(driver).hexdigest()
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the complete pinned #990 capture is required')
    verify_artifacts(capture, receipt, POLICY_NAMESPACE)
    config = receipt['options']
    if configuration(row) != extend_configuration(row, config['retained_roots'],
            [name for name in config['include'] if name not in config['roots']]):
        raise ValueError('current extraction must retain the exact original relevant #990 selection')
    plan = json.loads((output / 'plan.json').read_bytes())
    if (plan['root'], plan['variant'], plan['source'], plan['actual'], plan['reference']) != (
            'SourcePolicyRead', 'original', POLICY_SOURCE, receipt, receipt):
        raise ValueError('the #990 proof must use the exact checked original capture')
    for name, digest in plan['historical_proofs'].items():
        if sha256(retention.pinned(POLICY_PROVIDER, Path(name))).hexdigest() != digest:
            raise ValueError('the inherited Lean proof fragment has changed: ' + name)
    plan['audits'] = sorted({symbol for file in plan['files'] for symbol in re.findall(
        rb'^#audit_controller_proof (\S+)$', read_exact(output, plan, file), re.M)})
    plan['audits'] = [symbol.decode() for symbol in plan['audits']]
    checked_diagnostics(plan, json.loads(audit.read_bytes()))
    return capture / 'generated' / POLICY_NAMESPACE


def current_view(folder, mode, row, target):
    receipt = json.loads((folder / 'result.json').read_bytes())
    drivers = {name: retention.sha(HERE / name) for name in ('policy_capture.py', 'policy_inputs.py')}
    if (receipt['complete'] is not True or receipt['public_acceptance'] is not False
            or (receipt['mode'], receipt['source'], receipt['configuration'], receipt['driver'])
            != (mode, retention.SOURCE, configuration(row), drivers)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the complete exact current policy capture is required')
    verify_artifacts(folder, receipt, NAMESPACE)
    if retention.sha(folder / 'source.tar') != retention.SOURCE_ARCHIVE:
        raise ValueError('policy production archive differs')
    with tempfile.TemporaryDirectory(dir=folder.parent, prefix='policy-replay-') as directory:
        expected = Path(directory)
        with tarfile.open(folder / 'source.tar') as archive:
            archive.extractall(expected, filter='data')
        stage(row, expected, mode)
        actual = folder / 'source'
        digests = {str(name): retention.sha(actual / name) for name in SOURCE_FILES}
        if digests != receipt['rust']:
            raise ValueError('the staged form of the selected policy Rust inputs differs')
        names = {path.relative_to(expected) for path in expected.rglob('*') if path.is_file()}
        if names != {path.relative_to(actual) for path in actual.rglob('*') if path.is_file()}:
            raise ValueError('source inventory changed')
        for name in names:
            if (actual / name).is_symlink() or retention.sha(actual / name) != retention.sha(expected / name):
                raise ValueError('actual Rust source changed: ' + str(name))
    if admit(row, folder / 'actual.llbc', folder / 'generated', target) != receipt['admission']:
        raise ValueError('source admission changed')
    return folder / 'generated' / NAMESPACE


def correspondence(correct, mutant, capacity):
    result = {}
    for filename in ('Types.lean', 'Funs.lean'):
        good, bad, parent = (declarations(path / filename) for path in (correct, mutant, capacity))
        differences = {name for name, value in parent.items() if value != good.get(name)}
        changed = {name for name in good if good[name] != bad.get(name)}
        if good.keys() != bad.keys():
            raise ValueError('positive and negative policy declarations must have the same inventory')
        if filename == 'Types.lean':
            if differences or changed:
                raise ValueError('the actual parent and both policy views must agree on types')
        else:
            extra_slots = ('  lt := Array.Insts.CoreCmpPartialOrdArray.lt cmpPartialOrdInst\n'
                           '  le := Array.Insts.CoreCmpPartialOrdArray.le cmpPartialOrdInst\n')
            if (differences != {ARRAY_ORDER}
                    or good[ARRAY_ORDER].count(extra_slots) != 1
                    or good[ARRAY_ORDER].replace(extra_slots, '', 1) != parent[ARRAY_ORDER]):
                raise ValueError('only the two now-materialized Array comparison methods may extend the parent')
            if changed != {MUTATION}:
                raise ValueError('the real mutation may change only the decoded-policy validation path')
        result[filename] = dict(parent=len(parent), enlarged=len(good), changed_parent=sorted(differences),
                                changed_mutant=sorted(changed))
    return result


def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('proof-machine role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    args.output.mkdir(mode=0o700)
    rules = {item.name: retention.sha(item) for item in HERE.iterdir() if item.is_file()}
    report = dict(complete=False, conditional_formal_development=True, native_complete=False,
                  public_acceptance=False, rule_inputs=rules)
    try:
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
            for name, path in (('charon', args.tools / 'charon/bin/charon'), ('aeneas', args.tools / 'bin/aeneas')):
                if retention.sha(path) != fixed[name]['binary_sha256']:
                    raise ValueError('registered binary differs: ' + name)
            base = checked_capacity(args.capacity_output)
            packages = (base / '.lake/packages').resolve(strict=True)
            before = dependencies(packages)
            row = row_provider(args.output)
            prior = json.loads((args.capacity_capture / 'result.json').read_bytes())
            if prior['complete'] is not True or prior['mode'] != 'correct' or prior['source'] != retention.SOURCE:
                raise ValueError('the original #1036 capture is required')
            verify_artifacts(args.capacity_capture, prior, CAPACITY_NAMESPACE)
            original = checked_policy(args.policy_project, args.policy_audit, args.policy_capture, row)
            good, bad = (current_view(args.captures / mode, mode, row, fixed['target']) for mode in MODES)
            report['matching_definitions'] = correspondence(good, bad, args.capacity_capture / 'generated' / CAPACITY_NAMESPACE)
            report['dependencies'] = before
            project = args.output / 'project'
            plan = materialize(project, base, args.policy_project, good, bad, original)
            print(plan['root'], len(plan['files']), 'real files;', len(plan['audits']), 'audits', flush=True)
            report['build'] = run(['lake', 'build', plan['root']], project, args.output / 'build.log')
            lock = args.output / 'diagnostics.lock'
            lock.touch(mode=0o600)
            diag = args.output / 'diagnostics'
            report['diagnostics'] = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                '--project', project, '--output', diag, '--lock', lock], ROOT, args.output / 'diagnose.log')
            checked_diagnostics(plan, json.loads((diag / 'result.json').read_bytes()))
            if plan != materialize(args.output / 'reproduced', base, args.policy_project, good, bad, original):
                raise ValueError('the complete read/save closure did not reproduce')
            if before != dependencies(packages):
                raise ValueError('fixed Lean dependencies changed')
        if rules != {item.name: retention.sha(item) for item in HERE.iterdir() if item.is_file()}:
            raise ValueError('proof rules changed during the check')
        report.update(complete=True, actual_files=len(plan['files']), unique_audits=len(plan['audits']))
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        retention.write_json(args.output / 'result.json', report)
    print(json.dumps({key: value for key, value in report.items() if key not in ('dependencies', 'rule_inputs')}, indent=2), flush=True)
    return not report['complete']


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('captures', 'capacity-capture', 'capacity-output', 'policy-project', 'policy-capture',
                 'policy-audit', 'output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    for name, path in vars(args).items():
        setattr(args, name, path.resolve(strict=name != 'output'))
    raise SystemExit(check(args))
