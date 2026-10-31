"""Check full struct/operation assembly against the existing field inventory.

Source admission only. It does not establish the concrete parser contract or
prove the semantics of the library callbacks, cleanup, or raw byte scanner.
"""
import json
from pathlib import Path
from types import SimpleNamespace
from binding import Original, functions, require, same

DE = 'serde_core::de::'
RESULT = 'core::result::Result'
OPTION = 'core::option::Option'
CONTROL = 'core::ops::control_flow::ControlFlow'


def aggregate(value, name, variant):
    return (isinstance(value, dict) and value.get('aggregate') == name and
            value.get('variant') == variant)


def field(value, name):
    require(set(value) == {'field', 'of'} and value['field'] == name,
            'wrong original field projection: ' + name)
    return value['of']


def guarded(path, subject, label):
    return any(same(g['subject'], subject) and (same(g['cases'], [label]) or
        (same(label, {'constant': True}) and same(g['except_cases'], [{'constant': False}]))) for g in path)


def variant_guard(path, subject, enum, variant):
    return guarded(path, subject, dict(enum=enum, variant=variant))


def call_at(report, reference):
    require(set(reference) == {'call_result'}, 'field did not come from an original call')
    candidates = [r for r in report['rows'] if r['statement'] == reference['call_result'] and 'callee' in r]
    require(candidates and all(c == candidates[0] for c in candidates), 'ambiguous original call')
    return candidates[0]


def short_type(name):
    if name.startswith('alloc::vec::Vec<'):
        return name.removeprefix('alloc::vec::').lower().replace('vec', 'Vec')
    if name in ('U8', 'U16', 'U32', 'U64', 'I64', '[U8;32]'):
        return name.lower()
    return 'bool' if name == 'Bool' else name.rsplit('::', 1)[-1]


def typed_call(call, name, ty):
    missing = name == 'serde::private::de::missing_field'
    require(call['callee'] == name and len(call['types']) == (2 if missing else 1) and
            short_type(call['types'][0]) == ty,
            'wrong original field reader/type: ' + name + '/' + ty)
    require(len(call['dictionaries']) == (2 if missing else 1), 'wrong field dictionary count')
    dictionary = call['dictionaries'][0]
    require(dictionary['trait'] == DE + 'Deserialize' and dictionary['types'] == call['types'][:1] and
            'implementation' in dictionary, 'substituted original Deserialize dictionary')
    if missing:
        error = call['dictionaries'][1]
        require(error['trait'] == DE + 'Error' and error['types'] == call['types'][1:],
                'wrong missing-field Error dictionary')


def successful_call(report, value, path):
    result = field(value, 'Continue._0')
    branch = call_at(report, result)
    require(branch['callee'] == 'core::result::<impl>::branch' and len(branch['arguments']) == 1,
            'missing original Result branching')
    require(variant_guard(path, result, CONTROL, 'Continue'), 'field read without original success guard')
    call = call_at(report, branch['arguments'][0])
    errors = [r for r in report['rows'] if r.get('callee') == 'core::result::<impl>::from_residual' and
              r['arguments'] == [{'field': 'Break._0', 'of': result}]]
    require(len(errors) == 1 and errors[0]['destination'] == 0 and
            variant_guard(errors[0]['path'], result, CONTROL, 'Break'), 'original field error not returned')
    error = errors[0]
    require(len(error['types']) == 3 and error['types'][1] == error['types'][2],
            'field error converted to another type')
    dictionary, = error['dictionaries']
    require(dictionary['trait'] == 'core::convert::From' and
            dictionary['types'] == [error['types'][1]] * 2 and
            dictionary['implementation']['source_name'][:2] == [
                {'Ident': ['core', 0]}, {'Ident': ['convert', 0]}], 'unreviewed field error conversion')
    require(any(e['value'] == {'call_result': error['statement']} for e in report['exits']),
            'original field error overwritten before return')
    return call


def selector(report, enum, fields):
    selected = {}
    for e in report['exits']:
        value = e['value']
        if not aggregate(value, RESULT, 'Ok'):
            continue
        value, = value['fields']
        require(value['aggregate'] == enum and value['fields'] == [], 'wrong map-key result type')
        matched = []
        for call in report['rows']:
            if call.get('callee') != 'core::str::traits::<impl>::eq':
                continue
            if guarded(e['path'], {'call_result': call['statement']}, {'constant': True}):
                require(call['arguments'][0] == {'borrow': 'Shared', 'origin': {'deref': {'input': True}}},
                        'field selector compares a different input')
                name = call['arguments'][1]
                require(set(name) == {'constant'} and isinstance(name['constant'], str), 'nonliteral field name')
                matched.append(name['constant'])
        require(len(matched) == 1 and matched[0] not in selected, 'ambiguous original field selector')
        selected[matched[0]] = value['variant']
    require(selected == {name: '__field' + str(i) for i, (name, _) in enumerate(fields)},
            'original input field name routed to another field')
    return selected


