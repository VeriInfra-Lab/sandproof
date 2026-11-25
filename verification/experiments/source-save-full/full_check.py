"""Recheck the combined actual normal save result and its retained full row-evidence variant."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import sys

from full_inputs import (HERE, ROOT, PARENT, ROUNDTRIP_COMMIT, SERDE, ROUNDTRIP,
                         retention, row_provider, exact_package, ENCODER, ENCODER_SOURCE,
                         ENCODER_NAMESPACE, MODES)
from full_prepare import materialize
from roundtrip_check import correspondence
from roundtrip_prepare import wrong_view
from encoder_prepare import checked as checked_encoder
from canonical_check import view as checked_canonical
from canonical_prepare import canonical_view
from load_prepare import read_exact
from policy_check import checked_diagnostics
from check import dependencies, run


def checked_parent(output, rules):
    receipt = json.loads((output / 'result.json').read_bytes())
    if receipt['complete'] is not True or receipt['rule_inputs'] != rules:
        raise ValueError('exact complete prior formal results and source inputs are required')
    project = output / 'project'
    plan = json.loads((project / 'plan.json').read_bytes())
    if receipt['actual_files'] != len(plan['files']) or receipt['unique_audits'] != len(plan['audits']):
        raise ValueError('prior formal result does not cover its exact complete plan')
    checked_diagnostics(plan, json.loads((output / 'diagnostics/result.json').read_bytes()))
    if (output / 'reproduced' / 'plan.json').read_bytes() != (project / 'plan.json').read_bytes():
        raise ValueError('prior formal plan was not reproduced')
    return project, plan, receipt


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
            parents = ((SERDE, PARENT), (ROUNDTRIP, ROUNDTRIP_COMMIT), (ENCODER, ENCODER_SOURCE))
            pinned = [exact_package(package, commit) for package, commit in parents]
            parent, prior, _ = checked_parent(args.parent_output, pinned[0])
            old, old_plan, old_receipt = checked_parent(args.roundtrip_output, pinned[1])
            packages = (parent / '.lake/packages').resolve(strict=True)
            before = dependencies(packages)
            row = row_provider(args.output)
            current = checked_canonical(args.current_capture, 'correct', row, fixed, args.reference,
                args.bounds_reference, args.cursor_reference, args.collector_reference, args.collector_capture)
            if canonical_view(current) != read_exact(parent, prior, 'SaveLoadCanonicalView.lean').decode():
                raise ValueError('current actual reader view differs from the checked parent')
            for mode in MODES:
                checked_encoder(args.encoder_captures / mode, mode, row)
            correct, wrong = (args.encoder_captures / mode / 'generated' / ENCODER_NAMESPACE for mode in MODES)
            admitted = correspondence(parent, prior, current, correct, wrong)
            if admitted != old_receipt['matching_definitions']:
                raise ValueError('the actual historical and current encoder correspondence differs')
            if wrong_view(parent, wrong) != read_exact(old, old_plan, 'SaveRoundTripWrong.lean').decode():
                raise ValueError('the original actual altered encoder body differs from the checked witness')
            report['matching_definitions'] = admitted
            project = args.output / 'project'
            plan = materialize(project, parent, prior, old, old_plan)
            report['dependencies'] = before
            print(plan['root'], len(plan['files']), 'real files;', len(plan['audits']), 'audits', flush=True)
            report['build'] = run(['lake', 'build', plan['root']], project, args.output / 'build.log')
            lock = args.output / 'diagnostics.lock'; lock.touch(mode=0o600)
            diag = args.output / 'diagnostics'
            report['diagnostics'] = run([sys.executable, ROOT / retention.OLD_PACKAGE / 'diagnose.py',
                '--project', project, '--output', diag, '--lock', lock], ROOT, args.output / 'diagnose.log')
            checked_diagnostics(plan, json.loads((diag / 'result.json').read_bytes()))
            if plan != materialize(args.output / 'reproduced', parent, prior, old, old_plan):
                raise ValueError('the complete original save closure did not reproduce')
            if before != dependencies(packages) or pinned != [exact_package(package, commit) for package, commit in parents]:
                raise ValueError('the fixed prior inputs changed')
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
    for name in ('parent-output', 'roundtrip-output', 'encoder-captures', 'current-capture', 'collector-capture',
                 'reference', 'bounds-reference', 'cursor-reference', 'collector-reference', 'tools', 'lock', 'output'):
        parser.add_argument('--' + name, type=lambda value: Path(value).resolve(), required=True)
    check(parser.parse_args())
