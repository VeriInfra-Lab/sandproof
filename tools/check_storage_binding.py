"""Admit the isolated public Redis facades' mechanical calls, not their codec.

Requires freshly generated complete Charon LLBC. The frozen digest retains all
AST fields and ordered arrays; only Charon's unordered short-name map is sorted.
The semantic checks below independently reject changed arguments or policy in
these four call-through bodies. They are engineering admission, not refinement.
"""
import argparse
import copy
import json
from pathlib import Path
import sys

try:
    from tools.check_foundations_extraction import llbc_digest
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest

CRATE = 'sandproof_storage_extract'
OPAQUE = [CRATE + '::store::storage_io::' + name
          for name in ('eval', 'info', 'settings', 'trim_cr', 'split_colon')]


def identifiers(item):
    return [part['Ident'][0] for part in item['item_meta']['name'] if 'Ident' in part]


def named(items, module, method):
    matches = [item for item in items.values()
               if identifiers(item) == [CRATE, module, method]]
    if len(matches) != 1:
        raise ValueError('missing or duplicated real body: ' + module + '::' + method)
    return matches[0]


def local(place):
    if set(place['kind']) != {'Local'}:
        raise ValueError('forwarded value is no longer a complete local')
    return place['kind']['Local']


def body_call(function, functions, types, constants, module, target, argument_count, facade):
    signature = function['signature']
    if (function['item_meta']['opacity'] != 'Transparent'
            or not function['item_meta']['is_local']
            or signature['is_unsafe'] or signature['is_variadic'] or signature['abi'] != 'Rust'
            or set(function['body']) != {'Structured'}):
        raise ValueError('public call no longer has a transparent safe Rust body')
    body = function['body']['Structured']
    if body['locals']['arg_count'] != argument_count:
        raise ValueError('complete public input missing')
    origins = {index: ('input', index) for index in range(1, argument_count + 1)}
    calls = []
    for statement in body['body']['statements']:
        kind = statement['kind']
        tag = next(iter(kind)) if isinstance(kind, dict) else kind
        if tag in {'StorageLive', 'StorageDead', 'Return'}:
            continue
        if tag == 'Assign':
            if calls:
                raise ValueError('forwarding assignment after the call')
            destination, value = kind[tag]
            index = local(destination)
            if index in origins:
                raise ValueError('input or forwarding local overwritten')
            if set(value) == {'Ref'}:
                reference = value['Ref']
                place = reference['place']
                if set(place['kind']) == {'Projection'}:
                    place, projection = place['kind']['Projection']
                    if projection != 'Deref':
                        raise ValueError('public call forwards only part of an input')
                if reference['kind'] not in {'Shared', 'Mut', 'TwoPhaseMut'}:
                    raise ValueError('unreviewed borrow kind')
                origins[index] = origins[local(place)]
            elif facade and set(value) == {'Aggregate'}:
                constructor, fields = value['Aggregate']
                if fields or set(constructor) != {'Adt'}:
                    raise ValueError('callback construction has business state')
                adt, variant, field = constructor['Adt']
                name = identifiers(types[adt['id']])
                if (variant is not None or field is not None
                        or name not in ([CRATE, 'store', 'JsonCodec'], [CRATE, 'store', 'SystemClock'])):
                    raise ValueError('unreviewed callback object')
                origins[index] = ('callback', name[-1])
            else:
                raise ValueError('policy or nonmechanical assignment in public facade')
        elif tag == 'Call':
            calls.append(kind[tag])
        else:
            raise ValueError('policy, loop, effectful Drop or control branch in public facade')
    if len(calls) != 1 or body['body']['statements'][-1]['kind'] != 'Return':
        raise ValueError('public body is no longer one direct call and return')
    node = calls[0]
    call = node['call']
    regular = call['func']['Regular']
    callee = functions[regular['kind']['Fun']]
    if (identifiers(callee) != [CRATE, module, target]
            or callee['item_meta']['opacity'] != 'Transparent' or callee['body'] is None
            or local(call['dest']) != 0):
        raise ValueError('target body or full result forwarding changed')
    args = call['args']
    expected = [('input', index) for index in range(1, argument_count + 1)]
    expected += [('format', 0), ('callback', 'JsonCodec'), ('callback', 'SystemClock')] if facade else []
    actual = []
    for operand in args:
        if set(operand) == {'Move'}:
            actual.append(origins[local(operand['Move'])])
        elif facade and set(operand) == {'Const'}:
            constant = operand['Const']
            if set(constant) == {'Deduplicated'}:
                constant = constants[constant['Deduplicated']]
            elif set(constant) == {'Value'}:
                constant = constant['Value'][1]
            else:
                raise ValueError('unreviewed format constant')
            if (set(constant[0]) != {'TraitConst'} or constant[0]['TraitConst'][1] != 0):
                raise ValueError('format is no longer the original associated constant')
            actual.append(('format', 0))
        else:
            raise ValueError('complete argument is not forwarded')
    if actual != expected:
        raise ValueError('public arguments or codec/clock order changed')
    # Retain/inspect unwinding rather than treating panic as a normal return.
    cleanup = node['on_unwind']['statements']
    tags = [next(iter(s['kind'])) if isinstance(s['kind'], dict) else s['kind'] for s in cleanup]
    if not tags or tags[-1] != 'UnwindResume' or any(tag != 'StorageDead' for tag in tags[:-1]):
        raise ValueError('public unwind path changed')
    if facade:
        expected_source = PUBLIC_SOURCES[target.removesuffix('_using')]
        if function['item_meta']['source_text'] != expected_source:
            raise ValueError('concrete FORMAT_VERSION/codec/clock source binding changed')
        if (len(regular['generics']['types']) != 3
                or len(regular['generics']['trait_refs']) != 2):
            raise ValueError('payload or concrete callback dictionaries changed')
    return function, callee


