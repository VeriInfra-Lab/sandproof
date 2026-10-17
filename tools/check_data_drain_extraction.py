"""Audit the original DataLease read bodies and their three read-only providers."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

try:
    from tools.check_foundations_extraction import llbc_digest
    from tools.check_first_chain_extraction import check_tools
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest
    from check_first_chain_extraction import check_tools

CRATE = 'sandproof_data_drain_extract'
SOURCE = 'controller-service/src/application/data_drain.rs'
ENTRY = 'verification/extract-data-drain/src/lib.rs'
LLBC = 'verification/aeneas/llbc/sandproof_data_drain.llbc'
DEST = 'verification/aeneas/data-drain'
ENVIRONMENT = 'verification/data-drain-environment.json'
PREFIX = 'SandproofDataDrainRust.'
METHOD = CRATE + '::data_drain::{' + CRATE + '::data_drain::DataLease}::'
OWNED = {CRATE + '::read_drained', CRATE + '::read_routes_dropped',
         METHOD + 'drained', METHOD + 'routes_dropped'}
COUNT = 'alloc::sync::{alloc::sync::Arc<T>}::strong_count'
DEREF = 'alloc::sync::{impl core::ops::deref::Deref<T> for alloc::sync::Arc<T>}::deref'
LOAD = ('core::sync::atomic::{core::sync::atomic::Atomic<bool>'
        '[core::sync::atomic::{impl core::sync::atomic::AtomicPrimitive'
        '<core::sync::atomic::private::Align1<u8>> for bool}]}::load')
EXTERNALS = {COUNT, DEREF, LOAD}
TYPES = {'core::sync::atomic::private::Align1', 'core::sync::atomic::Atomic',
         'alloc::sync::Arc', 'core::sync::atomic::Ordering', CRATE + '::data_drain::DataLease'}


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def check(root, llbc, inventory):
    environment = json.loads((root / ENVIRONMENT).read_text())
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon reported errors or missing status')
    translated = llbc['translated']
    if translated['options'] != environment['charon_options']:
        raise ValueError('unreviewed extraction options or destination')
    targets = {item['key'] for item in translated['target_information']}
    if targets != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('unreviewed extraction target')
    entry = (root / ENTRY).read_text()
    expected = ('//! Call-through roots for the original process-local drain read methods.\n'
                '//! No copied business implementation and no mutation/drop extraction.\n'
                '#[path = "../../../controller-service/src/application/data_drain.rs"]\n'
                'pub mod data_drain;\n\n'
                'pub fn read_drained(lease: &data_drain::DataLease) -> bool {\n'
                '    lease.drained()\n}\n\n'
                'pub fn read_routes_dropped(lease: &data_drain::DataLease) -> bool {\n'
                '    lease.routes_dropped()\n}\n')
    if entry != expected:
        raise ValueError('entry is no longer the reviewed original-file call-through')
    if inventory.get('crate') != CRATE:
        raise ValueError('wrong extraction crate')
    functions = inventory['functions']
    if any(f['is_local'] != (f['rust_name'] in OWNED) for f in functions):
        raise ValueError('unreviewed function ownership')
    by_name = {f['rust_name']: f for f in functions}
    if (len(by_name) != len(functions)
            or len({f['def_id'] for f in functions}) != len(functions)
            or len({f['lean_name'] for f in functions}) != len(functions)):
        raise ValueError('duplicate function inventory')
    if set(by_name) != OWNED | EXTERNALS:
        raise ValueError('missing root or unreviewed function surface')
    if {f['rust_name'] for f in functions if f['is_opaque']} != EXTERNALS:
        raise ValueError('unreviewed opaque surface')
    declarations = {f['def_id']: f for f in translated['fun_decls'] if f is not None}
    own_ids = {f['def_id'] for f in declarations.values()
               if f['item_meta']['name'][0].get('Ident', [None])[0] == CRATE}
    if own_ids != {by_name[name]['def_id'] for name in OWNED}:
        raise ValueError('owned LLBC function omitted or added')
    ordered = []
    for declaration in translated['ordered_decls']:
        if 'Fun' in declaration:
            group = declaration['Fun']
            if set(group) == {'NonRec'}:
                ordered.append(group['NonRec'])
            elif set(group) == {'Rec'}:
                ordered.extend(group['Rec'])
            else:
                raise ValueError('unreviewed declaration order')
    if len(ordered) != len(set(ordered)):
        raise ValueError('duplicate declaration order')
    names_by_id = {f['def_id']: f['rust_name'] for f in functions}
    expected_calls = {
        CRATE + '::read_drained': [METHOD + 'drained'],
        CRATE + '::read_routes_dropped': [METHOD + 'routes_dropped'],
        METHOD + 'routes_dropped': [COUNT],
        METHOD + 'drained': [METHOD + 'routes_dropped', DEREF, LOAD],
    }
    for name, expected_calls_here in expected_calls.items():
        item = by_name[name]
        body = declarations[item['def_id']].get('body')
        if item['is_opaque'] or not isinstance(body, dict) or set(body) != {'Structured'}:
            raise ValueError('owned body missing')
        if item['def_id'] not in ordered:
            raise ValueError('owned body missing from declaration order')
        if item['lean_file'] != 'SandproofDataDrain/Funs.lean':
            raise ValueError('unexpected generated file')
        generated = (root / DEST / item['lean_file']).read_text()
        if not re.search(r'\bdef\s+' + re.escape(item['lean_name'].removeprefix(PREFIX))
                         + r'(?![\w.])', generated):
            raise ValueError('generated definition missing')
        expected_source = SOURCE if name.startswith(METHOD) else ENTRY
        if (root / item['source']['file']).resolve() != (root / expected_source).resolve():
            raise ValueError('function not bound to the original source file')
        calls = []
        for node in nodes(body):
            if 'Error' in node or 'Drop' in node or 'Loop' in node:
                raise ValueError('unreviewed error/drop/loop in read body')
            if 'Call' in node:
                target = node['Call']['call']['func']['Regular']['kind']['Fun']
                calls.append(names_by_id.get(target))
        if Counter(calls) != Counter(expected_calls_here):
            raise ValueError('unreviewed read call surface or multiplicity')
    type_entries = inventory['types']
    if (len(type_entries) != len(TYPES)
            or {t['rust_name'] for t in type_entries} != TYPES):
        raise ValueError('unreviewed type surface')
    ordering = next(t for t in type_entries if t['rust_name'] == 'core::sync::atomic::Ordering')
    enum = next(t for t in translated['type_decls'] if t and t['def_id'] == ordering['def_id'])
    acquire = next(v['id'] for v in enum['kind']['Enum'] if v['name'] == 'Acquire')
    body = declarations[by_name[METHOD + 'drained']['def_id']]['body']
    aggregates = [n['Aggregate'] for n in nodes(body) if 'Aggregate' in n]
    if len(aggregates) != 1 or aggregates[0] != [
            {'Adt': [{'id': ordering['def_id'], 'generics': {
                'regions': [], 'types': [], 'const_generics': [], 'trait_refs': []},
                'builtin': None}, acquire, None]}, []]:
        raise ValueError('atomic read ordering is not the reviewed Acquire')
    if any(inventory.get(group) for group in ('globals', 'trait_decls', 'trait_impls')):
        raise ValueError('unreviewed global or trait surface')
    return len(OWNED)


def check_environment(root, tool_root):
    # Reuse the original fixed-tool/compiler/cfg checks, not a new toolchain.
    check_tools(root, tool_root)
    environment = json.loads((root / ENVIRONMENT).read_text())
    charon = tool_root / 'aeneas/charon/bin/charon'
    production = subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip()
    extraction = subprocess.check_output([charon, 'toolchain-path'], text=True).strip()
    libraries = environment['standard_library']
    if (set(libraries) != {'production', 'extraction'}
            or any(set(items) != {'alloc/src/sync.rs', 'core/src/sync/atomic.rs'}
                   for items in libraries.values())):
        raise ValueError('incomplete standard-library source registry')
    for role, sysroot in (('production', production), ('extraction', extraction)):
        library = Path(sysroot) / 'lib/rustlib/src/rust/library'
        for relative, digest in environment['standard_library'][role].items():
            if hashlib.sha256((library / relative).read_bytes()).hexdigest() != digest:
                raise ValueError('standard-library source differs: ' + role + '/' + relative)


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
        check(args.root, llbc, inventory)
        if args.tool_root is not None:
            check_environment(args.root, args.tool_root)
    except (OSError, ValueError, KeyError, TypeError, StopIteration, subprocess.SubprocessError) as error:
        print('DataLease read extraction failed: ' + str(error), file=sys.stderr)
        return 1
    print('DataLease: 2 original methods, 2 call-through roots, 3 reviewed read providers, Acquire only.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
