"""Reject disconnected coordinator edges on private copies of an admitted compiler capture."""
from copy import deepcopy

from guard_binding import RESULT, check_guard
from inline_binding import ERROR, OPERATION, inspect_inline
from sender_binding import ALL, GET, ONE, VALUES, VEC_ITER, inspect_senders
from wiring import PARENT, SERVICE, declaration, name, nodes, require


def expect_rejection(label, diagnostic, check, *inputs):
    try:
        check(*inputs)
    except ValueError as error:
        require(str(error).split(':', 1)[0] == diagnostic, f'{label}: unexpected diagnostic: {error}')
    else:
        raise ValueError(f'{label}: admitted disconnected compiler evidence')


def check_guard_rejections(view):
    cases = [
        ('guard-source', 'journal guard does not come from lock Continue'),
        ('poison-error', 'lock error mapper does not return ControllerBusy'),
        ('break-residual', 'early return loses the original lock residual'),
    ]
    for label, diagnostic in cases:
        fixture = deepcopy(view)
        parent = declaration(fixture.functions.values(), PARENT)['body']['Structured']
        if label == 'guard-source':
            guard, = [v['index'] for v in parent['locals']['locals'] if v['name'] == 'inner']
            source, = [rhs['Use'][0]['Move'] for dst, rhs in nodes(parent['body'], 'Assign')
                       if dst['kind'] == {'Local': guard}]
            source['kind'] = {'Local': 1}
        elif label == 'poison-error':
            closure = declaration(fixture.functions.values(), PARENT + '::{impl}::call_once')
            error = declaration(fixture.type_decls.values(), SERVICE + '::JournalError')
            other, = [v['id'] for v in error['kind']['Enum'] if v['name'] == 'CorruptStore' and not v['fields']]
            result, = [rhs for dst, rhs in nodes(closure['body'], 'Assign') if dst['kind'] == {'Local': 0}]
            result['Aggregate'][0]['Adt'][1] = other
        else:
            residual, = [c['call'] for c in nodes(parent['body'], 'Call')
                         if name(fixture.functions[c['call']['func']['Regular']['kind']['Fun']])
                         == RESULT + 'from_residual']
            residual['args'][0]['Move']['kind'] = {'Local': 1}
        expect_rejection(label, diagnostic, check_guard, fixture)
    return [label for label, _ in cases]


def check_rejections(view, broadcast):
    cases = [
        ('lookup-key', GET, 1, 'sender call operand has a different origin'),
        ('lookup-none-return', GET, None, 'sender block has different calls or control flow'),
        ('targeted-sender', ALL, 0, 'send_replace receiver differs from the item yielded by next'),
        ('broadcast-vector', VEC_ITER, 0, 'sender call operand has a different origin'),
    ]
    rejected = []
    for label, target, operand, diagnostic in cases:
        fixture, all_arm = deepcopy((view, broadcast))
        targeted = declaration(fixture.functions.values(), ONE)['body']['Structured']['body']
        part = all_arm if label == 'broadcast-vector' else targeted
        call, = [c['call'] for c in nodes(part, 'Call')
                 if name(fixture.functions[c['call']['func']['Regular']['kind']['Fun']]) == target]
        if operand is None:
            switch, = [s for s in nodes(part, 'Switch')
                       if s['data']['scrutinee']['Discriminant']['kind'] == call['dest']['kind']]
            switch['branches'].reverse()
        else:
            # Keep the call, its type metadata and control flow. Disconnect only the operand's origin.
            call['args'][operand]['Move']['kind'] = call['dest']['kind']
        expect_rejection(label, diagnostic, inspect_senders, fixture, all_arm)
        rejected.append(label)
    return rejected


def check_inline_rejections(view):
    """Alter only a copied pre-refactor capture; the original packet remains available for readback."""
    cases = [('operation', 'original operation guard selects a different action'),
             ('current', 'original inactive arm has an effect'),
             ('sandbox', 'sender call operand has a different origin'),
             ('error', 'original inactive arm has an effect'),
             ('table', 'sender call operand has a different origin')]
    for label, diagnostic in cases:
        fixture = deepcopy(view)
        body = declaration(fixture.functions.values(), PARENT)['body']['Structured']['body']
        if label in ('operation', 'error'):
            domain = OPERATION if label == 'operation' else ERROR
            switch, = [sw for sw in nodes(body, 'Switch') if 'Discriminant' in sw['data']['scrutinee']
                       and fixture.type_name(sw['data']['scrutinee']['Discriminant']['ty']) == domain]
            switch['branches'].reverse()
        elif label == 'current':
            _, current = [sw for sw in nodes(body, 'Switch') if 'Value' in sw['data']['scrutinee']]
            current['branches'].reverse()
        else:
            symbol, index = (ONE, 1) if label == 'sandbox' else (VALUES, 0)
            call, = [entry['call'] for entry in nodes(body, 'Call') if
                     name(fixture.functions[entry['call']['func']['Regular']['kind']['Fun']]) == symbol]
            call['args'][index]['Move']['kind'] = call['dest']['kind']
        expect_rejection('original-'+label, diagnostic, inspect_inline, fixture, check_guard(fixture))
    expect_rejection('original-missing-broadcast', 'missing coordinator broadcast arm', inspect_senders, view, None)
    return [label for label, _ in cases] + ['missing-broadcast']
