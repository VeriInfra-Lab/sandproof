"""Check the physical SQL call boundary in the actual retirement LLBC."""

from collections import defaultdict
from functools import cache
from hashlib import sha256
import json
from pathlib import Path
import sys


BRANCH_SOURCE = 'controller-service/src/controller/branching.rs'
BRANCH_SHA256 = '74df2ce73bc26fe6484788fecc3b309d69e45df64d550a56ca2b40cf8a681ac3'
BRANCH = ('sandproof_controller_service', 'controller', 'branching')
WORLD_DEREF = ('sandproof_controller_service', 'controller', 'transaction', 'deref')
SQLITE_DEREF = ('rusqlite', 'transaction', 'deref')
SQLITE_PREPARE = ('rusqlite', 'prepare')
SQLITE_QUERY = ('rusqlite', 'statement', 'query_map')
SQLITE_EXECUTE = ('rusqlite', 'execute')
SQLITE_ROW_GET = ('rusqlite', 'row', 'get')
FN_MUT = ('core', 'ops', 'function', 'FnMut')
BYTE_VEC = ('alloc', 'vec', 'Vec')
RESULT_BRANCH = ('core', 'result', 'branch')
RESULT_RESIDUAL = ('core', 'result', 'from_residual')
RESULT = ('core', 'result', 'Result')
CONTROL_FLOW = ('core', 'ops', 'control_flow', 'ControlFlow')
IDENTITY_CONVERT = ('core', 'convert', 'From')
JOURNAL_ERROR = ('sandproof_controller_service', 'JournalError')
SQLITE_ERROR = ('rusqlite', 'error', 'Error')
READ_LIMITS = (*BRANCH, 'read_limits')
LOAD_BATCH = (*BRANCH, 'load')
DIGEST = (*BRANCH, 'digest')
NUMBER_FROM = ('core', 'convert', 'num', 'from')
VEC_NEW = ('alloc', 'vec', 'new')
VEC_PUSH = ('alloc', 'vec', 'push')
VEC_INTO_ITER = ('alloc', 'vec', 'into_iter')
VEC_NEXT = ('alloc', 'vec', 'into_iter', 'next')
ENUMERATE = ('core', 'iter', 'traits', 'iterator', 'Iterator', 'enumerate')
INTO_ITER = ('core', 'iter', 'traits', 'collect', 'into_iter')
ENUMERATE_NEXT = ('core', 'iter', 'adapters', 'enumerate', 'next')
ARRAY_AS_SLICE = ('core', 'array', 'as_slice')
USERS_OF_CONNECTION = {
    WORLD_DEREF, SQLITE_DEREF, SQLITE_PREPARE, SQLITE_QUERY, SQLITE_EXECUTE,
    RESULT_BRANCH, READ_LIMITS, ('core', 'mem', 'drop'),
    # The field-insensitive origin can reach the error projection of prepare.
    ('core', 'result', 'from_residual'),
}
CONNECTION_ORIGINS = {
    'world-transaction', 'sqlite-transaction', 'sqlite-connection', 'sqlite-statement',
}
RECEIVER_ADAPTERS = {
    WORLD_DEREF: ('world-transaction', 'sqlite-transaction'),
    SQLITE_DEREF: ('sqlite-transaction', 'sqlite-connection'),
    SQLITE_PREPARE: ('sqlite-connection', 'sqlite-statement'),
}
SELECT = (
    'SELECT operation,parent,execution,record,sha256 '
    'FROM controller_host_branch_batches LIMIT ?1'
)
DELETE = 'DELETE FROM controller_host_branch_batches WHERE operation=?1'


def require(condition, details):
    if not condition:
        raise ValueError(details)


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def name(entry):
    return tuple(part['Ident'][0] for part in entry['item_meta']['name'] if 'Ident' in part)


def statements(block, loop_depth=0):
    for entry in block['statements']:
        payload = entry['kind']
        if not isinstance(payload, dict):
            continue
        ((kind, value),) = payload.items()
        if kind == 'Switch':
            for arm in value['branches']:
                yield from statements(arm, loop_depth)
        elif kind == 'Loop':
            yield from statements(value, loop_depth + 1)
        else:
            yield kind, value, loop_depth


