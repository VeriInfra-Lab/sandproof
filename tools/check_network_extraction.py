"""Audit shared current-state roots, owned bodies and the reviewed library boundary."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

if __package__:
    from .check_foundations_extraction import llbc_digest
else:
    from check_foundations_extraction import llbc_digest

CRATE = 'sandproof_network_extract'
ROOTS = ('cidr_contains', 'protected_ipv4', 'destination_allowed', 'parse_ipv4_bits',
         'egress', 'ingress', 'clone_world', 'initial_world',
         'runtime_crash', 'runtime_recover', 'slot_transition')
OWNERS = {'sandbox_controller_core', CRATE}
FUNCTION_FILE = 'SandproofNetwork/Funs.lean'
TYPE_FILE = 'SandproofNetwork/Types.lean'
EXTERNALS = {
    'core::cmp::impls::{impl core::cmp::PartialEq<bool> for bool}::ne':
        'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core::net::ip_addr::{impl core::convert::From<core::net::ip_addr::Ipv4Addr> for u32}::from':
        'U32.Insts.CoreConvertFromIpv4Addr.from',
    'core::net::parser::{impl core::str::traits::FromStr<core::net::parser::AddrParseError> for core::net::ip_addr::Ipv4Addr}::from_str':
        'core.net.ip_addr.Ipv4Addr.Insts.CoreStrTraitsFromStrAddrParseError.from_str',
    'core::str::{str}::parse': 'core.str.Str.parse',
    'alloc::string::{impl core::cmp::PartialEq<alloc::string::String> for alloc::string::String}::eq':
        'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
    'alloc::string::{impl core::ops::deref::Deref<str> for alloc::string::String}::deref':
        'alloc.string.String.Insts.CoreOpsDerefDerefStr.deref',
    'alloc::string::{impl core::clone::Clone for alloc::string::String}::clone':
        'alloc.string.String.Insts.CoreCloneClone.clone',
    'core::option::{impl core::clone::Clone for core::option::Option<T>}::clone':
        'core.option.Option.Insts.CoreCloneClone.clone',
}
EXTERNAL_TYPES = {
    'core::net::ip_addr::Ipv4Addr': 'core.net.ip_addr.Ipv4Addr',
    'core::net::parser::AddrParseError': 'core.net.parser.AddrParseError',
}
LOOPS = {
    'sandbox_controller_core::network::ipv4_list_contains': {(0, (0,))},
    'sandbox_controller_core::network::{sandbox_controller_core::model::NetworkPolicy}::destination_allowed':
        {(0, (0,))},
    'sandbox_controller_core::store::{sandbox_controller_core::store::Store<K, V>}::get':
        {(0, (0,))},
}
STANDARD_SOURCES = {
    'core/src/net/parser.rs', 'core/src/net/ip_addr.rs',
    'alloc/src/string.rs', 'core/src/str/mod.rs',
    'core/src/option.rs', 'core/src/clone.rs',
    'core/src/tuple.rs', 'core/src/cmp.rs', 'alloc/src/vec/mod.rs', 'alloc/src/slice.rs',
}

# The fixed frontend materializes compiler drop glue; the fixed pure backend
# erases it. This is NOT permission to omit a project function or custom Drop.
DROP_GLUE_TYPES = {
    'model::Endpoint', 'auxiliary::AuxiliaryEvent', 'maintenance::MaintenanceState',
    'store::Store', 'maintenance::windows::WindowState', 'ids::VmId',
    'model::RestoreSource', 'ids::NetSlotId', 'model::NetworkPolicy', 'model::Authority',
    'model::Operation', 'model::OperationResult', 'model::OperationInput',
    'model::LogicalEffectId', 'model::OperationLedgerRecord', 'model::Command',
    'maintenance::slots::SlotReject',
}
U64_NEWTYPE_ALIASES = {f'sandbox_controller_core::ids::{name}' for name in (
    'TenantId', 'SandboxId', 'PolicyDigest', 'FsId', 'VmId', 'ManifestDigest',
    'SnapshotId', 'NetSlotId', 'OperationId', 'ResultDigest')}
U64_NEWTYPE_ALIASES |= {'sandbox_controller_core::maintenance::windows::WindowId',
                       'sandbox_controller_core::maintenance::slots::HostSlotId'}


def scalar_u64_ids(value):
    ids = set()
    if isinstance(value, dict):
        encoded = value.get('Value')
        if (isinstance(encoded, list) and len(encoded) == 2
                and encoded[1] == {'Scalar': {'Integer': {'Unsigned': 'U64'}}}):
            ids.add(encoded[0])
        for child in value.values():
            ids.update(scalar_u64_ids(child))
    elif isinstance(value, list):
        for child in value:
            ids.update(scalar_u64_ids(child))
    return ids


def drop_glue_name(declaration):
    name = declaration['item_meta']['name']
    if name[-1] != {'Builtin': ['DropGlue', 0]}:
        return None
    return '::'.join(part['Ident'][0] for part in name if 'Ident' in part)


def check_drop_glue(declaration, declarations):
    name = drop_glue_name(declaration)
    if name is None or name.removeprefix('sandbox_controller_core::') not in DROP_GLUE_TYPES:
        raise ValueError('unreviewed compiler drop glue')
    if not structured_body(declaration):
        raise ValueError('compiler drop glue body missing')

    def walk(value):
        if isinstance(value, list):
            for child in value:
                walk(child)
        elif isinstance(value, dict):
            if {'span', 'id', 'kind'}.issubset(value):
                kind = value['kind']
                tag = next(iter(kind)) if isinstance(kind, dict) else kind
                if tag not in {'StorageLive', 'StorageDead', 'Assign', 'Drop',
                               'Switch', 'Return', 'UnwindResume', 'Abort'}:
                    raise ValueError('custom operation inside compiler drop glue')
                if tag == 'Assign':
                    place, expression = kind['Assign']
                    if (place['kind'] != {'Local': 0} or set(expression) != {'Aggregate'}
                            or expression['Aggregate'][1] != []
                            or expression['Aggregate'][0]['Adt'][0]['builtin'] != 'Tuple'):
                        raise ValueError('non-unit assignment inside compiler drop glue')
                if tag == 'Drop':
                    drop = kind['Drop']
                    if (drop['kind'] != 'Precise'
                            or set(drop['fn_ptr']['kind']) != {'Fun'}):
                        raise ValueError('unreviewed drop dispatch inside compiler drop glue')
            for child in value.values():
                walk(child)
    walk(declaration['body'])
    for called in called_functions(declaration['body']):
        target = declarations.get(called)
        target_name = drop_glue_name(target) if target is not None else None
        if target_name is None:
            raise ValueError('custom destructor called by compiler drop glue')
        if (target_name.removeprefix('sandbox_controller_core::') not in DROP_GLUE_TYPES
                and target_name not in {'alloc::string::String', 'alloc::vec::Vec',
                                        'core::option::Option'}):
            raise ValueError('unreviewed destructor called by compiler drop glue')


def owned_name(name):
    return name.split('::', 1)[0] in OWNERS


def owned_declaration(declaration):
    first = declaration['item_meta']['name'][0]
    return first.get('Ident', [None])[0] in OWNERS


def declaration_map(declarations):
    present = [declaration for declaration in declarations if declaration is not None]
    result = {declaration['def_id']: declaration for declaration in present}
    if len(result) != len(present):
        raise ValueError('duplicate LLBC declaration ID')
    return result


def ordered_ids(llbc, kind):
    result = set()
    for declaration in llbc['translated']['ordered_decls']:
        if kind not in declaration:
            continue
        group = declaration[kind]
        if set(group) == {'NonRec'}:
            ids = [group['NonRec']]
        elif set(group) == {'Rec'}:
            ids = group['Rec']
        else:
            raise ValueError('unreviewed declaration-order schema')
        if any(type(index) is not int or index < 0 or index in result for index in ids):
            raise ValueError('duplicate or malformed declaration order')
        result.update(ids)
    return result


def count_loops(value):
    if isinstance(value, dict):
        return int('Loop' in value) + sum(count_loops(child) for child in value.values())
    if isinstance(value, list):
        return sum(count_loops(child) for child in value)
    return 0


def called_functions(value):
    if isinstance(value, dict):
        kind = value.get('kind')
        here = {kind['Fun']} if isinstance(kind, dict) and set(kind) == {'Fun'} else set()
        return here.union(*(called_functions(child) for child in value.values()))
    if isinstance(value, list):
        return set().union(*(called_functions(child) for child in value))
    return set()


def has_definition(root, lean_file, name, kinds='def'):
    generated = root / 'verification/aeneas/network' / lean_file
    lean_name = name.removeprefix('SandproofNetworkRust.')
    return re.search(r'\b(?:' + kinds + r')\s+' + re.escape(lean_name) + r'(?![\w.])',
                     generated.read_text()) is not None


def structured_body(declaration):
    body = declaration.get('body') if declaration is not None else None
    return isinstance(body, dict) and set(body) == {'Structured'}


def check(root, llbc, inventory):
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon reported errors or missing status')
    if inventory.get('crate') != CRATE or llbc['translated']['crate_name'] != CRATE:
        raise ValueError('unexpected extraction crate')
    environment = json.loads((root / 'verification/network-environment.json').read_text())
    if set(environment.get('erased_compiler_glue', {}).get('types', [])) != DROP_GLUE_TYPES:
        raise ValueError('erased compiler glue differs from frozen environment')
    targets = {target['key'] for target in llbc['translated']['target_information']}
    if targets != {environment['target']}:
        raise ValueError('extraction target does not match the frozen environment')
    destination = 'verification/aeneas/llbc/sandproof_network.llbc'
    if (environment.get('artifact_destination') != destination
            or environment.get('driver_destination') != '../aeneas/llbc/sandproof_network.llbc'
            or llbc['translated']['options']['dest_file'] != environment.get('driver_destination')):
        raise ValueError('extraction artifact destination must be the frozen project-relative path')
    roots = re.findall(r'--start-from crate::([a-z0-9_]+)',
                       (root / 'scripts/regenerate-network-lean.sh').read_text())
    public = re.findall(r'pub fn ([a-z0-9_]+)\(',
                        (root / 'verification/extract-network/src/lib.rs').read_text())
    if roots != list(ROOTS) or len(public) != len(ROOTS) or set(public) != set(ROOTS):
        raise ValueError('frozen production-call roots and script disagree')
    if environment['roots'] != list(ROOTS):
        raise ValueError('environment roots differ from the reviewed roots')
    functions = inventory['functions']
    parents = [function for function in functions if 'loop' not in function]
    by_name = {function['rust_name']: function for function in parents}
    parent_ids = [function['def_id'] for function in parents]
    if len(by_name) != len(parents) or len(set(parent_ids)) != len(parent_ids):
        raise ValueError('duplicate parent function inventory name or ID')
    lean_names = [function['lean_name'] for function in functions]
    if len(lean_names) != len(set(lean_names)):
        raise ValueError('duplicate Lean function inventory name')
    externals = {function['rust_name']: function['lean_name']
                 for function in parents if function['is_opaque']}
    if externals != EXTERNALS:
        raise ValueError('unreviewed external function surface')
    declarations = declaration_map(llbc['translated']['fun_decls'])
    ordered = ordered_ids(llbc, 'Fun')
    loop_roles = {}
    for function in functions:
        if 'loop' not in function:
            continue
        parent = by_name.get(function['rust_name'])
        loop = function['loop']
        if (parent is None or parent['def_id'] != function['def_id']
                or parent['lean_name'] != function.get('parent_lean_name')
                or parent['lean_file'] != function['lean_file']
                or function['is_opaque'] or parent['is_opaque']
                or type(loop.get('id')) is not int or loop['id'] < 0
                or not isinstance(loop.get('pos'), list)
                or any(type(index) is not int or index < 0 for index in loop['pos'])
                or type(loop.get('is_body')) is not bool):
            raise ValueError('unbound or malformed loop inventory')
        role = (function['def_id'], loop['id'], tuple(loop['pos']))
        roles = loop_roles.setdefault(role, set())
        if loop['is_body'] in roles:
            raise ValueError('duplicate loop inventory role')
        roles.add(loop['is_body'])
    if any(roles != {False, True} for roles in loop_roles.values()):
        raise ValueError('loop wrapper or body missing from inventory')
    loop_bindings = {}
    for function in functions:
        if 'loop' in function:
            loop = function['loop']
            loop_bindings.setdefault(function['rust_name'], set()).add((loop['id'], tuple(loop['pos'])))
    if loop_bindings != LOOPS:
        raise ValueError('loop identity or position differs from the fixed generated structure')
    covered = set()
    for function in functions:
        name = function['rust_name']
        declaration = declarations.get(function['def_id'])
        if owned_name(name):
            if (function['is_opaque'] or declaration is None
                    or not owned_declaration(declaration) or not structured_body(declaration)):
                raise ValueError(f'project-owned body missing: {name}')
            if function['def_id'] not in ordered:
                raise ValueError(f'project-owned body omitted from order: {name}')
            if function['lean_file'] != FUNCTION_FILE:
                raise ValueError(f'owned function has unexpected generated file: {name}')
            if not has_definition(root, FUNCTION_FILE, function['lean_name']):
                raise ValueError(f'owned loop/function has no generated definition: {name}')
            covered.add(function['def_id'])
        elif not function['is_opaque']:
            raise ValueError(f'unreviewed non-owned generated function: {name}')
        elif (declaration is None or declaration['body'] != 'Opaque'
              or function['lean_file'] != 'SandproofNetwork/FunsExternal_Template.lean'):
            raise ValueError('external function inventory does not bind its opaque declaration')
        elif not has_definition(root, 'SandproofNetwork/FunsExternal.lean', function['lean_name']):
            raise ValueError(f'reviewed external model definition missing: {name}')
    for parent in parents:
        if not owned_name(parent['rust_name']):
            continue
        loops = [role for role in loop_roles if role[0] == parent['def_id']]
        if len(loops) != count_loops(declarations[parent['def_id']]['body']):
            raise ValueError('LLBC loop omitted or invented by function inventory')
    for name in ROOTS:
        function = by_name.get(f'{CRATE}::{name}')
        if function is None or function['lean_file'] != FUNCTION_FILE:
            raise ValueError(f'root was not generated as a real function: {name}')
    globals_by_id = declaration_map(llbc['translated']['global_decls'])
    global_ids = []
    for global_entry in inventory['globals']:
        if not owned_name(global_entry['rust_name']):
            raise ValueError('unreviewed global inventory')
        declaration = globals_by_id.get(global_entry['def_id'])
        if (declaration is None or not owned_declaration(declaration)
                or global_entry['def_id'] not in ordered_ids(llbc, 'Global')
                or global_entry['lean_file'] != FUNCTION_FILE
                or not has_definition(root, FUNCTION_FILE, global_entry['lean_name'])):
            raise ValueError('owned global missing from LLBC/order/generated definitions')
        initializers = called_functions(declaration['value'])
        if len(initializers) != 1:
            raise ValueError('unreviewed owned global initializer schema')
        initializer = declarations.get(next(iter(initializers)))
        if (initializer is None or not owned_declaration(initializer)
                or not structured_body(initializer) or initializer['def_id'] not in ordered):
            raise ValueError('owned global initializer body missing')
        covered.update(initializers)
        global_ids.append(global_entry['def_id'])
    own_globals = {index for index, declaration in globals_by_id.items()
                   if owned_declaration(declaration)}
    if set(global_ids) != own_globals or len(global_ids) != len(own_globals):
        raise ValueError('owned global omitted or duplicated by inventory')
    own_functions = {index for index, declaration in declarations.items()
                     if owned_declaration(declaration)}
    erased = {index for index in own_functions
              if drop_glue_name(declarations[index]) is not None}
    if {drop_glue_name(declarations[index]).removeprefix('sandbox_controller_core::')
            for index in erased} != DROP_GLUE_TYPES:
        raise ValueError('compiler drop glue inventory differs from frozen types')
    for index in erased:
        if index not in ordered:
            raise ValueError('compiler drop glue omitted from declaration order')
        check_drop_glue(declarations[index], declarations)
    covered.update(erased)
    if covered != own_functions:
        raise ValueError('owned LLBC body omitted by function/global inventory')
    types = inventory['types']
    type_ids = [entry['def_id'] for entry in types]
    type_names = [entry['lean_name'] for entry in types]
    if len(set(type_ids)) != len(type_ids) or len(set(type_names)) != len(type_names):
        raise ValueError('duplicate type inventory ID or name')
    external_types = {entry['rust_name']: entry['lean_name'] for entry in types
                      if not owned_name(entry['rust_name'])}
    if external_types != EXTERNAL_TYPES:
        raise ValueError('unreviewed external type surface')
    type_declarations = declaration_map(llbc['translated']['type_decls'])
    own_types = set()
    u64_ids = scalar_u64_ids(llbc)
    for entry in types:
        declaration = type_declarations.get(entry['def_id'])
        if owned_name(entry['rust_name']):
            alias = entry['rust_name'] in U64_NEWTYPE_ALIASES
            if alias:
                fields = (declaration['kind'].get('Struct', [])
                          if declaration and isinstance(declaration['kind'], dict) else [])
                if (len(fields) != 1 or fields[0]['name'] != '_0'
                        or fields[0]['ty'] not in [{'Deduplicated': index} for index in u64_ids]):
                    raise ValueError('u64 newtype alias shape differs from frozen representation')
            if (declaration is None or not owned_declaration(declaration)
                    or not isinstance(declaration['kind'], dict)
                    or entry['def_id'] not in ordered_ids(llbc, 'Type')
                    or entry['lean_file'] != TYPE_FILE
                    or not has_definition(root, TYPE_FILE, entry['lean_name'],
                        'def' if alias else 'structure|inductive')):
                raise ValueError(f'owned type definition missing: {entry["rust_name"]}')
            own_types.add(entry['def_id'])
        elif (declaration is None or declaration['kind'] != 'Opaque'
              or entry['lean_file'] != 'SandproofNetwork/TypesExternal_Template.lean'
              or not has_definition(root, 'SandproofNetwork/TypesExternal.lean',
                                    entry['lean_name'], 'structure|inductive|abbrev')):
            raise ValueError('reviewed external type model missing or incorrectly bound')
    if own_types != {index for index, declaration in type_declarations.items()
                     if owned_declaration(declaration)}:
        raise ValueError('owned LLBC type omitted by inventory')
    return len(ROOTS)


def check_standard_sources(sysroot, expected):
    if set(expected) != STANDARD_SOURCES:
        raise ValueError('standard-library source inventory differs from the fixed interface files')
    library = Path(sysroot) / 'lib/rustlib/src/rust/library'
    for path, digest in expected.items():
        if hashlib.sha256((library / path).read_bytes()).hexdigest() != digest:
            raise ValueError(f'fixed Rust standard-library source changed: {path}')


def check_tools(root, tool_root, environment):
    paths = {'charon': ('aeneas/charon', 'aeneas/charon/bin/charon'),
             'aeneas': ('aeneas', 'aeneas/bin/aeneas')}
    for name, (repository, binary) in paths.items():
        revision = subprocess.check_output(
            ['git', '-C', str(tool_root / repository), 'rev-parse', 'HEAD'], text=True).strip()
        digest = hashlib.sha256((tool_root / binary).read_bytes()).hexdigest()
        if revision != environment[name]['revision'] or digest != environment[name]['binary_sha256']:
            raise ValueError(f'{name} differs from the frozen tool binary/revision')
    backend = subprocess.check_output(
        ['git', '-C', str(root / '.lake/packages/aeneas'), 'rev-parse', 'HEAD'], text=True).strip()
    if backend != environment['aeneas']['revision']:
        raise ValueError('Lean backend differs from the translator revision')
    version = subprocess.check_output(['lake', 'env', 'lean', '--version'], cwd=root, text=True).strip()
    if not version.startswith(f"Lean (version {environment['lean']},"):
        raise ValueError('Lean compiler differs from the frozen environment')
    charon = str(tool_root / paths['charon'][1])
    frontend = subprocess.check_output([charon, 'toolchain-version'], text=True).strip()
    extraction_sysroot = subprocess.check_output([charon, 'toolchain-path'], text=True).strip()
    extraction_rustc = str(Path(extraction_sysroot) / 'bin/rustc')
    production = subprocess.check_output(['rustc', '-vV'], text=True)
    extraction = subprocess.check_output([extraction_rustc, '-vV'], text=True)
    cfg = subprocess.check_output([extraction_rustc, '--print', 'cfg'], text=True).splitlines()
    if (frontend != environment['extraction_frontend']
            or production.splitlines()[0] != environment['production_compiler']
            or extraction.splitlines()[0] != environment['extraction_compiler']
            or cfg != environment['cfg']):
        raise ValueError('production/extraction compiler or configuration differs from the frozen environment')
    for role, version in (('production', production), ('extraction', extraction)):
        commit = re.search(r'^commit-hash: (.+)$', version, re.MULTILINE)
        if commit is None or commit.group(1) != environment['standard_library'][role]['compiler_commit']:
            raise ValueError(f'{role} standard-library compiler commit differs')
    production_sysroot = subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip()
    check_standard_sources(production_sysroot, environment['standard_library']['production']['sources'])
    check_standard_sources(extraction_sysroot, environment['standard_library']['extraction']['sources'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--llbc-digest-only', action='store_true')
    args = parser.parse_args()
    try:
        root = args.root
        llbc = json.loads((root / 'verification/aeneas/llbc/sandproof_network.llbc').read_text())
        if args.llbc_digest_only:
            print(llbc_digest(llbc))
            return 0
        inventory = json.loads((root / 'verification/aeneas/network/translation.json').read_text())
        count = check(root, llbc, inventory)
        if args.tool_root is not None:
            environment = json.loads((root / 'verification/network-environment.json').read_text())
            check_tools(root, args.tool_root, environment)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f'IPv4 extraction audit failed: {error}', file=sys.stderr)
        return 1
    print(f'IPv4 extraction: {count} real roots, all owned bodies/loops/globals present, '
          f'{len(EXTERNALS)} reviewed external functions and {len(EXTERNAL_TYPES)} external types.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