def byte_selector(report, enum, fields):
    successful = [e for e in report['exits'] if aggregate(e['value'], RESULT, 'Ok')]
    require(len(successful) == len(fields), 'missing/extra byte field selector')
    seen = set()
    for e in successful:
        value, = e['value']['fields']
        require(value['aggregate'] == enum and value['fields'] == [], 'wrong byte field result type')
        names = [(i, name) for i, (name, _) in enumerate(fields) if value['variant'] == '__field' + str(i)]
        require(len(names) == 1, 'unregistered byte selector result')
        index, name = names[0]
        require(index not in seen, 'duplicate byte field result')
        seen.add(index)
        encoded = name.encode('utf-8')
        length = {'original_eq': [{'ptr_metadata': {'input': True}}, {'constant': len(encoded)}]}
        require(guarded(e['path'], length, {'constant': True}), 'byte field lacks exact original slice length')
        matched = {}
        for guard in e['path']:
            subject = guard['subject']
            if not isinstance(subject, dict) or set(subject) != {'deref'}:
                continue
            call = call_at(report, subject['deref'])
            require(call['callee'] == 'core::slice::index::<impl>::index' and call['types'] == ['U8'] and
                    call['dictionaries'] == [] and len(call['arguments']) == 2 and
                    call['arguments'][1] == {'borrow': 'Shared', 'origin': {'deref': {'input': True}}},
                    'byte selector indexes a different slice')
            position = call['arguments'][0]
            require(set(position) == {'constant'} and type(position['constant']) is int,
                    'byte selector uses a nonliteral offset')
            offset = position['constant']
            if guard['cases']:
                require(len(guard['cases']) == 1 and set(guard['cases'][0]) == {'constant'} and
                        type(guard['cases'][0]['constant']) is int, 'byte selector has a nonliteral comparison')
                matched[offset] = guard['cases'][0]['constant']
        require(matched == dict(enumerate(encoded)), 'original byte field name routed to another field')
    return dict(fields=len(fields), exact_slice_length=True, all_bytes=True)


def numeric_selector(report, enum, fields):
    successful = [e for e in report['exits'] if aggregate(e['value'], RESULT, 'Ok')]
    require(len(successful) == len(fields), 'missing/extra numeric field selector')
    seen = set()
    for e in successful:
        value, = e['value']['fields']
        require(value['aggregate'] == enum and value['fields'] == [], 'wrong numeric field result type')
        indices = [i for i in range(len(fields)) if value['variant'] == '__field' + str(i)]
        require(len(indices) == 1 and indices[0] not in seen, 'unregistered numeric field selector')
        index, = indices
        seen.add(index)
        require(guarded(e['path'], {'input': True}, {'constant': index}),
                'original numeric field routed to another field')
    return dict(fields=len(fields), original_positions=True)


def seq(report, fields, exits):
    require(not report['iterations'] and len(exits) == 1, 'unexpected sequence control flow')
    e, = exits
    values = e['value']['fields'][0]['fields']
    require(len(values) == len(fields), 'incomplete sequence assembly')
    reads = []
    for index, (value, (_, ty)) in enumerate(zip(values, fields)):
        option = field(value, 'Some._0')
        require(variant_guard(e['path'], option, OPTION, 'Some'), 'sequence value used before Some')
        call = successful_call(report, option, e['path'])
        typed_call(call, DE + 'SeqAccess::next_element', ty)
        require(call['arguments'] == [{'borrow': 'TwoPhaseMut', 'origin': {'input': True}}],
                'sequence reader changed')
        for prior in reads:
            require(all(g in call['path'] for g in prior), 'sequence reads reordered')
        reads.append([g for g in e['path'] if g['subject'] in (option, field(option, 'Continue._0'))])
        missing = [r for r in report['rows'] if r.get('callee') == DE + 'Error::invalid_length' and
                   r['arguments'][0] == {'constant': index} and variant_guard(r['path'], option, OPTION, 'None')]
        require(len(missing) == 1, 'sequence missing value has wrong rejection index')
    require(len([r for r in report['rows'] if r.get('callee') == DE + 'SeqAccess::next_element']) == len(fields),
            'missing/extra original sequence field read')
    return dict(fields=len(fields), original_reader=True, declaration_order=True)


