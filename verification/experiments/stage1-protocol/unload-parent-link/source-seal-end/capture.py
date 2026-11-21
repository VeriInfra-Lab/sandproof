"""Capture the original complete seal_end and its one omitted-guard mutation."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))
_spec = importlib.util.spec_from_file_location('source_seal_end_capture_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(retire)
VARIANTS = ('original', 'ignore-source-generation')
ROOT = retire.ports.WORK + '::full::end::seal_end'
NAMESPACE = 'SandproofSourceSealEnd'
OPTIONS = dict(retire.OPTIONS, roots=[ROOT])
END = Path('controller-service/src/controller/lifecycle_inbox/source_work/full/end.rs')
BASE_STAGE, BASE_STAGED = retire.reconstruct, retire.STAGED
STAGED = (*BASE_STAGED, END)
OMITTED = '\n        || generation != original.native.generation'


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown original seal-end variant')
    start = text.index('pub(super) fn seal_end(')
    stop = text.index('\nimpl SourceWorkRecord {', start)
    body = text[start:stop]
    if body.count(OMITTED) != 1:
        raise ValueError('one exact original source-generation comparison is required')
    return text[:start] + (body if variant == 'original' else body.replace(OMITTED, '', 1)) + text[stop:]


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
    for name in (ROOT, 'sandbox_controller_core::model::EffectId::command'):
        # Rust impl paths have their original Charon spelling; the complete
        # mapping must also show the actual transparent command accessor.
        if name == ROOT:
            entry = retire.declaration(tree['fun_decls'], name)
            rows = [r for r in mapping if r['def_id'] == entry['def_id']]
            if 'Structured' not in (entry['body'] or {}):
                raise ValueError('the entire original seal_end must stay structured')
        else:
            rows = [r for r in mapping if r['lean_name'] == NAMESPACE + '.sandbox_controller_core.model.EffectId.command']
        if not rows or any(r['is_opaque'] for r in rows):
            raise ValueError('the actual original business root/accessor must be transparent: ' + name)
        entries = [entry for entry in tree['fun_decls'] if entry and entry['def_id'] in {r['def_id'] for r in rows}]
        if len(entries) != len(rows) or any('Structured' not in (entry['body'] or {}) for entry in entries):
            raise ValueError('every actual root/accessor body must remain structured: ' + name)
        result['business'][name] = [r['def_id'] for r in rows]
    owned = ('controller-service/', 'controller-core/', 'sandbox-protocol/')
    for row in mapping:
        if row['is_opaque']:
            if row['source']['file'].startswith(owned):
                raise ValueError('no owned business or command-accessor opacity is permitted')
            result['libraries'].append(row['lean_name'])
    result['libraries'].sort()
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
                mutation=None if variant == 'original' else 'omit only original source-generation comparison')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