def representations(root, *, trait_refs=None, switch_values=None):
    types, integers = {}, {}
    pending = [root]
    while pending:
        node = pending.pop()
        if isinstance(node, list):
            pending.extend(node)
        elif isinstance(node, dict):
            pending.extend(node.values())
            value = node.get('Value')
            if not (isinstance(value, list) and len(value) == 2 and isinstance(value[0], int)):
                continue
            index, data = value
            # Charon uses separate ID spaces for types, constants, spans and
            # trait references. Only select the relevant typed encodings.
            if isinstance(data, dict) and data.keys() & {'Scalar', 'Array', 'Ref', 'Slice', 'Adt'}:
                table = types
            elif (isinstance(data, list) and len(data) == 2
                  and isinstance(data[0], dict) and 'Integer' in data[0]):
                table = integers
            elif (trait_refs is not None and isinstance(data, dict)
                  and data.keys() == {'kind', 'trait_decl_ref'}):
                table = trait_refs
            elif (switch_values is not None and isinstance(data, list) and len(data) == 2
                  and isinstance(data[0], dict) and data[0].keys() & {'Discriminant', 'Bool'}):
                table = switch_values
            else:
                continue
            require(table.setdefault(index, data) == data, 'conflicting LLBC parameter representation')
    return types, integers


def unpack(encoded, deduplicated):
    if 'Deduplicated' in encoded:
        return deduplicated.get(encoded['Deduplicated'], {})
    return encoded['Value'][1]


def enum_variant(type_decls, adt, index, enum):
    decl = type_decls[adt['id']]
    require(name(decl) == enum, 'unexpected enum in the retirement flow')
    return decl['kind']['Enum'][index]['name']


def mapped_callback(translated, definitions, query, functions, types, trait_refs):
    """Follow the actual query_map operand and its compiler-selected FnMut method."""
    callback = query['args'][2].get('Move')
    require(callback is not None and 'Local' in callback['kind'],
            'retirement query must pass its original closure local')
    values = definitions[callback['kind']['Local']]
    require(len(values) == 1 and values[0][0] == 'assign',
            'retirement query callback has no unique construction')
    aggregate, captures = values[0][1]['Aggregate']
    closure, variant, union = aggregate['Adt']
    require(captures == [] and variant is None and union is None,
            'retirement query callback must be the original captureless closure')

    generics = query['func']['Regular']['generics']
    trait_names = {entry['def_id']: name(entry) for entry in translated['trait_decls'] if entry}
    bindings = [unpack(encoded, trait_refs) for encoded in generics['trait_refs']]
    dictionaries = [binding for binding in bindings
                    if trait_names.get(binding['trait_decl_ref']['skip_binder']['id']) == FN_MUT]
    require(len(dictionaries) == 1, 'retirement SELECT must pass one exact FnMut dictionary')
    dictionary = dictionaries[0]
    receiver = dictionary['trait_decl_ref']['skip_binder']['generics']['types'][0]
    require(all(unpack(encoded, types).get('Adt') == closure
                for encoded in (callback['ty'], generics['types'][2], receiver)),
            'retirement SELECT callback and FnMut dictionary disagree on the closure type')
    implementation = dictionary['kind']['TraitImpl']['id']
    impl, = (entry for entry in translated['trait_impls'] if entry and entry['def_id'] == implementation)
    methods = [functions[entry['skip_binder']['id']] for entry in impl['methods']]
    method, = (entry for entry in methods if name(entry)[-1] == 'call_mut')
    require(name(method) == (*BRANCH, 'retire_closed_parent', 'call_mut')
            and method['item_meta']['name'][-2] == {'Impl': {'Trait': implementation}}
            and method['item_meta']['opacity'] == 'Transparent',
            'retirement SELECT did not capture the original FnMut body')
    return method['body']['Structured']


def check_row_columns(translated, body, functions, types, integers):
    require(body['locals']['arg_count'] == 2, 'retirement SELECT changed its row callback arguments')
    assignments = defaultdict(list)
    row_reads = []
    for tag, value, _ in statements(body['body']):
        if tag == 'Assign':
            target, source = value
            if 'Local' in target['kind']:
                assignments[target['kind']['Local']].append(source)
        elif tag == 'Call':
            call = value['call']
            target = call['func'].get('Regular', {}).get('kind', {}).get('Fun')
            if target in functions and name(functions[target]) == SQLITE_ROW_GET:
                row_reads.append(call)

    type_names = {entry['def_id']: name(entry) for entry in translated['type_decls'] if entry}
    columns = []
    for call in row_reads:
        receiver, index = call['args']
        receiver = receiver.get('Move', receiver.get('Copy'))
        [borrow] = assignments[receiver['kind']['Local']]
        reference = borrow['Ref']
        place, projection = reference['place']['kind']['Projection']
        require(reference['kind'] == 'Shared' and projection == 'Deref',
                'retirement column read must borrow the original row')
        [row] = assignments[place['kind']['Local']]
        original = row['Use'][0]
        incoming = original.get('Move', original.get('Copy'))
        argument, projection = incoming['kind']['Projection']
        require(argument['kind'] == {'Local': 2} and projection == {'Field': [None, 0]},
                'retirement column read changed its original callback row')

        index_type, blob_type = call['func']['Regular']['generics']['types']
        vector = unpack(blob_type, types)['Adt']
        byte, = vector['generics']['types']
        require(unpack(index_type, types) == {'Scalar': {'Integer': {'Unsigned': 'Usize'}}}
                and type_names.get(vector['id']) == BYTE_VEC
                and unpack(byte, types) == {'Scalar': {'Integer': {'Unsigned': 'U8'}}},
                'retirement column read must use Row::get<usize, Vec<u8>>')
        literal = unpack(index['Const'], integers)
        scalar, value = literal[0]['Integer']['Unsigned']
        require(scalar == 'Usize', 'retirement column number must be a usize literal')
        columns.append(int(value))
    require(columns == list(range(5)), 'retirement row must read the original five BLOB columns in order')
    return columns


