"""Audit the actual canonical record decoder, error conversion and digest wiring."""

import argparse
import fcntl
from hashlib import sha256
import json
import os
import re
from pathlib import Path
import sys
import tarfile
import tempfile

from canonical_inputs import (HERE, COLLECTOR, MODES, NAMESPACE, PARENT, ROOT, SHA, SOURCE_WORK,
                              SOURCE_FILES, configuration, retention, row_provider, stage)
from canonical_capture import admit
from rows_prepare import declarations
from load_prepare import read_exact
from canonical_prepare import materialize
from policy_check import checked_diagnostics
from capacity_check import verify_artifacts
from check import dependencies, run

def checked_parent(output):
    receipt = json.loads((output / 'result.json').read_bytes())
    rules = {file.name: retention.sha(file) for file in COLLECTOR.iterdir() if file.is_file()}
    for file, digest in rules.items():
        if sha256(retention.pinned(PARENT, COLLECTOR.relative_to(ROOT) / file)).hexdigest() != digest:
            raise ValueError('the parent rules differ at ' + file)
    if receipt['complete'] is not True or receipt['rule_inputs'] != rules:
        raise ValueError('the complete #1045 parent check is required')
    parent = output / 'project'
    checked_diagnostics(json.loads((parent / 'plan.json').read_bytes()),
                        json.loads((output / 'diagnostics/result.json').read_bytes()))
    return parent


def view(folder, mode, row, fixed, reference, bounds_reference, cursor_reference, collector_reference, parent_capture):
    report = json.loads((folder / 'result.json').read_bytes())
    cfg = configuration(row, reference, bounds_reference, cursor_reference, collector_reference)
    expected_rules = {name: retention.sha(HERE / name) for name in ('canonical_capture.py', 'canonical_inputs.py')}
    if (report['complete'] is not True or report['public_acceptance'] is not False
            or (report['mode'], report['source'], report['configuration'], report['rules']) != (
                mode, retention.SOURCE, cfg, expected_rules)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the complete current canonical byte capture is required')
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
    if admit(row, folder / 'actual.llbc', folder / 'generated', fixed['target'], cfg, mode) != report['admission']:
        raise ValueError('actual collector call admission differs')
    return folder / 'generated' / NAMESPACE


def check_digest_forwarding(folder):
    from revision_capture import declaration, regular_calls
    translated = json.loads((folder / 'actual.llbc').read_bytes())['translated']
    mapping = json.loads((folder / 'generated/translation.json').read_bytes())['functions']
    functions = translated['fun_decls']
    work = declaration(functions, SOURCE_WORK + '::digest')
    sha = declaration(functions, SHA)
    (constructor,) = [entry for entry in mapping if entry['lean_name'] ==
        NAMESPACE + '.sandproof_protocol.lifecycle.digest.ContentDigest.from_sha256']
    if (constructor['is_opaque'] is not False or
            constructor['source']['file'] != 'sandbox-protocol/src/lifecycle/digest.rs'):
        raise ValueError('actual digest value constructor differs')
    for item, source, opaque in (
            (work, 'controller-service/src/controller/lifecycle_inbox/source_work.rs', False),
            (sha, 'controller-service/src/controller/lifecycle_inbox/sha256.rs', True)):
        (entry,) = [entry for entry in mapping if entry['def_id'] == item['def_id']]
        if entry['source']['file'] != source or entry['is_opaque'] is not opaque:
            raise ValueError('actual digest forwarding source differs: ' + source)
    calls = list(regular_calls(work['body']['Structured']['body']))
    if calls != [sha['def_id'], constructor['def_id']]:
        raise ValueError('work digest must call the same SHA and full array constructor in original order')
    return dict(work=work['def_id'], sha=sha['def_id'], constructor=constructor['def_id'])


def correspondence(parent, good, bad):
    result = {}
    root = 'controller.lifecycle_inbox.source_work.codec.'
    checked = root + 'checked_source_payload'
    added = {'Types.lean': {checked + '.closure'}, 'Funs.lean': {
        checked, checked + '.closure.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError',
        checked + '.closure.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError.call_once',
        root + 'encode_load_anchor', 'controller.lifecycle_inbox.source_work.digest',
        'sandproof_protocol.lifecycle.digest.ContentDigest.from_sha256'}}
    for filename in ('Types.lean', 'Funs.lean'):
        before, positive, mutant = (declarations(path / filename) for path in (parent, good, bad))
        if (any(positive.get(name) != definition for name, definition in before.items())
                or positive.keys() - before.keys() != added[filename]):
            raise ValueError('actual #1045 declarations or new selected symbols differ')
        changes = {name for name in positive if mutant.get(name) != positive[name]}
        if positive.keys() != mutant.keys() or changes != ({checked} if filename == 'Funs.lean' else set()):
            raise ValueError('the actual variant may change only the specialized record check body')
        result[filename] = dict(parent=len(before), joint=len(positive), added=sorted(added[filename]),
                                mutant_changed=sorted(changes))
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
            parent_receipt = json.loads((args.parent_capture / 'result.json').read_bytes())
            parent_rules = {name: retention.sha(COLLECTOR / name) for name in ('collector_capture.py', 'collector_inputs.py')}
            if (parent_receipt['complete'] is not True or (parent_receipt['mode'], parent_receipt['source'],
                    parent_receipt['rules']) != ('correct', retention.SOURCE, parent_rules)):
                raise ValueError('the original #1045 correct capture is required')
            verify_artifacts(args.parent_capture, parent_receipt, NAMESPACE)
            good, bad = (view(args.captures / mode, mode, row, fixed, args.reference, args.bounds_reference, args.cursor_reference, args.collector_reference, args.parent_capture)
                         for mode in MODES)
            parent_view = args.parent_capture / 'generated' / NAMESPACE
            report['matching_definitions'] = correspondence(parent_view, good, bad)
            report['digest_forwarding'] = {mode: check_digest_forwarding(args.captures / mode) for mode in MODES}
            project = args.output / 'project'
            plan = materialize(project, parent, good, bad)
            report['dependencies'] = before
            print(plan['root'], len(plan['files']), 'real files;', len(plan['audits']), 'audits', flush=True)
            report['build'] = run(['lake', 'build', plan['root']], project, args.output / 'build.log')
            lock = args.output / 'diagnostics.lock'
            lock.touch(mode=0o600)
            diag = args.output / 'diagnostics'
            report['diagnostics'] = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                '--project', project, '--output', diag, '--lock', lock], ROOT, args.output / 'diagnose.log')
            checked_diagnostics(plan, json.loads((diag / 'result.json').read_bytes()))
            if plan != materialize(args.output / 'reproduced', parent, good, bad):
                raise ValueError('the canonical byte and digest closure did not reproduce')
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
    for name in ('captures', 'parent-output', 'parent-capture', 'reference', 'bounds-reference', 'cursor-reference', 'collector-reference', 'tools', 'lock', 'output'):
        parser.add_argument('--' + name, type=lambda value: Path(value).resolve(), required=True)
    check(parser.parse_args())
