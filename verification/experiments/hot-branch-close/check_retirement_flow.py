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
RESULT_BRANCH = ('core', 'result', 'branch')
READ_LIMITS = (*BRANCH, 'read_limits')
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
                defs[local].append(('call', (path, call['args'])))

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
            callee, args = item
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
                separate_transaction_calls=0)


if __name__ == '__main__':
    result = check(Path(sys.argv[1]))
    print(json.dumps(result, indent=2))
