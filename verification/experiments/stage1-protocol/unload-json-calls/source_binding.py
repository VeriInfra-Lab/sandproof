"""Admit actual JSON use sites; this is not a Rust interpreter or Serde proof."""
import importlib.util
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
STANDARD = HERE.parent / 'serde-binding'
sys.path.insert(0, str(STANDARD))
from binding import Original, digest, require  # noqa: E402

spec = importlib.util.spec_from_file_location('original_serde_source_check', STANDARD / 'check.py')
standard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(standard)

SERVICE = 'sandproof_controller_service::'
OPTIONS = SERVICE + 'host_contract::CreateOptions'
ANCHOR = SERVICE + 'controller::lifecycle_inbox::source_work::SourceAnchor'
SNAPSHOT = SERVICE + 'controller::lifecycle_inbox::source_work::snapshot::'
ROOTS = {
    SNAPSHOT + 'encode_source_options': OPTIONS,
    SNAPSHOT + 'encode_source_anchor': ANCHOR,
    SERVICE + 'host_contract::serialize_create_options': OPTIONS,
}
MUTATED_ROOTS = {
    'origin-request': SNAPSHOT + 'encode_source_anchor',
    'options-empty': SNAPSHOT + 'encode_source_options',
}
FIELDS = {
    OPTIONS: ['env_vars', 'metadata', 'networking', 'template'],
    ANCHOR: ['request', 'execution_origin', 'local', 'native', 'continued_from',
             'node', 'host_bootstrap', 'source', 'options_sha256', 'credential',
             'original_actions', 'original_receipts'],
}
RESULT = 'core::result::Result<alloc::vec::Vec<U8>,serde_json::error::Error>'
ORIGINAL_ARGUMENT = {'borrow': 'Shared', 'origin': standard.SELF}


def declarations(original):
    result = {}
    for item in original.type_decls.values():
        name = item['item_meta']['name']
        if all('Ident' in part for part in name):
            name = original.direct_name(name)
            if name in FIELDS:
                fields = item['kind']['Struct']
                require([field['name'] for field in fields] == FIELDS[name],
                        'actual complete field declaration changed: ' + name)
                result[name] = fields
    require(set(result) == set(FIELDS), 'missing actual complete type declaration')
    return result


def wrapper_body(original, function, target):
    signature = function['signature']
    require(signature['is_unsafe'] is False and signature['abi'] == 'Rust' and
            signature['is_variadic'] is False and
            [original.type_name(t) for t in signature['inputs']] == ['&' + target] and
            original.type_name(signature['output']) == RESULT, 'wrong original wrapper signature')
    require(function['item_meta']['opacity'] == 'Transparent' and
            set(function['body']) == {'Structured'}, 'missing actual wrapper body')
    return function['body']['Structured']


def whole_value_call(call, target, argument):
    require(call['callee'] == 'serde_json::ser::to_vec' and call['path'] == [] and
            call['arguments'] == [argument] and
            call['destination'] == {'Local': 0} and call['types'] == [target],
            'not a single whole-value to_vec returning the original result')
    dictionary, = call['dictionaries']
    require(dictionary['trait'] == 'serde_core::ser::Serialize' and
            dictionary['types'] == [target] and
            dictionary['implementation']['self_type'] == target, 'substituted Serialize dictionary')


def wrapper(original, function, target):
    body = wrapper_body(original, function, target)
    require(body['locals']['arg_count'] == 1 and
            len(body['locals']['locals']) == 3, 'extra wrapper state')
    statements = body['body']['statements']
    kinds = [statement['kind'] for statement in statements]
    require(len(kinds) == 7 and kinds[0] == {'StorageLive': 0} and
            kinds[1] == {'StorageLive': 2} and set(kinds[2]) == {'Assign'} and
            set(kinds[3]) == {'Call'} and kinds[4] == {'StorageDead': 2} and
            kinds[5] == {'StorageDead': 1} and kinds[6] == 'Return',
            'wrapper has additional effects, guards or result rewriting')
    destination, expression = kinds[2]['Assign']
    require(destination['kind'] == {'Local': 2} and set(expression) == {'Ref'},
            'wrapper must only borrow the original argument')
    reference = expression['Ref']
    require(reference['kind'] == 'Shared' and
            reference['place']['kind']['Projection'][0]['kind'] == {'Local': 1} and
            reference['place']['kind']['Projection'][1] == 'Deref', 'changed complete argument')
    call, = original.calls(body)
    whole_value_call(call, target, ORIGINAL_ARGUMENT)
    unwind = kinds[3]['Call']['on_unwind']
    require([statement['kind'] for statement in unwind['statements']] ==
            [{'StorageDead': 1}, 'UnwindResume'], 'changed unwind path')
    return dict(function_id=function['def_id'], function_sha256=digest(function), call=call)


