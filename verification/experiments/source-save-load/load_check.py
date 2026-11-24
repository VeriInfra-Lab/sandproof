"""Audit the joint source-save and real metadata-loader closure."""

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

from load_capture import admit
from load_inputs import (HERE, MODES, NAMESPACE, PARENT, POLICY, REFERENCE_NAMESPACE, REFERENCE_SOURCE,
                         ROOT, SOURCE_FILES, configuration, historical_source, retention,
                         row_provider, stage)
from load_prepare import CODEC, LOAD, declarations, materialize, read_exact
from policy_check import checked_diagnostics, current_view as policy_view
from capacity_check import verify_artifacts
from check import dependencies, run


def checked_parent(output):
    receipt = json.loads((output / 'result.json').read_bytes())
    rules = {file.name: retention.sha(file) for file in POLICY.iterdir() if file.is_file()}
    for file, digest in rules.items():
        if sha256(retention.pinned(PARENT, POLICY.relative_to(ROOT) / file)).hexdigest() != digest:
            raise ValueError('the parent rules differ at ' + file)
    if receipt['complete'] is not True or receipt['rule_inputs'] != rules:
        raise ValueError('the complete #1037 parent check is required')
    parent = output / 'project'
    checked_diagnostics(json.loads((parent / 'plan.json').read_bytes()),
                        json.loads((output / 'diagnostics/result.json').read_bytes()))
    return parent


def checked_reference(project, audit, receipt):
    plan = json.loads((project / 'plan.json').read_bytes())
    if (plan['root'], plan['variant'], plan['source'], plan['actual']) != (
            'SourceLoadErrors', 'original', REFERENCE_SOURCE, receipt):
        raise ValueError('the #984 proof must use the exact original source capture')
    plan['audits'] = sorted({symbol.decode() for file in plan['files'] for symbol in re.findall(
        rb'^#audit_controller_proof (\S+)\s*$', read_exact(project, plan, file), re.M)})
    checked_diagnostics(plan, json.loads(audit.read_bytes()))


