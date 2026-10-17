"""Admit only the frozen real counter Deserialize leaf and reviewed providers."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

try:
    from tools.check_foundations_extraction import llbc_digest, check as check_foundations
    from tools.check_first_chain_extraction import check_tools
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest, check as check_foundations
    from check_first_chain_extraction import check_tools

CRATE = 'sandproof_foundations_extract'
PREFIX = 'SandproofFoundationsRust.'
DEST = 'verification/aeneas/foundations'
LLBC = 'verification/aeneas/llbc/sandproof_foundations.llbc'
ENVIRONMENT = 'verification/json-counter-environment.json'
ENTRY = 'verification/extract-foundations/src/json.rs'
SOURCE = 'sandbox-protocol/src/internal.rs'
ROOTS = {'counter_decode', 'counter_primitive', 'json_error_passthrough'}
DEFAULT_VISITOR = 'serde_core::de::Visitor::visit_u64'
FOCUSED = {
    "serde_core::de::Visitor::visit_u64",
    "serde_core::de::impls::{impl serde_core::de::Deserialize<'de> for u64}::deserialize::{impl serde_core::de::Visitor<'de, u64> for serde_core::de::impls::{impl serde_core::de::Deserialize<'de> for u64}::deserialize::PrimitiveVisitor}::visit_u64",
    "serde_core::de::impls::{impl serde_core::de::Deserialize<'de> for u64}::deserialize::{impl serde_core::de::Visitor<'de, u64> for serde_core::de::impls::{impl serde_core::de::Deserialize<'de> for u64}::deserialize::PrimitiveVisitor}::expecting",
    "serde_core::de::impls::{impl serde_core::de::Deserialize<'de> for u64}::deserialize",
    "sandproof_protocol::internal::{impl core::convert::TryFrom<u64, sandproof_protocol::internal::InternalError> for sandproof_protocol::internal::PositiveCounter}::try_from",
    "sandproof_protocol::internal::{impl core::fmt::Display for sandproof_protocol::internal::InternalError}::fmt",
    "sandproof_protocol::internal::{impl serde_core::de::Deserialize<'de> for sandproof_protocol::internal::PositiveCounter}::deserialize::{impl core::ops::function::FnOnce<(sandproof_protocol::internal::InternalError,), Clause0_Error> for sandproof_protocol::internal::{impl serde_core::de::Deserialize<'de> for sandproof_protocol::internal::PositiveCounter}::deserialize::{closure}<'de, D, Clause0_Error>}::call_once",
    "sandproof_protocol::internal::{impl serde_core::de::Deserialize<'de> for sandproof_protocol::internal::PositiveCounter}::deserialize",
    "sandproof_foundations_extract::json::counter_decode",
    "sandproof_foundations_extract::json::counter_primitive",
    "sandproof_foundations_extract::json::json_error_passthrough",
}


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def check(root, llbc, inventory):
    # The original V0 admission still checks the complete shared extraction.
    check_foundations(root, llbc, inventory)
    environment = json.loads((root / ENVIRONMENT).read_text())
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon reported errors or missing status')
    translated = llbc['translated']
    if translated['options'] != environment['charon_options']:
        raise ValueError('unreviewed extraction options or destination')
    if {x['key'] for x in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('unreviewed target')
    if inventory.get('crate') != CRATE:
        raise ValueError('unexpected extraction crate')
    functions = [f for f in inventory['functions'] if f['rust_name'] in FOCUSED]
    by_name = {f['rust_name']: f for f in functions}
    if (len(by_name) != len(functions)
            or len({f['def_id'] for f in functions}) != len(functions)
            or len({f['lean_name'] for f in functions}) != len(functions)):
        raise ValueError('duplicate function inventory')
    if set(by_name) != FOCUSED or set(environment['functions']) != FOCUSED:
        raise ValueError('missing function or unreviewed surface')
    if {f['rust_name'] for f in functions if f['is_opaque']} != {DEFAULT_VISITOR}:
        raise ValueError('project callback hidden or unreviewed opaque surface')
    if {f['rust_name'] for f in functions if f['is_local']} != {CRATE + '::json::' + r for r in ROOTS}:
        raise ValueError('unreviewed extraction roots')
    declarations = {f['def_id']: f for f in translated['fun_decls'] if f is not None}
    ordered = []
    for declaration in translated['ordered_decls']:
        if 'Fun' not in declaration:
            continue
        group = declaration['Fun']
        if set(group) == {'NonRec'}:
            ordered.append(group['NonRec'])
        elif set(group) == {'Rec'}:
            ordered.extend(group['Rec'])
        else:
            raise ValueError('unknown function-order schema')
    if len(ordered) != len(set(ordered)):
        raise ValueError('duplicate function declaration order')
    generated = (root / DEST / 'SandproofFoundations/Funs.lean').read_text()
    for name, function in by_name.items():
        if function['is_opaque']:
            continue
        declaration = declarations[function['def_id']]
        body = declaration.get('body')
        if not isinstance(body, dict) or set(body) != {'Structured'}:
            raise ValueError('transparent body missing: ' + name)
        if function['def_id'] not in ordered:
            raise ValueError('body omitted from declaration order')
        if function['lean_file'] != 'SandproofFoundations/Funs.lean':
            raise ValueError('unexpected generated definition file')
        lean_name = function['lean_name'].removeprefix(PREFIX)
        if not re.search(r'\bdef\s+' + re.escape(lean_name) + r'(?![\w.])', generated):
            raise ValueError('generated definition missing: ' + name)
        if name.startswith(('sandproof_protocol::', CRATE + '::')):
            expected = SOURCE if name.startswith('sandproof_protocol::') else ENTRY
            source_root = root if name.startswith('sandproof_protocol::') else (
                root / 'verification/extract-foundations')
            if (source_root / function['source']['file']).resolve() != (root / expected).resolve():
                raise ValueError('wrong production or call-through source')
        cleanup = []
        for node in nodes(body):
            if any(tag in node for tag in ('Error', 'Loop')):
                raise ValueError('unreviewed error/drop/loop body')
            if 'Drop' in node:
                drop = node['Drop']
                if not name.startswith(CRATE + '::') or drop['kind'] != 'Conditional':
                    raise ValueError('unreviewed cleanup or project destructor')
                cleanup.append(drop)
        # The fixed preset erases these exact conditional wrapper cleanups.
        # Register them; do not mistake that erasure for a Drop proof. The
        # interface excludes externally observable custom-destructor effects.
        if cleanup != environment['conditional_cleanup'].get(name, []):
            raise ValueError('unreviewed conditional cleanup location or type')
    for group in ('types', 'trait_decls', 'trait_impls', 'globals'):
        actual = [x['rust_name'] for x in inventory[group]]
        if len(set(actual)) != len(actual) or set(actual) != set(environment[group]):
            raise ValueError('unreviewed ' + group + ' surface')
    # Keep every AST field, including calls, generics and trait dictionaries.
    # Only the original unordered short-name map is canonicalized by V0.
    if llbc_digest(llbc) != environment['llbc_sha256']:
        raise ValueError('full LLBC changed; review semantics before freezing')
    return len(functions) - 1


def check_environment(root, tool_root):
    check_tools(root, tool_root)
    environment = json.loads((root / ENVIRONMENT).read_text())
    metadata = json.loads(subprocess.check_output([
        'cargo', 'metadata', '--format-version', '1', '--locked', '--offline',
        '--manifest-path', str(root / 'verification/extract-foundations/Cargo.toml'),
    ], text=True))
    packages = {p['name']: p for p in metadata['packages']}
    resolved = {node['id']: node['features'] for node in metadata['resolve']['nodes']}
    for name, expected in environment['library_sources'].items():
        package = packages[name]
        if package['version'] != expected['version']:
            raise ValueError('library version changed: ' + name)
        if sorted(resolved[package['id']]) != environment['extraction_features'][name]:
            raise ValueError('library feature configuration changed: ' + name)
        directory = Path(package['manifest_path']).parent
        for relative, digest in expected['files'].items():
            if hashlib.sha256((directory / relative).read_bytes()).hexdigest() != digest:
                raise ValueError('library source changed: ' + name + '/' + relative)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--llbc-digest-only', action='store_true')
    args = parser.parse_args(argv)
    try:
        llbc = json.loads((args.root / LLBC).read_text())
        if args.llbc_digest_only:
            print(llbc_digest(llbc))
            return 0
        inventory = json.loads((args.root / DEST / 'translation.json').read_text())
        count = check(args.root, llbc, inventory)
        if args.tool_root is not None:
            check_environment(args.root, args.tool_root)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print('JSON counter extraction failed: ' + str(error), file=sys.stderr)
        return 1
    print(f'JSON counter: {count} transparent functions; actual Deserialize/TryFrom remain owned.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
