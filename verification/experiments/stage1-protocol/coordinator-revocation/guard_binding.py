"""Connect the caller's mutex result, error closure, early return and journal guard in LLBC."""
from sender_binding import enum_edges, output
from wiring import JOURNAL, LOCK, PARENT, SERVICE, assignments_before, declaration, name, origin, require

RESULT = 'core::result::{impl}::'
DEREF_MUT = 'std::sync::poison::mutex::{impl}::deref_mut'


def flow(view, block):
    """Retain all normal control transfers and calls. Cleanup on unwind is a separate edge."""
    for statement in block['statements']:
        kind = statement['kind']
        tag = next(iter(kind)) if isinstance(kind, dict) else kind
        if tag in ('StorageLive', 'StorageDead', 'Assign', 'Borrowck', 'Drop'):
            continue
        if tag == 'Call':
            function = kind[tag]['call']['func']['Regular']['kind']['Fun']
            tag = name(view.functions[function])
        yield tag, statement


def check_mapper(view, block, mapped):
    call = mapped['kind']['Call']['call']
    error = declaration(view.type_decls.values(), SERVICE + '::JournalError')
    busy, = [v for v in error['kind']['Enum'] if v['name'] == 'ControllerBusy' and not v['fields']]
    callback = call['args'][1]['Move']
    callback_type = view.resolve(callback['ty'], view.types)['Adt']['id']
    aggregate, captures = assignments_before(block, mapped)[callback['kind']['Local']]['Aggregate']
    require(aggregate['Adt'][0]['id'] == callback_type and not captures, 'unexpected map_err callback value')
    traits = [view.resolve(t, view.traits) for t in call['func']['Regular']['generics']['trait_refs']]
    fn_once, = [t for t in traits if name(view.trait_decls[t['trait_decl_ref']['skip_binder']['id']])
                == 'core::ops::function::FnOnce']
    impl = view.impls[fn_once['kind']['TraitImpl']['id']]
    method, = impl['methods']
    closure = view.functions[method['skip_binder']['id']]
    require(impl['impl_trait']['id'] == fn_once['trait_decl_ref']['skip_binder']['id']
            and view.resolve(impl['impl_trait']['generics']['types'][0], view.types)['Adt']['id'] == callback_type
            and name(closure) == PARENT + '::{impl}::call_once'
            and view.resolve(closure['signature']['inputs'][0], view.types)['Adt']['id'] == callback_type,
            'map_err uses a different FnOnce implementation')
    body = closure['body']['Structured']['body']
    require([tag for tag, _ in flow(view, body)] == ['Return'], 'unexpected control in lock error mapper')
    result, = [rhs for s in body['statements'] if 'Assign' in s['kind']
               for dest, rhs in [s['kind']['Assign']] if dest['kind'] == {'Local': 0}]
    adt, fields = result['Aggregate']
    ty, variant, union_field = adt['Adt']
    require((ty['id'], variant, union_field, fields) == (error['def_id'], busy['id'], None, []),
            'lock error mapper does not return ControllerBusy')


def guard_uses(block, guard):
    """Track writes, drops and end of storage for the named guard on normal edges."""
    local = {'Local': guard}
    for statement in block['statements']:
        k = statement['kind']
        if not isinstance(k, dict):
            continue
        if ('Assign' in k and k['Assign'][0]['kind'] == local
                or 'Call' in k and k['Call']['call']['dest']['kind'] == local):
            yield 'write', statement
        elif 'Drop' in k and k['Drop']['place']['kind'] == local:
            yield 'drop', statement
        elif k.get('StorageDead') == guard:
            yield 'dead', statement
        if 'Switch' in k:
            for arm in k['Switch']['branches']:
                yield from guard_uses(arm, guard)
        elif 'Loop' in k:
            yield from guard_uses(k['Loop'], guard)


def check_guard(view):
    body = declaration(view.functions.values(), PARENT)['body']['Structured']
    block = body['body']
    journal_prefix = []
    for tag, statement in flow(view, block):
        journal_prefix.append((tag, statement))
        if tag == JOURNAL:
            break
    require([tag for tag, _ in journal_prefix] ==
            [LOCK, RESULT + 'map_err', RESULT + 'branch', 'Switch', DEREF_MUT, JOURNAL],
            'unexpected control before the journal call')
    lock, mapped, branch, switch, deref, _ = [s for _, s in journal_prefix]
    for consumer, producer in [(mapped, lock), (branch, mapped)]:
        args = consumer['kind']['Call']['call']['args']
        require(len(args) == (2 if consumer is mapped else 1)
                and origin(args[0]['Move'], assignments_before(block, consumer)) == (output(producer),),
                'lock result is disconnected from map_err or Try::branch')
    check_mapper(view, block, mapped)
    success, failure = enum_edges(view, switch, branch, 'core::ops::control_flow::ControlFlow',
                                  [(0, 'Continue', 1), (1, 'Break', 1)])
    require(not success['statements'], 'lock Continue arm does not fall through directly')
    failure_flow = list(flow(view, failure))
    require([tag for tag, _ in failure_flow] == [RESULT + 'from_residual', 'Return'],
            'lock Break arm does not return directly through from_residual')
    residual = failure_flow[0][1]
    call = residual['kind']['Call']['call']
    arg, = call['args']
    before = assignments_before(block, switch) | assignments_before(failure, residual)
    require(origin(arg['Move'], before) == (output(branch), (1, 0)) and call['dest']['kind'] == {'Local': 0}
            and all(s['kind']['Assign'][0]['kind'] != {'Local': 0} for s in failure['statements']
                    if 'Assign' in s['kind']), 'early return loses the original lock residual')

    guard, = [v['index'] for v in body['locals']['locals'] if v['name'] == 'inner']
    place = {'kind': {'Local': guard}}
    require(origin(place, assignments_before(block, deref)) == (output(branch), (0, 0)),
            'journal guard does not come from lock Continue')
    # The Break edge has already returned. Follow the Continue suffix through dispatch and release.
    tail = block['statements'][block['statements'].index(switch) + 1:]
    uses = list(guard_uses({'statements': tail}, guard))
    require([action for action, _ in uses] == ['write', 'drop', 'dead']
            and all(any(s is event for s in tail) for _, event in uses),
            'guard is replaced or released inside the dispatch')
    after_drop = tail[tail.index(uses[1][1]) + 1:]
    require([tag for tag, _ in flow(view, {'statements': after_drop})] == ['Return'],
            'coordinator dispatch continues after guard release')
    return guard
