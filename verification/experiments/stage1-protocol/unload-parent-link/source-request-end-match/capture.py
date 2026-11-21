"""Capture the original request-end match and transparent Full binding.

Only the two exact third-party encoding/hash calls are isolated. This is
a development extraction, not a formal or native acceptance receipt.
"""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))
spec = importlib.util.spec_from_file_location('source_request_end_match_capture_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retire)
ROOT = 'sandproof_controller_service::capture::_::matches'
NAMESPACE = 'SandproofSourceRequestEndMatch'
VARIANTS = ('original', 'process-only-worker')
CAPTURE = Path('controller-service/src/capture.rs')
WORK = Path('controller-service/src/capture/work.rs')
END = CAPTURE
BASE_STAGE, BASE_STAGED = retire.reconstruct, retire.STAGED
STAGED = (*BASE_STAGED, CAPTURE, WORK)
PORTS = ['sandproof_controller_service::capture::work::' + name
         for name in ('encode_full_work_binding', 'digest_full_work_binding')]
OPTIONS = dict(retire.OPTIONS, roots=[ROOT], opaque=[*retire.OPTIONS['opaque'], *PORTS])
ORIGINAL_WORKER = '            && self.worker == *worker\n'
WRONG_WORKER = '            && self.worker.process() == worker.process()\n'
LIBRARIES = {
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
    'capture.work.digest_full_work_binding',
    'capture.work.encode_full_work_binding',
    'core.ops.range.RangeInclusive.contains',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
}


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('one exact original occurrence is required: ' + old)
    return text.replace(old, new, 1)


def mutate(original, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown view')
    begin = original.index('impl CaptureRequestEndObservation {')
    end = original.index('\n}\n', begin) + 2
    body = original[begin:end]
    if body.count(ORIGINAL_WORKER) != 1:
        raise ValueError('one complete original worker comparison required')
    if variant == 'original':
        return original
    return original[:begin] + body.replace(ORIGINAL_WORKER, WRONG_WORKER, 1) + original[end:]


def reconstruct(archive, source, variant):
    retire.STAGED = BASE_STAGED
    try:
        BASE_STAGE(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / CAPTURE
    path.write_text(mutate(path.read_text(), variant))
    path = source / WORK
    text = once(path.read_text(), 'Sha256::digest(serde_json::to_vec(self)?).into()',
        'digest_full_work_binding(&encode_full_work_binding(self)?)')
    text += '''
fn encode_full_work_binding(value: &FullCaptureWorkRequest) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(value)
}
fn digest_full_work_binding(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
'''
    path.write_text(text)


def admit(ir, generated, target):
    tree = retire.checked_translation(ir, OPTIONS, target, ROOT, None)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = dict(business={}, libraries=[])
    for suffix in ('capture.CaptureRequestEndObservation.matches', 'capture.work.FullCaptureWorkRequest.binding_digest'):
        rows = [row for row in mapping if row['lean_name'] == NAMESPACE + '.' + suffix]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('one actual transparent business method is required: ' + suffix)
        declarations = [row for row in tree['fun_decls'] if row and row['def_id'] == rows[0]['def_id']]
        if len(declarations) != 1 or 'Structured' not in (declarations[0]['body'] or {}):
            raise ValueError('the complete original method body is required: ' + suffix)
        result['business'][suffix] = rows[0]['def_id']
    for row in mapping:
        if row['is_opaque']:
            if row['source']['file'].startswith(('controller-service/', 'controller-core/', 'sandbox-protocol/')) and not row['lean_name'].endswith(tuple('.' + port.rsplit('::', 1)[1] for port in PORTS)):
                raise ValueError('no additional owned business opacity: ' + row['lean_name'])
            result['libraries'].append(row['lean_name'])
    result['libraries'].sort()
    if set(result['libraries']) != LIBRARIES or len(result['libraries']) != 5:
        raise ValueError('the exact five typed library interfaces are required')
    return result


def capture(output, tools, lock, variant):
    retire.ROOT, retire.NAMESPACE, retire.OPTIONS, retire.STAGED = ROOT, NAMESPACE, OPTIONS, STAGED
    retire.reconstruct, retire.admit = reconstruct, admit
    try:
        retire.capture(output, tools, lock, variant)
    finally:
        path = output / 'result.json'
        if path.is_file():
            result = json.loads(path.read_bytes())
            result.update(base_driver=result['driver'], driver=sha256(Path(__file__).read_bytes()).hexdigest(),
                capture_root=ROOT, direct_original_match_and_binding=True, whole_receipt_proved=False, public_acceptance=False,
                exact_library_ports=PORTS,
                mutation=None if variant == 'original' else 'replace only complete worker comparison by original process getter comparison')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
