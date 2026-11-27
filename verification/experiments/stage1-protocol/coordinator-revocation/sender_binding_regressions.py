"""Reject disconnected operand or Option edges on private copies of an admitted compiler capture."""
from copy import deepcopy

from sender_binding import ALL, GET, ONE, VEC_ITER, inspect_senders
from wiring import declaration, name, nodes, require


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
        try:
            inspect_senders(fixture, all_arm)
        except ValueError as error:
            require(str(error).split(':', 1)[0] == diagnostic, f'{label}: unexpected diagnostic: {error}')
        else:
            raise ValueError(f'{label}: admitted a disconnected sender edge')
        rejected.append(label)
    return rejected
