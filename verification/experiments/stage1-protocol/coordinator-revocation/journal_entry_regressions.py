"""Reject detached Charon receipt metadata without changing any Rust source."""
from copy import deepcopy

from guard_binding import flow
from journal_entry import PROVIDER, RECORD, TRANSACTION, aggregates, check_entry
from sender_binding import enum_edges
from sender_binding_regressions import expect_rejection
from wiring import JOURNAL, declaration


def check_entry_rejections(view):
    cases = [('begin-input', 'journal BEGIN inputs or result differ'),
             ('residual-exit', 'journal BEGIN failure does not return immediately'),
             ('record-fields', 'journal transaction constructor differs'),
             ('encoding-argument', 'journal encoding, transaction or hook type differs'),
             ('provider-argument', 'transaction proof root arguments differ')]
    for label, diagnostic in cases:
        fixture = deepcopy(view)
        block = declaration(fixture.functions.values(), JOURNAL)['body']['Structured']['body']
        begin, branch, switch, _, record, _ = [s for _, s in flow(fixture, block)]
        if label == 'begin-input':
            begin['kind']['Call']['call']['args'].reverse()
        elif label == 'residual-exit':
            _, failure = enum_edges(fixture, switch, branch, 'core::ops::control_flow::ControlFlow',
                                    [(0, 'Continue', 1), (1, 'Break', 1)])
            failure['statements'].remove(next(s for s in failure['statements'] if s['kind'] == 'Return'))
        elif label == 'record-fields':
            constructor = aggregates(fixture, block, record)[RECORD][0]
            constructor['kind']['Assign'][1]['Aggregate'][1].reverse()
        elif label == 'encoding-argument':
            types = record['kind']['Call']['call']['func']['Regular']['generics']['types']
            types[0], types[1] = types[1], types[0]
        else:
            root = declaration(fixture.functions.values(), PROVIDER)['body']['Structured']['body']
            invoke, = [s for tag, s in flow(fixture, root) if tag == TRANSACTION]
            args = invoke['kind']['Call']['call']['args']
            args[2], args[3] = args[3], args[2]
        expect_rejection(label, diagnostic, check_entry, fixture)
    return [label for label, _ in cases]
