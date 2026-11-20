"""Admit actual transparent roots, complete carriers and finite SQL usepoint."""


def collect(node, key):
    values = []
    def visit(value):
        if isinstance(value, dict):
            if key in value:
                values.append(value[key])
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)
    visit(node)
    return values


def admit(raw, roots, includes, opaque):
    assert raw['has_errors'] is False
    translated = raw['translated']
    options = translated['options']
    assert options['start_from'] == roots and options['include'] == includes and options['opaque'] == opaque
    assert not any(options[n] for n in ['exclude', 'skip_borrowck', 'no_typecheck',
        'monomorphize', 'remove_unused_clauses'])
    assert {x['key'] for x in translated['target_information']} == {'x86_64-unknown-linux-gnu'}
    functions = [x for x in translated['fun_decls'] if x]
    types = [x for x in translated['type_decls'] if x]
    def path(names):
        return [{'Ident':[n,0]} for n in names]
    prefix = ['sandproof_controller_service', 'controller', 'lifecycle_inbox', 'lifetime_sql']
    actual = next(x for x in functions if x['item_meta']['name'] == path(prefix+['execute']))
    assert actual['item_meta']['opacity'] == 'Transparent'
    assert actual['body']['Structured']['locals']['arg_count'] == 4
    admitted_roots = {}
    for name, arity in [('proof_execute',4), ('proof_original_integer_to_sql',1),
                        ('proof_original_text_to_sql',1)]:
        root = next(x for x in functions if x['item_meta']['name'] == path(prefix+[name]))
        assert root['item_meta']['opacity'] == 'Transparent'
        assert root['body']['Structured']['locals']['arg_count'] == arity
        calls = collect(root['body'], 'Call')
        assert len(calls) == 1
        call = calls[0]['call']
        assert call['dest']['kind'] == {'Local':0} and len(call['args']) == arity
        target = next(x for x in functions if x['def_id'] == call['func']['Regular']['kind']['Fun'])
        assert target['item_meta']['opacity'] == 'Transparent'
        if name == 'proof_execute':
            assert target['def_id'] == actual['def_id']
            assignments = collect(root['body'], 'Assign')
            assert len(assignments) == 4
            for index, (destination, expression) in enumerate(assignments, 1):
                assert call['args'][index-1]['Move']['kind'] == destination['kind']
                if index == 3:
                    assert expression['Use'][0]['Copy']['kind'] == {'Local':index}
                else:
                    reference = expression['Ref']
                    assert reference['kind'] == 'Shared'
                    place, dereference = reference['place']['kind']['Projection']
                    assert dereference == 'Deref' and place['kind'] == {'Local':index}
        else:
            assert target['item_meta']['name'][-1] == {'Ident':['to_sql',0]}
        admitted_roots[name] = dict(root=root['def_id'], target=target['def_id'], arity=arity)
    complete = {}
    for names, expected in [
        (['rusqlite','types','to_sql','ToSqlOutput'], ['Borrowed','Owned']),
        (['rusqlite','types','value_ref','ValueRef'], ['Null','Integer','Real','Text','Blob']),
        (['rusqlite','types','value','Value'], ['Null','Integer','Real','Text','Blob'])]:
        typ = next(x for x in types if x['item_meta']['name'] == path(names))
        assert typ['item_meta']['opacity'] == 'Transparent'
        assert [v['name'] for v in typ['kind']['Enum']] == expected
        assert [len(v['fields']) for v in typ['kind']['Enum']] == ([1,1] if len(expected)==2 else [0,1,1,1,1])
        complete[names[-1]] = typ
    journal = next(x for x in types if x['item_meta']['name'] == path(['sandproof_controller_service','JournalError']))
    assert journal['item_meta']['opacity'] == 'Transparent' and 'Enum' in journal['kind']
    return dict(actual_helper=actual['def_id'], roots=admitted_roots,
        complete_value_carriers=complete, complete_journal_error=journal,
        whole_original_dynamic_macro_translated=False, direct_runtime_binder_contract=False)
