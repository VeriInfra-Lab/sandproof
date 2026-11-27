"""Bind the fourth journal argument and its concrete CrashInjector method across the two Rust views."""
from hashlib import sha256
from pathlib import Path

from wiring import JOURNAL, MODULE, PARENT, SERVICE, assignments_before, declaration, name, origin, require

HOOK = SERVICE + '::NeverCrash'
TRAIT = SERVICE + '::CrashInjector'
POINT = SERVICE + '::CrashPoint'
LIB = Path('controller-service/src/lib.rs')
LEAN_METHOD = ('SandproofStage1RequestRust.sandproof_controller_service.NeverCrash.Insts.'
               'Sandproof_controller_serviceCrashInjector.hit')


def implementation(view, impl_id, source):
    trait = declaration(view.trait_decls.values(), TRAIT)
    method, = trait['methods']
    slot = {'TraitMethod': [trait['def_id'], 0]}
    require(method['kind'] == slot and method['skip_binder']['name'] == 'hit',
            'unexpected crash hook method slot')
    impl = view.impls[impl_id]
    implemented = impl['impl_trait']
    require(implemented['id'] == trait['def_id'] and
            [view.type_name(t) for t in implemented['generics']['types']] == [HOOK],
            'journal hook dictionary uses another implementation')
    binding, = impl['methods']
    hit = view.functions[binding['skip_binder']['id']]
    require(binding['kind'] == slot and hit['src']['TraitImpl']['impl_ref']['id'] == impl_id
            and hit['src']['TraitImpl']['item_id'] == 0 and name(hit) == SERVICE + '::{impl}::hit'
            and [view.type_name(t) for t in hit['signature']['inputs']] == ['&mut ' + HOOK, POINT]
            and hit['item_meta']['opacity'] == 'Transparent' and 'Structured' in hit['body'],
            'unexpected concrete hook method')
    span = hit['item_meta']['span']['Untagged']['data']
    file = view.data['files'][span['file_id']]
    path = source / file['name']['Local']
    require(path == source/LIB and not path.is_symlink(), 'hook implementation is from another source')
    data = path.read_bytes()
    require(file['contents'] is None or file['contents'].encode() == data, 'embedded hook source changed')
    # Across crate boundaries Charon can retain only the signature span. Identify the
    # method by its dictionary slot, receiver, start position and complete Rust file.
    identity = dict(trait=TRAIT, receiver=HOOK, method='hit', source=str(LIB),
                    source_sha256=sha256(data).hexdigest(), start=span['beg'])
    return hit, identity


def check_journal_hook(view, source):
    journal = declaration(view.functions.values(), JOURNAL)
    block = declaration(view.functions.values(), PARENT)['body']['Structured']['body']
    statements = [s['kind'] for s in block['statements'] if not (
        isinstance(s['kind'], dict) and set(s['kind']) <= {'StorageLive', 'StorageDead'})]
    index, = [i for i, k in enumerate(statements) if 'Call' in k and
              k['Call']['call']['func']['Regular']['kind'] == {'Fun': journal['def_id']}]
    call = statements[index]['Call']['call']
    trait = declaration(view.trait_decls.values(), TRAIT)
    clause, = journal['generics']['trait_clauses']
    bound = clause['trait_']['skip_binder']
    inputs = journal['signature']['inputs']
    _, receiver, mode = view.resolve(inputs[3], view.types)['Ref']
    require(len(inputs) == len(call['args']) == 4 and mode == 'Mut'
            and view.resolve(receiver, view.types) == {'TypeVar': {'Free': 0}}
            and bound['id'] == trait['def_id'] and len(bound['generics']['types']) == 1
            and view.resolve(bound['generics']['types'][0], view.types) == view.resolve(receiver, view.types),
            'fourth parameter is not the journal crash injector')
    generic = call['func']['Regular']['generics']
    ref, = generic['trait_refs']
    dictionary = view.resolve(ref, view.traits)
    selected = dictionary['trait_decl_ref']['skip_binder']
    require([view.type_name(t) for t in generic['types']] == [HOOK] and selected['id'] == trait['def_id']
            and [view.type_name(t) for t in selected['generics']['types']] == [HOOK],
            'journal uses another crash hook type')
    _, identity = implementation(view, dictionary['kind']['TraitImpl']['id'], source)

    # The three consecutive value operations construct the fieldless value,
    # borrow it mutably, then reborrow exactly that reference for the journal.
    hook = declaration(view.type_decls.values(), HOOK)
    (value, init), (reference, borrow), (argument, reborrow) = [k['Assign'] for k in statements[index-3:index]]
    constructor, fields = init['Aggregate']
    adt, variant, union_field = constructor['Adt']
    require(hook['kind'] == {'Struct': []} and adt['id'] == hook['def_id']
            and variant is None and union_field is None and not fields and view.type_name(value['ty']) == HOOK,
            'journal hook did not construct NeverCrash')
    local_ids = [place['kind']['Local'] for place in (value, reference, argument)]
    first, second = borrow['Ref'], reborrow['Ref']
    actual = call['args'][3]['Move']
    require(len(set(local_ids)) == 3 and first['kind'] == 'Mut' and first['place']['kind'] == value['kind']
            and second['kind'] == 'TwoPhaseMut' and second['place']['kind'] == {'Projection': [reference, 'Deref']}
            and actual['kind'] == argument['kind'] and view.type_name(actual['ty']) == '&mut ' + HOOK,
            'fourth journal argument lost its original mutable hook')
    return identity


def check_joint_hook(view, mapping, source):
    wrapper = declaration(view.functions.values(), MODULE + '::proof_never_crash')
    block = wrapper['body']['Structured']['body']
    operations = [s for s in block['statements'] if not (
        isinstance(s['kind'], dict) and set(s['kind']) <= {'StorageLive', 'StorageDead'})]
    require([next(iter(k)) if isinstance(k := s['kind'], dict) else k for s in operations]
            == ['Assign', 'Assign', 'Call', 'Return'], 'unexpected hook forwarding operations')
    first, second, invoke, _ = operations
    temporaries = [s['kind']['Assign'][0]['kind']['Local'] for s in (first, second)]
    require(len(set(temporaries)) == 2 and min(temporaries) > 2, 'thin root overwrites a hook input')
    call = invoke['kind']['Call']['call']
    hit = view.functions[call['func']['Regular']['kind']['Fun']]
    impl_id = hit['src']['TraitImpl']['impl_ref']['id']
    method, identity = implementation(view, impl_id, source)
    before = assignments_before(block, invoke)
    require(method['def_id'] == hit['def_id'] and call['dest']['kind'] == {'Local': 0}
            and [origin(arg['Move'], before) for arg in call['args']] == [(1,), (2,)], 'thin hook root does not forward the actual call')
    translated, = [f for f in mapping['functions'] if f['def_id'] == hit['def_id']]
    require(translated['lean_name'] == LEAN_METHOD and not translated['is_opaque']
            and translated['lean_file'] == 'SandproofStage1Request/Funs.lean',
            'hook implementation missing from the generated Lean body')
    return identity
