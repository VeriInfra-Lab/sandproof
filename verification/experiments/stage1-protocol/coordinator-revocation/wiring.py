"""Check the normal-return wiring of the two original Charon parent bodies."""
from collections import Counter

SERVICE = 'sandproof_controller_service'
COORD = SERVICE + '::application::coordinator'
MODULE = COORD + '::lifecycle_intent'
MODEL = SERVICE + '::controller::lifecycle_inbox::model'
PARENT = MODULE + '::{impl}::record_lifecycle_intent'
JOURNAL = SERVICE + '::controller::lifecycle_inbox::{impl}::record_lifecycle_intent'
DECIDE = MODULE + '::data_revocation'
ONE = COORD + '::{impl}::revoke_data'
ALL = 'tokio::sync::watch::{impl}::send_replace'
LOCK = 'std::sync::poison::mutex::{impl}::lock'
VALUES = 'alloc::collections::btree::map::{impl}::values'


def require(ok, message):
    if not ok:
        raise ValueError(message)


def name(row):
    return '::'.join(part['Ident'][0] if 'Ident' in part else '{impl}' for part in row['item_meta']['name'])


def declaration(rows, target):
    matches = [r for r in rows if r and name(r) == target]
    require(len(matches) == 1, 'expected one declaration: ' + target)
    return matches[0]


def nodes(value, key):
    if isinstance(value, dict):
        if key in value:
            yield value[key]
        for child in value.values():
            yield from nodes(child, key)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child, key)


def origin(place, assignments):
    """Follow the actual temporary borrows/copies to a named local and its field offsets."""
    kind = place['kind']
    if 'Projection' in kind:
        base, proj = kind['Projection']
        path = origin(base, assignments)
        return path if proj == 'Deref' else (*path, tuple(proj['Field']))
    local = kind['Local']
    if local not in assignments:
        return (local,)
    rhs = assignments[local]
    if 'Ref' in rhs:
        return origin(rhs['Ref']['place'], assignments)
    operand = rhs['Use'][0]
    return origin(operand.get('Move', operand.get('Copy')), assignments)


def assignments_before(block, stop):
    assignments = {}
    for statement in block['statements']:
        if statement is stop:
            break
        kind = statement['kind']
        if isinstance(kind, dict) and 'Assign' in kind:
            dst, rhs = kind['Assign']
            if 'Local' in dst['kind']:
                assignments[dst['kind']['Local']] = rhs
    return assignments