def mutated_wrapper(original, function, target, variant):
    """Require the selected field edit on the clone actually passed to to_vec."""
    body = wrapper_body(original, function, target)
    require(body['locals']['arg_count'] == 1, 'wrong original mutant wrapper argument')
    calls = original.calls(body)
    require(len(calls) == 3 and all(call['path'] == [] for call in calls),
            'missing original clone, field edit, or serialization in selected wrapper')
    clone, edit, serialize = calls
    require(clone['callee'].endswith('::<impl>::clone') and clone.get('target_self_type') == target and
            clone['arguments'] == [ORIGINAL_ARGUMENT] and
            clone['destination'] == {'Local': 2}, 'mutant clone is not the full original value')
    cloned = {'call_result': clone['statement']}
    assignments = [statement['kind']['Assign'] for statement in body['body']['statements']
                   if isinstance(statement['kind'], dict) and 'Assign' in statement['kind']]
    writes = [(place, expression) for place, expression in assignments if 'Projection' in place['kind']]
    if variant == 'origin-request':
        request = {'field': 'request', 'of': cloned}
        require(edit['callee'] == 'sandproof_protocol::lifecycle::authority::<impl>::clone' and
                edit['arguments'] == [{'borrow': 'Shared', 'origin': request}] and
                edit['destination'] == {'Local': 4} and len(writes) == 1,
                'mutant must copy the request from the original anchor clone')
        place, expression = writes[0]
        edit_result = {'call_result': edit['statement']}
        aliases = {2: cloned, 4: edit_result}
        require(original.place(place, aliases) == {'field': 'execution_origin', 'of': cloned} and
                original.assigned(expression, aliases) == edit_result,
                'mutant did not replace the cloned execution_origin with request')
    else:
        metadata = {'field': 'metadata', 'of': cloned}
        require(edit['callee'] == 'alloc::collections::btree::map::<impl>::clear' and
                edit['arguments'] == [{'borrow': 'TwoPhaseMut', 'origin': metadata}] and
                edit['destination'] == {'Local': 4} and not writes,
                'mutant did not clear the cloned options metadata')
    whole_value_call(serialize, target, {'borrow': 'Shared', 'origin': cloned})
    return dict(function_id=function['def_id'], function_sha256=digest(function), call=serialize)


def inspect(raw, variant=None):
    original = Original(raw)
    require({row['key'] for row in original.data['target_information']} ==
            {'x86_64-unknown-linux-gnu'}, 'unreviewed extraction target')
    fields = declarations(original)
    changed = MUTATED_ROOTS[variant] if variant is not None else None
    roots = {}
    for function in original.functions.values():
        name = function['item_meta']['name']
        if all('Ident' in part for part in name):
            name = original.direct_name(name)
            if name in ROOTS:
                require(name not in roots, 'duplicate wrapper')
                if name == changed:
                    roots[name] = mutated_wrapper(original, function, ROOTS[name], variant)
                else:
                    roots[name] = wrapper(original, function, ROOTS[name])
    require(set(roots) == set(ROOTS), 'missing actual wrapper')
    call_ids = {row['call']['target_declaration_sha256'] for row in roots.values()}
    require(len(call_ids) == 1, 'wrappers do not use the same to_vec declaration')
    options = [row['call']['dictionaries'][0]['implementation']['id']
               for name, row in roots.items() if ROOTS[name] == OPTIONS]
    require(len(set(options)) == 1, 'stored and source options use different dictionaries')
    derives = {}
    for name, rows in fields.items():
        root = next(row for key, row in roots.items() if ROOTS[key] == name)
        impl_id = root['call']['dictionaries'][0]['implementation']['id']
        implementation = original.impls[impl_id]
        functions = [function for function in original.functions.values()
                     if function['item_meta']['name'][-1] == {'Ident': ['serialize', 0]} and
                     {'Impl': {'Trait': impl_id}} in function['item_meta']['name']]
        function, = functions
        require(function['item_meta']['opacity'] == 'Transparent' and
                set(function['body']) == {'Structured'}, 'original derive body missing')
        calls = original.calls(function['body']['Structured'])
        selected = [call for call in calls
                    if call['callee'] == standard.SER + 'SerializeStruct::serialize_field']
        seen = set()
        for call in selected:
            key = call['arguments'][1]['constant']
            require(key in FIELDS[name], 'unexpected serialized field')
            row = rows[FIELDS[name].index(key)]
            require(call['arguments'][2] == standard.original_field(key) and
                    call['types'] == [original.type_name(row['ty'])], 'field/value/type substitution')
            dictionary, = call['dictionaries']
            require(dictionary['trait'] == standard.SER + 'Serialize' and
                    dictionary['types'] == call['types'] and 'implementation' in dictionary,
                    'field Serialize dictionary substitution')
            seen.add(key)
        require(seen == set(FIELDS[name]), 'complete field omitted')
        if name == ANCHOR:
            standard.check_stages(calls, 'SourceAnchor', [
                (row['name'], standard.original_field(row['name']),
                 standard.short_type(original.type_name(row['ty']))) for row in rows], [])
        # Conditional CreateOptions derives have repeated branch bodies. Their
        # entire original AST is retained and compared below; this checker does
        # not interpret those conditions or prove Serde's general semantics.
        derives[name] = dict(function_id=function['def_id'], function_sha256=digest(function),
            implementation_sha256=digest(implementation), calls_sha256=digest(calls),
            field_calls=len(selected), original_fields=[
                [row['name'], original.type_name(row['ty'])] for row in rows],
            ordered_error_paths_checked=name == ANCHOR)
    return dict(kind='unload-json-call-source-admission', roots=roots, derives=derives,
                library_refinement_proved=False, actual_wrapper_lean_proved=False)


def check_configuration(raw, profile):
    options = raw['translated']['options']
    require(options['preset'] == 'Aeneas' and options['sysroot'] == 'default' and
            options['monomorphize'] is False and options['exclude'] == [] and
            all(options[key] == profile['configuration'][name] for key, name in
                [('start_from', 'roots'), ('include', 'include'), ('opaque', 'opaque')]),
            'extraction selectors/configuration changed')


def check(raw, profile):
    check_configuration(raw, profile)
    result = inspect(raw)
    require(result == profile['original_calls'], 'complete reviewed calls/derives changed')
    return result


def check_rejection(raw, profile, variant):
    check_configuration(raw, profile)
    result = inspect(raw, variant)
    result['rejected_wrapper'] = MUTATED_ROOTS[variant]
    return result


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = check(json.loads(args.llbc.read_text()), json.loads((HERE / 'profile.json').read_text()))
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    print('Three actual calls and two complete derives admitted; no new Lean theorem.')
