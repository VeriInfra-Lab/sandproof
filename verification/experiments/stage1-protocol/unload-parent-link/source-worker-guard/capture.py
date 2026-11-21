"""Capture the complete original worker constructor and its validity dependency.

The source/fixed tools and exact five standard interfaces are fixed.
No complete parent, commit, release or physical worker theorem is claimed.
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
spec = importlib.util.spec_from_file_location('worker_preflight_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retire)
VARIANTS = ('original', 'allow-zero-start-ticks')
ROOT = 'sandproof_controller_service::capture::launch::_::new'
NAMESPACE = 'SandproofSourceWorkerGuard'
OPTIONS = dict(retire.OPTIONS, roots=[ROOT],
    include=[*retire.OPTIONS['include'], 'core::num::_::is_ascii_graphic'])
END = Path('controller-service/src/capture/launch.rs')
BASE_STAGE, BASE_STAGED = retire.reconstruct, retire.STAGED
STAGED = (*BASE_STAGED, END)
OMITTED = '            && self.process.start_time_ticks > 0\n'
LIBRARIES = {
    'alloc.string.String.Insts.CoreOpsDerefDerefStr.deref',
    'alloc.string.String.is_empty',
    'alloc.string.String.len',
    'core.str.Str.bytes',
    'core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all',
}


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown worker preflight view')
    begin = text.index('impl CaptureWorkerBinding {')
    start = text.index('    pub fn valid(&self) -> bool {', begin)
    end = text.index('\n    }', start) + len('\n    }')
    body = text[start:end]
    if body.count(OMITTED) != 1:
        raise ValueError('one original positive start-tick guard is required')
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
    for method in ('new', 'valid'):
        rows = [row for row in mapping if row['lean_name'] ==
            NAMESPACE + '.capture.launch.CaptureWorkerBinding.' + method
            and row['source']['file'] == str(END)]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('one actual transparent worker method is required: ' + method)
        declarations = [row for row in tree['fun_decls'] if row and row['def_id'] == rows[0]['def_id']]
        if len(declarations) != 1 or 'Structured' not in (declarations[0]['body'] or {}):
            raise ValueError('one complete original worker body is required: ' + method)
        result['business'][method] = rows[0]['def_id']
    graphic = [row for row in mapping if row['lean_name'] == NAMESPACE + '.core.num.U8.is_ascii_graphic']
    if len(graphic) != 1 or graphic[0]['is_opaque']:
        raise ValueError('the actual ASCII primitive must be transparent')
    declarations = [row for row in tree['fun_decls'] if row and row['def_id'] == graphic[0]['def_id']]
    if len(declarations) != 1 or 'Structured' not in (declarations[0]['body'] or {}):
        raise ValueError('the complete original ASCII primitive body is required')
    result['graphic_primitive'] = graphic[0]['def_id']
    for row in mapping:
        if row['is_opaque']:
            if row['source']['file'].startswith(('controller-service/', 'controller-core/', 'sandbox-protocol/')):
                raise ValueError('no original worker business body may be opaque')
            result['libraries'].append(row['lean_name'])
    result['libraries'].sort()
    if set(result['libraries']) != LIBRARIES or len(result['libraries']) != 5:
        raise ValueError('the exact five existing per-symbol standard interfaces are required')
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
            result.update(base_driver=result['driver'], driver=digest(Path(__file__)),
                capture_root=ROOT, direct_original_constructor=True,
                whole_receipt_proved=False,
                mutation=None if variant == 'original' else 'omit only process.start_time_ticks>0 in actual worker validity')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