def mapping(report, fields, exits, all_reports, original):
    keys = [r for r in report['rows'] if r.get('callee') == DE + 'MapAccess::next_key']
    require(len(keys) == 1, 'missing/extra map key reader')
    key, = keys
    require(key['arguments'] == [{'borrow': 'TwoPhaseMut', 'origin': {'input': True}}], 'map key reader changed')
    key_ty, = key['original_call']['func']['Regular']['generics']['types']
    enum = original.declaration_name(original.resolve(key_ty, original.types)['Adt']['id'])
    selectors = [r for r in all_reports.values() if r['success_type'] == enum and
                 r['method'] in ('visit_str', 'visit_bytes', 'visit_u64')]
    require(len(selectors) == 3 and {r['method'] for r in selectors} == {'visit_str', 'visit_bytes', 'visit_u64'},
            'missing/ambiguous original key visitor')
    selected = {r['method']: r for r in selectors}
    selector(selected['visit_str'], enum, fields)
    byte_selector(selected['visit_bytes'], enum, fields)
    numeric_selector(selected['visit_u64'], enum, fields)
    if not fields:
        require(not report['iterations'] and len(exits) == 1 and
                exits[0]['value']['fields'][0]['fields'] == [], 'wrong empty variant assembly')
        require(original.type_decls[enum['original_type']]['kind'] == {'Enum': []},
                'empty variant key type is inhabited')
        require(not any(r.get('callee') == DE + 'MapAccess::next_value' for r in report['rows']),
                'empty variant unexpectedly reads a value')
        return dict(fields=0, empty_variant=True, original_uninhabited_key=True)
    loops = [r['loop_slots'] for r in report['rows'] if 'loop_slots' in r]
    require(len(loops) == 1, 'expected one original map loop')
    slots = {k for k, v in loops[0].items() if aggregate(v, OPTION, 'None')}
    require(len(slots) == len(fields), 'missing/extra original field slot')
    full = [e for e in exits if all(set(v) == {'field', 'of'} and v['field'] == 'Some._0' and
            set(v['of']) == {'loop_slot'} for v in e['value']['fields'][0]['fields'])]
    require(len(full) == 1, 'complete original map assembly missing')
    selected_slots = [field(v, 'Some._0')['loop_slot'] for v in full[0]['value']['fields'][0]['fields']]
    require(len(selected_slots) == len(fields) and {str(s) for s in selected_slots} == slots,
            'map assembly duplicates or omits an original field slot')
    key_branch = [r for r in report['rows'] if r.get('callee') == 'core::result::<impl>::branch' and
                  r['arguments'] == [{'call_result': key['statement']}]]
    require(len(key_branch) == 1, 'key does not use original Result branching')
    key_value = {'field': 'Some._0', 'of': {'field': 'Continue._0',
                 'of': {'call_result': key_branch[0]['statement']}}}
    enums = [g['cases'][0]['enum'] for r in report['rows']
             if r.get('callee') == DE + 'MapAccess::next_value' for g in r['path']
             if g['subject'] == key_value and len(g['cases']) == 1]
    require(enums and all(e == enums[0] for e in enums), 'map key discriminator changed')
    require(enums[0] == enum, 'map dispatch differs from original key decoder type')
    require(len(report['iterations']) == len(fields), 'missing/extra original map iteration')
    readers = set()
    for index, ((name, ty), slot) in enumerate(zip(fields, selected_slots)):
        stored = [e for e in report['iterations'] if aggregate(e['slots'][str(slot)], OPTION, 'Some')]
        require(len(stored) == 1, 'field slot written more than once per iteration')
        stored, = stored
        for other in slots - {str(slot)}:
            require(stored['slots'][other] == {'loop_slot': int(other)}, 'iteration overwrites another field')
        call = successful_call(report, stored['slots'][str(slot)]['fields'][0], stored['path'])
        readers.add(call['statement'])
        typed_call(call, DE + 'MapAccess::next_value', ty)
        require(call['arguments'] == [{'borrow': 'TwoPhaseMut', 'origin': {'input': True}}], 'map reader changed')
        require(variant_guard(call['path'], key_value, enum, '__field' + str(index)),
                'original input field name routed to another field')
        duplicate_checks = [r for r in report['rows'] if r.get('callee') == 'core::option::<impl>::is_some' and
                            r['arguments'] == [{'borrow': 'Shared', 'origin': {'loop_slot': slot}}]]
        require(len(duplicate_checks) == 1, 'missing/extra duplicate check')
        duplicate, = duplicate_checks
        require(guarded(call['path'], {'call_result': duplicate['statement']}, {'constant': False}),
                'value consumed before duplicate rejection')
        errors = [r for r in report['rows'] if r.get('callee') == DE + 'Error::duplicate_field' and
                  r['arguments'] == [{'constant': name}]]
        require(len(errors) == 1 and guarded(errors[0]['path'], {'call_result': duplicate['statement']},
                {'constant': True}), 'wrong duplicate-field rejection')
        require(any(aggregate(e['value'], RESULT, 'Err') and
                    e['value']['fields'] == [{'call_result': errors[0]['statement']}] for e in report['exits']),
                'duplicate-field error overwritten')
        for e in exits:
            values = e['value']['fields'][0]['fields']
            require(len(values) == len(fields), 'incomplete map assembly')
            value = values[index]
            if value == {'field': 'Some._0', 'of': {'loop_slot': slot}}:
                require(variant_guard(e['path'], {'loop_slot': slot}, OPTION, 'Some'), 'unguarded map slot')
            else:
                call = successful_call(report, value, e['path'])
                typed_call(call, 'serde::private::de::missing_field', ty)
                require(call['arguments'] == [{'constant': name}] and
                        variant_guard(call['path'], {'loop_slot': slot}, OPTION, 'None'),
                        'wrong missing-field fallback')
    require(readers == {r['statement'] for r in report['rows'] if r.get('callee') == DE + 'MapAccess::next_value'},
            'missing/extra original map field read')
    return dict(fields=len(fields), original_reader=True, string_key_routes=True,
                duplicate_before_value=True, success_assemblies=len(exits))