def check_row_returns(translated, body, functions, types, integers, trait_refs, switch_values):
    """Enumerate normal returns of the original captureless row callback."""
    type_decls = {entry['def_id']: entry for entry in translated['type_decls'] if entry}
    trait_names = {entry['def_id']: name(entry) for entry in translated['trait_decls'] if entry}
    returns = []

    def origin(place, values):
        kind = place['kind']
        if 'Local' in kind:
            return values.get(kind['Local'])
        base, projection = kind['Projection']
        incoming = origin(base, values)
        if (incoming is not None and incoming[0] == 'guarded'
                and projection == {'Field': [incoming[3], 0]}):
            return ('value' if incoming[2] == 'Continue' else 'residual', incoming[1])
        return None

    def operand(arg, values):
        place = arg.get('Move', arg.get('Copy'))
        return origin(place, values) if place is not None else None

    def assignment(source, values):
        if 'Use' in source:
            return operand(source['Use'][0], values)
        if 'Ref' in source:
            # The separate column checker binds each shared borrow to the Row argument.
            return None
        aggregate, inputs = source['Aggregate']
        adt, index, _ = aggregate['Adt']
        fields = tuple(operand(arg, values) for arg in inputs)
        if adt['builtin'] == 'Tuple':
            require(index is None, 'retirement Stored tuple must have no variant')
            return ('tuple', fields)
        require(enum_variant(type_decls, adt, index, RESULT) == 'Ok' and len(fields) == 1,
                'retirement row success must construct the original Result::Ok')
        return ('ok', fields[0])

    def execute_call(call, values, events):
        regular = call['func']['Regular']
        callee = name(functions[regular['kind']['Fun']])
        if callee == SQLITE_ROW_GET:
            _, index = call['args']
            bits = unpack(index['Const'], integers)[0]['Integer']['Unsigned']
            column = int(bits[1])
            event, result = 'get', ('read', column)
        else:
            arg, = call['args']
            incoming = operand(arg, values)
            if callee == RESULT_BRANCH:
                require(incoming is not None and incoming[0] == 'read',
                        'retirement row branches on a different get result')
                event, result = 'branch', ('control', incoming[1])
            else:
                require(callee == RESULT_RESIDUAL and incoming is not None
                        and incoming[0] == 'residual',
                        'retirement row must propagate the first original get error')
                reference, = regular['generics']['trait_refs']
                binding = unpack(reference, trait_refs)
                dictionary = binding['trait_decl_ref']['skip_binder']
                source, target = (unpack(ty, types)['Adt']
                                  for ty in dictionary['generics']['types'])
                require(trait_names[dictionary['id']] == IDENTITY_CONVERT and source == target
                        and name(type_decls[source['id']]) == SQLITE_ERROR
                        and 'TraitImpl' in binding['kind'],
                        'retirement row replaced the original error conversion')
                event, result = 'from-residual', ('error', incoming[1])
        values[call['dest']['kind']['Local']] = result
        return (*events, (event, result[1]))

    def arms(switch, values):
        selector = switch['data']
        place = selector['scrutinee']['Discriminant']
        control = origin(place, values)
        require(control is not None and control[0] == 'control' and selector['fallback'] is None,
                'retirement row must match the corresponding get ControlFlow')
        found = {}
        for encoded, position in selector['branches']:
            value, _ = unpack(encoded, switch_values)
            adt, index = value['Discriminant']
            tag = enum_variant(type_decls, adt, index, CONTROL_FLOW)
            require(tag not in found, 'retirement row duplicates a ControlFlow variant')
            found[tag] = (index, position)
        require(len(switch['branches']) == 2 and set(found) == {'Continue', 'Break'}
                and {position for _, position in found.values()} == {0, 1},
                'retirement row needs distinct Continue and Break arms')
        for tag, (index, position) in found.items():
            guarded = dict(values)
            guarded[place['kind']['Local']] = ('guarded', control[1], tag, index)
            yield switch['branches'][position], guarded, (tag, control[1])

    def walk(block, values, events):
        active = [(values, events)]
        for statement in block['statements']:
            flow = []
            effect = statement['kind']
            for state, history in active:
                if effect == 'Return':
                    returns.append((history, state.get(0)))
                    continue
                require(isinstance(effect, dict), 'unexpected terminal instruction in retirement row')
                ((kind, data),) = effect.items()
                if kind == 'Switch':
                    for arm, guarded, outcome in arms(data, state):
                        flow.extend(walk(arm, guarded, (*history, outcome)))
                    continue
                if kind == 'Assign':
                    destination, source = data
                    state[destination['kind']['Local']] = assignment(source, state)
                elif kind == 'Call':
                    history = execute_call(data['call'], state, history)
                elif kind == 'StorageDead':
                    state.pop(data, None)
                else:
                    require(kind in ('StorageLive', 'Borrowck', 'PlaceMention', 'Drop'),
                            'unexpected effect in retirement row callback: ' + kind)
                flow.append((state, history))
            active = flow
        return active

    require(not walk(body['body'], {}, ()), 'retirement row has a path without a Rust return')
    expected = []
    prefix = ()
    for index in range(5):
        prefix = (*prefix, ('get', index), ('branch', index))
        expected.append(((*prefix, ('Break', index), ('from-residual', index)), ('error', index)))
        prefix = (*prefix, ('Continue', index))
    failures = [path for path in returns if path[1] is not None and path[1][0] == 'error']
    require(len(failures) == 5 and set(failures) == set(expected),
            'retirement row must preserve five exact first errors without reading later columns')
    success = (prefix, ('ok', ('tuple', tuple(('value', index) for index in range(5)))))
    require([path for path in returns if path not in failures] == [success],
            'retirement row success must return the original five values in Stored order')

    def columns_read(events):
        return [value for tag, value in events if tag == 'get']

    failures.sort(key=lambda path: path[1][1])
    outcomes = [dict(read=columns_read(events), error_from=index)
                for events, (_, index) in failures]
    events, result = success
    outcomes.append(dict(read=columns_read(events),
                         stored_columns=[column for _, column in result[1][1]]))
    return outcomes


