"""Bind original operation tags to the already checked branch visitors."""
from binding import require


def check(original, report, schema, api):
    name = 'sandproof_protocol::lifecycle::request::LifecycleOperation'
    body, = [r for r in report['functions'].values() if r['method'] == 'deserialize' and r['success_type'] == name]
    calls = [r for r in body['rows'] if 'callee' in r]
    read, = [r for r in calls if r['callee'] == api.DE + 'Deserializer::deserialize_any']
    start = api.call_at(body, read['arguments'][1])
    require(start['callee'] == 'serde::private::de::content::<impl>::new' and start['arguments'] == [
        {'constant': 'kind'}, {'constant': 'internally tagged enum LifecycleOperation'}] and
        start['path'] == [] and read['arguments'][0] == {'input': True}, 'wrong original operation tag reader')
    branch, = [r for r in calls if r['callee'] == 'core::result::<impl>::branch']
    content_value = {'field': 'Continue._0', 'of': {'call_result': branch['statement']}}
    content = [r for r in calls if r['callee'] == 'serde::private::de::content::<impl>::new' and
               r['arguments'] == [{'field': '_1', 'of': content_value}]]
    require(len(content) == 1, 'operation buffer is not the original content')
    require(api.successful_call(body, content_value, content[0]['path']) == read,
            'operation buffer lost original reader result')
    tag = {'field': '_0', 'of': content_value}
    dispatch = [r for r in calls if r['callee'] == 'serde::private::de::content::<impl>::deserialize_any']
    require(len(dispatch) == 5 and len(body['exits']) == 6, 'operation dispatch/return count changed')
    enums = [g['cases'][0]['enum'] for r in dispatch for g in r['path'] if g['subject'] == tag]
    require(enums and all(e == enums[0] for e in enums), 'operation tag type changed')
    enum = enums[0]
    selectors = [r for r in report['functions'].values() if r['success_type'] == enum and
                 r['method'] in ('visit_str', 'visit_bytes', 'visit_u64')]
    require(len(selectors) == 3, 'operation tag selectors missing')
    selected = {r['method']: r for r in selectors}
    variants = [(variant.lower(), 'tag') for variant in schema['operations']]
    api.selector(selected['visit_str'], enum, variants)
    api.byte_selector(selected['visit_bytes'], enum, variants)
    api.numeric_selector(selected['visit_u64'], enum, variants)
    checked = {}
    for index, variant in enumerate(schema['operations']):
        call, = [r for r in dispatch if api.variant_guard(r['path'], tag, enum, '__field' + str(index))]
        require(call['arguments'][0] == {'call_result': content[0]['statement']} and call['destination'] == 0 and
                any(e['value'] == {'call_result': call['statement']} for e in body['exits']),
                'operation branch does not return the original buffer visitor')
        visitor = call['arguments'][1]['aggregate']
        ids = []
        for mode in ('visit_seq', 'visit_map'):
            checked_body = report['checked']['LifecycleOperation::' + variant + '::' + mode]
            fid = checked_body['function_id']
            function = original.functions[fid]
            input_type = original.resolve(function['signature']['inputs'][0], original.types)
            require(original.declaration_name(input_type['Adt']['id']) == visitor,
                    'operation tag selects another variant visitor')
            ids.append(fid)
        checked[variant] = dict(call=call['statement'], visitors=ids)
    return dict(root=body['function_id'], original_dispatch=checked, source_binding_only=True,
                library_refinement_proved=False)
