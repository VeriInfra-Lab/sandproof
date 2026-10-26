"""Source assembly for the existing adjacent ExpectedSession representation."""
from binding import require, digest

EXPECTED = 'sandproof_protocol::lifecycle::identity::ExpectedSession'
TAG_OR_CONTENT = 'serde::private::de::content::TagOrContentField'


def check(original, report, schema, api):
    aggregate, field, call_at = api.aggregate, api.field, api.call_at
    guarded, variant_guard = api.guarded, api.variant_guard
    successful_call, typed_call = api.successful_call, api.typed_call
    RESULT, OPTION, CONTROL, DE = api.RESULT, api.OPTION, api.CONTROL, api.DE
    rows = report['functions']
    selected = [r for r in rows.values() if r['success_type'] == EXPECTED]
    seq, = [r for r in selected if r['method'] == 'visit_seq']
    mapping, = [r for r in selected if r['method'] == 'visit_map']
    direct, = [r for r in selected if r['method'] == 'deserialize' and
               len(original.functions[r['function_id']]['signature']['inputs']) == 1]
    seed, = [r for r in selected if r['method'] == 'deserialize' and
             len(original.functions[r['function_id']]['signature']['inputs']) == 2]
    require(schema['expected_session'] == dict(tag='state', content='version',
        variants={'Absent': None, 'Version': 'u64'}, rename_all='snake_case', deny_unknown_fields=True),
        'ExpectedSession differs from the existing complete representation')
    root_call, = [r for r in direct['rows'] if 'callee' in r]
    require(root_call['callee'] == DE + 'Deserializer::deserialize_struct' and
            root_call['arguments'][:2] == [{'input': True}, {'constant': 'ExpectedSession'}] and
            direct['exits'][0]['value'] == {'call_result': root_call['statement']},
            'ExpectedSession root does not return the original struct visitor')

    # The original seed selects Absent or Version from its own variant field.
    branches = [g for e in seed['exits'] for g in e['path']
                if g['subject'] == {'field': 'variant', 'of': {'visitor': True}}]
    require(branches and all(len(g['cases']) == 1 for g in branches), 'seed discriminator missing')
    enum = branches[0]['cases'][0]['enum']
    require(all(g['cases'][0]['enum'] == enum for g in branches), 'seed discriminator changes type')
    selectors = {r['method']: r for r in rows.values() if r['success_type'] == enum and
                 r['method'] in ('visit_str', 'visit_bytes', 'visit_u64')}
    require(set(selectors) == {'visit_str', 'visit_bytes', 'visit_u64'}, 'variant selector incomplete')
    variants = [('absent', 'unit'), ('version', 'u64')]
    api.selector(selectors['visit_str'], enum, variants)
    api.byte_selector(selectors['visit_bytes'], enum, variants)
    api.numeric_selector(selectors['visit_u64'], enum, variants)
    version_exit, = [e for e in seed['exits'] if set(e['value']) == {'call_result'}]
    version_map = call_at(seed, version_exit['value'])
    require(version_map['callee'] == 'core::result::<impl>::map' and version_map['destination'] == 0 and
            version_map['types'][0] == 'U64' and version_map['types'][2] == EXPECTED and
            variant_guard(version_map['path'], {'field': 'variant', 'of': {'visitor': True}}, enum, '__field1'),
            'Version does not map the original u64 result')
    read = call_at(seed, version_map['arguments'][0])
    require(read['callee'] == 'serde_core::de::impls::<impl>::deserialize' and
            read['arguments'] == [{'input': True}], 'Version uses another input reader')
    fn_id = read['original_call']['func']['Regular']['kind']['Fun']
    impl_id, = [p['Impl']['Trait'] for p in original.functions[fn_id]['item_meta']['name'] if 'Impl' in p]
    require(original.type_name(original.impls[impl_id]['impl_trait']['generics']['types'][0]) == 'U64',
            'Version strengthened to a positive counter or other value')
    constructor = version_map['arguments'][1]['original_constant']['FnDef']
    require(set(constructor['kind']) == {'Fun'}, 'Version constructor is not original')
    ctor = original.functions[constructor['kind']['Fun']]
    require(ctor['item_meta']['opacity'] == 'Transparent' and set(ctor['body']) == {'Structured'},
            'original Version constructor missing')
    statements = [s['kind'] for s in ctor['body']['Structured']['body']['statements']
                  if not isinstance(s['kind'], dict) or next(iter(s['kind'])) not in ('StorageLive', 'StorageDead')]
    require(len(statements) == 2 and statements[1] == 'Return', 'Version constructor does extra work')
    target, expression = statements[0]['Assign']
    require(target['kind'] == {'Local': 0} and original.assigned(expression, {1: {'input': True}}) ==
            dict(aggregate=EXPECTED, variant='Version', fields=[{'input': True}]),
            'Version constructor changes its original value')
    absent, = [e for e in seed['exits'] if aggregate(e['value'], RESULT, 'Ok')]
    failed, = [e for e in seed['exits'] if aggregate(e['value'], RESULT, 'Err')]
    require(absent['value']['fields'] == [dict(aggregate=EXPECTED, variant='Absent', fields=[])],
            'Absent seed constructs another variant')
    unit_result = field(failed['value']['fields'][0], 'Err._0')
    unit = call_at(seed, unit_result)
    require(unit['callee'] == DE + 'Deserializer::deserialize_any' and unit['arguments'][0] == {'input': True}
            and unit['types'][0] == 'serde::private::de::content::UntaggedUnitVisitor',
            'Absent does not consume the original unit content')
    require(variant_guard(absent['path'], unit_result, RESULT, 'Ok') and
            variant_guard(failed['path'], unit_result, RESULT, 'Err') and
            variant_guard(unit['path'], {'field': 'variant', 'of': {'visitor': True}}, enum, '__field0'),
            'Absent loses original unit result/error')

    def seed_value(value):
        require(isinstance(value.get('aggregate'), dict) and value['aggregate']['name'][-1] ==
                {'Ident': ['__Seed', 0]} and value['variant'] is None and len(value['fields']) == 3,
                'ExpectedSession uses another seed')
        seed_type = value['aggregate']
        input_type = original.resolve(original.functions[seed['function_id']]['signature']['inputs'][0], original.types)
        require(original.declaration_name(input_type['Adt']['id']) == seed_type, 'seed calls another implementation')
        return value['fields'][0]

    # Sequence returns the original seed value after tag and content success.
    normal, = [e for e in seq['exits'] if aggregate(e['value'], RESULT, 'Ok')]
    option = field(normal['value']['fields'][0], 'Some._0')
    require(variant_guard(normal['path'], option, OPTION, 'Some'), 'sequence content is absent')
    content = successful_call(seq, option, normal['path'])
    require(content['callee'] == DE + 'SeqAccess::next_element_seed' and
            content['arguments'][0] == {'borrow': 'TwoPhaseMut', 'origin': {'input': True}}, 'wrong sequence seed call')
    tag_option = field(seed_value(content['arguments'][1]), 'Some._0')
    tag = successful_call(seq, tag_option, content['path'])
    require(tag['callee'] == DE + 'SeqAccess::next_element' and
            variant_guard(content['path'], tag_option, OPTION, 'Some'), 'sequence seed loses original tag')

    # Map readers keep the same input. The standard tag-or-content seed gets
    # the actual two field names, independently of the order in the input.
    key_calls = [r for r in mapping['rows'] if r.get('callee') == DE + 'MapAccess::next_key_seed']
    require(len(key_calls) == 5, 'adjacent map key dispatch changed')
    for key in key_calls:
        require(key['arguments'] == [{'borrow': 'TwoPhaseMut', 'origin': {'input': True}},
            dict(aggregate='serde::private::de::content::TagOrContentFieldVisitor', variant=None,
                 fields=[{'constant': 'state'}, {'constant': 'version'}])], 'wrong actual tag/content field names')
    normal = [e for e in mapping['exits'] if aggregate(e['value'], RESULT, 'Ok')]
    require(len(normal) == 3, 'adjacent map success assembly changed')
    used = set()
    for e in normal:
        value, = e['value']['fields']
        if aggregate(value, EXPECTED, 'Absent'):
            require(value['fields'] == [] and any(g['cases'] == [dict(enum=enum, variant='__field0')]
                    for g in e['path']), 'tag-only map assembles a different variant')
            used.add('absent')
            continue
        content = successful_call(mapping, value, e['path'])
        if content['callee'] == DE + 'MapAccess::next_value_seed':
            require(content['arguments'][0] == {'borrow': 'TwoPhaseMut', 'origin': {'input': True}},
                    'tag-first path uses another map')
            tag_value = seed_value(content['arguments'][1])
            used.add('tag-first')
        else:
            require(content['original_call']['func']['Regular']['kind'] == {'Fun': seed['function_id']},
                    'buffered content calls another own seed')
            tag_value = seed_value(content['arguments'][0])
            buffered = call_at(mapping, content['arguments'][1])
            require(buffered['callee'] == 'serde::private::de::content::<impl>::new' and
                    buffered['types'] == content['types'][1:], 'buffered Error type changed')
            prior = successful_call(mapping, buffered['arguments'][0], buffered['path'])
            require(prior['callee'] == DE + 'MapAccess::next_value_seed' and prior['types'] == [
                'serde::private::de::content::ContentVisitor', 'serde_core::private::content::Content'] and
                prior['arguments'][0] == {'borrow': 'TwoPhaseMut', 'origin': {'input': True}},
                'buffered seed lost the original content')
            used.add('content-first')
        tag = successful_call(mapping, tag_value, content['path'])
        require(tag['callee'] == DE + 'MapAccess::next_value_seed' and
                tag['arguments'][0] == {'borrow': 'TwoPhaseMut', 'origin': {'input': True}} and
                tag['arguments'][1]['aggregate'] == 'serde::private::de::AdjacentlyTaggedEnumVariantSeed' and
                tag['arguments'][1]['fields'][0] == {'constant': 'ExpectedSession'},
                'seed lost the original adjacent tag')
        tail = [key for key in key_calls if any(g['subject'].get('field') == 'Continue._0' and
                g['cases'] == [dict(enum=OPTION, variant='None')] and
                call_at(mapping, field(g['subject'], 'Continue._0'))['arguments'] == [{'call_result': key['statement']}]
                for g in e['path'])]
        require(len(tail) == 1 and all(g in tail[0]['path'] for g in content['path']),
                'map returns before checking the remaining key')
    require(used == {'absent', 'tag-first', 'content-first'}, 'missing actual adjacent input path')
    missing_maps = [r for r in mapping['rows'] if r.get('callee') == 'core::result::<impl>::map']
    require(len(missing_maps) == 1 and missing_maps[0]['arguments'][1] == version_map['arguments'][1],
            'missing Version maps to a different constructor')
    missing = call_at(mapping, missing_maps[0]['arguments'][0])
    typed_call(missing, 'serde::private::de::missing_field', 'u64')
    require(missing['arguments'] == [{'constant': 'version'}], 'wrong adjacent missing content field')
    return dict(direct=direct['function_id'], seed=seed['function_id'], seq=seq['function_id'],
        map=mapping['function_id'], constructor_sha256=digest(ctor),
        actual_seed_paths=sorted(used), version_uses_plain_u64=True,
        source_binding_only=True, library_refinement_proved=False)
