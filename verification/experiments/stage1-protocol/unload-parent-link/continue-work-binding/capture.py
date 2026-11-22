"""Precheck the real binding root; new interfaces remain unadopted and rejected."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


parent = module('continue_binding_parent', HERE.parent / 'source-end-accessor/capture.py')
ports = module('continue_binding_candidate_ports', HERE / 'ports.py')
retire = parent.retire
MODULE = parent.CONTINUATION
TYPE = MODULE + '::OriginalContinueWorkRequest'
ROOT = TYPE + '::binding_digest'
ROOTS = (ROOT,)
METHOD_ROOTS = (*parent.METHOD_ROOTS, (MODULE, 'binding_digest'))
RETAINED_ROOTS = list(dict.fromkeys((*parent.OPTIONS['retained_roots'], *parent.ROOTS,
                                     MODULE + '::*::binding_digest')))
NAMESPACE = 'SandproofContinueWorkBinding'
OPTIONS = dict(parent.OPTIONS, roots=list(ROOTS), retained_roots=RETAINED_ROOTS,
               opaque=[*parent.OPTIONS['opaque'], *ports.PORTS])
STAGED = parent.STAGED
VARIANTS = ports.VARIANTS
CANDIDATES = {'capture.continuation.encode_continue_work_binding',
              'capture.continuation.digest_continue_work_binding'}


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def reconstruct(archive, source, variant):
    try:
        parent.reconstruct(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / ports.SOURCE
    path.write_text(ports.mutate(path.read_text(), variant))
    if ports.stage(source) != ports.SOURCE or ports.SOURCE not in STAGED:
        raise ValueError('only the existing continuation staged file may be extended')


def candidate(ir, generated, target):
    raw = json.loads(ir.read_bytes())
    tree = raw['translated']
    options = tree['options']
    if (raw.get('has_errors') is not False
            or any(options[n] is not False for n in ('skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or options['exclude'] != [] or {r['key'] for r in tree['target_information']} != {target}
            or any(options[k] != OPTIONS[v] for k,v in (('start_from','roots'),('include','include'),('opaque','opaque')))
            or options['start_from_if_exists'] != RETAINED_ROOTS):
        raise ValueError('exact binding root, retained parent and safe fixed options required')
    from prepare_transaction_capture import method
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    report = dict(roots=[], business={}, libraries=[], interface_adoption_complete=False)
    ordered = {item for group in tree['ordered_decls'] if 'Fun' in group
               for item in ([group['Fun']['NonRec']] if 'NonRec' in group['Fun'] else group['Fun']['Rec'])}
    for owner, member in METHOD_ROOTS:
        fn = method(tree['fun_decls'], owner, member)
        if fn['def_id'] not in ordered:
            raise ValueError('original method missing from ordered actual extraction: ' + member)
        rows = [r for r in mapping if r['def_id'] == fn['def_id']]
        if len(rows) != 1:
            raise ValueError('one actual generated mapping required for original method: ' + member)
        if rows[0]['is_opaque'] or 'Structured' not in (fn['body'] or {}):
            raise ValueError('all original business methods must remain transparent: ' + member)
        report['roots'].append(dict(member=member, lean=rows[0]['lean_name'], def_id=fn['def_id']))
    for name in (*parent.parent.BUSINESS, *parent.parent.TRANSPARENT_STANDARD):
        rows = [r for r in mapping if r['lean_name'] == NAMESPACE + '.' + name]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('the original complete parent body is mandatory: ' + name)
        fn = [f for f in tree['fun_decls'] if f and f['def_id'] == rows[0]['def_id']]
        if len(fn) != 1 or 'Structured' not in (fn[0]['body'] or {}):
            raise ValueError('the complete structured original parent body is mandatory: ' + name)
        report['business'][name] = rows[0]['def_id']
    wanted = parent.LIBRARIES | CANDIDATES
    for row in mapping:
        if row['is_opaque']:
            if row['source']['file'].startswith(('controller-service/', 'controller-core/', 'sandbox-protocol/')) and row['lean_name'] not in wanted:
                raise ValueError('no whole business opacity permitted: ' + row['lean_name'])
            report['libraries'].append(row['lean_name'])
    report['libraries'].sort()
    if len(report['libraries']) != 31 or set(report['libraries']) != wanted:
        raise ValueError('exact old29 and two candidate thin interfaces required')
    report['candidate_interfaces'] = sorted(CANDIDATES)
    return report


def admit(ir, generated, target):
    candidate(ir, generated, target)
    raise ValueError('old29 admission retained: two new concrete JSON/SHA uses remain unadopted; raw precheck only')


def capture(output, tools, lock, variant):
    retire.ROOT, retire.NAMESPACE, retire.OPTIONS, retire.STAGED = ROOT, NAMESPACE, OPTIONS, STAGED
    retire.reconstruct, retire.admit = reconstruct, admit
    try:
        retire.capture(output, tools, lock, variant)
    finally:
        path = output / 'result.json'
        if path.is_file():
            row = json.loads(path.read_bytes())
            row.update(base_driver=row['driver'], driver=digest(Path(__file__)),
                       parent_driver=digest(Path(parent.__file__)), candidate_ports_driver=digest(HERE/'ports.py'),
                       capture_roots=list(ROOTS), complete_binding_fields=6, behavior_proved=False,
                       interface_adoption_complete=False, public_acceptance=False)
            if all((output/n).exists() for n in ('actual.llbc','generated/translation.json')):
                fixed=json.loads((retire.REPO/'verification/foundations-environment.json').read_bytes())
                try:
                    row['candidate_mapping'] = candidate(output/'actual.llbc', output/'generated', fixed['target'])
                except ValueError as error:
                    row['candidate_mapping_error'] = str(error)
            path.write_text(json.dumps(row,indent=2)+'\n')


if __name__ == '__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True)
    for name in ('output','tools','lock'):
        cli.add_argument('--'+name,type=Path,required=True)
    a=cli.parse_args()
    capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
