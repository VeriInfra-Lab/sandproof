"""Extend the original IR admission by the single real caller forwarding root."""
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('caller_original_ir_admission',
                                            HERE.parent/'inbox-record-validation/admission.py')
original = importlib.util.module_from_spec(spec)
spec.loader.exec_module(original)
ROOTS = [*original.ROOTS,
    'sandproof_controller_service::controller::lifecycle_inbox::recording::proof_protocol_record_using']
INCLUDE, OPAQUE = original.INCLUDE, original.OPAQUE
PRIMITIVE_ROOTS, PRIMITIVE_INCLUDE = original.PRIMITIVE_ROOTS, original.PRIMITIVE_INCLUDE
original.ROOTS = ROOTS


def roots(variant):
    if variant not in json.loads((HERE/'inventory.json').read_text())['views']:
        raise ValueError('unknown caller extraction variant')
    return ROOTS


def admit_ir(raw, primitive=False):
    links = original.admit_ir(raw, primitive=primitive, variant='correct')
    if primitive:
        return links
    functions = [f for f in raw['translated']['fun_decls'] if f]
    roots = [f for f in functions if f['item_meta']['name'][-1] ==
             {'Ident': ['proof_protocol_record_using', 0]}]
    if (len(roots) != 1 or roots[0]['item_meta']['opacity'] != 'Transparent'
            or 'Structured' not in roots[0]['body']):
        raise ValueError('missing transparent caller root')
    calls = []

    def visit(node):
        if isinstance(node, dict):
            if 'Call' in node:
                calls.append(node['Call']['call'])
            for value in node.values():
                visit(value)
        elif isinstance(node, list):
            for value in node:
                visit(value)

    visit(roots[0]['body'])
    if len(calls) != 1:
        raise ValueError('caller forwarding root must have exactly one call')
    callee_id = calls[0]['func']['Regular']['kind']['Fun']
    callee = next(f for f in functions if f['def_id'] == callee_id)
    expected = [{'Ident': [name, 0]} for name in (
        'sandproof_controller_service', 'controller', 'lifecycle_inbox', 'recording', 'record_using')]
    if (callee['item_meta']['name'] != expected
            or callee['item_meta']['opacity'] != 'Transparent'
            or 'Structured' not in callee['body']):
        raise ValueError('caller root does not call actual transparent record_using')
    return [*links, dict(root=roots[0]['item_meta']['name'], callee=callee['item_meta']['name'])]


def admit_generated(directory, variant='correct'):
    inventory = json.loads((HERE/'inventory.json').read_text())
    view = inventory['primitive'] if variant == 'primitive' else inventory['views'][variant]
    translation = json.loads((directory/'translation.json').read_text())
    functions = sorted(f['lean_name'] for f in translation['functions'] if not f['is_opaque'])
    external = sorted(f['lean_name'] for f in translation['functions'] if f['is_opaque'])
    if (functions != view.get('generated_functions', view.get('functions'))
            or external != view.get('external_functions', view.get('external'))):
        raise ValueError('caller generated symbol/opacity closure differs')
    namespace = 'SandproofStage1Graphic' if variant == 'primitive' else 'SandproofStage1Request'
    if any(f['lean_file'] != namespace+'/Funs.lean'
           for f in translation['functions'] if not f['is_opaque']):
        raise ValueError('generated body is outside the reviewed physical file')
    return translation
