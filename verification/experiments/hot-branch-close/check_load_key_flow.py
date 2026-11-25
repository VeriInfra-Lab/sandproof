"""Follow successful load returns in the original transparent retirement capture."""

import argparse
import json
from pathlib import Path

from check_retirement_flow import (
    CONTROL_FLOW, DIGEST, LOAD_BATCH, RESULT, RESULT_BRANCH, RESULT_RESIDUAL,
    check, enum_variant, name, representations, require, statements, unpack,
)


OPERATION_KEY = ('sandproof_protocol', 'lifecycle', 'identity', 'OperationKey')
SERIALIZE = ('serde_core', 'ser', 'Serialize')
PARTIAL_EQ = ('core', 'cmp', 'PartialEq')
VEC = ('alloc', 'vec', 'Vec')
GLOBAL = ('alloc', 'alloc', 'Global')
VEC_DEREF = ('alloc', 'vec', 'deref')
VEC_ARRAY_NE = ('alloc', 'vec', 'partial_eq', 'ne')
STD_VEC_EQ = '/rustc/library/alloc/src/vec/partial_eq.rs'
OPERATION_KEY_SOURCE = 'sandbox-protocol/src/lifecycle/identity.rs'


def key_dispatch(translated, functions, type_decls, types, integers, trait_refs):
    trait_decls = {entry['def_id']: entry for entry in translated['trait_decls'] if entry}
    trait_impls = {entry['def_id']: entry for entry in translated['trait_impls'] if entry}
    sources = {entry['id']: entry['name'] for entry in translated['files'] if entry}
    u8 = 'U8'
    vector = (VEC, (u8,))
    array32 = ('array', u8, ('Usize', 32))
    operation = (OPERATION_KEY, ())

    def uint(encoded):
        kind, value = unpack(encoded, integers)[0]['Integer']['Unsigned']
        return kind, int(value)

    def shape(encoded):
        ty = unpack(encoded, types)
        if 'Scalar' in ty:
            return ty['Scalar']['Integer']['Unsigned']
        if 'Adt' in ty:
            adt = ty['Adt']
            return name(type_decls[adt['id']]), generic_types(adt['generics'])
        if 'Ref' in ty:
            _, inner, kind = ty['Ref']
            return 'ref', kind, shape(inner)
        if 'Array' in ty:
            inner, length, _ = ty['Array']
            return 'array', shape(inner), uint(length)
        if 'Slice' in ty:
            return 'slice', shape(ty['Slice'][0])
        raise ValueError('unexpected type in the operation-key dispatch')

    def generic_types(generic):
        return tuple(shape(encoded) for encoded in generic['types'])

    def argument_types(call):
        return tuple(shape(next(iter(argument.values()))['ty']) for argument in call['args'])

    def source(entry):
        span = entry['item_meta']['span']['Untagged']['data']
        return sources[span['file_id']], span['beg']['line']

    def method(implementation, trait_id, slot):
        entry, = (item for item in implementation['methods']
                  if item['kind'] == {'TraitMethod': [trait_id, slot]})
        return entry['skip_binder']['id']

    def ground_dictionary(generic, trait, wanted_types):
        encoded, = generic['trait_refs']
        bound = unpack(encoded, trait_refs)
        declared = bound['trait_decl_ref']['skip_binder']
        actual = bound['kind']['TraitImpl']
        chosen = trait_impls[actual['id']]
        require(name(trait_decls[declared['id']]) == trait
                and generic_types(declared['generics']) == wanted_types
                and not any(actual['generics'].values())
                and chosen['impl_trait'] == declared,
                'wrong concrete dictionary: ' + '::'.join(trait))
        return declared['id'], chosen

    def digest_application(call):
        regular = call['func']['Regular']
        target = regular['kind']['Fun']
        generic = regular['generics']
        require(name(functions[target]) == DIGEST and generic_types(generic) == (operation,)
                and not generic['const_generics']
                and argument_types(call) == (('ref', 'Shared', operation),),
                'operation digest did not receive the original shared OperationKey')
        trait_id, implementation = ground_dictionary(generic, SERIALIZE, (operation,))
        serialize = method(implementation, trait_id, 0)
        require(source(implementation)[0] == {'Local': OPERATION_KEY_SOURCE}
                and {'Builtin': 'AutomaticallyDerived'} in
                    implementation['item_meta']['attr_info']['attributes']
                and name(functions[serialize])[-1] == 'serialize'
                and functions[serialize]['item_meta']['name'][-2] ==
                    {'Impl': {'Trait': implementation['def_id']}},
                'operation digest lost the original derived Serialize dictionary')
        return target, implementation['def_id'], serialize

    def vec_array_comparison(call):
        regular = call['func']['Regular']
        target = regular['kind']['Fun']
        selected = functions[target]
        generic = regular['generics']
        require(name(selected) == VEC_ARRAY_NE
                and generic_types(generic) == (u8, u8, (GLOBAL, ()))
                and [uint(value) for value in generic['const_generics']] == [('Usize', 32)]
                and argument_types(call) == (('ref', 'Shared', vector), ('ref', 'Shared', array32)),
                'key comparison must be the actual stored-first Vec<u8>/[u8;32] method')
        trait_id, byte_impl = ground_dictionary(generic, PARTIAL_EQ, (u8, u8))
        implementation = trait_impls[selected['item_meta']['name'][-2]['Impl']['Trait']]
        require(implementation['impl_trait']['id'] == trait_id
                and method(implementation, trait_id, 1) == target
                and name(functions[method(byte_impl, trait_id, 1)]) == ('core', 'cmp', 'impls', 'ne')
                and source(selected) == ({'Local': STD_VEC_EQ}, 17)
                and source(implementation)[0] == {'Local': STD_VEC_EQ},
                'key comparison changed the original Vec/Array or u8 PartialEq selection')
        return selected['item_meta']['opacity']

    def vec_deref(call):
        require(argument_types(call) == (('ref', 'Shared', vector),)
                and shape(call['dest']['ty']) == ('ref', 'Shared', ('slice', u8)),
                'record decode must borrow the complete original Vec<u8> as a slice')

    return digest_application, vec_array_comparison, vec_deref


