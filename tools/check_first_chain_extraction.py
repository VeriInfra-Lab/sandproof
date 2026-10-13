"""Check the separate Linux first-chain roots without changing frozen V0 inputs."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

try:
    from tools.check_foundations_extraction import llbc_digest
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest

OPTION_EQ = 'core::option::{impl core::cmp::PartialEq<core::option::Option<T>> for core::option::Option<T>}::eq'
EXTERNALS = {
    'alloc::string::{impl core::cmp::PartialEq<alloc::string::String> for alloc::string::String}::eq',
    'core::option::{core::option::Option<T>}::as_ref',
}
CRATE = 'sandproof_first_chain_extract'
DEST = Path('verification/aeneas/first-chain')
ENVIRONMENT = Path('verification/first-chain-linux-environment.json')
LAKE_EXTENSION = '\n[[lean_lib]]\nname = "SandproofFirstChain"\nsrcDir = "verification/aeneas/first-chain"\n'


def check(root, llbc, inventory):
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon reported errors or missing status')
    environment = json.loads((root / ENVIRONMENT).read_text())
    targets = {target['key'] for target in llbc['translated']['target_information']}
    if environment['target'] != 'x86_64-unknown-linux-gnu' or targets != {environment['target']}:
        raise ValueError('extraction target differs from the Linux environment')
    destination = 'verification/aeneas/llbc/sandproof_first_chain.llbc'
    if (environment.get('artifact_destination') != destination
            or llbc['translated']['options']['dest_file'] != destination):
        raise ValueError('extraction artifact destination must be the frozen relative path')
    script = (root / 'scripts/regenerate-first-chain-lean.sh').read_text()
    roots = re.findall(r'--start-from crate::([a-z0-9_]+)', script)
    public = re.findall(r'pub fn ([a-z0-9_]+)\(',
                        (root / 'verification/extract-first-chain/src/lib.rs').read_text())
    if roots != ['identity_response', 'lifecycle_response', 'operation_response'] or set(roots) != set(public):
        raise ValueError('production-call roots and script disagree')
    if inventory.get('crate') != CRATE:
        raise ValueError('unexpected extraction crate')
    base_config = (root / 'lakefile.toml').read_text().rstrip()
    linux_config = (root / 'lake-first-chain.toml').read_text().rstrip()
    if linux_config != (base_config + '\n' + LAKE_EXTENSION).rstrip():
        raise ValueError('Linux Lake view must preserve the complete V0 configuration')
    functions = inventory['functions']
    names = [f['rust_name'] for f in functions]
    lean_names = [f.get('lean_name') for f in functions]
    if len(names) != len(set(names)) or len(lean_names) != len(set(lean_names)):
        raise ValueError('duplicate function inventory name')
    by_name = dict(zip(names, functions))
    required = {CRATE + '::' + name for name in roots} | {OPTION_EQ}
    if not required <= by_name.keys():
        raise ValueError('root or actual Option body missing from inventory')
    declarations = {f['def_id']: f for f in llbc['translated']['fun_decls'] if f is not None}
    ordered = set()
    for declaration in llbc['translated']['ordered_decls']:
        if 'Fun' not in declaration:
            continue
        group = declaration['Fun']
        if 'NonRec' in group:
            ordered.add(group['NonRec'])
        elif 'Rec' in group:
            ordered.update(group['Rec'])
        else:
            raise ValueError('unreviewed function-order schema')
    actual = {f['rust_name'] for f in functions if f['is_opaque']}
    if actual != EXTERNALS:
        raise ValueError(f'unreviewed external surface: added={actual-EXTERNALS}, removed={EXTERNALS-actual}')
    # Charon stores a named constant's initializer as a function, while Aeneas
    # emits that same body in the globals inventory. Account for it explicitly;
    # do not treat its missing function entry as permission to drop the body.
    globals_by_id = {g['def_id']: g for g in llbc['translated']['global_decls'] if g is not None}
    globals_inventory = inventory.get('globals', [])
    if (len(globals_inventory) != 1 or globals_inventory[0]['rust_name'] !=
            'sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION'):
        raise ValueError('protocol version global missing or unreviewed globals added')
    global_entry = globals_inventory[0]
    global_id = global_entry['def_id']
    global_decl = globals_by_id.get(global_id)
    if (set(globals_by_id) != {global_id} or global_decl is None
            or global_entry['lean_file'] != 'SandproofFirstChain/Funs.lean'):
        raise ValueError('global has no matching declaration/generated file')
    initializer_ids = set()
    for declaration in declarations.values():
        src = declaration['src']
        if not isinstance(src, dict) or 'GlobalInitializer' not in src:
            continue
        if src['GlobalInitializer']['id'] != global_id:
            raise ValueError('unreviewed global initializer')
        body = declaration.get('body')
        if not isinstance(body, dict) or 'Structured' not in body:
            raise ValueError('global initializer body missing')
        initializer_ids.add(declaration['def_id'])
    if len(initializer_ids) != 1:
        raise ValueError('global initializer missing or duplicated')
    initializer_id = next(iter(initializer_ids))
    value = global_decl['value']
    if (value['Value'][1][0]['Call'][0]['kind'] != {'Fun': initializer_id}
            or {'Global': {'NonRec': global_id}} not in llbc['translated']['ordered_decls']):
        raise ValueError('global initializer is not bound to the ordered constant')
    global_name = global_entry['lean_name'].removeprefix('SandproofFirstChainRust.')
    if not re.search(r'\bdef\s+' + re.escape(global_name) + r'\b',
                     (root / DEST / global_entry['lean_file']).read_text()):
        raise ValueError('global has no generated definition')
    owned_ids = {
        declaration['def_id'] for declaration in declarations.values()
        if declaration['item_meta']['name'][0].get('Ident', [None])[0] in {CRATE, 'sandproof_protocol', 'sandproof_center_core'}
    }
    inventory_owned_ids = {
        function['def_id'] for function in functions
        if function['rust_name'].startswith((CRATE + '::', 'sandproof_protocol::', 'sandproof_center_core::'))
    }
    if owned_ids != inventory_owned_ids | initializer_ids:
        raise ValueError('owned LLBC function omitted or invented in the inventory')
    for function in functions:
        name = function['rust_name']
        if name in EXTERNALS:
            continue
        if not name.startswith((CRATE + '::', 'sandproof_protocol::', 'sandproof_center_core::')) and name != OPTION_EQ:
            raise ValueError(f'unreviewed transparent dependency: {name}')
        body = declarations.get(function['def_id'], {}).get('body')
        if function['is_opaque'] or not isinstance(body, dict) or 'Structured' not in body:
            raise ValueError(f'owned/Option body missing: {name}')
        if function['def_id'] not in ordered:
            raise ValueError(f'function omitted from order: {name}')
        if function['lean_file'] != 'SandproofFirstChain/Funs.lean':
            raise ValueError(f'unexpected generated file: {name}')
        generated = (root / DEST / function['lean_file']).read_text()
        lean_name = function['lean_name'].removeprefix('SandproofFirstChainRust.')
        if not re.search(r'\bdef\s+' + re.escape(lean_name) + r'\b', generated):
            raise ValueError(f'no generated definition: {name}')
    return len(roots)


def check_tools(root, tool_root):
    environment = json.loads((root / ENVIRONMENT).read_text())
    for name, repository, binary in (
        ('charon', 'aeneas/charon', 'aeneas/charon/bin/charon'),
        ('aeneas', 'aeneas', 'aeneas/bin/aeneas'),
    ):
        revision = subprocess.check_output(['git', '-C', str(tool_root / repository), 'rev-parse', 'HEAD'], text=True).strip()
        digest = hashlib.sha256((tool_root / binary).read_bytes()).hexdigest()
        dirty = subprocess.check_output(['git', '-C', str(tool_root / repository), 'status', '--porcelain', '--untracked-files=no'], text=True)
        if revision != environment[name]['revision'] or digest != environment[name]['binary_sha256'] or dirty:
            raise ValueError(f'{name} revision/binary/source differs from frozen Linux tools')
    driver = hashlib.sha256((tool_root / 'aeneas/charon/bin/charon-driver').read_bytes()).hexdigest()
    if driver != environment['charon']['driver_sha256']:
        raise ValueError('Charon driver differs from frozen Linux tools')
    backend = subprocess.check_output(['git', '-C', str(root / '.lake/packages/aeneas'), 'rev-parse', 'HEAD'], text=True).strip()
    if backend != environment['aeneas']['revision']:
        raise ValueError('Lean backend revision differs from translator')
    lean = subprocess.check_output(['lean', '--version'], text=True)
    if not lean.startswith('Lean (version ' + environment['lean'] + ','):
        raise ValueError('Lean differs from the frozen version')
    charon = tool_root / 'aeneas/charon/bin/charon'
    frontend = subprocess.check_output([str(charon), 'toolchain-version'], text=True).strip()
    sysroot = subprocess.check_output([str(charon), 'toolchain-path'], text=True).strip()
    rustc = Path(sysroot) / 'bin/rustc'
    extraction = subprocess.check_output([str(rustc), '--version'], text=True).strip()
    cfg = subprocess.check_output([str(rustc), '--print', 'cfg', '--target', environment['target']], text=True).splitlines()
    production = subprocess.check_output(['rustc', '--version'], text=True).strip()
    if (frontend != environment['extraction_frontend'] or extraction != environment['extraction_compiler']
            or production != environment['production_compiler'] or cfg != environment['cfg']):
        raise ValueError('compiler/configuration differs from frozen Linux environment')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--llbc-digest-only', action='store_true')
    args = parser.parse_args(argv)
    try:
        root = args.root
        llbc = json.loads((root / 'verification/aeneas/llbc/sandproof_first_chain.llbc').read_text())
        if args.llbc_digest_only:
            print(llbc_digest(llbc))
            return 0
        inventory = json.loads((root / DEST / 'translation.json').read_text())
        count = check(root, llbc, inventory)
        if args.tool_root is not None:
            check_tools(root, args.tool_root)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f'Linux first-chain extraction failed: {error}', file=sys.stderr)
        return 1
    print(f'Linux first chain: {count} real roots, actual Option body, all owned bodies present; reviewed String/Option borrow models.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