def check_delete_returns(translated, body, functions, execute, types, integers, trait_refs, switch_values):
    """Follow each original execute result to the actual function exit or loop back edge."""
    type_decls = {entry['def_id']: entry for entry in translated['type_decls'] if entry}
    trait_names = {entry['def_id']: name(entry) for entry in translated['trait_decls'] if entry}

    def continuations(block, parents=(), in_loop=False):
        for index, statement in enumerate(block['statements']):
            effect = statement['kind']
            remaining = (*parents, (block, index + 1, in_loop))
            if not isinstance(effect, dict):
                continue
            if 'Call' in effect and effect['Call']['call'] is execute:
                yield remaining
            elif 'Switch' in effect:
                for arm in effect['Switch']['branches']:
                    yield from continuations(arm, remaining)
            elif 'Loop' in effect:
                yield from continuations(effect['Loop'], remaining, True)

    initial, = continuations(body['body'])
    delete_loop = next(block for block, _, in_loop in reversed(initial) if in_loop)

    def origin(place, values):
        kind = place['kind']
        if 'Local' in kind:
            return values.get(kind['Local'])
        base, projection = kind['Projection']
        result = origin(base, values)
        if (isinstance(result, tuple) and result[0] == 'guarded'
                and projection == {'Field': [result[2], 0]}):
            return 'count' if result[1] == 'Continue' else 'sql-residual'
        return None

    def operand(argument, values):
        place = argument.get('Move', argument.get('Copy'))
        return origin(place, values) if place is not None else None

    def assignment(source, values, reply, events):
        if 'Use' in source:
            return operand(source['Use'][0], values)
        if 'BinaryOp' in source:
            op, left, right = source['BinaryOp']
            require(op in ('Eq', 'Ne'), 'retirement DELETE must compare its count with exactly one')
            if 'Const' in left:
                left, right = right, left
            require(operand(left, values) == 'count' and 'Const' in right,
                    'retirement DELETE did not compare the original affected-row count')
            literal = unpack(right['Const'], integers)
            scalar, count = literal[0]['Integer']['Unsigned']
            require(scalar == 'Usize' and int(count) == 1,
                    'retirement DELETE must compare its count with exactly one usize row')
            events.append('compare-one')
            return (reply == 'one') == (op == 'Eq')
        aggregate, inputs = source['Aggregate']
        adt, index, _ = aggregate['Adt']
        if name(type_decls[adt['id']]) == JOURNAL_ERROR:
            error = enum_variant(type_decls, adt, index, JOURNAL_ERROR)
            require(error == 'CorruptStore' and not inputs,
                    'retirement DELETE mismatch must return the original CorruptStore')
            return 'corrupt-store'
        value, = inputs
        require(enum_variant(type_decls, adt, index, RESULT) == 'Err'
                and operand(value, values) == 'corrupt-store',
                'retirement DELETE mismatch must return Err(CorruptStore)')
        return 'returned-corrupt-store'

    def select_arm(switch, values, reply, events):
        selector = switch['data']
        scrutinee = selector['scrutinee']
        if 'Discriminant' in scrutinee:
            place = scrutinee['Discriminant']
            require(origin(place, values) == 'control' and selector['fallback'] is None,
                    'retirement DELETE must branch on its original execute result')
            arms = {}
            for encoded, position in selector['branches']:
                value, _ = unpack(encoded, switch_values)
                adt, index = value['Discriminant']
                tag = enum_variant(type_decls, adt, index, CONTROL_FLOW)
                require(tag not in arms, 'repeated retirement DELETE ControlFlow variant')
                arms[tag] = (index, position)
            require(set(arms) == {'Continue', 'Break'}
                    and {position for _, position in arms.values()} == {0, 1}
                    and len(switch['branches']) == 2,
                    'retirement DELETE needs distinct Continue and Break arms')
            tag = 'Break' if reply == 'sql-error' else 'Continue'
            index, position = arms[tag]
            values[place['kind']['Local']] = ('guarded', tag, index)
            events.append(tag)
        else:
            check = operand(scrutinee['Value'], values)
            require(type(check) is bool, 'retirement DELETE branched on a different count predicate')
            arms = {}
            for encoded, position in selector['branches']:
                value, _ = unpack(encoded, switch_values)
                boolean = value['Bool']
                require(type(boolean) is bool and boolean not in arms,
                        'invalid retirement DELETE count branch')
                arms[boolean] = position
            positions = {value: arms.get(value, selector['fallback']) for value in (False, True)}
            require(set(positions.values()) == {0, 1} and len(switch['branches']) == 2,
                    'retirement DELETE must separate the two count outcomes')
            position = positions[check]
            events.append('count-branch')
        return switch['branches'][position]

    def call_result(call, values, events):
        regular = call['func']['Regular']
        callee = name(functions[regular['kind']['Fun']])
        require(callee in (RESULT_BRANCH, RESULT_RESIDUAL),
                'unexpected call after the original retirement DELETE')
        argument, = call['args']
        value = operand(argument, values)
        if callee == RESULT_BRANCH:
            require(value == 'execute', 'retirement DELETE changed the Result being checked')
            events.append('branch')
            return 'control'
        require(value == 'sql-residual', 'retirement DELETE must propagate the original SQL error')
        reference, = regular['generics']['trait_refs']
        binding = unpack(reference, trait_refs)
        dictionary = binding['trait_decl_ref']['skip_binder']
        converted = tuple(name(type_decls[unpack(ty, types)['Adt']['id']])
                          for ty in dictionary['generics']['types'])
        require(trait_names[dictionary['id']] == IDENTITY_CONVERT
                and converted == (JOURNAL_ERROR, SQLITE_ERROR) and 'TraitImpl' in binding['kind'],
                'retirement DELETE changed the original SQL error conversion')
        events.append('from-residual')
        return 'returned-sql-error'

    def follow(reply):
        scopes = list(initial)
        values = {execute['dest']['kind']['Local']: 'execute'}
        events = []
        while scopes:
            block, index, in_loop = scopes.pop()
            if index == len(block['statements']):
                require(not in_loop, 'retirement DELETE fell through the loop without checking its result')
                continue
            scopes.append((block, index + 1, in_loop))
            effect = block['statements'][index]['kind']
            if effect == 'Return':
                return events, values.get(0)
            require(isinstance(effect, dict), 'retirement DELETE has an unexpected terminal instruction')
            ((tag, detail),) = effect.items()
            if tag == 'Switch':
                scopes.append((select_arm(detail, values, reply, events), 0, False))
            elif tag == 'Assign':
                target, source = detail
                values[target['kind']['Local']] = assignment(source, values, reply, events)
            elif tag == 'Call':
                call = detail['call']
                values[call['dest']['kind']['Local']] = call_result(call, values, events)
            elif tag in ('Break', 'Continue'):
                loops = [pos for pos, (_, _, loop) in enumerate(scopes) if loop]
                require(type(detail) is int and 0 <= detail < len(loops),
                        'retirement DELETE has an invalid loop target')
                target = loops[-1 - detail]
                if tag == 'Continue':
                    require(scopes[target][0] is delete_loop,
                            'retirement DELETE continued a different loop')
                    return events, 'continue-delete-loop'
                del scopes[target:]
            elif tag == 'StorageDead':
                values.pop(detail, None)
            else:
                require(tag in ('StorageLive', 'Borrowck', 'PlaceMention', 'Drop'),
                        'unexpected effect after the original retirement DELETE: ' + tag)
        raise ValueError('retirement DELETE has no original loop continuation or function return')

    success = ['branch', 'Continue', 'compare-one', 'count-branch']
    require(follow('one') == (success, 'continue-delete-loop'),
            'retirement DELETE must continue the original loop only for exactly one row')
    require(follow('other') == (success, 'returned-corrupt-store'),
            'retirement DELETE non-one count must return Err(CorruptStore)')
    require(follow('sql-error') == (['branch', 'Break', 'from-residual'], 'returned-sql-error'),
            'retirement DELETE SQL error must return the original error without continuing')
    return dict(exactly_one='continue-delete-loop', other_count='return-corrupt-store',
                sql_error='return-original-sql-error')


