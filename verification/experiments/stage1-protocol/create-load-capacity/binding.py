"""Bind all original objects, IDs, generic types and dictionaries in the four new roots."""
from pathlib import Path
from copy import deepcopy
import json
import hashlib

PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::'


def admit(raw):
    functions = [f for f in raw['translated']['fun_decls'] if f]
    type_variables, clauses = {}, {}

    def collect(node):
        if isinstance(node, dict):
            value = node.get('Value')
            if isinstance(value, list) and len(value) == 2 and isinstance(value[1], dict):
                index, body = value
                if set(body) == {'TypeVar'}:
                    if index in type_variables and type_variables[index] != body:
                        raise ValueError('inconsistent type-variable reference')
                    type_variables[index] = body
                if 'trait_decl_ref' in body and 'Clause' in body.get('kind', {}):
                    if index in clauses and clauses[index] != body:
                        raise ValueError('inconsistent dictionary reference')
                    clauses[index] = body
            for child in node.values():
                collect(child)
        elif isinstance(node, list):
            for child in node:
                collect(child)

    collect(raw)

    def resolve(node, table):
        if 'Value' in node:
            return node['Value'][1]
        return table[node['Deduplicated']]

    def select(path):
        name = [{'Ident': [n, 0]} for n in path.split('::')]
        values = [f for f in functions if f['item_meta']['name'] == name]
        if len(values) != 1 or values[0]['item_meta']['opacity'] != 'Transparent' or 'Structured' not in values[0]['body']:
            raise ValueError('missing or external original root '+path)
        return values[0]

    result = {}
    for root_name, last, refs, copies, params in [
        (PREFIX+'codec::proof_inbox_load_using', 'load_using', ['TwoPhaseMut'], [], ['R', 'T', 'N', 'U', 'B', 'E']),
        (PREFIX+'model::proof_inbox_validate_using', 'validate_using', ['Shared'], [], ['R', 'T', 'N', 'U', 'B']),
        (PREFIX+'proof_create_attachment_selection', 'create_attachment_selection', ['Shared', 'Shared'], [], []),
        (PREFIX+'proof_create_attachment_install', 'create_attachment_install', ['TwoPhaseMut'], [2, 3, 4], [])]:
        root = select(root_name)
        calls, assignments = [], []

        def walk(node):
            if isinstance(node, dict):
                if 'Call' in node:
                    calls.append(node['Call']['call'])
                if 'Assign' in node:
                    assignments.append(node['Assign'])
                for child in node.values():
                    walk(child)
            elif isinstance(node, list):
                for child in node:
                    walk(child)

        walk(root['body'])
        count = len(refs)+len(copies)
        if len(calls) != 1 or root['body']['Structured']['locals']['arg_count'] != count:
            raise ValueError('changed complete argument count')
        call = calls[0]
        actual = next(f for f in functions if f['def_id'] == call['func']['Regular']['kind']['Fun'])
        expected_prefix = [{'Ident': [part, 0]} for part in root_name.split('::')[:-1]]
        if (actual['item_meta']['name'][:len(expected_prefix)] != expected_prefix
                or actual['item_meta']['name'][-1] != {'Ident': [last, 0]}
                or actual['item_meta']['opacity'] != 'Transparent' or 'Structured' not in actual['body']
                or call['dest']['kind'] != {'Local': 0} or len(call['args']) != count):
            raise ValueError('changed original business target or result')
        if last == 'create_attachment_install':
            unit, *assignments = assignments
            if unit[0]['kind'] != {'Local': 0} or unit[1].get('Aggregate', [None, None])[1] != []:
                raise ValueError('changed unit result initialization')
        if len(assignments) != count:
            raise ValueError('extra or omitted root computation')
        for index, (kind, argument, (destination, expression)) in enumerate(zip(refs, call['args'], assignments), 1):
            ref = expression.get('Ref', {})
            projection = ref.get('place', {}).get('kind', {}).get('Projection')
            if (ref.get('kind') != kind or argument.get('Move', {}).get('kind') != destination['kind']
                    or not projection or projection[0]['kind'] != {'Local': index} or projection[1] != 'Deref'):
                raise ValueError('original full object borrow changed')
        for original, argument, (destination, expression) in zip(copies, call['args'][len(refs):], assignments[len(refs):]):
            if (argument.get('Move', {}).get('kind') != destination['kind'] or expression.get('Use') !=
                    [{'Copy': {'kind': {'Local': original}, 'ty': destination['ty']}}, 'Yes']):
                raise ValueError('original whole index/binding copy changed')
        generics = call['func']['Regular']['generics']
        if [p['name'] for p in root['generics']['types']] != params or len(generics['types']) != len(params):
            raise ValueError('original generic types changed')
        for index, argument in enumerate(generics['types']):
            if resolve(argument, type_variables) != {'TypeVar': {'Free': index}}:
                raise ValueError('generic type reordered or replaced')
        if len(generics['trait_refs']) != len(params) or len(root['generics']['trait_clauses']) != len(params):
            raise ValueError('generic dictionary count changed')
        for index, argument in enumerate(generics['trait_refs']):
            clause = resolve(argument, clauses)
            if (clause['kind'] != {'Clause': {'Free': index}} or
                    clause['trait_decl_ref'] != root['generics']['trait_clauses'][index]['trait_']):
                raise ValueError('dictionary reordered or substituted')
        result[last] = dict(root=root['def_id'], actual=actual['def_id'],
            complete_original_objects_and_values=True, generic_types_and_dictionaries=params)
    return result



