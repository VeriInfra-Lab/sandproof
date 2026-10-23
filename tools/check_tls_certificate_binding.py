"""Admit four frozen TLS bodies and their sources; this is not refinement."""
import argparse
import hashlib
import json
import re
from pathlib import Path

try:
    from tools.check_foundations_extraction import llbc_digest
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest

REL = Path('verification/experiments/tls-certificate-binding')
FROZEN = Path('verification/tls-certificate-binding-inputs.sha256')
EXPECTED_LOCAL = {11, 12, 13, 23, 27, 32, 34, 38, 40, 41, 42, 43, 44, 143}
COMPILER_DROP_GLUE = {34, 40, 43, 44}
EXPECTED_CALLS = {
    11: [('alloc', 'vec', 'deref'), ('sandproof_internal', 'tls', 'verify_pin')],
    12: [('alloc', 'vec', 'deref'), ('sandproof_internal', 'tls', 'verify_pin')],
    13: [('sandproof_internal', 'tls', 'verify_peer'), ('core', 'result', 'branch'),
         ('core', 'result', 'from_residual'), ('core', 'option', 'and_then'),
         ('core', 'option', 'map'), ('core', 'option', 'ok_or')],
    23: [('core', 'option', 'and_then'), ('core', 'option', 'is_some_and')],
    27: [('rustls_pki_types', 'as_ref'), ('digest', 'digest', 'digest'),
         ('core', 'convert', 'into')],
    32: [('core', 'slice', 'first')], 34: [],
    38: [('sandproof_internal', 'tls', 'from_certificate')], 40: [],
    41: [('core', 'slice', 'first')],
    42: [('sandproof_internal', 'tls', 'from_certificate'), ('core', 'slice', 'contains')],
    43: [], 44: [], 143: [('core', 'array', 'equality', 'eq')],
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def object_sha(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def identifiers(item):
    return tuple(p['Ident'][0] for p in item['item_meta']['name'] if 'Ident' in p)


def calls(function):
    return [n['kind']['Call']['call'] for n in nodes(function['body'])
            if isinstance(n.get('kind'), dict) and 'Call' in n['kind']]


def local(place):
    if set(place['kind']) != {'Local'}:
        raise ValueError('expected complete local')
    return place['kind']['Local']


def forwarded_peer(function, declarations):
    """Only the two actual, linear field/deref/call facades, no CFG evaluator."""
    body = function['body']['Structured']
    if body['locals']['arg_count'] != 2:
        raise ValueError('peer input count changed')
    origins = {1: ('input', 1), 2: ('input', 2)}

    def origin(place):
        kind = place['kind']
        if set(kind) == {'Local'}:
            return origins[kind['Local']]
        if set(kind) == {'Projection'}:
            base, projection = kind['Projection']
            if projection == 'Deref':
                return origin(base)
            if projection == {'Field': [None, 1]}:
                return ('field', origin(base), 1)
        raise ValueError('peer field or borrow projection changed')

    def operand(value):
        if set(value) not in ({'Move'}, {'Copy'}):
            raise ValueError('peer operand no longer forwards an original value')
        return origin(next(iter(value.values())))

    for statement in body['body']['statements']:
        kind = statement['kind']
        tag = next(iter(kind)) if isinstance(kind, dict) else kind
        if tag in {'StorageLive', 'StorageDead', 'Return'}:
            continue
        if tag == 'Assign':
            destination, value = kind[tag]
            index = local(destination)
            if index in (1, 2):
                raise ValueError('peer original input overwritten')
            if set(value) == {'Ref'} and value['Ref']['kind'] == 'Shared':
                origins[index] = origin(value['Ref']['place'])
            elif set(value) == {'Use'}:
                origins[index] = operand(value['Use'][0])
            else:
                raise ValueError('new policy or effect in peer forwarding')
        elif tag == 'Call':
            call = kind[tag]['call']
            name = identifiers(declarations[call['func']['Regular']['kind']['Fun']])
            values = [operand(arg) for arg in call['args']]
            if name == ('alloc', 'vec', 'deref'):
                if values != [('field', ('input', 1), 1)]:
                    raise ValueError('not the original self.peer_pins')
                origins[local(call['dest'])] = ('slice', values[0])
            elif name == ('sandproof_internal', 'tls', 'verify_pin'):
                if values != [('input', 2), ('slice', ('field', ('input', 1), 1))]:
                    raise ValueError('certificates or full original pins not forwarded')
                if local(call['dest']) != 0:
                    raise ValueError('original result not returned directly')
                origins[0] = ('result', 'verify_pin')
            else:
                raise ValueError('new call in peer forwarding')
        else:
            raise ValueError('new branch, Drop or effect in peer forwarding')
    if origins.get(0) != ('result', 'verify_pin'):
        raise ValueError('missing original peer result')


def check_shape(tree, frozen):
    translated = tree['translated']
    if tree.get('has_errors') is not False or translated['crate_name'] != 'sandproof_internal':
        raise ValueError('invalid original Charon input')
    functions = {f['def_id']: f for f in translated['fun_decls'] if f}
    owned = {k: f for k, f in functions.items() if f['item_meta']['is_local']}
    if set(owned) != EXPECTED_LOCAL:
        raise ValueError('missing or unexpected local root/closure/DropGlue')
    for key, function in owned.items():
        if (function['item_meta']['opacity'] != 'Transparent'
                or not isinstance(function['body'], dict) or set(function['body']) != {'Structured'}
                or function['signature']['is_unsafe'] is not (key in COMPILER_DROP_GLUE)
                or function['signature']['is_variadic']):
            raise ValueError('owned body is opaque, missing or changed')
        if key in COMPILER_DROP_GLUE:
            if (function['item_meta']['name'][-1] != {'Builtin': ['DropGlue', 0]}
                    or function['src']['TraitImpl']['trait_ref']['id'] != 5):
                raise ValueError('not the frozen compiler-generated DropGlue')
        observed = []
        for call in calls(function):
            if (set(call['func']) != {'Regular'}
                    or set(call['func']['Regular']['kind']) != {'Fun'}):
                raise ValueError('unknown indirect call')
            observed.append(identifiers(functions[call['func']['Regular']['kind']['Fun']]))
        if observed != EXPECTED_CALLS[key]:
            raise ValueError('original call/callback order changed: '+str(key))
    if calls(owned[13])[0]['func']['Regular']['kind']['Fun'] != 11:
        raise ValueError('peer_pin no longer calls the same ServerTls facade')
    for key in (11, 12):
        forwarded_peer(owned[key], functions)
    # Retain every ordered node/argument/branch/type/unwind/Drop field. This
    # guards a manually reviewed finite tree; it does not prove its semantics.
    for key, function in owned.items():
        if object_sha(function) != frozen['local_functions'][str(key)]['sha256']:
            raise ValueError('reviewed complete owned AST changed: '+str(key))
    if llbc_digest(tree) != frozen['llbc_digest']:
        raise ValueError('complete IR/types/traits/globals/config changed')
    return len(owned)


def check(root, tree=None):
    frozen = json.loads((root/REL/'environment.json').read_text())
    for name, digest in frozen['sources'].items():
        if sha(root/name) != digest:
            raise ValueError('original source/caller/candidate changed: '+name)
    tree = tree if tree is not None else json.loads((root/REL/'original.llbc').read_text())
    if tree['translated']['options']['dest_file'] != (REL/'original.llbc').as_posix():
        raise ValueError('unreviewed artifact destination')
    count = check_shape(tree, frozen)
    return {'local_bodies': count, 'production_wrappers': 4,
            'new_machine_refinement_theorems': 0, 'kind': 'finite-source-AST-admission-only'}


def check_frozen(root):
    """Mandatory independent leaf: its source references include parent freezes."""
    values = {}
    for line in (root / FROZEN).read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        if match is None or match[2] in values:
            raise ValueError('malformed or duplicate TLS source freeze')
        name = Path(match[2])
        if name.is_absolute() or '..' in name.parts:
            raise ValueError('escaping TLS source freeze')
        path = root / name
        if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(root.resolve()):
            raise ValueError('missing or non-regular TLS source freeze input')
        values[str(name)] = match[1]
        if sha(path) != match[1]:
            raise ValueError('changed TLS source freeze input: ' + str(name))
    required = {'tools/check_tls_certificate_binding.py', 'tools/tests/test_tls_certificate_binding.py'}
    required |= {str(REL / name) for name in ('README.md', 'collect.py', 'correspondence.json',
        'environment.json', 'native.py', 'native.rs', 'original.llbc', 'primitive-audit.json', 'results.json')}
    required |= set(json.loads((root / REL / 'environment.json').read_text())['sources'])
    if not required <= set(values):
        raise ValueError('incomplete TLS source freeze closure')
    return values


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    check_frozen(args.root.resolve())
    print(json.dumps(check(args.root.resolve())))
