"""Reject disconnected argument, dictionary or translation metadata on copies of admitted evidence."""
from copy import deepcopy

from journal_hook import LEAN_METHOD, check_joint_hook, check_journal_hook
from sender_binding_regressions import expect_rejection
from wiring import JOURNAL, PARENT, declaration, name, nodes


def check_parent_rejections(view, source):
    cases = [('hook-argument', 'fourth journal argument lost its original mutable hook'),
             ('hook-dictionary', 'journal hook dictionary uses another implementation')]
    for label, diagnostic in cases:
        fixture = deepcopy(view)
        parent = declaration(fixture.functions.values(), PARENT)
        call, = [c['call'] for c in nodes(parent['body'], 'Call')
                 if name(fixture.functions[c['call']['func']['Regular']['kind']['Fun']]) == JOURNAL]
        if label == 'hook-argument':
            call['args'][3]['Move']['kind'] = call['args'][2]['Move']['kind']
        else:
            ref, = call['func']['Regular']['generics']['trait_refs']
            impl = fixture.impls[fixture.resolve(ref, fixture.traits)['kind']['TraitImpl']['id']]
            method, = impl['methods']
            point = fixture.functions[method['skip_binder']['id']]['signature']['inputs'][1]
            impl['impl_trait']['generics']['types'] = [point]
        expect_rejection(label, diagnostic, check_journal_hook, fixture, source)
    return [label for label, _ in cases]


def check_joint_rejection(view, mapping, source):
    fixture = deepcopy(mapping)
    row, = [f for f in fixture['functions'] if f['lean_name'] == LEAN_METHOD]
    row['is_opaque'] = True
    expect_rejection('hook-translation', 'hook implementation missing from the generated Lean body',
                     check_joint_hook, view, fixture, source)
    return ['hook-translation']
