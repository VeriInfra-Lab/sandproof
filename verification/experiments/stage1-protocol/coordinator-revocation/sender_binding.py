"""Admit the operand and loop edges up to the actual Tokio send call in fixed Charon LLBC."""
from wiring import ALL, COORD, ONE, VALUES, assignments_before, declaration, name, origin, require

GET = 'alloc::collections::btree::map::{impl}::get'
VEC_ITER = 'alloc::vec::{impl}::into_iter'
SLICE_NEXT = 'core::slice::iter::{impl}::next'
MAP_ITER = 'core::iter::traits::collect::{impl}::into_iter'
MAP_NEXT = 'alloc::collections::btree::map::{impl}::next'
DEREF = 'std::sync::poison::mutex::{impl}::deref'
SENDER = 'tokio::sync::watch::Sender<Bool>'
TABLE = f'alloc::collections::btree::map::BTreeMap<U64,alloc::vec::Vec<{SENDER}>,alloc::alloc::Global>'
SOME_FIELD = (1, 0)


def steps(block, view, *expected):
    """Check every normal-edge control or call, allowing only local temporary preparation around them."""
    tags = []
    selected = []
    for statement in block['statements']:
        kind = statement['kind']
        tag = next(iter(kind)) if isinstance(kind, dict) else kind
        if tag in ('StorageLive', 'StorageDead'):
            continue
        if tag == 'Assign':
            dest, expr = kind[tag]
            require('Local' in dest['kind'], 'unexpected nonlocal sender write')
            if 'Aggregate' in expr:
                adt, args = expr['Aggregate']
                require(adt['Adt'][0]['builtin'] == 'Tuple' and not args, 'unexpected sender aggregate')
            else:
                require(set(expr) in ({'Ref'}, {'Use'}), 'unexpected sender temporary')
            continue
        if tag == 'Call':
            call = kind[tag]['call']
            tag = name(view.functions[call['func']['Regular']['kind']['Fun']])
        elif tag in ('Continue', 'Break'):
            require(kind[tag] == 0, 'sender loop targets a different enclosing loop')
        tags.append(tag)
        selected.append(statement)
    require(tags == list(expected), 'sender block has different calls or control flow: ' + repr(tags))
    return selected


def check_definitions(block, protected):
    """This slice only defines fresh temporaries. No branch can overwrite an admitted origin."""
    defined = set(protected)

    def visit(part):
        for statement in part['statements']:
            kind = statement['kind']
            if not isinstance(kind, dict):
                continue
            if 'Switch' in kind:
                for arm in kind['Switch']['branches']:
                    visit(arm)
            elif 'Loop' in kind:
                visit(kind['Loop'])
            elif 'Assign' in kind or 'Call' in kind:
                if 'Assign' in kind:
                    dest, expr = kind['Assign']
                    if dest['kind'] == {'Local': 0} and 'Aggregate' in expr:
                        continue  # steps() also checks that this constructs unit.
                else:
                    dest = kind['Call']['call']['dest']
                local = dest['kind']['Local']
                require(local not in defined, 'sender operand local has multiple definitions')
                defined.add(local)
    visit(block)


def output(statement):
    return statement['kind']['Call']['call']['dest']['kind']['Local']


def check_args(block, statement, inherited, *expected):
    sources = inherited | assignments_before(block, statement)
    actual = statement['kind']['Call']['call']['args']
    require(len(actual) == len(expected), 'sender call arity differs')
    for arg, path in zip(actual, expected, strict=True):
        place = arg.get('Move', arg.get('Copy'))
        require(place is not None and origin(place, sources) == path, 'sender call operand has a different origin')


def enum_edges(view, statement, producer, enum_name, layout):
    switch = statement['kind']['Switch']
    data = switch['data']
    require(data['scrutinee']['Discriminant']['kind'] == {'Local': output(producer)},
            'enum branch uses a different call result')
    enum = declaration(view.type_decls.values(), enum_name)
    require([(v['id'], v['name'], len(v['fields'])) for v in enum['kind']['Enum']] == layout,
            'unexpected enum layout: ' + enum_name)
    arms = {}
    for const, edge in data['branches']:
        adt, variant = view.resolve(const, view.constants)[0]['Discriminant']
        require(adt['id'] == enum['def_id'] and variant in (0, 1) and variant not in arms,
                'unexpected enum discriminant')
        arms[variant] = edge
    edges = [arms.get(i, data['fallback']) for i in (0, 1)]
    require(len(switch['branches']) == 2 and set(edges) == {0, 1}, 'enum arms are not distinct and exhaustive')
    return [switch['branches'][i] for i in edges]


