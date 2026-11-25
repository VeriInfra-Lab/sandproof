"""Follow successful load returns in the original transparent retirement capture."""

import argparse
import json
from pathlib import Path

from check_retirement_flow import (
    CONTROL_FLOW, DIGEST, LOAD_BATCH, RESULT, RESULT_BRANCH, RESULT_RESIDUAL,
    check, enum_variant, name, representations, require, unpack,
)


def check_load(capture):
    report = check(capture)
    translated = json.loads(capture.read_bytes())['translated']
    functions = {entry['def_id']: entry for entry in translated['fun_decls'] if entry}
    type_decls = {entry['def_id']: entry for entry in translated['type_decls'] if entry}
    switches = {}
    representations(translated, switch_values=switches)
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
        if path[-1] in ('deref', 'as_slice') and path[0] in ('alloc', 'core'):
            return first
        if path[0] == 'serde_json' and path[-1] == 'from_slice':
            require(first == ('stored-column', 3), 'branch record decoded from a different stored column')
            return ('raw-result', ('decoded-field', ()))
        if path[0] == 'core' and path[-1] == 'map_err' and first and first[0] == 'raw-result':
            return first
        if path == DIGEST:
            if first == ('decoded-field', (0, 0)):
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
    report['load_operation_key'] = dict(
        stored_column=0, record_column=3, decoded_field='key.operation',
        comparison='full Vec<u8> against original digest',
        normal_return_requires_match=True,
    )
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('retirement_capture', type=Path)
    print(json.dumps(check_load(parser.parse_args().retirement_capture), indent=2))
