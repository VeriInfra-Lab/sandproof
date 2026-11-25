"""Audit the complete original encoder-to-reader relation on the exact retained captures."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import sys

from roundtrip_inputs import (HERE, CANONICAL, ENCODER, MODES, NAMESPACE, ENCODER_NAMESPACE, ENCODER_SOURCE,
                              PARENT, ROOT, exact_package, retention, row_provider)
from canonical_check import view as checked_canonical, check_digest_forwarding
from canonical_prepare import canonical_view
from encoder_prepare import checked as checked_encoder
from roundtrip_prepare import materialize
from rows_prepare import declarations
from load_prepare import read_exact
from policy_check import checked_diagnostics
from check import dependencies, run


def checked_parent(output, rules):
    receipt = json.loads((output / 'result.json').read_bytes())
    if receipt['complete'] is not True or receipt['rule_inputs'] != rules:
        raise ValueError('the complete exact #1046 parent is required')
    parent = output / 'project'
    plan = json.loads((parent / 'plan.json').read_bytes())
    checked_diagnostics(plan, json.loads((output / 'diagnostics/result.json').read_bytes()))
    return parent, plan, receipt


def actual_same_hash(capture):
    codec = (capture / 'source' / retention.CODEC).read_text()
    sha = (capture / 'source/controller-service/src/controller/lifecycle_inbox/sha256.rs').read_text()
    pattern = r'fn {name}\(bytes: &\[u8\]\) -> \[u8; 32\] \{\n(.*?)\n\}'
    left, = re.findall(pattern.replace('{name}', 'digest_source_payload'), codec, re.S)
    right, = re.findall(pattern.replace('{name}', 'digest'), sha, re.S)
    if (left != right or left.strip() != 'Sha256::digest(bytes).into()'
            or 'use sha2::{Digest, Sha256};' not in codec or 'use sha2::{Digest, Sha256};' not in sha):
        raise ValueError('the actual row checksum and unique shared SHA call differ')
    return {'same_complete_input': True, 'same_original_library_calls': True}


def correspondence(parent, plan, current, correct, wrong):
    result = {}
    encoder = 'controller.lifecycle_inbox.source_work.codec.encode_candidate_records'
    dictionary = 'Array.Insts.CoreCmpPartialOrdArray'
    suffix = ('  lt := Array.Insts.CoreCmpPartialOrdArray.lt cmpPartialOrdInst\n'
              '  le := Array.Insts.CoreCmpPartialOrdArray.le cmpPartialOrdInst\n')
    for name in ('Types.lean', 'Funs.lean'):
        before, positive, negative = (declarations(path / name) for path in (current, correct, wrong))
        if positive.keys() != negative.keys():
            raise ValueError('the exact prior original and wrong declaration inventories must match')
        changes = {sym for sym in positive if positive[sym] != negative[sym]}
        if changes != ({encoder + '_loop.body'} if name == 'Funs.lean' else set()):
            raise ValueError('prior actual variant must change only the actual emitted key step')
        different = {sym for sym in positive if before.get(sym) != positive[sym]}
        if different != ({dictionary} if name == 'Funs.lean' else set()):
            raise ValueError('the current actual type/provider inventory differs from the prior encoder')
        if name == 'Funs.lean':
            if before[dictionary] != positive[dictionary].removesuffix('}') + suffix + '}':
                raise ValueError('the only shared dictionary difference must be the additional standard lt/le methods')
            selected = {sym for sym in positive if sym.startswith(encoder)}
            if len(selected) != 3:
                raise ValueError('exact original encoder declarations absent')
            checked_view = read_exact(parent, plan, 'EncoderPorts.lean').decode()
            if any(positive[sym] not in checked_view or before[sym] != positive[sym] for sym in selected):
                raise ValueError('actual encoder bodies are not those checked in the complete parent')
        result[name] = dict(current=len(before), encoder=len(positive), differences=sorted(different),
                            variant_changes=sorted(changes))
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
            canonical_rules = exact_package(CANONICAL, PARENT)
            encoder_rules = exact_package(ENCODER, ENCODER_SOURCE)
            parent, prior, receipt = checked_parent(args.parent_output, canonical_rules)
            packages = (parent / '.lake/packages').resolve(strict=True)
            before = dependencies(packages)
            row = row_provider(args.output)
            current = checked_canonical(args.parent_capture, 'correct', row, fixed, args.reference,
                args.bounds_reference, args.cursor_reference, args.collector_reference, args.collector_capture)
            if canonical_view(current) != read_exact(parent, prior, 'SaveLoadCanonicalView.lean').decode():
                raise ValueError('actual new canonical view differs from the exact checked #1046 closure')
            if check_digest_forwarding(args.parent_capture) != receipt['digest_forwarding']['correct']:
                raise ValueError('actual original SHA forwarding differs from the checked parent')
            report['same_hash'] = actual_same_hash(args.parent_capture)
            for mode in MODES:
                checked_encoder(args.encoder_captures / mode, mode, row)
            correct, wrong = (args.encoder_captures / mode / 'generated' / ENCODER_NAMESPACE for mode in MODES)
            report['matching_definitions'] = correspondence(parent, prior, current, correct, wrong)
            project = args.output / 'project'
            plan = materialize(project, parent, wrong)
            report['dependencies'] = before
            print(plan['root'], len(plan['files']), 'real files;', len(plan['audits']), 'audits', flush=True)
            report['build'] = run(['lake', 'build', plan['root']], project, args.output / 'build.log')
            lock = args.output / 'diagnostics.lock'
            lock.touch(mode=0o600)
            diag = args.output / 'diagnostics'
            report['diagnostics'] = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                '--project', project, '--output', diag, '--lock', lock], ROOT, args.output / 'diagnose.log')
            checked_diagnostics(plan, json.loads((diag / 'result.json').read_bytes()))
            if plan != materialize(args.output / 'reproduced', parent, wrong):
                raise ValueError('the complete original encoder-to-reader closure did not reproduce')
            if before != dependencies(packages) or exact_package(CANONICAL, PARENT) != canonical_rules or exact_package(ENCODER, ENCODER_SOURCE) != encoder_rules:
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
    for name in ('encoder-captures', 'parent-output', 'parent-capture', 'collector-capture', 'reference',
                 'bounds-reference', 'cursor-reference', 'collector-reference', 'tools', 'lock', 'output'):
        parser.add_argument('--' + name, type=lambda value: Path(value).resolve(), required=True)
    check(parser.parse_args())
