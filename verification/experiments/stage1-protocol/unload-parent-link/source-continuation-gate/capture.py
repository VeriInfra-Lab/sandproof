"""Capture the complete original continuation classifier and its actual wrong callback."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))
_spec = importlib.util.spec_from_file_location('source_continuation_gate_capture_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(retire)
VARIANTS = ('original', 'accept-terminated')
ROOT = retire.ports.WORK + '::full::end::continuation_allowed'
NAMESPACE = 'SandproofSourceContinuationGate'
OPTIONS = dict(retire.OPTIONS, roots=[ROOT])
END = Path('controller-service/src/controller/lifecycle_inbox/source_work/full/end.rs')
BASE_STAGE, BASE_STAGED = retire.reconstruct, retire.STAGED
STAGED = (*BASE_STAGED, END)
PATTERN = 'Some(CaptureEndRecord::NeverReleased { .. } | CaptureEndRecord::WorkerEnded { .. })'
WRONG = 'Some(CaptureEndRecord::NeverReleased { .. } | CaptureEndRecord::WorkerEnded { .. } | CaptureEndRecord::Terminated { .. })'


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown continuation-gate probe')
    start = text.index('pub(super) fn continuation_allowed(')
    stop = text.index('\npub(super) fn seal_end(', start)
    body = text[start:stop]
    if body.count(PATTERN) != 1:
        raise ValueError('one exact original continuation classifier is required')
    return text[:start] + (body if variant == 'original' else body.replace(PATTERN, WRONG, 1)) + text[stop:]


def reconstruct(archive, source, variant):
    retire.STAGED = BASE_STAGED
    try:
        BASE_STAGE(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / END
    path.write_text(mutate(path.read_text(), variant))


def admit(ir, generated, target):
    tree = retire.checked_translation(ir, OPTIONS, target, ROOT, None)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    names = (NAMESPACE + '.controller.lifecycle_inbox.source_work.full.end.continuation_allowed',
        NAMESPACE + '.controller.lifecycle_inbox.source_work.full.end.continuation_allowed.closure.Insts.CoreOpsFunctionFnOnceTupleSharedCaptureWorkBool.call_once')
    result = dict(business={}, libraries=[])
    for name in names:
        rows = [r for r in mapping if r['lean_name'] == name]
        entries = [f for f in tree['fun_decls'] if f and f['def_id'] in {r['def_id'] for r in rows}]
        if (len(rows) != 1 or rows[0]['is_opaque'] or len(entries) != 1
                or 'Structured' not in (entries[0]['body'] or {})):
            raise ValueError('the actual root and exact FnOnce callback must remain structured: ' + name)
        result['business'][name] = rows[0]['def_id']
    for row in mapping:
        if row['is_opaque']:
            if row['source']['file'].startswith(('controller-service/', 'controller-core/', 'sandbox-protocol/')):
                raise ValueError('no owned business/callback opacity is permitted')
            result['libraries'].append(row['lean_name'])
    result['libraries'].sort()
    if result['libraries'] != ['core.option.Option.as_ref', 'core.option.Option.is_some_and']:
        raise ValueError('only the two existing exact Option/FnOnce interfaces are allowed')
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
            result.update(base_driver=result['driver'], driver=digest(Path(__file__)), capture_root=ROOT,
                direct_original_root=True, whole_receipt_proved=False,
                mutation=None if variant == 'original' else 'accept Terminated in the original end classifier')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
