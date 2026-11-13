"""Reject incomplete extraction, weakened options, or a substituted business root."""
from pathlib import Path
import json

HERE = Path(__file__).resolve().parent
PROTOCOL_ROOTS = ['crate::'+n for n in ('request_validate','replay_matches','accepted_validate',
    'recorded_matches','terminal_matches','renewal_valid','renewal_matches','renewal_confirmed_matches')]
ROOTS = PROTOCOL_ROOTS + ['sandproof_controller_service::controller::lifecycle_inbox::model::proof_inbox_validate_using']
PRIMITIVE_ROOTS = ['sandproof_controller_service::capture::proof_standard_graphic_bytes',
    'sandproof_controller_service::capture::proof_result_is_error']
COMMON_INCLUDE = ['sandproof_protocol','sandbox_controller_core','sandproof_controller_service',
    'core::num::_::is_ascii_graphic','core::ops::range::_::contains','core::ops::range::_::start_bound',
    'core::ops::range::_::end_bound','core::ops::range::RangeInclusive']
INCLUDE = COMMON_INCLUDE + ['core::option::*::and_then']
PRIMITIVE_INCLUDE = COMMON_INCLUDE + ['core::result::*::is_err']
OPAQUE = ['rusqlite::error::Error']


def admit_ir(raw, opaque=OPAQUE, primitive=False):
    if raw.get('has_errors') is not False:
        raise ValueError('incomplete Charon extraction')
    translated = raw['translated']; options = translated['options']
    if (options['start_from'] != (PRIMITIVE_ROOTS if primitive else ROOTS)
            or options['include'] != (PRIMITIVE_INCLUDE if primitive else INCLUDE)
            or opaque != OPAQUE or options['opaque'] != OPAQUE or options['exclude']
            or options['skip_borrowck'] or options['no_typecheck'] or options['monomorphize']):
        raise ValueError('unreviewed extraction options')
    if {t['key'] for t in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('wrong target')
    functions = [f for f in translated['fun_decls'] if f]
    def single_call(name, expected, prefix):
        roots = [f for f in functions if f['item_meta']['name'][-1] == {'Ident':[name,0]}]
        if len(roots)!=1 or roots[0]['item_meta']['opacity']!='Transparent' or 'Structured' not in roots[0]['body']:
            raise ValueError('missing transparent forwarding root')
        calls=[]
        def visit(node):
            if isinstance(node,dict):
                if 'Call' in node: calls.append(node['Call']['call'])
                for value in node.values(): visit(value)
            elif isinstance(node,list):
                for value in node: visit(value)
        visit(roots[0]['body'])
        if len(calls)!=1:
            raise ValueError('forwarding root must have exactly one actual call')
        callee_id=calls[0]['func']['Regular']['kind']['Fun']
        callee=next(f for f in functions if f['def_id']==callee_id)
        if (callee['item_meta']['name'][-1]!={'Ident':[expected,0]}
                or callee['item_meta']['name'][:len(prefix)]!=[{'Ident':[n,0]} for n in prefix]
                or callee['item_meta']['opacity']!='Transparent' or 'Structured' not in callee['body']):
            raise ValueError('forwarding root does not call the actual transparent method')
        return dict(root=roots[0]['item_meta']['name'],callee=callee['item_meta']['name'])
    if primitive:
        return [single_call('proof_standard_graphic_bytes','all_graphic',['sandproof_controller_service','capture']),
                single_call('proof_result_is_error','is_err',['core','result'])]
    return single_call('proof_inbox_validate_using','validate_using',
        ['sandproof_controller_service','controller','lifecycle_inbox','model'])


def admit_generated(directory, variant='correct'):
    translation=json.loads((directory/'translation.json').read_text())
    inventory=json.loads((HERE/'inventory.json').read_text())['views'][variant]
    actual=sorted(f['lean_name']for f in translation['functions']if not f['is_opaque'])
    if actual!=inventory['generated_functions']:
        raise ValueError('actual generated root/body missing, duplicated or changed')
    namespace='SandproofStage1Graphic' if variant=='primitive'else'SandproofStage1Request'
    if any(f['lean_file']!=namespace+'/Funs.lean' for f in translation['functions']if not f['is_opaque']):
        raise ValueError('actual generated body is in an unreviewed file')
    external=sorted(f['lean_name']for f in translation['functions']if f['is_opaque'])
    if external!=inventory['external_functions']:
        raise ValueError('unreviewed function model or business opacity')
    return translation
