"""Capture original terminal matching, constructor and both transparent bindings.

Only the two new exact tuple encoding/hash calls and the two inherited Full calls are isolated. This is
a development extraction, not a formal or native acceptance receipt.
"""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-request-end-match'
sys.path.insert(0, str(PARENT.parent / 'source-remove-retired'))
spec = importlib.util.spec_from_file_location('source_terminal_match_capture_parent', PARENT / 'capture.py')
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)
retire = parent.retire
ROOT = 'sandproof_controller_service::capture::terminated::_::matches'
CONSTRUCTOR = 'sandproof_controller_service::capture::terminated::_::from_source'
NAMESPACE = 'SandproofSourceTerminalMatch'
VARIANTS = ('original', 'omit-host-slot')
END = Path('controller-service/src/capture/terminated.rs')
WORK = parent.WORK
BASE_STAGED = parent.STAGED
STAGED = (*BASE_STAGED, END)
PORTS = [*parent.PORTS, *('sandproof_controller_service::capture::terminated::' + name
    for name in ('encode_terminated_work_binding', 'digest_terminated_work_binding'))]
OPTIONS = dict(parent.OPTIONS, roots=[ROOT], retained_roots=[CONSTRUCTOR],
    opaque=[*parent.OPTIONS['opaque'], *PORTS[len(parent.PORTS):]])
ORIGINAL_SLOT = '            && *host_slot == work.host_slot'
LIBRARIES = {
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
    'capture.work.digest_full_work_binding',
    'capture.work.encode_full_work_binding',
    'capture.terminated.digest_terminated_work_binding',
    'capture.terminated.encode_terminated_work_binding',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
}
ENCODE_HELPER = '''
fn encode_terminated_work_binding(value: &(
    OriginalContinueIdentity,
    &WorkAction<core::EffectId>,
    &FullCaptureWorkRequest,
    &SourceObservationRequirement,
    &Option<CaptureWorkerBinding>,
    &WorkAction<core::EffectId>,
    core::StopCoverage,
    HostSlotContext,
)) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(value)
}
fn digest_terminated_work_binding(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
'''


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('one exact original occurrence is required: ' + old)
    return text.replace(old, new, 1)


def mutate(original, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown terminal matching view')
    begin = original.index('impl TerminatedCaptureObservation {')
    end = original.index('\n}\n', begin) + 2
    body = original[begin:end]
    if body.count(ORIGINAL_SLOT) != 1:
        raise ValueError('one complete original host-slot comparison required')
    if variant == 'original':
        return original
    body = once(body, '            host_slot,\n', '            host_slot: _,\n')
    return original[:begin] + body.replace(ORIGINAL_SLOT, '', 1) + original[end:]


def reconstruct(archive, source, variant):
    try:
        parent.reconstruct(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / END
    text = mutate(path.read_text(), variant)
    text = once(text, 'Sha256::digest(serde_json::to_vec(&(',
        'digest_terminated_work_binding(&encode_terminated_work_binding(&(')
    text = once(text, '            ))?)\n            .into(),', '            ))?),')
    path.write_text(text + ENCODE_HELPER)


def admit(ir, generated, target):
    tree = retire.checked_translation(ir, OPTIONS, target, ROOT, CONSTRUCTOR)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = dict(business={}, libraries=[])
    for suffix in ('capture.terminated.TerminatedCaptureObservation.matches',
            'capture.terminated.TerminatedCaptureWorkRequest.binding_digest',
            'capture.terminated.TerminatedCaptureWorkRequest.from_source',
            'capture.work.FullCaptureWorkRequest.binding_digest'):
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
    if set(result['libraries']) != LIBRARIES or len(result['libraries']) != 6:
        raise ValueError('the exact six typed library interfaces are required')
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
                capture_root=ROOT, direct_original_terminal_match_and_bindings=True,
                parent_driver=sha256(Path(parent.__file__).read_bytes()).hexdigest(), whole_receipt_proved=False, public_acceptance=False,
                exact_library_ports=PORTS,
                mutation=None if variant == 'original' else 'omit original host-slot comparison and discard its now-unused pattern binding')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
