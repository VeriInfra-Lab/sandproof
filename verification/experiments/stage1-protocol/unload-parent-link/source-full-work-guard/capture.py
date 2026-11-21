"""Capture the original complete Full/Pause validity methods and zero-state mutant."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))
_spec = importlib.util.spec_from_file_location('source_full_work_guard_capture_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(retire)
VARIANTS = ('original', 'allow-empty-state')
ROOT = 'sandproof_controller_service::capture::work::_::valid'
NAMESPACE = 'SandproofSourceFullWorkGuard'
OPTIONS = dict(retire.OPTIONS, roots=[ROOT],
    include=[*retire.OPTIONS['include'], 'core::num::_::is_ascii_graphic'])
END = Path('controller-service/src/capture/work.rs')
BASE_STAGE, BASE_STAGED = retire.reconstruct, retire.STAGED
STAGED = (*BASE_STAGED, END)
OMITTED = '            && self.state_bytes > 0\n'
LIBRARIES = {
    'I64.Insts.CoreConvertTryFromU64TryFromIntError.try_from',
    'Str.Insts.CoreCmpPartialEqStr.eq',
    'alloc.string.String.Insts.CoreCmpPartialEqShared0Str.eq',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
    'alloc.string.String.Insts.CoreOpsDerefDerefStr.deref',
    'alloc.string.String.is_empty',
    'alloc.string.String.len',
    'alloc.vec.Vec.is_empty',
    'core.ops.range.RangeInclusive.contains',
    'core.str.Str.as_bytes',
    'core.str.Str.bytes',
    'core.str.Str.is_empty',
    'core.str.Str.len',
    'core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all',
}


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown Full work guard capture')
    begin = text.index('impl FullCaptureWorkRequest {')
    start = text.index('    pub fn valid(&self) -> bool {', begin)
    end = text.index('\n    }\n}', start) + len('\n    }')
    body = text[start:end]
    if body.count(OMITTED) != 1:
        raise ValueError('one original positive state-byte guard is required')
    return text[:start] + (body if variant == 'original' else body.replace(OMITTED, '', 1)) + text[end:]


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
    result = dict(business={}, libraries=[])
    for name in ('PauseWorkRequest', 'FullCaptureWorkRequest'):
        rows = [r for r in mapping if ('.' + name + '.') in r['lean_name']
                and r['lean_name'].endswith('.valid') and r['source']['file'] == str(END)]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('one actual transparent original validity method is required: ' + name)
        entries = [f for f in tree['fun_decls'] if f and f['def_id'] == rows[0]['def_id']]
        if len(entries) != 1 or 'Structured' not in (entries[0]['body'] or {}):
            raise ValueError('the complete original validity body is required: ' + name)
        result['business'][name] = rows[0]['def_id']
    for r in mapping:
        if r['is_opaque']:
            if r['source']['file'].startswith(('controller-service/', 'controller-core/', 'sandbox-protocol/')):
                raise ValueError('no owned complete validity/callback/compatibility body may be opaque')
            result['libraries'].append(r['lean_name'])
    result['libraries'].sort()
    if set(result['libraries']) != LIBRARIES or len(result['libraries']) != 14:
        raise ValueError('the exact14 existing per-symbol standard interfaces are required')
    graphic = [r for r in mapping if r['lean_name'] == NAMESPACE + '.core.num.U8.is_ascii_graphic']
    if len(graphic) != 1 or graphic[0]['is_opaque']:
        raise ValueError('the actual included ASCII graphic primitive must remain transparent')
    entries = [f for f in tree['fun_decls'] if f and f['def_id'] == graphic[0]['def_id']]
    if len(entries) != 1 or 'Structured' not in (entries[0]['body'] or {}):
        raise ValueError('the complete original ASCII primitive body is required')
    result['graphic_primitive'] = graphic[0]['def_id']
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
                direct_original_roots=True, whole_receipt_proved=False,
                mutation=None if variant == 'original' else 'omit only state_bytes>0 in actual Full validity')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