def inspect_parent(raw, *, refactored):
    require(raw['has_errors'] is False, 'incomplete parent capture')
    trans = raw['translated']
    rows = trans['fun_decls']
    parent = declaration(rows, PARENT)
    body = parent['body']['Structured']
    block = body['body']
    labels = {v['name']: v['index'] for v in body['locals']['locals'] if v['name']}

    def callee(call):
        return name(rows[call['func']['Regular']['kind']['Fun']])

    def calls(part):
        return [call['call'] for call in nodes(part, 'Call')]

    effects = {LOCK, JOURNAL, ONE, VALUES, ALL}
    if refactored:
        effects.add(DECIDE)
    require(Counter(callee(c) for c in calls(block) if callee(c) in effects) == Counter(effects),
            'missing or duplicated parent call')
    top = [(s, s['kind']['Call']['call']) for s in block['statements']
           if isinstance(s['kind'], dict) and 'Call' in s['kind']]
    require([callee(c) for _, c in top if callee(c) in effects] ==
            [LOCK, JOURNAL] + ([DECIDE] if refactored else []), 'lock, record or decision out of order')
    lock_statement, lock_call = next((s, c) for s, c in top if callee(c) == LOCK)
    require(origin(lock_call['args'][0]['Move'], assignments_before(block, lock_statement)) ==
            (labels['self'], (None, 0)), 'lock uses another application field')
    fields = declaration(trans['type_decls'], COORD + '::Inner')['kind']['Struct']
    require(fields[0]['name'] == 'journal' and fields[4]['name']
            == 'data_revocations', 'inner field layout differs')
    application = declaration(trans['type_decls'], COORD + '::ControllerApplication')['kind']['Struct']
    require(application[0]['name'] == 'inner', 'wrong application mutex')

    def guard_argument(part, consumer, member, fields):
        prior = assignments_before(part, consumer)
        guard_calls = [(s, s['kind']['Call']['call']) for s in part['statements']
                       if isinstance(s['kind'], dict) and 'Call' in s['kind']
                       and callee(s['kind']['Call']['call']).endswith('::' + member)]
        (statement, borrow), = guard_calls
        require(part['statements'].index(statement) <
                part['statements'].index(consumer), 'borrow after effect')
        before = assignments_before(part, statement)
        before.pop(labels['inner'], None)
        require(origin(borrow['args'][0]['Move'], before) == (labels['inner'],), 'different guard')
        arg = consumer['kind']['Call']['call']['args'][0]['Move']
        require(origin(arg, prior) == (borrow['dest']['kind']['Local'], *fields), 'wrong guarded object')
    record_statement, journal = next((s, c) for s, c in top if callee(c) == JOURNAL)
    guard_argument(block, record_statement, 'deref_mut', [(None, 0)])
    result = journal['dest']['kind']['Local']
    require(result == labels['result'], 'journal result local differs')
    actuals = assignments_before(block, record_statement)
    require([origin(op['Move'], actuals) for op in journal['args'][1:3]] ==
            [(labels['intent'],), (labels['context'],)], 'journal received another intent or context')
    returns = [k['Assign'][1]['Use'][0] for s in block['statements'] if isinstance(k := s['kind'], dict)
               and 'Assign' in k and k['Assign'][0]['kind'] == {'Local': 0}]
    require(len(returns) == 1 and returns[0]['Move']['kind'] == {'Local': result}
            and block['statements'][-1]['kind'] == 'Return', 'parent does not return its original journal result')
    send, = [c for c in calls(block) if callee(c) == ALL]
    literal = send['args'][1]['Const']
    bools = {ident: value[0]['Bool'] for pair in nodes(block, 'Value')
             if isinstance(pair, list) and len(pair) == 2 for ident, value in [pair]
             if isinstance(value, list) and len(value) == 2 and isinstance(value[0], dict) and 'Bool' in value[0]}
    idx = literal['Value'][0] if 'Value' in literal else literal['Deduplicated']
    require(bools[idx] is True, 'signal value differs')
    if not refactored:
        return

    decision_statement, decision = next((s, c) for s, c in top if callee(c) == DECIDE)
    before = assignments_before(block, decision_statement)
    require([origin(op['Move'], before) for op in decision['args']] ==
            [(labels['intent'], (None, 0), (None, 1)), (result,)], 'wrong operation or result passed')
    for path, fields in [('sandproof_protocol::lifecycle::authority::AcceptedLifecycleIntent', ['request']),
                         ('sandproof_protocol::lifecycle::request::LifecycleRequest', ['binding', 'operation'])]:
        actual = declaration(trans['type_decls'], path)['kind']['Struct']
        require([field['name'] for field in actual[:len(fields)]]
                == fields, 'projected field differs: ' + path)
    decision_local = decision['dest']['kind']['Local']
    sw, = [s for s in nodes(block, 'Switch') if s['data']['scrutinee'].get('Discriminant', {}).get('kind')
           == {'Local': decision_local}]
    enum = declaration(trans['type_decls'], MODULE + '::DataRevocation')
    variants = {v['id']: v['name'] for v in enum['kind']['Enum']}
    branches = {}
    for const, index in sw['data']['branches']:
        adt, variant = const['Value'][1][0]['Discriminant']
        require(adt['id'] == enum['def_id'], 'dispatch uses another discriminant')
        branches[variants[variant]] = sw['branches'][index]
    require(sw['data']['fallback'] is None and set(branches) ==
            {'None', 'Sandbox', 'All'}, 'incomplete dispatch')
    require(not calls(branches['None']), 'no-revocation arm calls a function')
    require([callee(c) for c in calls(branches['Sandbox']) if callee(c) in effects] == [ONE]
            and [callee(c) for c in calls(branches['All']) if callee(c) in effects] == [VALUES, ALL],
            'targeted and broadcast effects differ')
    targeted = branches['Sandbox']
    stmt, = [s for s in targeted['statements'] if isinstance(s['kind'], dict) and 'Call' in s['kind']
             and callee(s['kind']['Call']['call']) == ONE]
    guard_argument(targeted, stmt, 'deref_mut', [])
    broadcast = branches['All']
    values, = [s for s in broadcast['statements'] if isinstance(s['kind'], dict) and 'Call' in s['kind']
               and callee(s['kind']['Call']['call']) == VALUES]
    guard_argument(broadcast, values, 'deref', [(None, 4)])
    argument = stmt['kind']['Call']['call']['args'][1]['Move']
    require(origin(argument, assignments_before(targeted, stmt)) == (decision_local, (1, 0)),
            'targeted revocation lost the selected sandbox')
    require(len(list(nodes(branches['All'], 'Loop'))) == 2, 'broadcast loops differ')
