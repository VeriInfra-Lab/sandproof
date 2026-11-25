"""Check the whole nullable decoder, byte pipeline and real old wrong view on the exact current parent."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import sys

from decision_inputs import (HERE, ROUNDTRIP, COLLECTOR, COLLECTOR_SOURCE, PARENT, ROOT, NAMESPACE, MODES,
                             exact_package, retention, row_provider)
from canonical_check import view as checked_canonical
from collector_check import view as checked_collector, correspondence as collector_correspondence
from collector_inputs import COLLECTOR_NAMESPACE
from collector_prepare import wrong_view
from decision_prepare import materialize
from rows_prepare import declarations
from load_prepare import read_exact
from policy_check import checked_diagnostics
from check import dependencies, run


def checked_parent(output, rules, label):
    receipt = json.loads((output / 'result.json').read_bytes())
    if receipt['complete'] is not True or receipt['rule_inputs'] != rules:
        raise ValueError('the complete exact ' + label + ' proof is required')
    parent = output / 'project'
    plan = json.loads((parent / 'plan.json').read_bytes())
    checked_diagnostics(plan, json.loads((output / 'diagnostics/result.json').read_bytes()))
    return parent, plan, receipt


def correspondence(parent, plan, old, old_plan, current, good, bad):
    result = {}
    codec = 'controller.lifecycle_inbox.source_work.codec.'
    for name in ('Types.lean', 'Funs.lean'):
        before, correct, wrong = (declarations(path / name) for path in (current, good, bad))
        if any(before.get(symbol) != value for symbol, value in correct.items()):
            raise ValueError('the entire #1045 original inventory must still match the current combined capture')
        changes = {symbol for symbol in correct.keys() & wrong.keys() if correct[symbol] != wrong[symbol]}
        removed = {'Slice.Insts.CoreCmpPartialEqArray'} if name == 'Funs.lean' else set()
        if (changes != ({codec + 'decode_loaded_record'} if name == 'Funs.lean' else set())
                or correct.keys()-wrong.keys() != removed or wrong.keys()-correct.keys()):
            raise ValueError('the retained actual wrong decoder has a different scope')
        if name == 'Funs.lean':
            selected = {symbol for symbol in correct if symbol.startswith((codec + 'decode_loaded_record', codec + 'read_loaded_rows'))}
            view = read_exact(parent, plan, 'SaveLoadCollectorView.lean')
            if view != read_exact(old, old_plan, 'SaveLoadCollectorView.lean') or len(selected) != 4 or any(
                    correct[sym].encode() not in view for sym in selected):
                raise ValueError('the earlier original decoder/collection functions must be the exact checked current view')
        result[name] = dict(current=len(before), collector=len(correct), changed=sorted(changes),
                            removed=sorted(removed))
    if wrong_view(bad).encode() != read_exact(old, old_plan, 'SaveLoadCollectorWrong.lean'):
        raise ValueError('the previously checked wrong view must be the exact original actual variant')
    if read_exact(old, old_plan, 'SaveLoadCollectorWitness.lean') != (COLLECTOR / 'SaveLoadCollectorWitness.lean').read_bytes():
        raise ValueError('the previously checked full key witness must not change')
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
            roundtrip_rules = exact_package(ROUNDTRIP, PARENT)
            collector_rules = exact_package(COLLECTOR, COLLECTOR_SOURCE)
            parent, prior, _ = checked_parent(args.parent_output, roundtrip_rules, '#1047')
            old, old_plan, old_receipt = checked_parent(args.collector_output, collector_rules, '#1045')
            packages = (parent / '.lake/packages').resolve(strict=True)
            before = dependencies(packages)
            row = row_provider(args.output)
            current = checked_canonical(args.current_capture, 'correct', row, fixed, args.reference,
                args.bounds_reference, args.cursor_reference, args.collector_reference,
                args.collector_captures / 'correct')
            good, bad = (checked_collector(args.collector_captures / mode, mode, row, fixed, args.reference,
                      args.bounds_reference, args.cursor_reference, args.collector_reference, args.cursor_capture)
                         for mode in MODES)
            if collector_correspondence(args.cursor_capture / 'generated' / NAMESPACE, good, bad,
                    args.collector_reference / 'generated' / COLLECTOR_NAMESPACE) != old_receipt['matching_definitions']:
                raise ValueError('the actual retained full key correspondence is not the checked #1045 one')
            report['matching_definitions'] = correspondence(parent, prior, old, old_plan, current, good, bad)
            project = args.output / 'project'
            plan = materialize(project, parent, old)
            report['dependencies'] = before
            print(plan['root'], len(plan['files']), 'real files;', len(plan['audits']), 'audits', flush=True)
            report['build'] = run(['lake', 'build', plan['root']], project, args.output / 'build.log')
            lock = args.output / 'diagnostics.lock'
            lock.touch(mode=0o600)
            diag = args.output / 'diagnostics'
            report['diagnostics'] = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                '--project', project, '--output', diag, '--lock', lock], ROOT, args.output / 'diagnose.log')
            checked_diagnostics(plan, json.loads((diag / 'result.json').read_bytes()))
            if plan != materialize(args.output / 'reproduced', parent, old):
                raise ValueError('the complete original nullable read decision closure did not reproduce')
            if before != dependencies(packages) or exact_package(ROUNDTRIP, PARENT) != roundtrip_rules or exact_package(COLLECTOR, COLLECTOR_SOURCE) != collector_rules:
                raise ValueError('fixed prior input changed')
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
    for name in ('parent-output', 'current-capture', 'collector-output', 'collector-captures', 'cursor-capture',
                 'reference', 'bounds-reference', 'cursor-reference', 'collector-reference', 'tools', 'lock', 'output'):
        parser.add_argument('--' + name, type=lambda value: Path(value).resolve(), required=True)
    check(parser.parse_args())