def check_load(capture):
    report = check(capture)
    translated = json.loads(capture.read_bytes())['translated']
    functions = {entry['def_id']: entry for entry in translated['fun_decls'] if entry}
    type_decls = {entry['def_id']: entry for entry in translated['type_decls'] if entry}
    switches, trait_refs = {}, {}
    types, integers = representations(translated, trait_refs=trait_refs, switch_values=switches)
    digest_application, vec_array_comparison, vec_deref = key_dispatch(
        translated, functions, type_decls, types, integers, trait_refs,
    )
    retirement, = (entry for entry in functions.values()
                   if name(entry) == (*DIGEST[:-1], 'retire_closed_parent')
                   and entry['item_meta']['name'][-1] == {'Ident': ['retire_closed_parent', 0]})
    retirement_calls = (data['call'] for tag, data, _
                        in statements(retirement['body']['Structured']['body']) if tag == 'Call')
    delete_digest, = (call for call in retirement_calls
                      if name(functions[call['func']['Regular']['kind']['Fun']]) == DIGEST)
    delete_application = digest_application(delete_digest)
    comparison_opacities = set()
    root, = (entry for entry in functions.values()
             if name(entry) == LOAD_BATCH and 'Ident' in entry['item_meta']['name'][-1])
    require(root['item_meta']['opacity'] == 'Transparent',
            'the original branch record load body must be transparent')
    body = root['body']['Structured']
    require(body['locals']['arg_count'] == 2, 'loaded row and limits arguments changed')
    endings = []

    def place_origin(place, values):
        kind = place['kind']
        if 'Local' in kind:
            return values.get(kind['Local'])
        base, projection = kind['Projection']
        origin = place_origin(base, values)
        if projection == 'Deref':
            return origin
        if not isinstance(projection, dict) or 'Field' not in projection:
            return None
        variant, field = projection['Field']
        if origin == ('stored-row',) and variant is None:
            return ('stored-column', field)
        if origin and origin[0] == 'decoded-field' and variant is None:
            return ('decoded-field', (*origin[1], field))
        if origin and origin[0] == 'guarded-result' and (variant, field) == (origin[2], 0):
            if origin[1] == 'Continue':
                return origin[3]
            return ('original-residual',)
        return None

    def operand(operand_value, values):
        place = operand_value.get('Move', operand_value.get('Copy'))
        return place_origin(place, values) if place is not None else None

    def assignment(source, values):
        if 'Use' in source:
            return operand(source['Use'][0], values)
        if 'Ref' in source:
            return place_origin(source['Ref']['place'], values)
        if 'Aggregate' in source:
            aggregate, inputs = source['Aggregate']
            if 'Adt' not in aggregate:
                return None
            adt, variant, _ = aggregate['Adt']
            if adt.get('id') not in type_decls or name(type_decls[adt['id']]) != RESULT:
                return None
            tag = enum_variant(type_decls, adt, variant, RESULT)
            value, = inputs
            return ('return-ok', operand(value, values)) if tag == 'Ok' else ('return-err',)
        return None

    def apply_call(call, values):
        regular = call['func'].get('Regular', {})
        target = regular.get('kind', {}).get('Fun')
        require(target in functions, 'indirect call in the loaded operation-key boundary')
        path = name(functions[target])
        arguments = [operand(arg, values) for arg in call['args']]
        first = arguments[0] if arguments else None
        if path == VEC_DEREF and first == ('stored-column', 3):
            vec_deref(call)
            return first
        if path[0] == 'serde_json' and path[-1] == 'from_slice':
            require(first == ('stored-column', 3), 'branch record decoded from a different stored column')
            return ('raw-result', ('decoded-field', ()))
        if path[0] == 'core' and path[-1] == 'map_err' and first and first[0] == 'raw-result':
            return first
        if path == DIGEST:
            if first == ('decoded-field', (0, 0)):
                require(digest_application(call) == delete_application,
                        'load and DELETE did not select the same operation digest and Serialize dictionary')
                return ('raw-result', ('loaded-operation-digest',))
            return ('raw-result', None)
        if path == RESULT_BRANCH:
            payload = first[1] if first and first[0] == 'raw-result' else None
            return ('result-branch', payload)
        if path == RESULT_RESIDUAL:
            require(first == ('original-residual',), 'load propagated an unrelated residual')
            return ('return-err',)
        if path[-1] == 'ne' and path[0] in ('core', 'alloc'):
            exact = [('stored-column', 0), ('loaded-operation-digest',)]
            if arguments == exact or arguments == exact[::-1]:
                require(arguments == exact, 'key comparison changed the original argument order')
                comparison_opacities.add(vec_array_comparison(call))
                return ('original-key-mismatch',)
        return None

    def split(switch, values, compared):
        selector = switch['data']
        scrutinee = selector['scrutinee']
        if 'Discriminant' in scrutinee:
            place = scrutinee['Discriminant']
            head = place_origin(place, values)
            if head and head[0] == 'result-branch':
                tags = {}
                for encoded, position in selector['branches']:
                    value, _ = unpack(encoded, switches)
                    adt, index = value['Discriminant']
                    tag = enum_variant(type_decls, adt, index, CONTROL_FLOW)
                    require(tag not in tags, 'loaded operation Result repeats a ControlFlow variant')
                    tags[tag] = (index, position)
                require(set(tags) == {'Continue', 'Break'}
                        and {position for _, position in tags.values()} == {0, 1}
                        and len(switch['branches']) == 2 and selector['fallback'] is None,
                        'loaded operation lost an original Result branch')
                for tag, (index, position) in tags.items():
                    guarded = dict(values)
                    guarded[place['kind']['Local']] = ('guarded-result', tag, index, head[1])
                    yield switch['branches'][position], guarded, compared
                return
        if 'Value' in scrutinee and operand(scrutinee['Value'], values) == ('original-key-mismatch',):
            arms = {}
            for encoded, position in selector['branches']:
                value, _ = unpack(encoded, switches)
                flag = value['Bool']
                require(type(flag) is bool and flag not in arms, 'load repeated a key comparison arm')
                arms[flag] = position
            positions = {flag: arms.get(flag, selector['fallback']) for flag in (False, True)}
            require(set(positions.values()) == {0, 1} and len(switch['branches']) == 2,
                    'load does not distinguish equality from key mismatch')
            require(compared is None, 'load compared the original key more than once')
            for mismatch, position in positions.items():
                yield switch['branches'][position], dict(values), mismatch
            return
        for branch in switch['branches']:
            yield branch, dict(values), compared

    def walk(block, values, compared):
        active = [(values, compared)]
        for statement in block['statements']:
            effect = statement['kind']
            next_paths = []
            for values, compared in active:
                if effect == 'Return':
                    endings.append((compared, values.get(0)))
                    continue
                require(isinstance(effect, dict), 'unexpected terminal instruction in branch load')
                ((tag, data),) = effect.items()
                if tag == 'Switch':
                    for arm, state, guard in split(data, values, compared):
                        next_paths.extend(walk(arm, state, guard))
                    continue
                if tag == 'Assign':
                    dest, source = data
                    index = dest['kind'].get('Local')
                    require(index is not None, 'branch load assigned a field of an existing value')
                    values[index] = assignment(source, values)
                elif tag == 'Call':
                    call = data['call']
                    index = call['dest']['kind'].get('Local')
                    require(index is not None, 'branch load call overwrote an existing field')
                    values[index] = apply_call(call, values)
                elif tag == 'StorageDead':
                    values.pop(data, None)
                else:
                    require(tag in ('StorageLive', 'Drop', 'Borrowck', 'PlaceMention'),
                            'unexpected branch load effect: ' + tag)
                next_paths.append((values, compared))
            active = next_paths
        return active

    require(not walk(body['body'], {1: ('stored-row',)}, None),
            'branch load has an unterminated path')
    require(all(output and output[0] in ('return-ok', 'return-err') for _, output in endings),
            'branch load has an unrecognized return value')
    successes = [(guard, value) for guard, value in endings if value[0] == 'return-ok']
    require(successes and all(guard is False and value[1] == ('decoded-field', ())
                              for guard, value in successes),
            'load can return a decoded batch without matching its original operation column')
    require(any(guard is True for guard, _ in endings),
            'missing original mismatched operation-key path')
    opacity, = comparison_opacities
    report['load_operation_key'] = dict(
        stored_column=0, record_column=3, decoded_field='key.operation',
        comparison='full Vec<u8> against original digest',
        normal_return_requires_match=True,
        digest_application='digest::<OperationKey> with the same derived Serialize method',
        digest_body_opacity=functions[delete_application[0]]['item_meta']['opacity'],
        comparison_dispatch=dict(method='::'.join(VEC_ARRAY_NE), left='Vec<u8>', right='[u8;32]',
                                 allocator='::'.join(GLOBAL), element='PartialEq<u8> for u8',
                                 source=STD_VEC_EQ, source_line=17, body_opacity=opacity),
    )
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('retirement_capture', type=Path)
    print(json.dumps(check_load(parser.parse_args().retirement_capture), indent=2))
