"""Audit the fixed Redis candidate's complete AST, models and Drop domain.

This is source/coverage admission, not a proof of the libraries or an
interpreter. The ordinary Aeneas pure backend erases these reviewed Drop sites;
normal-return theorems do not describe panic/unwind, abort or physical close.
"""
import argparse
import copy
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

CRATE = 'sandproof_storage_extract'
OWNERS = {CRATE, 'sandproof_protocol', 'sandproof_center_store'}
ROOTS = {CRATE + '::finish', CRATE + '::commit', CRATE + '::reaffirm'}
REGISTRY = 'verification/experiments/redis-result/environment.json'


def hashed(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def ids(function):
    return [part['Ident'][0] for part in function['item_meta']['name'] if 'Ident' in part]


def nodes(value, path=(), unwind=False):
    if isinstance(value, dict):
        yield value, path, unwind
        for key, child in value.items():
            yield from nodes(child, path + (key,), unwind or key == 'on_unwind')
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from nodes(child, path + (index,), unwind)


def drop_inventory(llbc):
    translated = llbc['translated']
    functions = {f['def_id']: f for f in translated['fun_decls'] if f is not None}
    destruct = [t['def_id'] for t in translated['trait_decls'] if t is not None
                and ids(t) == ['core', 'marker', 'Destruct']]
    if len(destruct) != 1:
        raise ValueError('original Destruct trait is missing')
    type_variables = {}
    for node, _, _ in nodes(translated):
        value = node.get('Value')
        if (isinstance(value, list) and len(value) == 2 and isinstance(value[1], dict)
                and set(value[1]) == {'TypeVar'}):
            if value[0] in type_variables and type_variables[value[0]] != value[1]['TypeVar']:
                raise ValueError('conflicting type-variable serialization')
            type_variables[value[0]] = value[1]['TypeVar']
    sites = []
    for function in functions.values():
        for node, path, unwind in nodes(function['body']):
            if 'Drop' not in node:
                continue
            drop = node['Drop']
            kind = drop['fn_ptr']['kind']
            if set(kind) == {'Fun'}:
                target = functions[kind['Fun']]
                src = target['src']
                if (not isinstance(src, dict) or 'TraitImpl' not in src
                        or src['TraitImpl']['trait_ref']['id'] != destruct[0]):
                    raise ValueError('unreviewed destructor target')
                name = '::'.join(ids(target))
                if name == 'core::option::Option':
                    if ids(function) != [CRATE, 'store', 'fence_unknown']:
                        raise ValueError('connection Drop outside the reviewed fence assignment')
                    category = 'A6/B4: original non-PubSub TCP connection disposal; no physical-close claim'
                elif name in {
                    'alloc::string::String', 'alloc::vec::Vec', 'core::result::Result',
                    'core::ops::control_flow::ControlFlow', 'alloc::collections::btree::map::BTreeMap',
                    'sandproof_protocol::DeploymentId', CRATE + '::store::DecodedWrite',
                    CRATE + '::store::Observed', CRATE + '::store::Committed', CRATE + '::store::storage_io::Cas',
                }:
                    category = 'plain value/container, fixed String/str/byte dictionaries and plain T'
                else:
                    raise ValueError('unreviewed resource Drop: ' + name)
            elif set(kind) == {'Trait'}:
                ty = drop['place']['ty']
                variable = type_variables.get(ty.get('Deduplicated'))
                if 'Value' in ty:
                    variable = ty['Value'][1].get('TypeVar')
                if variable != {'Free': 0}:
                    raise ValueError('generic Drop is no longer the supported plain payload T')
                name = 'core::marker::Destruct on plain payload T'
                category = 'caller must instantiate T without observable Drop/shared mutable resources'
            else:
                raise ValueError('unreviewed destructor invocation shape')
            sites.append({'function': function['def_id'], 'path': list(path), 'unwind': unwind,
                          'target': name, 'category': category, 'drop_sha256': hashed(drop)})
    return sites


def drop_summary(sites):
    # Keep a reviewable owner/type/unwind summary while hashing every complete
    # individual site (including its ordered path and nested unwind node).
    groups = {}
    for site in sites:
        key = (site['function'], site['target'], site['category'], site['unwind'])
        groups.setdefault(key, []).append(site)
    return {'count': len(sites), 'all_sites_sha256': hashed(sites), 'groups': [
        {'function': key[0], 'target': key[1], 'category': key[2], 'unwind': key[3],
         'count': len(items), 'sites_sha256': hashed(items)} for key, items in groups.items()]}


def check_audit_surface(root):
    directory = root / 'verification/aeneas/storage/SandproofStorage'
    expected = set()
    for file in directory.glob('*.lean'):
        text = file.read_text()
        declarations = re.findall(r'^theorem ([A-Za-z0-9_]+)', text, re.M)
        if declarations:
            spaces = re.findall(r'^namespace ([A-Za-z0-9_.]+)$', text, re.M)
            if len(spaces) != 1:
                raise ValueError('unreviewed theorem namespace in ' + file.name)
            expected.update(spaces[0] + '.' + name for name in declarations)
    text = (directory / 'Audit.lean').read_text()
    actual = re.findall(r'^#audit_controller_proof ([A-Za-z0-9_.]+)$', text, re.M)
    if not expected or len(actual) != len(set(actual)) or set(actual) != expected:
        raise ValueError('missing, duplicate or extra Redis theorem audit')
    extension = '\n[[lean_lib]]\nname = "SandproofStorage"\nsrcDir = "verification/aeneas/storage"\n'
    if (root / 'lake-storage.toml').read_text().rstrip() != (
            (root / 'lake-first-chain.toml').read_text().rstrip() + '\n' + extension).rstrip():
        raise ValueError('storage Lake view must preserve the complete base/Linux configuration')
    return expected


def proof_sources_digest(root):
    fingerprint = hashlib.sha256()
    sources = sorted((root / 'SandboxModel').rglob('*.lean'))
    sources += sorted((root / 'verification/aeneas').rglob('*.lean'))
    for source in sources:
        name = str(source.relative_to(root)).encode()
        fingerprint.update(len(name).to_bytes(8, 'big'))
        fingerprint.update(name)
        fingerprint.update(hashlib.sha256(source.read_bytes()).digest())
    return fingerprint.hexdigest()


def check_service_record(root, record):
    expected = check_audit_surface(root)
    name = 'verification/aeneas/storage/SandproofStorage/Audit.lean'
    digest = hashlib.sha256((root / name).read_bytes()).hexdigest()
    request = record['request']
    if (request.get('tool') != 'lean_diagnostic_messages'
            or request.get('arguments') != {'file_path': name}
            or request.get('expected_sha256') != digest):
        raise ValueError('unfiltered physical Redis Audit request with exact source pin is required')
    envelope = record['result']
    if set(envelope) != {'result'}:
        raise ValueError('actual Redis file diagnostics returned an error or unreviewed envelope')
    result = envelope['result']
    if (record.get('stale') is not False or result.get('success') is not True
            or result.get('timed_out') is not False or result.get('partial') is not False
            or result.get('still_elaborating_lines') not in (None, [])
            or result.get('failed_dependencies') not in (None, [])
            or record['revision']['file_sha256'] != digest
            or record['revision']['sources_sha256'] != proof_sources_digest(root)):
        raise ValueError('actual Redis Audit service result was incomplete, stale or unsuccessful')
    diagnostics = result['items']
    if any(d['severity'] in {'error', 'warning'} or 'unsolvedGoals' in (d.get('lean_tags') or [])
           for d in diagnostics):
        raise ValueError('Redis Audit service returned errors, warnings or unsolved goals')
    checked = [re.match(r'^Checked ([A-Za-z0-9_.]+):', d['message']) for d in diagnostics]
    names = [m.group(1) for m in checked if m]
    if len(names) != len(expected) or set(names) != expected:
        raise ValueError('actual Redis service dependency audit omitted a theorem')
    return len(expected)


def check_service_mutations(root, record):
    changes = [
        lambda r: r.update(stale=True),
        lambda r: r['revision'].update(file_sha256='changed'),
        lambda r: r['revision'].update(sources_sha256='changed'),
        lambda r: r['request'].update(expected_sha256='changed'),
        lambda r: r['request'].update(tool='lean_run_code'),
        lambda r: r['request']['arguments'].update(severity='info'),
        lambda r: r['request']['arguments'].update(file_path='another.lean'),
        lambda r: r['result'].update(isError=True),
        lambda r: r['result']['result'].update(success=False),
        lambda r: r['result']['result'].update(partial=True),
        lambda r: r['result']['result'].update(timed_out=True),
        lambda r: r['result']['result'].update(still_elaborating_lines=[[1, 135]]),
        lambda r: r['result']['result'].update(failed_dependencies=['missing.lean']),
        lambda r: r['result']['result']['items'][0].update(severity='warning'),
        lambda r: r['result']['result']['items'][0].update(lean_tags=['unsolvedGoals']),
        lambda r: r['result']['result']['items'].pop(),
        lambda r: r['result']['result']['items'].append(copy.deepcopy(r['result']['result']['items'][0])),
        lambda r: r['result']['result']['items'][0].update(message='Checked Unregistered.theorem: 0 standard logical dependencies'),
    ]
    for change in changes:
        mutant = copy.deepcopy(record)
        change(mutant)
        try:
            check_service_record(root, mutant)
        except (ValueError, KeyError, TypeError, IndexError):
            continue
        raise ValueError('incomplete, filtered, stale or false Redis service receipt accepted')
    return len(changes)


def check_service(root, path, mutations=False):
    records = []
    for line in path.read_text().splitlines():
        if line.startswith('{'):
            record = json.loads(line)
            request = record.get('request', {})
            if request.get('arguments', {}).get('file_path') == 'verification/aeneas/storage/SandproofStorage/Audit.lean':
                records.append(record)
    if len(records) != 1:
        raise ValueError('one actual physical Redis Audit service result is required')
    count = check_service_record(root, records[0])
    if mutations:
        rejected = check_service_mutations(root, records[0])
        print(f'Redis service admission rejected {rejected} invalid receipt mutations.')
    return count


def check(root, llbc, inventory):
    check_audit_surface(root)
    registry = json.loads((root / REGISTRY).read_text())
    profile = json.loads((root / 'verification/experiments/redis-result/candidate.json').read_text())
    if llbc.get('has_errors') is not False or inventory.get('crate') != CRATE:
        raise ValueError('wrong crate or Charon errors')
    translated = llbc['translated']
    if translated['options'] != registry['charon_options']:
        raise ValueError('unreviewed core extraction options')
    if {item['key'] for item in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('unreviewed target')
    functions = inventory['functions']
    if {f['rust_name'] for f in functions if f['is_opaque']} != set(registry['opaque_functions']):
        raise ValueError('unreviewed opaque function surface')
    if not ROOTS <= {f['rust_name'] for f in functions if not f['is_opaque']}:
        raise ValueError('real caller root missing')
    if len({f['lean_name'] for f in functions}) != len(functions):
        raise ValueError('duplicate generated function name')
    actual = {f['def_id']: f for f in translated['fun_decls'] if f is not None}
    generated = (root / 'verification/aeneas/storage/SandproofStorage/Funs.lean').read_text()
    external = (root / 'verification/aeneas/storage/SandproofStorage/FunsExternal.lean').read_text()
    covered = set()
    for item in functions:
        f = actual[item['def_id']]
        covered.add(f['def_id'])
        if item['is_opaque']:
            expected_opacity = 'Opaque' if item['rust_name'].startswith(CRATE + '::') else 'Foreign'
            if f['body'] != 'Opaque' or f['item_meta']['opacity'] != expected_opacity:
                raise ValueError('opaque inventory differs from actual AST')
            lean_name = item['lean_name'].removeprefix('SandproofStorageRust.')
            if not re.search(r'\bdef\s+' + re.escape(lean_name) + r'(?![\w.])', external):
                raise ValueError('opaque call has no registered model: ' + item['rust_name'])
            if item['rust_name'].startswith(CRATE + '::') and item['rust_name'] not in registry['owned_opaque']:
                raise ValueError('own business body was hidden')
        else:
            if not isinstance(f['body'], dict) or set(f['body']) != {'Structured'}:
                raise ValueError('transparent body missing: ' + item['rust_name'])
            lean_name = item['lean_name'].removeprefix('SandproofStorageRust.')
            if not re.search(r'\bdef\s+' + re.escape(lean_name) + r'(?![\w.])', generated):
                raise ValueError('actual generated definition missing')
    destruct = next(t['def_id'] for t in translated['trait_decls'] if t and ids(t) == ['core', 'marker', 'Destruct'])
    skipped = []
    for f in actual.values():
        if f['def_id'] in covered:
            continue
        src = f['src']
        is_drop = isinstance(src, dict) and src.get('TraitImpl', {}).get('trait_ref', {}).get('id') == destruct
        is_init = isinstance(src, dict) and 'GlobalInitializer' in src
        if ids(f)[0] in OWNERS and not is_drop and not is_init:
            raise ValueError('own function missing from Aeneas inventory')
        skipped.append({'def_id': f['def_id'], 'name': f['item_meta']['name'], 'src': src,
                        'body_sha256': hashed(f['body']),
                        'class': 'reviewed Drop' if is_drop else ('global initializer' if is_init else 'original Aeneas builtin')})
    if skipped != registry['builtin_or_drop_declarations']:
        raise ValueError('unreviewed skipped/builtin/global/Drop declaration')
    if drop_summary(drop_inventory(llbc)) != registry['drop_review']:
        raise ValueError('Drop positions, kinds, types or unwind paths changed')
    if inventory['types'] != registry['types'] or inventory['globals'] != registry['globals']:
        raise ValueError('type or constant inventory changed')
    if (inventory['trait_decls'] != registry['trait_decls']
            or inventory['trait_impls'] != registry['trait_impls']):
        raise ValueError('own callback or concrete trait dictionary changed')
    if hashlib.sha256((root / 'sandbox-center-store/src/store.rs').read_bytes()).hexdigest() != profile['original_store_sha256']:
        raise ValueError('original production store changed; isolated candidate is not a new production proof')
    for name, expected in profile['base_files'].items():
        if hashlib.sha256((root / name).read_bytes()).hexdigest() != expected:
            raise ValueError('original production dependency source changed: ' + name)
    if llbc_digest(llbc) != profile['core_llbc_digest']:
        raise ValueError('complete core LLBC changed')
    return len(functions), registry['drop_review']['count']


def check_libraries(root, tool_root):
    registry = json.loads((root / REGISTRY).read_text())
    backend = root / '.lake/packages/aeneas'
    dirty = subprocess.check_output(['git', '-C', str(backend), 'status', '--porcelain', '--untracked-files=no'], text=True)
    if dirty:
        raise ValueError('registered Lean backend has tracked source changes')
    charon = tool_root / 'aeneas/charon/bin/charon'
    production = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    extraction = Path(subprocess.check_output([str(charon), 'toolchain-path'], text=True).strip())
    for role, base in [('production', production), ('extraction', extraction)]:
        for name, expected in registry['library_sources'][role].items():
            path = base / 'lib/rustlib/src/rust/library' / name
            if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
                raise ValueError('fixed standard-library source changed: ' + role + '/' + name)
    cargo_home = Path(__import__('os').environ['CARGO_HOME'])
    for name, expected in registry['registry_sources'].items():
        found = list((cargo_home / 'registry/src').glob('*/' + name))
        if len(found) != 1 or hashlib.sha256(found[0].read_bytes()).hexdigest() != expected:
            raise ValueError('fixed Redis/codec dependency source changed: ' + name)


def check_mutations(root, llbc, inventory):
    cases = []
    bad = copy.deepcopy(inventory)
    bad['functions'] = [f for f in bad['functions'] if f['rust_name'] != CRATE + '::finish']
    cases.append((llbc, bad))
    bad = copy.deepcopy(llbc)
    function = next(f for f in bad['translated']['fun_decls'] if f and ids(f) == [CRATE, 'store', 'fence_unknown'])
    function['body'] = 'Opaque'
    cases.append((bad, inventory))
    bad = copy.deepcopy(inventory)
    next(f for f in bad['functions'] if f['is_opaque'])['rust_name'] = 'unreviewed::external'
    cases.append((llbc, bad))
    bad = copy.deepcopy(llbc)
    function = next(f for f in bad['translated']['fun_decls'] if f and ids(f) == [CRATE, 'store', 'fence_unknown'])
    next(n for n, _, _ in nodes(function['body']) if 'Drop' in n)['Drop']['kind'] = 'Unconditional'
    cases.append((bad, inventory))
    bad = copy.deepcopy(inventory)
    bad['types'][0]['rust_name'] = 'unreviewed::resource'
    cases.append((llbc, bad))
    for mutant_llbc, mutant_inventory in cases:
        try:
            check(root, mutant_llbc, mutant_inventory)
        except (ValueError, KeyError, TypeError, IndexError):
            continue
        raise ValueError('missing root/owned body/external/Drop/type mutation accepted')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--check-mutations', action='store_true')
    parser.add_argument('--service-log', type=Path)
    args = parser.parse_args()
    try:
        llbc = json.loads(args.llbc.read_text())
        inventory = json.loads((args.root / 'verification/aeneas/storage/translation.json').read_text())
        counts = check(args.root, llbc, inventory)
        if args.tool_root is not None:
            check_libraries(args.root, args.tool_root)
        if args.check_mutations:
            check_mutations(args.root, llbc, inventory)
        if args.service_log is not None:
            check_service(args.root, args.service_log, args.check_mutations)
    except (OSError, ValueError, KeyError, TypeError, IndexError, subprocess.SubprocessError) as error:
        print('Redis extraction admission failed: ' + str(error), file=sys.stderr)
        return 1
    print(f'Redis candidate: {counts[0]} generated functions, {counts[1]} individually frozen Drop sites; reviewed value/connection domain only.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
