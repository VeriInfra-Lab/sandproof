"""Capture the original Full binding guard, reusing complete Full/Pause validity."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-full-work-guard'
sys.path.insert(0, str(PARENT.parent / 'source-remove-retired'))
_spec = importlib.util.spec_from_file_location('source_full_binding_capture_parent', PARENT / 'capture.py')
parent = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(parent)
retire = parent.retire
VARIANTS = ('original', 'omit-source-work-binding')
ROOT = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::full::validate::full_binding'
NAMESPACE = 'SandproofSourceFullBinding'
OPTIONS = dict(parent.OPTIONS, roots=[ROOT])
END = Path('controller-service/src/controller/lifecycle_inbox/source_work/full/validate.rs')
BASE_STAGED = parent.STAGED
STAGED = (*BASE_STAGED, END)
OMITTED = '        || full.pause.source_work_binding != work.requirement.work_binding\n'
LIBRARIES = parent.LIBRARIES


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown Full binding view')
    begin = text.index('pub(super) fn full_binding(')
    end = text.index('pub(in crate::controller) fn validate_record(', begin)
    body = text[begin:end]
    if body.count(OMITTED) != 1:
        raise ValueError('one exact original source work-binding guard required')
    return text[:begin] + (body if variant == 'original' else body.replace(OMITTED, '', 1)) + text[end:]


def reconstruct(archive, source, variant):
    try:
        parent.reconstruct(archive, source, 'original')
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
                and r['lean_name'].endswith('.valid') and r['source']['file'] == str(parent.END)]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('one actual transparent original validity method is required: ' + name)
        entries = [f for f in tree['fun_decls'] if f and f['def_id'] == rows[0]['def_id']]
        if len(entries) != 1 or 'Structured' not in (entries[0]['body'] or {}):
            raise ValueError('the complete original validity body is required: ' + name)
        result['business'][name] = rows[0]['def_id']
    roots = [row for row in mapping if row['lean_name'] == NAMESPACE +
             '.controller.lifecycle_inbox.source_work.full.validate.full_binding']
    if len(roots) != 1 or roots[0]['is_opaque']:
        raise ValueError('the complete original Full binding caller must be transparent')
    entries = [row for row in tree['fun_decls'] if row and row['def_id'] == roots[0]['def_id']]
    if len(entries) != 1 or 'Structured' not in (entries[0]['body'] or {}):
        raise ValueError('the complete original Full binding body is required')
    result['business']['full_binding'] = roots[0]['def_id']
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
                direct_original_full_binding=True, whole_receipt_proved=False, parent_driver=digest(Path(parent.__file__)),
                mutation=None if variant == 'original' else 'omit only original source_work_binding comparison')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
