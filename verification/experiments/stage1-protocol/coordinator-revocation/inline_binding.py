"""Check the inlined normal-return decision on the fixed pre-refactor coordinator LLBC."""
from guard_binding import DEREF_MUT
from sender_binding import DEREF, MAP_ITER, check_args, check_definitions, output, steps
from wiring import COORD, JOURNAL, MODEL, ONE, PARENT, SERVICE, VALUES, assignments_before, declaration, name, origin, require

PROTOCOL = 'sandproof_protocol::lifecycle::'
OPERATION = PROTOCOL + 'request::LifecycleOperation'
ERROR = SERVICE + '::JournalError'
METADATA = {'StorageLive', 'StorageDead', 'Borrowck', 'PlaceMention'}


def executable(block):
    return [statement for statement in block['statements']
            if not (isinstance(statement['kind'], dict) and set(statement['kind']) <= METADATA)]


def field(view, owner, member):
    fields = declaration(view.type_decls.values(), owner)['kind']['Struct']
    return (None, [item['name'] for item in fields].index(member))


def arms(view, statement, sources, expected, domain):
    switch = statement['kind']['Switch']
    data = switch['data']
    scrutinee = data['scrutinee']
    if domain == 'Bool':
        operand = scrutinee['Value']
        subject = operand.get('Move', operand.get('Copy'))
        choices = {True, False}
    else:
        subject = scrutinee['Discriminant']
        enum = declaration(view.type_decls.values(), domain)
        variants = {v['id']: v['name'] for v in enum['kind']['Enum']}
        choices = set(variants.values())
    require(view.type_name(subject['ty']).split('<', 1)[0] == domain and origin(subject, sources) == expected,
            'original switch reads another value: ' + domain)
    edges = {}
    for constant, index in data['branches']:
        value = view.resolve(constant, view.constants)[0]
        if domain == 'Bool':
            label = value['Bool']
        else:
            adt, variant = value['Discriminant']
            require(adt['id'] == enum['def_id'], 'original switch tests another enum: ' + domain)
            label = variants[variant]
        require(label in choices and label not in edges, 'invalid original branch label: ' + domain)
        edges[label] = index
    selected = {label: edges.get(label, data['fallback']) for label in choices}
    require(set(selected.values()) == set(range(len(switch['branches']))), 'incomplete original switch: ' + domain)
    return {label: switch['branches'][index] for label, index in selected.items()}


def split(view, block, sources, path, domain, protected):
    *prefix, branch = executable(block)
    require('Switch' in branch['kind'], 'original conditional has another exit: ' + domain)
    before = dict(sources)
    for statement in prefix:
        dest, value = statement['kind']['Assign']
        local = dest['kind']['Local']
        require(local not in before and local not in protected and set(value) in ({'Ref'}, {'Use'}),
                'original conditional overwrites an operand: ' + domain)
        before[local] = value
    return arms(view, branch, before, path, domain), before


def empty(block):
    require(not executable(block), 'original inactive arm has an effect')


def inspect_inline(view, guard):
    """Return the original broadcast block for the same sender checks as the refactored arm."""
    body = declaration(view.functions.values(), PARENT)['body']['Structured']
    block = body['body']
    intent, = [item['index'] for item in body['locals']['locals'] if item['name'] == 'intent']
    stmts = executable(block)
    journal, = [statement for statement in stmts if 'Call' in statement['kind']
                and name(view.functions[statement['kind']['Call']['call']['func']['Regular']['kind']['Fun']]) == JOURNAL]
    result = output(journal)
    gate, dispatch, *tail = stmts[stmts.index(journal) + 1:]
    require(len(tail) >= 2 and 'Assign' in tail[0]['kind'] and tail[-1]['kind'] == 'Return'
            and all('Drop' in s['kind'] for s in tail[1:-1]), 'original coordinator continues after dispatch')
    dest, value = tail[0]['kind']['Assign']
    require(dest['kind'] == {'Local': 0} and value['Use'][0]['Move']['kind'] == {'Local': result},
            'original coordinator returns another value')
    operation = (intent, field(view, PROTOCOL + 'authority::AcceptedLifecycleIntent', 'request'),
                 field(view, PROTOCOL + 'request::LifecycleRequest', 'operation'))
    cases = arms(view, gate, assignments_before(block, gate), operation, OPERATION)
    require({'Unload', 'Destroy'} <= cases.keys(), 'missing original access-ending operation')
    protected = set(range(body['locals']['arg_count'] + 1)) | {result, guard}
    flags = set()
    for label, arm in cases.items():
        write, = executable(arm)
        dest, value = write['kind']['Assign']
        flag = dest['kind']['Local']
        require(flag not in protected and view.type_name(dest['ty']) == 'Bool' and
                view.resolve(value['Use'][0]['Const'], view.constants)[0] == {'Bool': label in ('Unload', 'Destroy')},
                'original operation guard selects a different action: ' + label)
        flags.add(flag)
    require(len(flags) == 1, 'original operation arms write different guards')
    flag, = flags
    active = arms(view, dispatch, {}, (flag,), 'Bool')
    empty(active[False])
    protected.add(flag)
    results, sources = split(view, active[True], {}, (result,), 'core::result::Result', protected)
    require(set(results) == {'Ok', 'Err'}, 'incomplete original journal result')
    result_variants = declaration(view.type_decls.values(), 'core::result::Result')['kind']['Enum']
    result_fields = {v['name']: (v['id'], 0) for v in result_variants if len(v['fields']) == 1}
    state = (result, result_fields['Ok'])
    current, success = split(view, results['Ok'], sources, (*state, field(view, MODEL + '::LifecycleWorkState', 'current')),
                             'Bool', protected)
    empty(current[False])
    local = (*state, field(view, MODEL + '::LifecycleWorkState', 'local'))
    binding, success = split(view, current[True], success, local, 'core::option::Option', protected)
    empty(binding['None'])
    targeted = binding['Some']
    check_definitions(targeted, protected | success.keys())
    borrow, revoke = steps(targeted, view, DEREF_MUT, ONE)
    check_args(targeted, borrow, success, (guard,))
    sandbox = field(view, SERVICE + '::controller::internal_inbox::model::InternalLocalBinding', 'sandbox')
    variants = declaration(view.type_decls.values(), 'core::option::Option')['kind']['Enum']
    some, = [v['id'] for v in variants if v['name'] == 'Some' and len(v['fields']) == 1]
    check_args(targeted, revoke, success, (output(borrow),), (*local, (some, 0), sandbox))
    failures, before_error = split(view, results['Err'], sources, (result, result_fields['Err']), ERROR, protected)
    for label, arm in failures.items():
        if label != 'CommitUncertain':
            empty(arm)
    broadcast = failures['CommitUncertain']
    check_definitions(broadcast, protected | before_error.keys())
    borrow, values, _, _ = steps(broadcast, view, DEREF, VALUES, MAP_ITER, 'Loop')
    check_args(broadcast, borrow, before_error, (guard,))
    check_args(broadcast, values, before_error, (output(borrow), field(view, COORD + '::Inner', 'data_revocations')))
    return broadcast
