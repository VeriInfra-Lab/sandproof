"""Extend the original append checker with an unconditional record contract."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import traceback
import time

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/inbox-record-total')
APPEND = HERE.parent / 'inbox-append'
RECORD = HERE.parent / 'inbox-record'
spec = importlib.util.spec_from_file_location('record_total_parent_checker', ROOT/APPEND/'formal.py')
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)


def templates(variant):
    inventory = json.loads((ROOT/HERE/'inventory.json').read_text())
    observed = {p.name.removesuffix('.lean.in'): re.findall(
        r'^#audit_controller_proof (\S+)$', p.read_text(), re.M)
        for p in sorted((ROOT/HERE).glob('*.lean.in'))}
    if observed != inventory['audits'] or sum(map(len, observed.values())) != 5:
        raise ValueError('missing, extra, or changed total-record proof inventory')
    target = {'correct': 'InboxRecordTotal', 'skip-sdk-gate': 'InboxRecordTotalWrongSdk',
              'skip-prior-capture': 'InboxRecordTotalWrongCapture'}[variant]
    sources = [HERE/(stem+'.lean.in') for stem in ('InboxRecordTotalModel', target)]
    if variant == 'correct':
        sources += [RECORD/(stem+'.lean.in') for stem in (
            'InboxRecordWindowComparison', 'InboxRecordFresh', 'InboxRecordNewSession')]
    return sources, observed['InboxRecordTotalModel'] + observed[target]


def run(output, variant, require_native=False, development_native=None, development_record_native=None):
    start = time.monotonic()
    sources, audits = templates(variant)
    fresh_result = None
    fresh_receipt = None
    if require_native:
        if development_native is not None or development_record_native is not None:
            raise ValueError('development results cannot substitute for ning receipts')
        sys.path.insert(0, str(ROOT))
        from tools.acceptance_receipts import native_case, digest
        fresh_receipt = native_case('stage1-inbox-record', ROOT, None)
        evidence = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'commands'/digest(fresh_receipt['key'])/'evidence'
        fresh_result = json.loads((evidence/'result.json').read_text())
    elif development_native is not None or development_record_native is not None:
        if development_native is None or development_record_native is None:
            raise ValueError('both same-input fresh-record and append development results are required')
        fresh_result = json.loads(development_record_native.read_text())
    if fresh_result is not None:
        if (fresh_result.get('complete') is not True
                or fresh_result.get('kind') != 'inbox-record-native-development'
                or set(fresh_result.get('variants', {})) != {
                    'original', 'candidate', 'skip-retention-window', 'wrong-recorded-version', 'restored'}):
            raise ValueError('incomplete fresh-record native evidence')
    result = parent.run(output, variant, require_native, development_native,
                        additional_templates=sources)
    try:
        result['kind'] = 'inbox-record-total-formal-development'
        result['total_record'] = dict(templates=[str(p) for p in sources], audits=audits,
            native_cases=['stage1-inbox-record', 'stage1-inbox-append'], new_production_algorithm=False,
            persistent_record_lifecycle_intent=False)
        if fresh_receipt is not None:
            result['fresh_record_native_receipt'] = fresh_receipt
        if development_record_native is not None:
            result['development_record_native'] = dict(path=str(development_record_native),
                sha256=parent.sha(development_record_native))
        if result['complete'] and fresh_result is not None:
            result['fresh_native_admission'] = {}
            parent.native_admission.admit(result['fresh_native_admission'], fresh_result,
                result['inputs'], result['registry'], ROOT,
                parent.native.registry_module.HERE / 'Cargo.lock')
            for name, current in result['business_inputs'].items():
                expected = result['mutation']['before'] if result['mutation'] and name == parent.MODEL else current
                for source_variant in ('candidate', 'restored'):
                    if fresh_result['variants'][source_variant]['inputs'].get(name) != expected:
                        raise ValueError('fresh-record and append candidates differ')
        if result['complete']:
            destinations = {'SandboxModel/Lifecycle/Protocol/'+p.name.removesuffix('.in')
                            for p in sources}
            if (set(result['additional_proofs']) != destinations
                    or not set(audits) <= set(result['actual']['audits'])):
                result.update(complete=False, error='total-record additions or audits missing')
        if result['complete']:
            log = output/'extension-admission.log'
            with log.open('xb') as stream:
                checked = subprocess.run([sys.executable, str(HERE/'test_extensions.py')],
                    cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, timeout=60,
                    env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1'))
            result['extension_admission'] = dict(exit_code=checked.returncode, sha256=parent.sha(log))
            if checked.returncode or parent.base.snapshot(ROOT) != result['inputs']:
                result.update(complete=False, error='extension admission failed or source inputs changed')
    except BaseException as error:
        result.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    result['parent_seconds'] = result['seconds']
    result['seconds'] = time.monotonic() - start
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=['correct', 'skip-sdk-gate', 'skip-prior-capture'], default='correct')
    parser.add_argument('--require-native', action='store_true')
    parser.add_argument('--development-native', type=Path)
    parser.add_argument('--development-record-native', type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.variant, args.require_native, args.development_native, args.development_record_native)
    print(json.dumps({k: result.get(k) for k in ('variant', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
