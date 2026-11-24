"""Replay the exact capacity captures and audit both original save outcomes."""

import argparse
import fcntl
from hashlib import sha256
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile

from capacity_capture import admit
from capacity_inputs import CODEC, COUNT_PROVIDER, ENCODER, HERE, MODES, NAMESPACE, ROOT, configuration, retention, row_provider, stage
from capacity_prepare import declarations, materialize
from check import dependencies, run
from encoder_prepare import checked as checked_encoder

ALIGNED = ('Types.lean', 'Funs.lean')
ENCODER_PROVIDER = 'dd82c2cdf2921bfdc04f11d032a2fcddd5771132'
COUNT_SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
COUNT_NAMESPACE = 'SandproofSourceRecordBounds'
ENCODER_NAMESPACE = 'SandproofSaveEncoderJoin'
CAPACITY_TYPES = ('controller.capacity.CapacityConfig', 'controller.checkpoint.config.MaintenanceConfig')
CAPACITY_FUNCTIONS = tuple(CODEC + name for name in ('records_per_session', 'reserved_records'))


def captures(folder, mode, row, target):
    receipt = json.loads((folder / 'result.json').read_bytes())
    drivers = {name: retention.sha(HERE / name) for name in ('capacity_capture.py', 'capacity_inputs.py')}
    if (receipt['complete'] is not True or receipt['public_acceptance'] is not False
            or (receipt['mode'], receipt['source'], receipt['configuration'], receipt['driver'])
            != (mode, retention.SOURCE, configuration(row), drivers)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('complete exact current capture required: ' + mode)
    verify_artifacts(folder, receipt, NAMESPACE)
    if retention.sha(folder / 'source.tar') != retention.SOURCE_ARCHIVE:
        raise ValueError('the original production archive differs')
    with tempfile.TemporaryDirectory(dir=folder.parent, prefix='capacity-verify-') as directory:
        expected = Path(directory)
        with tarfile.open(folder / 'source.tar') as archive:
            archive.extractall(expected, filter='data')
        stage(row, expected, mode)
        actual = folder / 'source'
        names = {p.relative_to(expected) for p in expected.rglob('*') if p.is_file()}
        if names != {p.relative_to(actual) for p in actual.rglob('*') if p.is_file()}:
            raise ValueError('actual capacity source inventory changed')
        for name in names:
            if (actual / name).is_symlink() or retention.sha(actual / name) != retention.sha(expected / name):
                raise ValueError('actual capacity source changed: ' + str(name))
    if admit(row, folder / 'actual.llbc', folder / 'generated', target) != receipt['admission']:
        raise ValueError('current source count admission changed')
    return folder / 'generated' / NAMESPACE


def verify_artifacts(folder, receipt, namespace):
    for path, digest in [('actual.llbc', receipt['ir']), *[
            (item['label'] + '.log', item['log_sha256']) for item in receipt['commands']]]:
        if retention.sha(folder / path) != digest:
            raise ValueError('fixed extraction evidence changed: ' + path)
    generated = folder / 'generated'
    names = {'translation.json', *(f'{namespace}/{name}' for name in
        (*ALIGNED, 'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    found = {str(p.relative_to(generated)): retention.sha(p) for p in generated.rglob('*') if p.is_file()}
    if names != found.keys() or found != receipt['generated']:
        raise ValueError('the generated capture differs from its actual extraction')


def count_reference(folder):
    receipt = json.loads((folder / 'result.json').read_bytes())
    driver = retention.pinned(COUNT_PROVIDER, retention.OLD_PACKAGE / 'source-record-count/extract.py')
    if (receipt['complete'] is not True or receipt['variant'] != 'original' or receipt['source'] != COUNT_SOURCE
            or receipt['driver'] != sha256(driver).hexdigest()
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the complete pinned #989 original capture is required')
    verify_artifacts(folder, receipt, COUNT_NAMESPACE)
    return folder / 'generated' / COUNT_NAMESPACE


def checked_parent(encoder_output):
    receipt = json.loads((encoder_output / 'result.json').read_bytes())
    rules = {p.name: retention.sha(p) for p in ENCODER.iterdir() if p.is_file()}
    for name, digest in rules.items():
        original = retention.pinned(ENCODER_PROVIDER, ENCODER.relative_to(ROOT) / name)
        if sha256(original).hexdigest() != digest:
            raise ValueError('the #1035 parent rules changed: ' + name)
    if (receipt['complete'] is not True or receipt['rule_inputs'] != rules
            or any(view['complete'] is not True for view in receipt['views'].values())
            or set(receipt['views']) != {'correct', 'zero-row-key'}):
        raise ValueError('the complete exact #1035 parent closure is required')
    project = encoder_output / 'correct' / 'project'
    diag = json.loads((encoder_output / 'correct' / 'diagnostics' / 'result.json').read_bytes())
    plan = json.loads((project / 'plan.json').read_bytes())
    checked = {entry['message'].split(':', 1)[0].removeprefix('Checked ') for entry in diag['audited']}
    if (diag['complete'] is not True or diag['errors'] or diag['warnings'] or diag['rejected']
            or diag['physical_files'] != len(plan['files']) or checked != set(plan['audits'])):
        raise ValueError('the actual parent plan and its complete diagnostic run disagree')
    return project


def correspondence(correct, mutant, encoder, counting):
    result = {}
    for file, count_symbols in zip(ALIGNED, (CAPACITY_TYPES, CAPACITY_FUNCTIONS), strict=True):
        good, bad, parent, provider = (declarations(folder / file) for folder in (correct, mutant, encoder, counting))
        if set(parent) | set(count_symbols) != good.keys() or good.keys() != bad.keys():
            raise ValueError('unexpected generated capacity inventory: ' + file)
        if any(good.get(name) != body for name, body in parent.items()):
            raise ValueError('the exact #1035 save parent has changed: ' + file)
        if any(good[name] != provider.get(name) for name in count_symbols):
            raise ValueError('the #989 capacity provider declarations changed: ' + file)
        differences = {name for name in good if good[name] != bad[name]}
        if file == 'Types.lean':
            if differences:
                raise ValueError('the mutant must preserve every actual generated type')
        else:
            step = CAPACITY_FUNCTIONS[0]
            original = 'val1.max_operations_per_sandbox - 2#u64'
            if (differences != {step} or good[step].count(original) != 1
                    or good[step].replace(original, 'val1.max_operations_per_sandbox - 1#u64') != bad[step]):
                raise ValueError('the actual mutation must only reserve one position')
        result[file] = dict(parent=len(parent), capacity=len(count_symbols))
    return result


def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('proof-machine role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    args.output.mkdir(mode=0o700)
    rules = {p.name: retention.sha(p) for p in HERE.iterdir() if p.is_file()}
    report = dict(complete=False, conditional_formal_development=True, native_complete=False, public_acceptance=False,
                  rule_inputs=rules, views={})
    try:
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
            for name, path in (('charon', args.tools / 'charon/bin/charon'), ('aeneas', args.tools / 'bin/aeneas')):
                if retention.sha(path) != fixed[name]['binary_sha256']:
                    raise ValueError('registered binary differs: ' + name)
            parent = checked_parent(args.encoder_output)
            packages = (parent / '.lake/packages').resolve(strict=True)
            before = dependencies(packages)
            report['dependencies'] = before
            row = row_provider(args.output)
            checked_encoder(args.encoder_capture, 'correct', row)
            counting = count_reference(args.count_reference)
            views = {mode: captures(args.captures / mode, mode, row, fixed['target']) for mode in MODES}
            report['matching_definitions'] = correspondence(views['correct'], views['keep-one-position'],
                args.encoder_capture / 'generated' / ENCODER_NAMESPACE, counting)
            lock = args.output / 'diagnostics.lock'
            lock.touch(mode=0o600)
            for mode in MODES:
                out = args.output / mode
                out.mkdir(mode=0o700)
                project = out / 'project'
                plan = materialize(project, mode, parent, views[mode])
                print(mode, len(plan['files']), 'real Lean files;', len(plan['audits']), 'unique audits', flush=True)
                build = run(['lake', 'build', plan['root']], project, out / 'build.log')
                diagnostics = out / 'diagnostics'
                service = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                    '--project', project, '--output', diagnostics, '--lock', lock], ROOT, out / 'diagnose.log')
                actual = json.loads((diagnostics / 'result.json').read_bytes())
                audited = {entry['message'].split(':', 1)[0].removeprefix('Checked ') for entry in actual['audited']}
                if (actual['complete'] is not True or actual['errors'] or actual['warnings'] or actual['rejected']
                        or actual['physical_files'] != len(plan['files']) or audited != set(plan['audits'])):
                    raise ValueError('each admitted Lean file and requested audit must pass completely')
                if plan != materialize(out / 'reproduced', mode, parent, views[mode]):
                    raise ValueError('the complete proof closure must reproduce')
                report['views'][mode] = dict(build=build, diagnostics=service, actual_files=len(plan['files']),
                                             unique_audits=len(audited), complete=True)
            if before != dependencies(packages):
                raise ValueError('fixed Lean dependencies changed')
        if rules != {p.name: retention.sha(p) for p in HERE.iterdir() if p.is_file()}:
            raise ValueError('proof rules changed during the check')
        report['complete'] = True
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        retention.write_json(args.output / 'result.json', report)
    print(json.dumps({key: value for key, value in report.items() if key not in ('dependencies', 'rule_inputs')}, indent=2), flush=True)
    return not report['complete']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('captures', 'encoder-capture', 'encoder-output', 'count-reference', 'output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for name, value in vars(args).items():
        setattr(args, name, value.resolve(strict=name != 'output'))
    raise SystemExit(check(args))
