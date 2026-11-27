"""Bind the public BEGIN and StandardRecord call to the original transaction proof root."""
import json

from guard_binding import RESULT, flow
from sender_binding import enum_edges, output
from wiring import JOURNAL, MODEL, SERVICE, assignments_before, declaration, name, origin, require

INBOX = MODEL.removesuffix('::model')
RECORDING = INBOX + '::recording'
RECORD = RECORDING + '::StandardRecord'
CODEC = RECORDING + '::StandardRecordCodec'
PROVIDER = RECORDING + '::proof_record_actual_parent'
TRANSACTION = RECORDING + '::record_transaction_using'
CONTROLLER = SERVICE + '::controller::ControllerJournal'
DATABASE = SERVICE + '::journal::Journal'
BEGIN = SERVICE + '::controller::transaction::{impl}::begin'
DEREF = 'alloc::string::{impl}::deref'
ENCODINGS = [
    (INBOX+'::native_retirement', 'RetirementEncoding'),
    (MODEL, 'TerminalEncoding'),
    (INBOX+'::renewal_confirmation', 'RenewalEncoding'),
    (INBOX+'::unload_resolution', 'UnloadEncoding'),
    (SERVICE+'::capture', 'GraphicBytes'),
]


def local_writes(block, arity):
    """Value origins below rely on single definitions, including the two disjoint return slots."""
    defined = set(range(1, arity + 1))

    def visit(part):
        for statement in part['statements']:
            kind = statement['kind']
            if not isinstance(kind, dict):
                continue
            if 'Switch' in kind:
                for arm in kind['Switch']['branches']:
                    visit(arm)
            elif 'Assign' in kind or 'Call' in kind:
                if 'Assign' in kind:
                    place, rhs = kind['Assign']
                    require(set(rhs) in ({'Ref'}, {'Use'}, {'Aggregate'}), 'unexpected journal temporary')
                else:
                    place = kind['Call']['call']['dest']
                local = place['kind'].get('Local')
                require(local is not None and local not in defined, 'journal local overwritten or projected write')
                if local == 0:
                    require('Call' in kind, 'journal result overwritten')
                else:
                    defined.add(local)
    visit(block)


def forwarding(block, statement):
    # Constructed structs are producers. Their fields are checked separately.
    return {local: expr for local, expr in assignments_before(block, statement).items()
            if 'Aggregate' not in expr}


def operands(statement, assignments):
    args = statement['kind']['Call']['call']['args']
    return [origin(arg['Move'], assignments) for arg in args]


def aggregates(view, block, consumer):
    result = {}
    for statement in block['statements'][:block['statements'].index(consumer)]:
        kind = statement['kind']
        if not isinstance(kind, dict) or 'Assign' not in kind:
            continue
        dest, expr = kind['Assign']
        if 'Aggregate' not in expr:
            continue
        adt, args = expr['Aggregate']
        ty = view.type_decls[adt['Adt'][0]['id']]
        type_name = name(ty)
        require(type_name not in result, 'duplicate journal constructor')
        fields = [field['name'] for field in ty['kind']['Struct']]
        definitions = forwarding(block, statement)
        result[type_name] = (statement, dest['kind']['Local'], fields, [origin(x['Move'], definitions) for x in args])
    return result


def direct_field(view, ty, field):
    row = declaration(view.type_decls.values(), ty)
    return (None, [f['name'] for f in row['kind']['Struct']].index(field))


def type_args(view, generics):
    require(not generics['const_generics'], 'unexpected journal const arguments')
    return [view.type_name(t) for t in generics['types']]


