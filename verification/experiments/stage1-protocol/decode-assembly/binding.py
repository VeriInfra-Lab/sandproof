"""Source links in the original derived decoder, not a Rust proof translator.

The complete input remains authoritative. This finite inspection records
assignments, branch guards, original dictionaries and one syntactic loop body;
it does not execute a parser or claim a library refinement theorem.
"""
import importlib.util
import json
from pathlib import Path

_PATH = Path(__file__).resolve().parents[1] / 'serde-binding/binding.py'
_SPEC = importlib.util.spec_from_file_location('decode_original', _PATH)
_PARENT = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_PARENT)
require, digest = _PARENT.require, _PARENT.digest


def same(left, right):
    # Python equates True with 1; original Rust branch constants do not.
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


def member(value, values):
    return any(same(value, other) for other in values)


class Original(_PARENT.Original):
    def _intern(self, node):
        if (isinstance(node, dict) and set(node) == {'Value'} and
                isinstance(node['Value'], list)):
            index, value = node['Value']
            if value == 'Never' or (isinstance(value, dict) and set(value) == {'FnDef'}):
                self.types[index] = value
        super()._intern(node)

    def type_name(self, reference):
        value = self.resolve(reference, self.types)
        if value == 'Never':
            return '!'
        if isinstance(value, dict) and set(value) in ({'TraitType'}, {'DynTrait'}, {'FnDef'}):
            return 'original-type:' + json.dumps(value, sort_keys=True)
        return super().type_name(reference)

    def place(self, value, aliases):
        if set(value['kind']) == {'Global'}:
            return {'original_global': value['kind']['Global']}
        if (set(value['kind']) == {'Projection'} and value['kind']['Projection'][1] == 'PtrMetadata'):
            return {'ptr_metadata': self.place(value['kind']['Projection'][0], aliases)}
        return super().place(value, aliases)

    def declaration_name(self, index):
        parts = self.type_decls[index]['item_meta']['name']
        if all('Ident' in p and p['Ident'][1] == 0 for p in parts):
            return self.direct_name(parts)
        return {'original_type': index, 'name': parts}

    def field_name(self, base, field):
        variant, index = field
        ty = self.resolve(base['ty'], self.types)
        require('Adt' in ty, 'projection receiver is not an ADT')
        if ty['Adt']['builtin'] == 'Tuple':
            require(variant is None and 0 <= index < len(ty['Adt']['generics']['types']),
                    'invalid original tuple projection')
            return '_' + str(index)
        declaration = self.type_decls[ty['Adt']['id']]
        if variant is None:
            return declaration['kind']['Struct'][index]['name']
        selected = declaration['kind']['Enum'][variant]
        return selected['name'] + '.' + selected['fields'][index]['name']

    def assigned(self, expression, aliases):
        if 'BinaryOp' in expression and expression['BinaryOp'][0] == 'Eq':
            _, left, right = expression['BinaryOp']
            return {'original_eq': [self.operand(left, aliases), self.operand(right, aliases)]}
        if 'Aggregate' in expression and 'Adt' in expression['Aggregate'][0]:
            aggregate, fields = expression['Aggregate']
            adt, variant, union_field = aggregate['Adt']
            require(union_field is None, 'union assembly needs separate review')
            declaration = self.type_decls[adt['id']]
            return dict(aggregate=self.declaration_name(adt['id']),
                variant=None if variant is None else declaration['kind']['Enum'][variant]['name'],
                fields=[self.operand(f, aliases) for f in fields])
        return super().assigned(expression, aliases)

    def branch_guard(self, switch, index, aliases):
        data = switch['data']
        subject = data['scrutinee']
        if 'Discriminant' in subject:
            subject = self.place(subject['Discriminant'], aliases)
        else:
            require(set(subject) == {'Value'}, 'unreviewed branch subject')
            subject = self.operand(subject['Value'], aliases)
        labels, others = [], []
        for constant, branch in data['branches']:
            value, _ = self.resolve(constant, self.constants)
            if 'Discriminant' in value:
                adt, variant = value['Discriminant']
                declaration = self.type_decls[adt['id']]
                label = dict(enum=self.declaration_name(adt['id']),
                             variant=declaration['kind']['Enum'][variant]['name'])
            else:
                label = self.constant(constant)
            (labels if branch == index else others).append(label)
        return dict(subject=subject, cases=labels,
                    except_cases=others if data['fallback'] == index else [])

    def links(self, function, result_required=True):
        require(function['item_meta']['opacity'] == 'Transparent' and
                set(function['body']) == {'Structured'}, 'original visitor body missing')
        rows, exits, iterations = [], [], []

        def add_guard(path, guard):
            # Discard only contradictory literal/discriminant branch choices.
            # This does not evaluate any library or business call.
            subject = guard['subject']
            known = subject if set(subject) == {'constant'} else None
            if isinstance(subject, dict) and 'aggregate' in subject and subject['variant'] is not None:
                known = dict(enum=subject['aggregate'], variant=subject['variant'])
            if known is not None:
                if guard['cases'] and not member(known, guard['cases']):
                    return None
                if member(known, guard['except_cases']):
                    return None
            for prior in path:
                if not same(prior['subject'], subject):
                    continue
                if (prior['cases'] and guard['cases'] and
                        not any(member(x, prior['cases']) for x in guard['cases'])):
                    return None
                if prior['cases'] and all(member(x, guard['except_cases']) for x in prior['cases']):
                    return None
                if guard['cases'] and all(member(x, prior['except_cases']) for x in guard['cases']):
                    return None
            return path + [guard]

        def assigned_locals(block):
            found = set()
            for st in block['statements']:
                k = st['kind']
                if not isinstance(k, dict):
                    continue
                if 'Assign' in k and 'Local' in k['Assign'][0]['kind']:
                    found.add(k['Assign'][0]['kind']['Local'])
                if 'Switch' in k:
                    for branch in k['Switch']['branches']:
                        found.update(assigned_locals(branch))
            return found

        def walk(block, states, loop=None):
            for statement in block['statements']:
                kind = statement['kind']
                next_states = []
                for old_aliases, path in states:
                    aliases = dict(old_aliases)
                    row = dict(statement=statement['id'], path=path,
                               statement_sha256=digest(statement))
                    if kind == 'Return':
                        exits.append(dict(**row, value=aliases.get(0), exit='Return'))
                        continue
                    if isinstance(kind, dict) and ('Break' in kind or 'Continue' in kind):
                        tag = 'Break' if 'Break' in kind else 'Continue'
                        require(loop is not None and kind[tag] == 0, 'nested loop control needs review')
                        value = dict(**row, value=aliases.get(0), exit=tag,
                            slots={str(k): aliases[k] for k in loop['slots']})
                        if tag == 'Break':
                            loop['breaks'].append((aliases, path))
                        else:
                            iterations.append(value)
                        continue
                    if (isinstance(kind, str) and kind in ('UnwindResume', 'Abort')) or (
                            isinstance(kind, dict) and 'Abort' in kind):
                        continue
                    if not isinstance(kind, dict):
                        require(kind == 'Nop', 'unreviewed normal statement: ' + str(kind))
                    elif 'Assign' in kind:
                        place, expression = kind['Assign']
                        require(set(place['kind']) == {'Local'}, 'nonlocal assignment needs review')
                        value = self.assigned(expression, aliases)
                        aliases[place['kind']['Local']] = value
                        rows.append(dict(**row, assignment=place['kind']['Local'], value=value))
                    elif 'Call' in kind:
                        call = kind['Call']['call']
                        require(set(call['dest']['kind']) == {'Local'}, 'nonlocal call result')
                        generics = call['func']['Regular']['generics']
                        rows.append(dict(**row, callee=self.callee(call['func']),
                            arguments=[self.operand(a, aliases) for a in call['args']],
                            destination=call['dest']['kind']['Local'], original_call=call,
                            types=[self.type_name(t) for t in generics['types']],
                            dictionaries=[self.dictionary(t) for t in generics['trait_refs']]))
                        aliases[call['dest']['kind']['Local']] = {'call_result': statement['id']}
                    elif 'Switch' in kind:
                        switch = kind['Switch']
                        for index, branch in enumerate(switch['branches']):
                            guarded = add_guard(path, self.branch_guard(switch, index, aliases))
                            if guarded is not None:
                                next_states.extend(walk(branch, [(aliases, guarded)], loop))
                        continue
                    elif 'Loop' in kind:
                        require(loop is None, 'nested loop requires independent review')
                        slots = assigned_locals(kind['Loop']) & aliases.keys()
                        rows.append(dict(**row, loop_slots={str(k): aliases[k] for k in slots}))
                        for local in slots:
                            aliases[local] = {'loop_slot': local}
                        context = {'slots': slots, 'breaks': []}
                        remaining = walk(kind['Loop'], [(aliases, path)], context)
                        for a, p in remaining:
                            iterations.append(dict(statement=statement['id'], path=p,
                                exit='Fallthrough', slots={str(k): a[k] for k in slots}))
                        next_states.extend(context['breaks'])
                        # Continue records retain the slots and all guards;
                        # no loop is unrolled and no convergence is claimed.
                        continue
                    elif 'PlaceMention' in kind:
                        # This may require projection validity. Preserve it as
                        # an explicit source obligation, never call it a no-op.
                        rows.append(dict(**row, place_validity_obligation=kind['PlaceMention']))
                    else:
                        require(next(iter(kind)) in {'StorageLive', 'StorageDead', 'FakeRead', 'Drop'},
                                'unreviewed statement in decoder: ' + str(next(iter(kind))))
                    next_states.append((aliases, path))
                states = next_states
                require(len(states) <= 4096, 'source path inspection limit exceeded')
            return states

        initial = {1: {'input': True}} if len(function['signature']['inputs']) == 1 else {
            1: {'visitor': True}, 2: {'input': True}}
        remaining = walk(function['body']['Structured']['body'], [(initial, [])])
        require(not remaining, 'visitor has an unaccounted fallthrough')
        output = self.resolve(function['signature']['output'], self.types)
        is_result = 'Adt' in output and self.declaration_name(output['Adt']['id']) == 'core::result::Result'
        require(is_result or not result_required, 'visitor result is not Result')
        success_ref = output['Adt']['generics']['types'][0] if is_result else function['signature']['output']
        success = self.resolve(success_ref, self.types)
        success_type = self.declaration_name(success['Adt']['id']) if 'Adt' in success else self.type_name(success_ref)
        return dict(function_id=function['def_id'], function_sha256=digest(function),
                    method=function['item_meta']['name'][-1]['Ident'][0],
                    success_type=success_type,
                    rows=rows, exits=exits, iterations=iterations)


def functions(raw):
    original = Original(raw)
    result = {}
    for function in original.functions.values():
        name = function['item_meta']['name']
        if (name[0] == {'Ident': ['sandproof_protocol', 0]} and
                name[-1] in ({'Ident': ['visit_seq', 0]}, {'Ident': ['visit_map', 0]},
                             {'Ident': ['visit_str', 0]}, {'Ident': ['visit_bytes', 0]},
                             {'Ident': ['visit_u64', 0]}, {'Ident': ['deserialize', 0]})):
            result[function['def_id']] = original.links(function)
    return dict(kind='original-decoder-source-links', library_refinement_proved=False,
                original_llbc_sha256=digest(raw), functions=result)


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    with args.output.open('x') as stream:
        json.dump(functions(json.loads(args.llbc.read_text())), stream, indent=2)
        stream.write('\n')
