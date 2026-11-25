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


def representations(root, *, trait_refs=None):
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
            else:
                continue
            require(table.setdefault(index, data) == data, 'conflicting LLBC parameter representation')
    return types, integers


def unpack(encoded, deduplicated):
    if 'Deduplicated' in encoded:
        return deduplicated.get(encoded['Deduplicated'], {})
    return encoded['Value'][1]


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


def check_sql_parameters(translated, definitions, calls, functions):
    trait_refs = {}
    types, integers = representations(translated, trait_refs=trait_refs)

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
    callback = mapped_callback(translated, definitions, query_call, functions, types, trait_refs)
    columns = check_row_columns(translated, callback, functions, types, integers)
    return dict(
        select_parameter=dict(slot=1, rust_type='u32', source='HostBranchLimits.max_batches', add=1),
        delete_parameter=dict(slot=1, rust_type='&[u8]', source='digest(loaded_record.key.operation)',
                              bytes=32, through='original retirement vector'),
        row_callback_columns=columns,
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
                **check_sql_parameters(translated, defs, calls, functions))


if __name__ == '__main__':
    result = check(Path(sys.argv[1]))
    print(json.dumps(result, indent=2))
