"""Capture actual Full material arithmetic; reuse exact29 parent interfaces."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('full_material_reservation_parent', HERE.parent / 'source-end-receipt/capture.py')
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)
retire = parent.retire
FULL = Path('controller-service/src/controller/lifecycle_inbox/source_work/full.rs')
ROOT = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::full::reservation'
NAMESPACE = 'SandproofFullMaterialReservation'
STAGED = (*parent.STAGED, FULL)
VARIANTS = ('original', 'omit-second-material-copy')
RETAINED_ROOTS = list(dict.fromkeys((*parent.OPTIONS['retained_roots'], parent.ROOT)))
OPTIONS = dict(parent.OPTIONS, roots=[ROOT], retained_roots=RETAINED_ROOTS,
    include=[*parent.OPTIONS['include'], 'core::option::_::and_then'])
LIBRARIES = parent.LIBRARIES


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown Full material reservation variant')
    start = text.index('fn reservation(full: &FullCaptureWorkRequest)')
    stop = text.index('\nimpl SourceWorkRecord', start)
    body = text[start:stop]
    original = '.and_then(|n| n.checked_mul(2))'
    if body.count(original) != 1 or body.count('2 * 262_144 + 65_536') != 1:
        raise ValueError('complete original arithmetic and one double-copy multiplier required')
    if variant == 'omit-second-material-copy':
        body = body.replace(original, '.and_then(|n| n.checked_mul(1))', 1)
    return text[:start] + body + text[stop:]


def reconstruct(archive, source, variant):
    if FULL not in STAGED:
        raise ValueError('the original full module must be in the inherited staging inventory')
    try:
        parent.reconstruct(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / FULL
    path.write_text(mutate(path.read_text(), variant))


def admit(ir, generated, target):
    raw = json.loads(ir.read_bytes())
    tree, options = raw['translated'], raw['translated']['options']
    if (raw.get('has_errors') is not False
            or any(options[n] is not False for n in
                ('skip_borrowck','no_typecheck','remove_unused_clauses','monomorphize'))
            or options['exclude'] != []
            or any(options[k] != OPTIONS[g] for k,g in
                (('start_from','roots'),('include','include'),('opaque','opaque')))
            or OPTIONS['roots'] != [ROOT] or OPTIONS['retained_roots'] != RETAINED_ROOTS
            or options['start_from_if_exists'] != RETAINED_ROOTS
            or {r['key'] for r in tree['target_information']} != {target}):
        raise ValueError('exact original root, both complete parent retained roots, safe options and fixed target required')
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    required = (ROOT.removeprefix('sandproof_controller_service::').replace('::','.'),
                *parent.BUSINESS, *parent.TRANSPARENT_STANDARD, 'core.option.Option.and_then')
    report = dict(business={}, libraries=[])
    for name in required:
        rows = [r for r in mapping if r['lean_name'] == NAMESPACE + '.' + name]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('one complete actual transparent function required: ' + name)
        functions = [f for f in tree['fun_decls'] if f and f['def_id'] == rows[0]['def_id']]
        if len(functions) != 1 or 'Structured' not in (functions[0]['body'] or {}):
            raise ValueError('original structured body required: ' + name)
        report['business'][name] = rows[0]['def_id']
    for row in mapping:
        if row['is_opaque']:
            if (row['source']['file'].startswith(('controller-service/','controller-core/','sandbox-protocol/'))
                    and row['lean_name'] not in LIBRARIES):
                raise ValueError('no extra owned opacity: ' + row['lean_name'])
            report['libraries'].append(row['lean_name'])
    report['libraries'].sort()
    if len(report['libraries']) != 29 or set(report['libraries']) != LIBRARIES:
        raise ValueError('only all29 existing original interfaces are allowed')
    return report


def capture(output, tools, lock, variant):
    retire.ROOT,retire.NAMESPACE,retire.OPTIONS,retire.STAGED = ROOT,NAMESPACE,OPTIONS,STAGED
    retire.reconstruct,retire.admit = reconstruct,admit
    try:
        retire.capture(output,tools,lock,variant)
    finally:
        path = output / 'result.json'
        if path.is_file():
            row = json.loads(path.read_bytes())
            row.update(base_driver=row['driver'],driver=digest(Path(__file__)),
                parent_driver=digest(Path(parent.__file__)),capture_root=ROOT,
                original_full_reservation_arithmetic=True,standard_and_then_transparent=True,
                behavior_proved=False,mutation=None if variant=='original' else 'only multiplier2 to1')
            path.write_text(json.dumps(row,indent=2)+'\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True)
    for name in ('output','tools','lock'):
        cli.add_argument('--'+name,type=Path,required=True)
    args = cli.parse_args()
    capture(args.output.resolve(),args.tools.resolve(strict=True),args.lock.resolve(strict=True),args.variant)