def rejections(raw):
    """Admission evidence only, distinct from the real compiled business mutant."""
    rejected = []
    labels = ('missing-load-root', 'external-load-root', 'different-original-state',
              'reordered-generic-type', 'reordered-generic-dictionary',
              'swapped-selection-arguments', 'changed-full-local-copy', 'extra-root-computation')
    for label in labels:
        changed = deepcopy(raw)
        functions = changed['translated']['fun_decls']
        suffix = ('proof_create_attachment_selection' if label == 'swapped-selection-arguments'
                  else 'proof_create_attachment_install' if label == 'changed-full-local-copy'
                  else 'codec::proof_inbox_load_using')
        name = [{'Ident': [part, 0]} for part in (PREFIX+suffix).split('::')]
        root = next(f for f in functions if f and f['item_meta']['name'] == name)
        calls, assignments = [], []

        def walk(node):
            if isinstance(node, dict):
                if 'Call' in node:
                    calls.append(node['Call']['call'])
                if 'Assign' in node:
                    assignments.append(node['Assign'])
                for child in node.values():
                    walk(child)
            elif isinstance(node, list):
                for child in node:
                    walk(child)

        walk(root['body'])
        if label == 'missing-load-root':
            functions[functions.index(root)] = None
        elif label == 'external-load-root':
            root['item_meta']['opacity'] = 'Opaque'
        elif label == 'different-original-state':
            assignments[0][1]['Ref']['place']['kind']['Projection'][0]['kind'] = {'Local': 2}
        elif label == 'reordered-generic-type':
            types = calls[0]['func']['Regular']['generics']['types']
            types[0], types[1] = types[1], types[0]
        elif label == 'reordered-generic-dictionary':
            traits = calls[0]['func']['Regular']['generics']['trait_refs']
            traits[0], traits[1] = traits[1], traits[0]
        elif label == 'swapped-selection-arguments':
            args = calls[0]['args']
            args[0], args[1] = args[1], args[0]
        elif label == 'changed-full-local-copy':
            assignments[-1][1]['Use'][0]['Copy']['kind'] = {'Local': 2}
        else:
            root['body']['Structured']['locals']['arg_count'] += 1
        try:
            admit(changed)
        except (ValueError, StopIteration, KeyError):
            rejected.append(label)
        else:
            raise ValueError('admitted modified root: '+label)
    return rejected