def view(folder, mode, row, fixed, reference, parent_capture):
    report = json.loads((folder / 'result.json').read_bytes())
    cfg = configuration(row, reference)
    expected_rules = {name: retention.sha(HERE / name) for name in ('load_capture.py', 'load_inputs.py')}
    if (report['complete'] is not True or report['public_acceptance'] is not False
            or (report['mode'], report['source'], report['configuration'], report['rules']) != (
                mode, retention.SOURCE, cfg, expected_rules)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the complete current save/load capture is required')
    verify_artifacts(folder, report, NAMESPACE)
    archive = folder / 'source.tar'
    if retention.sha(archive) != retention.SOURCE_ARCHIVE:
        raise ValueError('the current Rust source archive differs')
    with tempfile.TemporaryDirectory(dir=folder.parent, prefix='load-replay-') as temporary:
        expected = Path(temporary)
        with tarfile.open(archive) as stream:
            stream.extractall(expected, filter='data')
        stage(row, expected, mode)
        actual = folder / 'source'
        rust = {str(name): retention.sha(actual / name) for name in SOURCE_FILES}
        if report['rust'] != rust:
            raise ValueError('selected Rust source differs')
        names = {path.relative_to(expected) for path in expected.rglob('*') if path.is_file()}
        if names != {path.relative_to(actual) for path in actual.rglob('*') if path.is_file()}:
            raise ValueError('staged Rust source inventory differs')
        for name in names:
            selected = actual / name
            if selected.is_symlink() or retention.sha(selected) != retention.sha(expected / name):
                raise ValueError('staged input differs: ' + str(name))
    for file in SOURCE_FILES:
        if file != retention.CODEC and (folder / 'source' / file).read_bytes() != (
                parent_capture / 'source' / file).read_bytes():
            raise ValueError('parent input differs outside the selected codec')
    if admit(row, folder / 'actual.llbc', folder / 'generated', fixed['target'], cfg) != report['admission']:
        raise ValueError('actual same-connection call admission differs')
    return folder / 'generated' / NAMESPACE


def correspondence(parent, good, bad, reference, negative):
    result = {}
    for filename in ('Types.lean', 'Funs.lean'):
        before, positive, mutant, old, old_mutant = (declarations(path / filename)
            for path in (parent, good, bad, reference, negative))
        if any(positive.get(name) != declaration for name, declaration in before.items()):
            raise ValueError('selected #1037 original declarations differ')
        changes = {name for name, declaration in positive.items() if mutant.get(name) != declaration}
        if positive.keys() != mutant.keys() or changes != ({LOAD} if filename == 'Funs.lean' else set()):
            raise ValueError('the early-count variant must change only the real metadata loader')
        exact = {name for name in old if name.startswith(tuple(CODEC + '.' + item for item in (
            'load', 'schema', 'checked_source_configuration', 'Configuration', 'METADATA_BYTES',
            'RECORDS_TABLE', 'TABLE', 'Sources'))) or name.startswith('capture.source.SourceObservationConfig.valid')}
        exact.difference_update({'controller.lifecycle_inbox.source_work.codec.load_records'})
        if (not exact or any(positive.get(name) != old[name] or mutant.get(name) != old_mutant[name]
                             for name in exact)):
            raise ValueError('the #984 real metadata and typed Rust declarations differ')
        result[filename] = dict(parent=len(before), shared_metadata=len(exact), mutant_changed=sorted(changes))
    return result


def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('proof-machine role required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    args.output.mkdir(mode=0o700)
    rules = {file.name: retention.sha(file) for file in HERE.iterdir() if file.is_file()}
    report = dict(complete=False, conditional_formal_development=True, native_complete=False,
                  public_acceptance=False, rule_inputs=rules)
    try:
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
            for name, path in (('charon', args.tools / 'charon/bin/charon'), ('aeneas', args.tools / 'bin/aeneas')):
                if retention.sha(path) != fixed[name]['binary_sha256']:
                    raise ValueError('registered extraction tool differs: ' + name)
            parent = checked_parent(args.parent_output)
            packages = (parent / '.lake/packages').resolve(strict=True)
            before = dependencies(packages)
            row = row_provider(args.output)
            parent_view = policy_view(args.parent_capture, 'correct', row, fixed['target'])
            old_positive, old_negative = (args.reference_captures / name for name in ('original', 'early-count-guard'))
            positive_receipt = historical_source(old_positive, 'original')
            historical_source(old_negative, 'early-count-guard')
            checked_reference(args.reference_project, args.reference_audit, positive_receipt)
            good, bad = (view(args.captures / mode, mode, row, fixed, old_positive, args.parent_capture)
                         for mode in MODES)
            report['matching_definitions'] = correspondence(parent_view, good, bad,
                old_positive / 'generated' / REFERENCE_NAMESPACE, old_negative / 'generated' / REFERENCE_NAMESPACE)
            project = args.output / 'project'
            plan = materialize(project, parent, args.reference_project, good, bad)
            report['dependencies'] = before
            print(plan['root'], len(plan['files']), 'real files;', len(plan['audits']), 'audits', flush=True)
            report['build'] = run(['lake', 'build', plan['root']], project, args.output / 'build.log')
            lock = args.output / 'diagnostics.lock'
            lock.touch(mode=0o600)
            diag = args.output / 'diagnostics'
            report['diagnostics'] = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                '--project', project, '--output', diag, '--lock', lock], ROOT, args.output / 'diagnose.log')
            checked_diagnostics(plan, json.loads((diag / 'result.json').read_bytes()))
            if plan != materialize(args.output / 'reproduced', parent, args.reference_project, good, bad):
                raise ValueError('the source/load closure did not reproduce')
            if before != dependencies(packages):
                raise ValueError('fixed Lean dependencies changed')
        if rules != {file.name: retention.sha(file) for file in HERE.iterdir() if file.is_file()}:
            raise ValueError('rules changed during the check')
        report.update(complete=True, actual_files=len(plan['files']), unique_audits=len(plan['audits']))
    except BaseException as error:
        report['error'] = repr(error)
        raise
    finally:
        retention.write_json(args.output / 'result.json', report)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('captures', 'parent-output', 'parent-capture', 'reference-captures', 'reference-project',
                 'reference-audit', 'tools', 'lock', 'output'):
        parser.add_argument('--' + name, type=lambda value: Path(value).resolve(), required=True)
    check(parser.parse_args())
