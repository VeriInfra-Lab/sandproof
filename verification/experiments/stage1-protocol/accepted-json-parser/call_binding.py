"""Current callback parameter/result binding using the original IR observer.

The generic observer's two-argument labels are positional: `visitor` is
argument 1 and `input` is argument 2, including the probe's bytes/context.
This does not certify the async caller or authenticated context provenance.
"""


def require(condition, message):
    if not condition:
        raise ValueError(message)


def method(ir, impl, name):
    matches = [f for f in ir.functions.values()
               if f['item_meta']['name'][-2:] == [{'Impl': {'Trait': impl}}, {'Ident': [name, 0]}]]
    require(len(matches) == 1, 'callback dictionary has no unique actual method')
    return matches[0]


def dictionary(ir, reference, trait):
    actual = ir.resolve(reference, ir.traits)
    declaration = actual['trait_decl_ref']['skip_binder']
    require(ir.direct_name(ir.trait_decls[declaration['id']]['item_meta']['name']) == trait and
            set(actual['kind']) == {'TraitImpl'}, 'callback dictionary target differs')
    return actual['kind']['TraitImpl'], declaration['generics']['types']


def report(ir, accepted_decoder, digest):
    roots = [f for f in ir.functions.values() if f['item_meta']['name'] == [
        {'Ident': ['sandproof_internal', 0]}, {'Ident': ['server', 0]},
        {'Ident': ['lifecycle', 0]}, {'Ident': ['proof_current_accepted_parser', 0]}]]
    require(len(roots) == 1, 'actual transparent parser probe is missing or duplicated')
    root, = roots
    root_report = ir.links(root)
    root_calls = [row for row in root_report['rows'] if 'callee' in row]
    require(len(root_calls) == 1, 'parser probe has extra calls')
    call, = root_calls
    require(call['callee'] == 'sandproof_internal::lifecycle::decode_accepted_with' and
            call['arguments'][:2] == [
                {'borrow': 'Shared', 'origin': {'deref': {'visitor': True}}},
                {'borrow': 'Shared', 'origin': {'deref': {'input': True}}}],
            'parser probe changes the complete input slice or context')
    closure = call['arguments'][2]
    require(closure.get('variant') is None and closure.get('fields') == [],
            'parser callback captures other state')
    generic = call['original_call']['func']['Regular']['generics']
    require(len(generic['types']) == 2 and ir.type_name(generic['types'][1]) == 'serde_json::error::Error',
            'parser probe changes the concrete JSON error type')
    reference, = generic['trait_refs']
    implementation, types = dictionary(ir, reference, 'core::ops::function::FnOnce')
    require(ir.declaration_name(ir.resolve(types[0], ir.types)['Adt']['id']) == closure['aggregate'],
            'parser callback uses a different closure dictionary')
    callback = method(ir, implementation['id'], 'call_once')
    callback_report = ir.links(callback)
    parser_calls = [row for row in callback_report['rows'] if 'callee' in row]
    require(len(parser_calls) == 1, 'actual parser callback has extra calls')
    parser, = parser_calls
    require(parser['callee'] == 'serde_json::de::from_slice' and parser['arguments'] == [
        {'borrow': 'Shared', 'origin': {'deref': {'field': '_0', 'of': {'input': True}}}}],
        'actual parser callback changes its full input')
    generic = parser['original_call']['func']['Regular']['generics']
    ty, = generic['types']
    require(ir.type_name(ty) == 'sandproof_protocol::lifecycle::authority::AcceptedLifecycleIntent',
            'actual parser decodes a different protocol value')
    reference, = generic['trait_refs']
    implementation, types = dictionary(ir, reference, 'serde_core::de::Deserialize')
    require(len(types) == 1 and ir.type_name(types[0]) == ir.type_name(ty) and
            method(ir, implementation['id'], 'deserialize')['def_id'] == accepted_decoder['def_id'],
            'actual parser substitutes the complete Accepted Deserialize dictionary')
    for observed, invocation in [(root_report, call), (callback_report, parser)]:
        require(len(observed['exits']) == 1 and observed['exits'][0]['path'] == [] and
                observed['exits'][0]['value'] == {'call_result': invocation['statement']},
                'thin parser binding changes or replaces the original result')
    return dict(complete_slice_and_context_source_checked=True,
                same_current_accepted_Deserialize_dictionary=True,
                root_sha256=digest(root), callback_sha256=digest(callback),
                root_call_sha256=digest(call['original_call']),
                parser_call_sha256=digest(parser['original_call']),
                async_route_refinement_proved=False, parser_semantics_proved=False,
                authenticated_context_provenance_proved=False, durable_acceptance_proved=False)