def check_sql_parameters(translated, body, definitions, calls, functions):
    trait_refs = {}
    switch_values = {}
    types, integers = representations(
        translated, trait_refs=trait_refs, switch_values=switch_values,
    )

    def uint(encoded):
        data = unpack(encoded, integers)
        if isinstance(data, list) and 'Integer' in data[0]:
            unsigned = data[0]['Integer'].get('Unsigned')
            if unsigned is not None:
                return ('uint', unsigned[0], int(unsigned[1]))
        return ('other-constant',)

    def type_shape(raw):
        kind = unpack(raw, types)
        if 'Scalar' in kind:
            return kind['Scalar'].get('Integer', {}).get('Unsigned')
        if 'Ref' in kind:
            _, pointee, borrow = kind['Ref']
            return ('ref-type', borrow, type_shape(pointee))
        if 'Slice' in kind:
            return ('slice', type_shape(kind['Slice'][0]))
        if 'Array' in kind:
            item, size, _ = kind['Array']
            return ('array', type_shape(item), uint(size))
        return None

    def invoke(path, *args):
        return ('call', path, args)

    def field(value, variant, index):
        return ('field', variant, index, value)

    def referenced(value, kind):
        return ('ref', kind, value)

    def place(value):
        kind = value['kind']
        if 'Local' in kind:
            return local(kind['Local'])
        base, projection = kind['Projection']
        value = place(base)
        if projection == 'Deref' and value[0] == 'ref':
            return value[2]
        if isinstance(projection, dict) and 'Field' in projection:
            return field(value, *projection['Field'])
        raise ValueError('unsupported projection in SQL parameter provenance')

    def operand(value):
        for mode in ('Move', 'Copy'):
            if mode in value:
                return place(value[mode])
        if 'Const' in value:
            return uint(value['Const'])
        raise ValueError('unsupported SQL parameter operand')

    def place_type(value):
        for mode in ('Move', 'Copy'):
            if mode in value:
                return type_shape(value[mode]['ty'])
        raise ValueError('expected a typed SQL parameter place')

    @cache
    def local(index):
        rows = definitions[index]
        require(len(rows) == 1, 'ambiguous definition of SQL parameter local ' + str(index))
        tag, data = rows[0]
        if tag == 'call':
            path, args, call = data
            if path in {READ_LIMITS, SQLITE_QUERY}:
                # The outer checker separately pins both original receivers.
                return invoke(path)
            if path == NUMBER_FROM:
                return invoke(path, place_type(args[0]), type_shape(call['dest']['ty']),
                              operand(args[0]))
            if path == ARRAY_AS_SLICE:
                generic = call['func']['Regular']['generics']
                return invoke(path, type_shape(generic['types'][0]),
                              uint(generic['const_generics'][0]), operand(args[0]))
            return invoke(path, *(operand(arg) for arg in args))
        if 'Use' in data:
            return operand(data['Use'][0])
        if 'Ref' in data:
            ref = data['Ref']
            return referenced(place(ref['place']), ref['kind'])
        if 'BinaryOp' in data:
            operation, left, right = data['BinaryOp']
            require(operation == {'Add': 'Panic'}, 'retirement limit lost its checked addition')
            return ('add', operand(left), operand(right))
        if 'Aggregate' in data and 'Array' in data['Aggregate'][0]:
            build, values = data['Aggregate']
            item, length, _ = build['Array']
            return ('array', type_shape(item), uint(length), tuple(map(operand, values)))
        raise ValueError('unsupported assignment in SQL parameter provenance')

    query, = (args for path, args, _ in calls if path == SQLITE_QUERY)
    delete, = (args for path, args, _ in calls if path == SQLITE_EXECUTE)
    require(len(query) == len(delete) == 3, 'wrong SQL parameter call arity')
    require(sum(path == READ_LIMITS for path, _, _ in calls) == 1,
            'retirement must read the original branch limits once')
    bounds = field(field(invoke(RESULT_BRANCH, invoke(READ_LIMITS)), 0, 0), 1, 0)
    max_batches = field(bounds, None, 0)
    one = ('uint', 'Usize', 1)
    limit = ('add', invoke(NUMBER_FROM, 'U16', 'U32', max_batches), ('uint', 'U32', 1))
    require(operand(query[1]) == ('array', 'U32', one, (limit,)),
            'retirement SELECT must bind the single u32 max_batches + 1')

    # Follow the original row cursor to load and through both field projections
    # to the digest. The column checker separately resolves its FnMut body.
    issued = field(invoke(RESULT_BRANCH, invoke(SQLITE_QUERY)), 0, 0)
    rows = invoke(INTO_ITER, invoke(ENUMERATE, issued))
    row_result = field(field(invoke(ENUMERATE_NEXT, referenced(rows, 'TwoPhaseMut')), 1, 0), None, 1)
    decoded = field(invoke(RESULT_BRANCH, invoke(
        LOAD_BATCH, field(invoke(RESULT_BRANCH, row_result), 0, 0), bounds)), 0, 0)
    operation = field(field(decoded, None, 0), None, 0)
    digest = field(invoke(RESULT_BRANCH, invoke(DIGEST, referenced(operation, 'Shared'))), 0, 0)
    batches = invoke(VEC_NEW)
    push, = ((args, depth) for path, args, depth in calls if path == VEC_PUSH)
    args, depth = push
    require(depth > 0 and tuple(map(operand, args)) == (referenced(batches, 'TwoPhaseMut'), digest),
            'retirement list must receive the digest of the loaded batch operation')
    require(sum(path == VEC_NEW for path, _, _ in calls) == 1,
            'the retirement digest list must be the original vector')
    key = field(invoke(VEC_NEXT, referenced(invoke(VEC_INTO_ITER, batches), 'TwoPhaseMut')), 1, 0)
    key_slice = invoke(ARRAY_AS_SLICE, 'U8', ('uint', 'Usize', 32), referenced(key, 'Shared'))
    require(operand(delete[2]) == ('array', ('ref-type', 'Shared', ('slice', 'U8')), one, (key_slice,)),
            'retirement DELETE must bind the single full 32-byte operation digest')

    query_call, = (
        data[2] for entries in definitions.values() for kind, data in entries
        if kind == 'call' and data[0] == SQLITE_QUERY
    )
    delete_call, = (
        data[2] for entries in definitions.values() for kind, data in entries
        if kind == 'call' and data[0] == SQLITE_EXECUTE
    )
    callback = mapped_callback(translated, definitions, query_call, functions, types, trait_refs)
    columns = check_row_columns(translated, callback, functions, types, integers)
    row_outcomes = check_row_returns(
        translated, callback, functions, types, integers, trait_refs, switch_values,
    )
    delete_outcomes = check_delete_returns(
        translated, body, functions, delete_call, types, integers, trait_refs, switch_values,
    )
    return dict(
        select_parameter=dict(slot=1, rust_type='u32', source='HostBranchLimits.max_batches', add=1),
        delete_parameter=dict(slot=1, rust_type='&[u8]', source='digest(loaded_record.key.operation)',
                              bytes=32, through='original retirement vector'),
        row_callback_columns=columns,
        row_callback_outcomes=row_outcomes,
        delete_result_outcomes=delete_outcomes,
    )