def check(raw, schema):
    original = Original(raw)
    report = functions(raw)
    structs = schema['structs']
    expected = {(name, None): data['fields_in_declaration_order'] for name, data in structs.items()}
    expected.update({('LifecycleOperation', v): fields for v, fields in schema['operations'].items()})
    roots = [f['item_meta']['name'][-1]['Ident'][0] for f in original.functions.values()
             if len(f['item_meta']['name']) == 2 and
             f['item_meta']['name'][0] == {'Ident': ['sandproof_envelope_bytes_check', 0]} and
             f['item_meta']['name'][-1] in ({'Ident': ['accepted_decode', 0]}, {'Ident': ['recorded_decode', 0]})]
    require(len(roots) == 1, 'missing/ambiguous original decoder root')
    if roots == ['accepted_decode']:
        del expected['RecordedLifecycleIntent', None]
    found = {}
    for fid, data in report['functions'].items():
        mode = original.functions[fid]['item_meta']['name'][-1]['Ident'][0]
        if mode not in ('visit_seq', 'visit_map'):
            continue
        successes = [e for e in data['exits'] if aggregate(e['value'], RESULT, 'Ok')]
        if not successes:
            continue
        target = successes[0]['value']['fields'][0]
        if not isinstance(target.get('aggregate'), str):
            continue
        key = (target['aggregate'].rsplit('::', 1)[-1], target['variant'])
        if key not in expected:
            continue  # ExpectedSession special dispatch is retained, not certified here.
        require(all(e['value']['fields'][0]['aggregate'] == target['aggregate'] and
                    e['value']['fields'][0]['variant'] == target['variant'] for e in successes),
                'visitor returns a different original variant')
        full_key = (*key, mode)
        require(full_key not in found, 'duplicate original assembly visitor')
        fields = expected[key]
        result = seq(data, fields, successes) if mode == 'visit_seq' else mapping(
            data, fields, successes, report['functions'], original)
        found[full_key] = dict(function_id=fid, **result)
    require(set(found) == {(*key, mode) for key in expected for mode in ('visit_seq', 'visit_map')},
            'missing original struct/operation assembly visitor')
    report['checked'] = {'::'.join(str(v) for v in key): value for key, value in found.items()}
    import expected as adjacent
    api = SimpleNamespace(**{
        name: globals()[name] for name in ('aggregate', 'field', 'call_at', 'guarded', 'variant_guard',
            'successful_call', 'typed_call', 'selector', 'byte_selector', 'numeric_selector',
            'RESULT', 'OPTION', 'CONTROL', 'DE')})
    report['expected_session'] = adjacent.check(original, report, schema, api)
    import leaves
    report['leaf_forwarding'] = leaves.check(original, report, api)
    import operation
    report['operation_dispatch'] = operation.check(original, report, schema, api)
    report['byte_key_matching_checked'] = True
    report['expected_session_dispatch_checked'] = True
    return report


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[4]
    schema = json.loads((root / 'verification/experiments/stage1-protocol/envelope-schema/inventory.json').read_text())
    result = check(json.loads(args.llbc.read_text()), schema)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