def dictionaries(view, call, *, public):
    generic = call['kind']['Call']['call']['func']['Regular']['generics']
    concrete = [namespace+'::Standard'+encoding for namespace, encoding in ENCODINGS]
    parameters = ['parameter:'+json.dumps({'Free': i}) for i in range(7)]
    codec = CODEC if public else parameters[5]
    expected = (concrete if public else parameters[:5]) + [f'{RECORD}<{codec}>', parameters[0 if public else 6]]
    require(type_args(view, generic) == expected,
            'journal encoding, transaction or hook type differs')
    traits = [ns+'::'+encoding for ns, encoding in ENCODINGS] + [RECORDING+'::RecordTransaction', SERVICE+'::CrashInjector']
    record_impl = None
    for i, (ref, trait, ty) in enumerate(zip(generic['trait_refs'], traits, expected, strict=True)):
        dictionary = view.resolve(ref, view.traits)
        declared = dictionary['trait_decl_ref']['skip_binder']
        require(name(view.trait_decls[declared['id']]) == trait and
                type_args(view, declared['generics']) == [ty], 'journal trait receiver differs')
        if i == 6 or (not public and i < 5):
            require(dictionary['kind'] == {'Clause': {'Free': 0 if public else i}}, 'journal caller dictionary differs')
            continue
        inst = dictionary['kind']['TraitImpl']
        impl = view.impls[inst['id']]['impl_trait']
        args = inst['generics']
        require(impl['id'] == declared['id'], 'journal implementation trait differs')
        if i < 5:
            require(type_args(view, impl['generics']) == [ty]
                    and not args['types'] and not args['trait_refs'], 'journal concrete encoding implementation differs')
        else:
            require(type_args(view, impl['generics']) == [f'{RECORD}<{parameters[0]}>'] and
                    type_args(view, args) == [codec], 'journal codec substitution differs')
            nested, = args['trait_refs']
            dictionary = view.resolve(nested, view.traits)
            codec_trait = dictionary['trait_decl_ref']['skip_binder']
            require(name(view.trait_decls[codec_trait['id']]) == RECORDING+'::RecordCodec' and
                    type_args(view, codec_trait['generics']) == [codec],
                    'journal codec trait differs')
            if public:
                bound = view.impls[dictionary['kind']['TraitImpl']['id']]['impl_trait']
                require(bound['id'] == codec_trait['id'] and
                        type_args(view, bound['generics']) == [codec],
                        'journal concrete codec implementation differs')
            else:
                require(dictionary['kind'] == {'Clause': {'Free': 5}}, 'transaction proof codec dictionary differs')
            record_impl = inst['id']
    return record_impl


def check_residual(view, residual):
    generic = residual['kind']['Call']['call']['func']['Regular']['generics']
    error = SERVICE+'::JournalError'
    require(type_args(view, generic) == [MODEL+'::LifecycleWorkState', error, error],
            'journal residual error types differ')
    ref, = generic['trait_refs']
    dictionary = view.resolve(ref, view.traits)
    concrete = dictionary['kind']['TraitImpl']
    impl = view.impls[concrete['id']]['impl_trait']
    declared = dictionary['trait_decl_ref']['skip_binder']
    param = 'parameter:'+json.dumps({'Free': 0})
    require(name(view.trait_decls[impl['id']]) == 'core::convert::From' and impl['id'] == declared['id']
            and type_args(view, impl['generics']) == [param, param]
            and type_args(view, concrete['generics']) == [error] and type_args(view, declared['generics']) == [error, error],
            'journal residual conversion differs')