def check(capture):
    raw = json.loads(capture.read_bytes())
    require(raw['has_errors'] is False, 'retirement capture has frontend errors')
    translated = raw['translated']
    options = translated['options']
    require(not any(options[flag] for flag in ('no_typecheck', 'skip_borrowck', 'monomorphize')),
            'retirement capture omitted original frontend checking')
    files = [entry for entry in translated['files'] if entry['name'] == {'Local': BRANCH_SOURCE}]
    require(len(files) == 1 and sha256(files[0]['contents'].encode()).hexdigest() == BRANCH_SHA256,
            'retirement did not capture the reviewed branch source')
    functions = {entry['def_id']: entry for entry in translated['fun_decls'] if entry}
    root, = (
        entry for entry in functions.values()
        if name(entry) == (*BRANCH, 'retire_closed_parent')
        and entry['item_meta']['name'][-1] == {'Ident': ['retire_closed_parent', 0]}
    )
    require(root['item_meta']['opacity'] == 'Transparent', 'retirement body is opaque')
    body = root['body']['Structured']
    require(body['locals']['arg_count'] == 3 and body['locals']['locals'][1]['name'] == 'tx',
            'retirement no longer receives the original owner argument')

    defs = defaultdict(list)
    calls = []
    for kind, payload, depth in statements(body['body']):
        if kind == 'Assign':
            destination, source = payload
            local = destination['kind'].get('Local')
            if local is not None:
                defs[local].append(('assign', source))
        elif kind == 'Call':
            call = payload['call']
            fun = call['func'].get('Regular', {}).get('kind', {}).get('Fun')
            require(fun in functions, 'retirement introduced an indirect call')
            path = name(functions[fun])
            calls.append((path, call['args'], depth))
            local = call['dest']['kind'].get('Local')
            if local is not None:
                defs[local].append(('call', (path, call['args'], call)))

    def place(source, visiting):
        origin = source['kind']
        if 'Projection' in origin:
            return place(origin['Projection'][0], visiting)
        return resolve(origin['Local'], visiting) if 'Local' in origin else frozenset()

    def operand(source, visiting=frozenset()):
        for kind in ('Move', 'Copy'):
            if kind in source:
                return place(source[kind], visiting)
        constant = source.get('Const', {}).get('Value', [None, None])[1]
        if isinstance(constant, list) and 'Str' in constant[0]:
            return frozenset((('sql', constant[0]['Str']),))
        return frozenset()

    @cache
    def resolve(local, visiting):
        if local == 1:
            return frozenset(('world-transaction',))
        if local in visiting:
            return frozenset(('unknown',))
        visiting = visiting | {local}
        origins = set()
        for kind, item in defs[local]:
            if kind == 'assign':
                if 'Use' in item:
                    origins.update(operand(item['Use'][0], visiting) or {'unknown'})
                elif 'Ref' in item:
                    origins.update(place(item['Ref']['place'], visiting) or {'unknown'})
                else:
                    origins.add('unknown')
                continue
            callee, args, _ = item
            if callee in RECEIVER_ADAPTERS:
                original, returned = RECEIVER_ADAPTERS[callee]
                incoming = operand(args[0], visiting)
                if original in incoming:
                    origins.add(returned)
                if incoming != {original}:
                    origins.add('unknown')
            elif callee == RESULT_BRANCH:
                origins.update(operand(args[0], visiting) or {'unknown'})
            else:
                origins.add('unknown')
        return frozenset(origins)

    # These are the only direct SQL calls. The standard SQLite library methods
    # and the separate read_limits helper remain explicit provider boundaries.
    sql_paths = [
        path for path, _, _ in calls
        if path[0] == 'rusqlite' and path != SQLITE_DEREF
    ]
    require(sql_paths == [SQLITE_PREPARE, SQLITE_QUERY, SQLITE_EXECUTE],
            'retirement gained or reordered a direct SQL effect')
    for callee, args, _ in calls:
        if callee in USERS_OF_CONNECTION:
            continue
        require(not any(operand(arg) & CONNECTION_ORIGINS for arg in args),
                'original transaction escaped through ' + '::'.join(callee))
        require(callee[-1] not in ('begin', 'commit', 'rollback', 'execute_batch'),
                'unexpected transaction effect: ' + '::'.join(callee))
    for callee, args, depth in calls:
        if callee == READ_LIMITS:
            require(operand(args[0]) == {'sqlite-connection'}, 'limit read changed its connection')
        elif callee == SQLITE_PREPARE:
            require(depth == 0 and operand(args[0]) == {'sqlite-connection'}
                    and operand(args[1]) == {('sql', SELECT)}, 'retirement changed its SELECT')
        elif callee == SQLITE_QUERY:
            require(depth == 0 and operand(args[0]) == {'sqlite-statement'},
                    'row read is detached from the original statement')
        elif callee == SQLITE_EXECUTE:
            require(depth > 0 and operand(args[0]) == {'sqlite-connection'}
                    and operand(args[1]) == {('sql', DELETE)}, 'retirement changed its DELETE')

    return dict(root='::'.join(name(root)), capture_sha256=sha(capture),
                branch_source_sha256=BRANCH_SHA256, original_owner_argument=1,
                direct_calls=len(calls), select=SELECT, delete=DELETE,
                separate_transaction_calls=0,
                **check_sql_parameters(translated, body, defs, calls, functions))


if __name__ == '__main__':
    result = check(Path(sys.argv[1]))
    print(json.dumps(result, indent=2))