def option_edges(view, statement, producer):
    return enum_edges(view, statement, producer, 'core::option::Option', [(0, 'None', 0), (1, 'Some', 1)])


def check_vec_loop(view, block, loop, iterator, inherited):
    body = loop['kind']['Loop']
    before = inherited | assignments_before(block, loop)
    advance, branch, send, _ = steps(body, view, SLICE_NEXT, 'Switch', ALL, 'Continue')
    check_args(body, advance, before, (output(iterator),))
    exhausted, present = option_edges(view, branch, advance)
    steps(exhausted, view, 'Break')
    steps(present, view)
    args = send['kind']['Call']['call']['args']
    before_send = before | assignments_before(body, send)
    require(len(args) == 2 and origin(args[0]['Move'], before_send) == (output(advance), SOME_FIELD),
            'send_replace receiver differs from the item yielded by next')
    require(view.type_name(args[0]['Move']['ty']) == '&' + SENDER and
            view.resolve(args[1]['Const'], view.constants)[0] == {'Bool': True}, 'wrong sender type or value')


def inspect_senders(view, broadcast):
    method = declaration(view.functions.values(), ONE)
    require(method['item_meta']['opacity'] == 'Transparent' and method['item_meta']['is_local'],
            'expected the original transparent Inner::revoke_data method')
    body = method['body']['Structured']
    require(body['locals']['arg_count'] == 2, 'unexpected revoke_data signature')
    sandbox = 'sandbox_controller_core::ids::SandboxId'
    require(view.type_name(method['signature']['inputs'][1]) == sandbox, 'unexpected sandbox argument')
    key, = declaration(view.type_decls.values(), sandbox)['kind']['Struct']
    fields = declaration(view.type_decls.values(), COORD + '::Inner')['kind']['Struct']
    require(view.type_name(key['ty']) == 'U64' and fields[4]['name'] == 'data_revocations'
            and view.type_name(fields[4]['ty']) == TABLE, 'unexpected revocation table or key')

    block = body['body']
    check_definitions(block, {0, 1, 2})
    lookup, branch, iterator, loop, _ = steps(block, view, GET, 'Switch', VEC_ITER, 'Loop', 'Return')
    check_args(block, lookup, {}, (1, (None, 4)), (2, (None, 0)))
    missing, found = option_edges(view, branch, lookup)
    steps(missing, view, 'Return')
    steps(found, view)
    check_args(block, iterator, {}, (output(lookup), SOME_FIELD))
    check_vec_loop(view, block, loop, iterator, {})

    if broadcast is not None:
        # The caller checker already binds the original .values() receiver to the locked Inner's table.
        check_definitions(broadcast, {0})
        _, values, outer_iter, outer_loop = steps(broadcast, view, DEREF, VALUES, MAP_ITER, 'Loop')
        check_args(broadcast, outer_iter, {}, (output(values),))
        outer = outer_loop['kind']['Loop']
        before = assignments_before(broadcast, outer_loop)
        advance, branch = steps(outer, view, MAP_NEXT, 'Switch')
        check_args(outer, advance, before, (output(outer_iter),))
        exhausted, present = option_edges(view, branch, advance)
        steps(exhausted, view, 'Break')
        iterator, loop, _ = steps(present, view, VEC_ITER, 'Loop', 'Continue')
        before_item = before | assignments_before(outer, branch)
        check_args(present, iterator, before_item, (output(advance), SOME_FIELD))
        check_vec_loop(view, present, loop, iterator, before_item)
    return {'method_def_id': method['def_id'], 'table_type': TABLE, 'targeted_loop': True,
            'broadcast_loops': broadcast is not None}