def check_entry(view):
    journal = declaration(view.functions.values(), JOURNAL)
    provider = declaration(view.functions.values(), PROVIDER)
    transaction = declaration(view.functions.values(), TRANSACTION)
    require('Structured' in transaction['body'] and transaction['item_meta']['opacity'] == 'Transparent',
            'actual transaction method is not transparent')
    for f, owner in ((journal, CONTROLLER), (declaration(view.functions.values(), BEGIN),
                                             SERVICE+'::controller::transaction::WorldTransaction')):
        impl = f['item_meta']['name'][-2]['Impl']['Ty']
        require(view.type_name(impl['skip_binder']) == owner, 'journal or BEGIN inherent receiver differs')
    public, root = [f['body']['Structured']['body'] for f in (journal, provider)]
    for f, block, arity in ((journal, public, 4), (provider, root, 6)):
        require(len(f['signature']['inputs']) == arity and f['item_meta']['opacity'] == 'Transparent',
                'journal entry or provider root is not transparent')
        local_writes(block, arity)

    events = list(flow(view, public))
    require([tag for tag, _ in events] == [BEGIN, RESULT+'branch', 'Switch', DEREF, TRANSACTION, 'Return'],
            'public journal normal control flow differs')
    begin, branch, switch, namespace, invoke, _ = [s for _, s in events]
    connection = (1, direct_field(view, CONTROLLER, 'journal'), direct_field(view, DATABASE, 'connection'))
    cache = (1, direct_field(view, CONTROLLER, 'committed_world'))
    require(operands(begin, forwarding(public, begin)) == [connection, cache] and
            operands(branch, forwarding(public, branch)) == [(output(begin),)], 'journal BEGIN inputs or result differ')
    success, failure = enum_edges(view, switch, branch, 'core::ops::control_flow::ControlFlow',
                                 [(0, 'Continue', 1), (1, 'Break', 1)])
    failure_path = list(flow(view, failure))
    require(not success['statements'] and [tag for tag, _ in failure_path] == [RESULT+'from_residual', 'Return'],
            'journal BEGIN failure does not return immediately')
    residual = failure_path[0][1]
    check_residual(view, residual)
    require(output(residual) == 0 and operands(residual, forwarding(public, switch) | forwarding(failure, residual))
            == [(output(branch), (1, 0))], 'journal BEGIN residual differs')
    require(operands(namespace, forwarding(public, namespace)) == [(1, direct_field(view, CONTROLLER, 'namespace'))],
            'journal namespace differs')

    values = aggregates(view, public, invoke)
    make_codec, codec, codec_fields, codec_args = values[CODEC]
    make_record, tx, fields, record_args = values[RECORD]
    require(set(values) == {CODEC, RECORD} and codec_fields == codec_args == [] and fields == ['tx', 'namespace', 'codec']
            and public['statements'].index(make_record) > max(public['statements'].index(namespace),
                                                              public['statements'].index(make_codec))
            and record_args == [(output(branch), (0, 0)), (output(namespace),), (codec,)],
            'journal transaction constructor differs')
    require(operands(invoke, forwarding(public, invoke)) == [(tx,), (2,), (3,), (4,)] and output(invoke) == 0,
            'journal recording arguments or return differ')
    calls = list(flow(view, root))
    require([tag for tag, _ in calls] == [TRANSACTION, 'Return'], 'transaction proof root control flow differs')
    root_call = calls[0][1]
    root_values = aggregates(view, root, root_call)
    _, owner, names, inputs = root_values[RECORD]
    require(set(root_values) == {RECORD} and names == fields and inputs == [(1,), (2,), (3,)] and
            operands(root_call, forwarding(root, root_call)) == [(owner,), (4,), (5,), (6,)] and output(root_call) == 0,
            'transaction proof root arguments differ')
    require(invoke['kind']['Call']['call']['func']['Regular']['kind'] ==
            root_call['kind']['Call']['call']['func']['Regular']['kind'] and
            dictionaries(view, invoke, public=True) == dictionaries(view, root_call, public=False),
            'journal and proof root select a different recording method or implementation')
    return {'public': JOURNAL, 'provider': PROVIDER, 'record': TRANSACTION,
            'encodings': [ns+'::Standard'+enc for ns, enc in ENCODINGS], 'codec': CODEC}


def translated_entry(view, mapping, wrapper):
    root = declaration(view.functions.values(), wrapper)
    block = root['body']['Structured']['body']
    local_writes(block, 4)
    events = list(flow(view, block))
    require([tag for tag, _ in events] == [JOURNAL, 'Return'] and len(root['signature']['inputs']) == 4,
            'public journal forwarding root differs')
    call = events[0][1]
    require(output(call) == 0 and operands(call, forwarding(block, call)) == [(1,), (2,), (3,), (4,)],
            'public journal forwarding arguments differ')
    generated = {}
    for symbol in (JOURNAL, PROVIDER, TRANSACTION, wrapper):
        f = declaration(view.functions.values(), symbol)
        rows = [row for row in mapping['functions'] if row['def_id'] == f['def_id']]
        require('Structured' in f['body'] and f['item_meta']['opacity'] == 'Transparent' and len(rows) == 1
                and rows[0]['is_opaque'] is False and rows[0]['lean_file'] == 'SandproofActualRecord/Funs.lean',
                'public journal translation missing actual definition: '+symbol)
        generated[symbol] = rows[0]['lean_name']
    return generated
