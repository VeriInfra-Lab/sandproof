"""Original forwarding to the already represented ID/counter/digest leaves."""
from binding import require

P = 'sandproof_protocol::'
I = P + 'lifecycle::identity::'
WRAPPERS = {
    P + 'lifecycle::authority::CenterId': P + 'DeploymentId',
    I + 'SessionId': P + 'SandboxId',
    **{I + name: P + 'internal::PositiveCounter' for name in (
        'IncarnationId', 'CaptureId', 'RequestWindowId', 'RequestSequence', 'RetentionWindowId')},
    P + 'lifecycle::digest::ContentDigest': '[U8;32]',
}
CONVERSIONS = {**{P + name: 'alloc::string::String' for name in (
    'DeploymentId', 'SandboxId', 'NodeId', 'TenantId', 'ResourceId')},
    P + 'internal::PositiveCounter': 'U64'}


def implementation_self(original, call):
    function = original.functions[call['original_call']['func']['Regular']['kind']['Fun']]
    impls = [p['Impl']['Trait'] for p in function['item_meta']['name'] if 'Impl' in p and 'Trait' in p['Impl']]
    implementation, = impls
    return original.type_name(original.impls[implementation]['impl_trait']['generics']['types'][0])


def callback(original, mapped):
    once, destruct = mapped['dictionaries']
    require(once['trait'] == 'core::ops::function::FnOnce' and
            destruct['trait'] == 'core::marker::Destruct' and
            once['types'][0] == mapped['types'][3] and destruct['types'] == [mapped['types'][3]],
            'original map closure dictionaries changed')
    closure = mapped['arguments'][1]
    require(closure['variant'] is None and closure['fields'] == [], 'map closure captures another value')
    implementation = original.impls[once['implementation']['id']]
    method, = implementation['methods']
    function = original.functions[method['skip_binder']['id']]
    first = original.resolve(function['signature']['inputs'][0], original.types)
    require(original.declaration_name(first['Adt']['id']) == closure['aggregate'],
            'map invokes another closure implementation')
    result = original.links(function, result_required=False)
    require(not result['iterations'] and len(result['exits']) == 1 and result['exits'][0]['path'] == [],
            'conditional/repeated original map closure')
    return result


def check(original, report, api):
    bodies = {r['success_type']: r for r in report['functions'].values()
              if r['method'] == 'deserialize' and isinstance(r['success_type'], str)
              and r['success_type'] in WRAPPERS.keys() | CONVERSIONS.keys()}
    require(set(bodies) == WRAPPERS.keys() | CONVERSIONS.keys(), 'original leaf Deserialize body missing')
    checked = {}
    for name, inner in WRAPPERS.items():
        body = bodies[name]
        read, mapped = [r for r in body['rows'] if 'callee' in r]
        require(read['path'] == [] and mapped['path'] == [] and read['arguments'] == [{'input': True}] and
                mapped['callee'] == 'core::result::<impl>::map' and mapped['destination'] == 0 and
                mapped['arguments'][0] == {'call_result': read['statement']} and
                mapped['types'][0] == inner and mapped['types'][2] == name and
                body['exits'][0]['value'] == {'call_result': mapped['statement']},
                'original transparent input/result forwarding changed: ' + name)
        target = implementation_self(original, read)
        require(target == inner or (inner == '[U8;32]' and target == '[parameter:{"Free": 0};32]' and
                read['types'][0] == 'U8'), 'transparent decoder reads another underlying type')
        closure = callback(original, mapped)
        require(closure['exits'][0]['value'] == dict(aggregate=name, variant=None,
                fields=[{'field': '_0', 'of': {'input': True}}]) and
                not any('callee' in r for r in closure['rows']), 'transparent closure changes the original value')
        checked[name] = dict(function_id=body['function_id'], underlying=inner, closure=closure)
    for name, primitive in CONVERSIONS.items():
        body = bodies[name]
        mapped, = [r for r in body['rows'] if r.get('callee') == 'core::result::<impl>::map_err']
        convert = api.call_at(body, mapped['arguments'][0])
        require(convert['callee'].endswith('::try_from') and implementation_self(original, convert) == name and
                mapped['destination'] == 0 and mapped['types'][0] == name and
                any(e['value'] == {'call_result': mapped['statement']} for e in body['exits']),
                'original own conversion not returned')
        read = api.successful_call(body, convert['arguments'][0], convert['path'])
        require(read['callee'] == 'serde_core::de::impls::<impl>::deserialize' and
                implementation_self(original, read) == primitive and read['arguments'] == [{'input': True}],
                'own conversion reads another primitive/input')
        closure = callback(original, mapped)
        custom, = [r for r in closure['rows'] if 'callee' in r]
        require(custom['callee'] == 'serde_core::de::Error::custom' and custom['destination'] == 0 and
                custom['arguments'] == [{'field': '_0', 'of': {'input': True}}] and
                custom['types'] == [mapped['types'][1]] and closure['exits'][0]['value'] ==
                {'call_result': custom['statement']}, 'own conversion error replaced before custom')
        dictionary, = custom['dictionaries']
        require(dictionary['trait'] == 'core::fmt::Display' and dictionary['types'] == custom['types'] and
                dictionary['implementation']['source_name'][0] == {'Ident': ['sandproof_protocol', 0]},
                'own error uses another Display dictionary')
        checked[name] = dict(function_id=body['function_id'], primitive=primitive, custom_error=closure)
    return dict(original_bodies=checked, source_binding_only=True, business_leaf_proofs_reused=True,
                digest_means_computed_hash=False)