PUBLIC_SOURCES = {
    'commit': 'pub fn commit(&mut self, expected: &Observed<T>, next: &T) -> Result<Committed<T>, StoreError> {\n'
              '        self.commit_using(\n            expected,\n            next,\n            T::FORMAT_VERSION,\n'
              '            &mut JsonCodec,\n            &mut SystemClock,\n        )\n    }',
    'reaffirm': 'pub fn reaffirm(&mut self, expected: &Observed<T>) -> Result<Committed<T>, StoreError> {\n'
                '        self.reaffirm_using(\n            expected,\n            T::FORMAT_VERSION,\n'
                '            &mut JsonCodec,\n            &mut SystemClock,\n        )\n    }',
}


def check_semantics(llbc):
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon reported errors')
    translated = llbc['translated']
    options = translated['options']
    if (options['start_from'] != ['crate::bindings::commit', 'crate::bindings::reaffirm']
            or options['include'] != ['sandproof_center_store', 'sandproof_protocol', 'sandproof_center_core']
            or options['opaque'] != OPAQUE or options['exclude']
            or options['monomorphize'] or options['skip_borrowck'] or options['no_typecheck']
            or options['dest_file'] != '../aeneas/llbc/sandproof_storage_bindings.llbc'):
        raise ValueError('unreviewed binding extraction options')
    if {item['key'] for item in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('unreviewed binding target')
    constants = {}

    def collect(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key == 'Const' and isinstance(child, dict) and 'Value' in child:
                    index, item = child['Value']
                    if index in constants and constants[index] != item:
                        raise ValueError('conflicting deduplicated constant')
                    constants[index] = item
                collect(child)
        elif isinstance(value, list):
            for child in value:
                collect(child)

    collect(llbc)
    functions = {f['def_id']: f for f in translated['fun_decls'] if f is not None}
    types = {t['def_id']: t for t in translated['type_decls'] if t is not None}
    for method, count in [('commit', 3), ('reaffirm', 2)]:
        root = named(functions, 'bindings', method)
        _, facade = body_call(root, functions, types, constants, 'store', method, count, False)
        body_call(facade, functions, types, constants, 'store', method + '_using', count, True)


def check(root, path):
    profile = json.loads((root / 'verification/experiments/redis-result/candidate.json').read_text())
    llbc = json.loads(path.read_text())
    check_semantics(llbc)
    if llbc_digest(llbc) != profile['public_binding_llbc_digest']:
        raise ValueError('complete public LLBC changed; review every AST/source/option difference')
    return llbc


def check_mutations(llbc):
    # Use freshly generated real AST. These checks intentionally bypass the
    # digest so a rejected mutation demonstrates the forwarding checks themselves.
    for fault in ('argument', 'result', 'extra_call', 'branch', 'callback', 'format', 'unwind'):
        bad = copy.deepcopy(llbc)
        functions = {f['def_id']: f for f in bad['translated']['fun_decls'] if f is not None}
        root = named(functions, 'bindings', 'commit')
        facade = named(functions, 'store', 'commit')
        statements = (facade if fault in {'callback', 'format'} else root)['body']['Structured']['body']['statements']
        call = next(s['kind']['Call'] for s in statements if isinstance(s['kind'], dict) and 'Call' in s['kind'])
        if fault == 'argument':
            call['call']['args'][1] = copy.deepcopy(call['call']['args'][2])
        elif fault == 'result':
            call['call']['dest']['kind']['Local'] = 1
        elif fault == 'extra_call':
            statements.insert(-1, copy.deepcopy(next(s for s in statements if isinstance(s['kind'], dict) and 'Call' in s['kind'])))
        elif fault == 'branch':
            statements.insert(-1, {'kind': {'Switch': {}}})
        elif fault == 'callback':
            call['call']['args'][-2:] = list(reversed(call['call']['args'][-2:]))
        elif fault == 'format':
            for operand in call['call']['args']:
                if 'Const' in operand:
                    operand['Const'] = {'Value': [99999, [{'Literal': 1}, {}]]}
                    break
        else:
            call['on_unwind']['statements'][-1]['kind'] = 'Return'
        try:
            check_semantics(bad)
        except (ValueError, KeyError, TypeError, IndexError):
            continue
        raise ValueError('binding mutation was accepted: ' + fault)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--check-mutations', action='store_true')
    args = parser.parse_args()
    try:
        llbc = check(args.root, args.llbc)
        if args.check_mutations:
            check_mutations(llbc)
    except (OSError, ValueError, KeyError, TypeError, IndexError) as error:
        print('Redis public binding failed: ' + str(error), file=sys.stderr)
        return 1
    print('Four Redis call-throughs mechanically admitted; concrete own codec remains an explicit V2 obligation.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
