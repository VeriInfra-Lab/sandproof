"""Admit the original full identity call, exact two inputs and original errors."""


def collect(node, key):
    result = []
    def visit(value):
        if isinstance(value, dict):
            if key in value:
                result.append(value[key])
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)
    visit(node)
    return result


def admit(raw, roots, includes, opaque):
    assert raw['has_errors'] is False
    translated = raw['translated']
    options = translated['options']
    assert options['start_from'] == roots and options['include'] == includes
    assert options['opaque'] == opaque
    assert not any(options[n] for n in ('exclude', 'skip_borrowck', 'no_typecheck',
        'monomorphize', 'remove_unused_clauses'))
    assert {x['key'] for x in translated['target_information']} == {'x86_64-unknown-linux-gnu'}
    functions = [f for f in translated['fun_decls'] if f]
    types = [t for t in translated['type_decls'] if t]
    def path(name):
        return [{'Ident': [n, 0]} for n in name.split('::')]
    def function(name):
        selected = [f for f in functions if f['item_meta']['name'] == path(name)]
        assert len(selected) == 1
        value = selected[0]
        assert value['item_meta']['opacity'] == 'Transparent' and 'Structured' in value['body']
        return value
    root = function(roots[0])
    actual = function('sandproof_controller_service::controller::identity::sandbox_id')
    assert root['body']['Structured']['locals']['arg_count'] == 2
    assert actual['body']['Structured']['locals']['arg_count'] == 2
    calls = collect(root['body'], 'Call')
    assignments = collect(root['body'], 'Assign')
    assert len(calls) == 1 and len(assignments) == 2
    call = calls[0]['call']
    assert call['func']['Regular']['kind'] == {'Fun': actual['def_id']}
    assert call['dest']['kind'] == {'Local': 0} and len(call['args']) == 2
    # Each original argument is forwarded once, in order, through the generated
    # move local. Checking the arity alone would miss swapped/dropped inputs.
    for index, argument in enumerate(call['args'], 1):
        selected = [(destination, expression) for destination, expression in assignments
            if destination['kind'] == argument['Move']['kind']]
        assert len(selected) == 1
        expression = selected[0][1]
        if index == 1:
            reference = expression['Ref']
            assert reference['kind'] == 'Shared'
            place, projection = reference['place']['kind']['Projection']
            assert projection == 'Deref' and place['kind'] == {'Local': 1}
        else:
            assert expression['Use'][0]['Copy']['kind'] == {'Local': index}
    hint = function('core::hint::must_use')
    assert hint['body']['Structured']['locals']['arg_count'] == 1
    hint_assignments = collect(hint['body'], 'Assign')
    assert len(hint_assignments) == 1 and not collect(hint['body'], 'Call')
    destination, expression = hint_assignments[0]
    assert destination['kind'] == {'Local': 0}
    assert expression['Use'][0]['Move']['kind'] == {'Local': 1}
    errors = {}
    for name in ['sandproof_protocol::ContractError', 'sandproof_controller_service::JournalError']:
        selected = [t for t in types if t['item_meta']['name'] == path(name)]
        assert len(selected) == 1 and selected[0]['item_meta']['opacity'] == 'Transparent'
        assert 'Enum' in selected[0]['kind']
        errors[name] = selected[0]
    return dict(root=root['def_id'], actual=actual['def_id'], full_input_call=call,
        two_original_arguments_forwarded_once_in_order=True, actual_business_transparent=True,
        actual_std_hint_transparent_identity=True, original_complete_errors=errors)
