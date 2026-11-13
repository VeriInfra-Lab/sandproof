"""Admit the exact forwarding call and actual generated allocation body."""
import re

PREFIX = ['sandproof_controller_service', 'controller', 'lifecycle_inbox']


def admit_ir(raw):
    functions = [f for f in raw['translated']['fun_decls'] if f]

    def select(name):
        path = [{'Ident': [n, 0]} for n in [*PREFIX, name]]
        found = [f for f in functions if f['item_meta']['name'] == path]
        assert len(found) == 1
        value = found[0]
        assert value['item_meta']['opacity'] == 'Transparent' and 'Structured' in value['body']
        return value

    root = select('proof_owner_for')
    actual = select('owner_for')
    calls = []
    assignments = []

    def walk(value):
        if isinstance(value, dict):
            if 'Call' in value:
                calls.append(value['Call']['call'])
            if 'Assign' in value:
                assignments.append(value['Assign'])
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(root['body'])
    assert len(calls) == 1
    assert calls[0]['func']['Regular']['kind']['Fun'] == actual['def_id']
    assert root['body']['Structured']['locals']['arg_count'] == 3
    assert len(calls[0]['args']) == 3 and calls[0]['dest']['kind'] == {'Local': 0}
    assert len(assignments) == 3
    for argument, assignment, original in zip(calls[0]['args'], assignments, [1, 2, 3]):
        destination, expression = assignment
        assert argument['Move']['kind'] == destination['kind']
        assert expression['Ref']['kind'] == 'Shared'
        projection = expression['Ref']['place']['kind']['Projection']
        assert projection[0]['kind'] == {'Local': original} and projection[1] == 'Deref'
    return dict(root=root['def_id'], actual=actual['def_id'], single_actual_call=True,
                original_argument_order_and_shared_borrows=True)


def admit_generated(translation, generated):
    name = 'SandproofStage1OwnerMappingRust.' + '.'.join([*PREFIX, 'owner_for'])
    found = [f for f in translation['functions'] if f['lean_name'] == name]
    assert len(found) == 1 and found[0]['is_opaque'] is False
    assert found[0]['lean_file'] == 'SandproofStage1OwnerMapping/Funs.lean'
    text = (generated / found[0]['lean_file']).read_text()
    namespace = 'SandproofStage1OwnerMappingRust'
    assert len(re.findall(r'^namespace ' + namespace + r'$', text, re.M)) == 1
    assert len(re.findall(r'^end ' + namespace + r'$', text, re.M)) == 1
    within = text.split('namespace ' + namespace + '\n', 1)[1].rsplit('end ' + namespace, 1)[0]
    short = name.removeprefix(namespace + '.')
    assert len(re.findall(r'^def\s+' + re.escape(short) + r'\b', within, re.M)) == 1
    assert found[0]['rust_name'] == 'sandproof_controller_service::controller::lifecycle_inbox::owner_for'
    external = [f['lean_name'] for f in translation['functions'] if f['is_opaque']]
    assert external == ['alloc.string.String.Insts.CoreCmpPartialEqString.eq']
    return dict(actual=name, generated_body_present=True, external_functions=external)


def regression_checks(raw, translation, generated):
    """Reject omitted/opaque actual roots and a reordered original call."""
    from copy import deepcopy
    rejected = []
    for case in ('missing-root', 'opaque-root', 'wrong-rust-root'):
        changed = deepcopy(translation)
        name = 'SandproofStage1OwnerMappingRust.' + '.'.join([*PREFIX, 'owner_for'])
        if case == 'missing-root':
            changed['functions'] = [f for f in changed['functions'] if f['lean_name'] != name]
        else:
            actual = next(f for f in changed['functions'] if f['lean_name'] == name)
            if case == 'opaque-root': actual['is_opaque'] = True
            else: actual['rust_name'] = 'wrong::owner_for'
        try:
            admit_generated(changed, generated)
        except AssertionError:
            rejected.append(case)
        else:
            raise ValueError('admission accepted ' + case)
    changed = deepcopy(raw)
    root = next(f for f in changed['translated']['fun_decls'] if f and
        f['item_meta']['name'][-1] == {'Ident': ['proof_owner_for', 0]})
    def swap(value):
        if isinstance(value, dict):
            if 'Call' in value:
                arguments = value['Call']['call']['args']
                arguments[0], arguments[1] = arguments[1], arguments[0]
            for child in value.values(): swap(child)
        elif isinstance(value, list):
            for child in value: swap(child)
    swap(root['body'])
    try:
        admit_ir(changed)
    except AssertionError:
        rejected.append('reordered-original-arguments')
    else:
        raise ValueError('admission accepted reordered arguments')
    return rejected
