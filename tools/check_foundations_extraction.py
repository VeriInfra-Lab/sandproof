"""Audit real V0 roots, bodies and the explicitly reviewed library surface."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import sys
import subprocess

EXTERNALS = {
    "core::option::{core::option::Option<T>}::as_ref",
    "core::option::{impl core::ops::try_trait::Try for core::option::Option<T>}::branch",
    "core::option::{impl core::ops::try_trait::FromResidual<core::option::Option<core::convert::Infallible>> for core::option::Option<T>}::from_residual",
    "core::result::{core::result::Result<T, E>}::ok",
    "alloc::string::{impl core::cmp::PartialEq<alloc::string::String> for alloc::string::String}::eq",
    "alloc::vec::{alloc::vec::Vec<T>}::is_empty",
}


def llbc_digest(llbc):
    # Charon serializes its short-name HashMap as an unordered key/value list.
    # Canonicalize ONLY that map, reject duplicate keys, retain every other AST
    # field and all ordered arrays. Generated Lean is still byte-compared.
    data = copy.deepcopy(llbc)
    names = data['translated']['short_names']
    keys = [json.dumps(item['key'], sort_keys=True) for item in names]
    if len(keys) != len(set(keys)):
        raise ValueError('duplicate short-name key')
    names.sort(key=lambda item: json.dumps(item['key'], sort_keys=True))
    return hashlib.sha256(json.dumps(data, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def check(root, llbc, inventory):
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon reported errors or missing status')
    crate = 'sandproof_foundations_extract'
    if inventory.get('crate') != crate:
        raise ValueError('unexpected extraction crate')
    environment = json.loads((root / 'verification/foundations-environment.json').read_text())
    targets = {target['key'] for target in llbc['translated']['target_information']}
    if targets != {environment['target']}:
        raise ValueError('extraction target does not match the frozen environment')
    script = (root / 'scripts/regenerate-foundations-lean.sh').read_text()
    roots = re.findall(r'--start-from crate::([a-z0-9_]+)', script)
    source = (root / 'verification/extract-foundations/src/lib.rs').read_text()
    public = re.findall(r'pub fn ([a-z0-9_]+)\(', source)
    if not roots or len(roots) != len(set(roots)) or set(roots) != set(public):
        raise ValueError('free production-call roots and script disagree')
    functions = inventory['functions']
    # Aeneas emits the parent, loop wrapper and loop body under ONE Rust name.
    # Keep parent names unique, validate the alias binding and paired loop roles;
    # never collapse aliases silently or accept a metadata-only loop entry.
    parents = [f for f in functions if 'loop' not in f]
    by_name = {f['rust_name']: f for f in parents}
    if len(by_name) != len(parents):
        raise ValueError('duplicate function inventory name')
    lean_names = [f['lean_name'] for f in functions if f.get('lean_name')]
    if len(lean_names) != len(set(lean_names)):
        raise ValueError('duplicate Lean function inventory name')
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
        role = (function['rust_name'], loop['id'], tuple(loop['pos']))
        roles = loop_roles.setdefault(role, set())
        if loop['is_body'] in roles:
            raise ValueError('duplicate loop inventory role')
        roles.add(loop['is_body'])
    if any(roles != {False, True} for roles in loop_roles.values()):
        raise ValueError('loop wrapper or body missing from inventory')
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
    for function in functions:
        name = function['rust_name']
        if name.startswith(('sandproof_protocol::', 'sandproof_center_core::',
                            'sandbox_controller_core::', crate + '::')):
            body = declarations.get(function['def_id'], {}).get('body')
            if function['is_opaque'] or not isinstance(body, dict) or 'Structured' not in body:
                raise ValueError(f'project-owned body missing: {name}')
            if function['def_id'] not in ordered:
                raise ValueError(f'project-owned body omitted from order: {name}')
            if function['lean_file'] != 'SandproofFoundations/Funs.lean':
                raise ValueError(f'owned function has unexpected generated file: {name}')
            generated = root / 'verification/aeneas/foundations' / function['lean_file']
            lean_name = function['lean_name'].removeprefix('SandproofFoundationsRust.')
            if not re.search(r'\bdef\s+' + re.escape(lean_name) + r'\b', generated.read_text()):
                raise ValueError(f'owned loop/function has no generated definition: {name}')
    for name in roots:
        function = by_name.get(f'{crate}::{name}')
        if function is None or function['lean_file'] != 'SandproofFoundations/Funs.lean':
            raise ValueError(f'root was not generated as a real function: {name}')
        generated = root / 'verification/aeneas/foundations' / function['lean_file']
        lean_name = function['lean_name'].removeprefix('SandproofFoundationsRust.')
        if not re.search(r'\bdef\s+' + re.escape(lean_name) + r'\b', generated.read_text()):
            raise ValueError(f'root has inventory only, no generated definition: {name}')
    actual = {f['rust_name'] for f in functions if f['is_opaque']}
    if actual != EXTERNALS:
        raise ValueError(f'unreviewed external surface: added={actual-EXTERNALS}, removed={EXTERNALS-actual}')
    return len(roots)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--llbc-digest-only', action='store_true')
    args = parser.parse_args()
    try:
        root = args.root
        llbc = json.loads((root / 'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
        if args.llbc_digest_only:
            print(llbc_digest(llbc))
            return 0
        inventory = json.loads((root / 'verification/aeneas/foundations/translation.json').read_text())
        count = check(root, llbc, inventory)
        if args.tool_root is not None:
            environment = json.loads((root / 'verification/foundations-environment.json').read_text())
            paths = {
                'charon': ('aeneas/charon', 'aeneas/charon/bin/charon'),
                'aeneas': ('aeneas', 'aeneas/bin/aeneas'),
            }
            for name, (repository, binary) in paths.items():
                revision = subprocess.check_output(
                    ['git', '-C', str(args.tool_root / repository), 'rev-parse', 'HEAD'], text=True).strip()
                digest = hashlib.sha256((args.tool_root / binary).read_bytes()).hexdigest()
                if revision != environment[name]['revision'] or digest != environment[name]['binary_sha256']:
                    raise ValueError(f'{name} differs from the frozen tool binary/revision')
            backend = subprocess.check_output(
                ['git', '-C', str(root / '.lake/packages/aeneas'), 'rev-parse', 'HEAD'], text=True).strip()
            if backend != environment['aeneas']['revision']:
                raise ValueError('Lean backend differs from the translator revision')
            frontend = subprocess.check_output(
                [str(args.tool_root / paths['charon'][1]), 'toolchain-version'], text=True).strip()
            toolchain = subprocess.check_output(
                [str(args.tool_root / paths['charon'][1]), 'toolchain-path'], text=True).strip()
            extraction_rustc = Path(toolchain) / 'bin/rustc'
            extraction_version = subprocess.check_output([str(extraction_rustc), '--version'], text=True).strip()
            cfg = subprocess.check_output([str(extraction_rustc), '--print', 'cfg'], text=True).splitlines()
            compiler = subprocess.check_output(['rustc', '--version'], text=True).strip()
            if frontend != environment['extraction_frontend'] or compiler != environment['production_compiler']:
                raise ValueError('production/extraction compiler differs from the frozen environment')
            if extraction_version != environment['extraction_compiler'] or cfg != environment['cfg']:
                raise ValueError('extraction compiler/configuration differs from the frozen environment')
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f'V0 extraction audit failed: {error}', file=sys.stderr)
        return 1
    print(f'V0 extraction: {count} real roots, all owned bodies present, {len(EXTERNALS)} reviewed external models.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
