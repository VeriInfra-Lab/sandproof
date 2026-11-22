"""Capture the public End receipt and six-field continuation value interface.

All original receipt business remains transparent; exact29 libraries only.
No observation, authorization, persistence or actual dispatch is supplied here.
"""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('source_end_accessor_parent', HERE.parent/'source-end-receipt/capture.py')
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)
retire = parent.retire
WORK = 'sandproof_controller_service::controller::lifecycle_inbox::source_work'
CONTINUATION = 'sandproof_controller_service::capture::continuation'
TYPE = CONTINUATION+'::OriginalContinueWorkRequest'
GETTER_TYPE = WORK+'::SourceWorkRecord'
ROOT = GETTER_TYPE+'::capture_work_end_receipt'
METHODS = ('from_ended_source', 'identity', 'full', 'source', 'end_receipt', 'network_policy', 'file_policy')
METHOD_ROOTS = ((WORK+'::full::end', 'capture_work_end_receipt'), *((CONTINUATION, m) for m in METHODS))
ROOTS = (ROOT, *(TYPE+'::'+member for member in METHODS))
METHOD_RETAINED_ROOTS = tuple(module+'::*::'+member for module, member in METHOD_ROOTS)
RETAINED_ROOTS = list(dict.fromkeys((*parent.OPTIONS['retained_roots'], parent.ROOT, *METHOD_RETAINED_ROOTS)))
NAMESPACE = 'SandproofSourceEndAccessor'
OPTIONS = dict(parent.OPTIONS, roots=list(ROOTS), retained_roots=RETAINED_ROOTS)
LIBRARIES = parent.LIBRARIES
CONTINUATION_SOURCE = Path('controller-service/src/capture/continuation.rs')
STAGED = (*parent.STAGED, CONTINUATION_SOURCE)
VARIANTS = ('original', 'receipt-no-request-as-returned', 'swap-receipt-bindings')


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate_constructor(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown complete End accessor variant')
    start = text.index('    pub(crate) const fn from_ended_source(')
    stop = text.index('    #[must_use]', start)
    body = text[start:stop]
    original = '            end_receipt,\n'
    if body.count(original) != 1:
        raise ValueError('one exact complete receipt move in original constructor is required')
    if variant == 'swap-receipt-bindings':
        replacement = ('            end_receipt: CaptureWorkEndReceipt {\n'
            '                work_binding: end_receipt.end_binding,\n'
            '                end_binding: end_receipt.work_binding,\n'
            '                ..end_receipt\n'
            '            },\n')
        body = body.replace(original, replacement, 1)
    return text[:start]+body+text[stop:]


def reconstruct(archive, source, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown accessor input')
    try:
        parent.reconstruct(archive, source,
            'no-request-as-returned' if variant == 'receipt-no-request-as-returned' else 'original')
    finally:
        retire.STAGED = STAGED
    path = source/CONTINUATION_SOURCE
    path.write_text(mutate_constructor(path.read_text(), variant))


def admit(ir, generated, target):
    raw = json.loads(ir.read_bytes())
    tree = raw['translated']
    options = tree['options']
    if (raw.get('has_errors') is not False
            or any(options[name] is not False for name in
                ('skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or options['exclude'] != []
            or any(options[key] != OPTIONS[group] for key, group in
                (('start_from', 'roots'), ('include', 'include'), ('opaque', 'opaque')))
            or OPTIONS['roots'] != list(ROOTS)
            or OPTIONS['retained_roots'] != RETAINED_ROOTS
            or options['start_from_if_exists'] != OPTIONS['retained_roots']
            or {row['key'] for row in tree['target_information']} != {target}):
        raise ValueError('the exact eight roots, fixed target and original safe tool options are required')
    mapping = json.loads((generated/'translation.json').read_bytes())['functions']
    report = dict(roots=[], business={}, libraries=[])
    from prepare_transaction_capture import method
    for module, member in METHOD_ROOTS:
        root = module+'::*::'+member
        function = method(tree['fun_decls'], module, member)
        rows = [r for r in mapping if r['def_id'] == function['def_id']]
        if len(rows) != 1 or rows[0]['is_opaque'] or 'Structured' not in (function['body'] or {}):
            raise ValueError('entire original method must remain transparent: '+root)
        report['roots'].append(dict(selector=root, actual_name=function['item_meta']['name'], lean=rows[0]['lean_name'], def_id=function['def_id']))
    for name in (*parent.BUSINESS, *parent.TRANSPARENT_STANDARD):
        rows = [r for r in mapping if r['lean_name'] == NAMESPACE+'.'+name]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('all original receipt business and standard bodies are required: '+name)
        functions = [f for f in tree['fun_decls'] if f and f['def_id'] == rows[0]['def_id']]
        if len(functions) != 1 or 'Structured' not in (functions[0]['body'] or {}):
            raise ValueError('a complete original structured body is required: '+name)
        report['business'][name] = rows[0]['def_id']
    for row in mapping:
        if row['is_opaque']:
            if (row['source']['file'].startswith(('controller-service/', 'controller-core/', 'sandbox-protocol/'))
                    and row['lean_name'] not in LIBRARIES):
                raise ValueError('no extra owned opacity: '+row['lean_name'])
            report['libraries'].append(row['lean_name'])
    report['libraries'].sort()
    if len(report['libraries']) != 29 or set(report['libraries']) != LIBRARIES:
        raise ValueError('exact29 existing original receipt interfaces required: '+str(report['libraries']))
    return report


def capture(output, tools, lock, variant):
    retire.ROOT, retire.NAMESPACE, retire.OPTIONS, retire.STAGED = ROOT, NAMESPACE, OPTIONS, STAGED
    retire.reconstruct, retire.admit = reconstruct, admit
    try:
        retire.capture(output, tools, lock, variant)
    finally:
        path = output/'result.json'
        if path.is_file():
            row = json.loads(path.read_bytes())
            row.update(base_driver=row['driver'], driver=digest(Path(__file__)),
                parent_driver=digest(Path(parent.__file__)), capture_roots=list(ROOTS),
                direct_original_public_receipt=True, complete_constructor_fields=6,
                behavior_proved=False, mutation=None if variant == 'original' else variant)
            path.write_text(json.dumps(row, indent=2)+'\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--'+name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
