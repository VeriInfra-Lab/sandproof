"""Check the fixed Status SliceRead/Vec binding, not an async/server Writer proof."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

try:
    from tools.check_schema_domains import ENVIRONMENT, load_environment, sha
    from tools.check_json_request_binding import LIBRARY_FILES, simple_name
    from tools.check_foundations_extraction import llbc_digest
except ModuleNotFoundError:
    from check_schema_domains import ENVIRONMENT, load_environment, sha
    from check_json_request_binding import LIBRARY_FILES, simple_name
    from check_foundations_extraction import llbc_digest

KINDS = ('request_decode', 'request_encode', 'response_decode', 'response_encode')
WRAPPER = 'verification/extract-schema-domains/src/lib.rs'
BINDING_SCOPE = 'existing-A6-fixed-SliceRead-Vec-standard-derive-and-B4-direct-call'
LEAVES = {'node.deployment_id': 'DeploymentId', 'node.node_id': 'NodeId',
          'node.node_generation': 'PositiveCounter'}
DROP_DOMAIN = {
    'owned': ['String', 'Vec<u8>', 'serde_json::Error from SliceRead/Vec without custom IO source'],
    'borrowed': ['&[u8]', '&StatusRequest', '&HostStatus', 'private SliceRead cursor/scratch'],
    'excluded': ['server encode_bounded Writer', 'stateful I/O', 'custom Drop',
                 'resource-bearing values', 'panic/unwind completion', 'OOM/abort', 'async cancellation'],
}


def direct_source(kind):
    ty = 'StatusRequest' if kind.startswith('request') else 'HostStatus'
    if kind.endswith('decode'):
        return (f'pub fn {kind}(bytes: &[u8]) -> Result<{ty}, serde_json::Error> {{\n'
                f'    serde_json::from_slice::<{ty}>(bytes)\n}}')
    value = 'request' if kind.startswith('request') else 'response'
    return (f'pub fn {kind}({value}: &{ty}) -> Result<Vec<u8>, serde_json::Error> {{\n'
            f'    serde_json::to_vec({value})\n}}')


def byte_snapshot(llbc, kind):
    """Retain full wrapper, callee, type/dictionary references and unwind path."""
    if kind not in KINDS or llbc.get('has_errors') is not False:
        raise ValueError('invalid byte root or Charon errors')
    translated = llbc['translated']
    options = translated['options']
    if (translated.get('crate_name', 'sandproof_schema_domains_extract')
            != 'sandproof_schema_domains_extract'
            or options['start_from'] != ['crate::' + kind]
            or options['include'] != ['sandproof_internal', 'sandproof_protocol']
            or options['dest_file'] != '../../artifacts/schema-domains-binding/' + kind + '.llbc'
            or options['targets'] or options['translate_all_methods']
            or not options['unbind_item_vars']
            or any(options[name] for name in ('opaque', 'exclude', 'monomorphize',
                                             'skip_borrowck', 'no_typecheck', 'no_normalize'))):
        raise ValueError('unreviewed byte extraction scope/options')
    if {item['key'] for item in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('unreviewed byte target')
    functions = {item['def_id']: item for item in translated['fun_decls'] if item}
    roots = [item for item in functions.values() if item['item_meta']['name'] == [
        {'Ident': ['sandproof_schema_domains_extract', 0]}, {'Ident': [kind, 0]}]]
    if len(roots) != 1:
        raise ValueError('missing or duplicate byte root')
    root = roots[0]
    if (not root['item_meta']['is_local'] or root['item_meta']['opacity'] != 'Transparent'
            or root['item_meta']['source_text'] != direct_source(kind)
            or root['signature']['is_unsafe'] or root['signature']['is_variadic']
            or root['signature']['abi'] != 'Rust'
            or not isinstance(root['body'], dict) or set(root['body']) != {'Structured'}):
        raise ValueError('original transparent direct byte body missing')
    body = root['body']['Structured']
    statements = body['body']['statements']
    kinds = [statement['kind'] for statement in statements]
    tags = [next(iter(item)) if isinstance(item, dict) else item for item in kinds]
    if body['locals']['arg_count'] != 1 or tags != [
            'StorageLive', 'StorageLive', 'Assign', 'Call', 'StorageDead', 'StorageDead', 'Return']:
        raise ValueError('byte root no longer a single direct call')
    assigned, reference = kinds[2]['Assign']
    if assigned['kind'] != {'Local': 2} or set(reference) != {'Ref'}:
        raise ValueError('byte argument reborrow changed')
    reference = reference['Ref']
    if (reference['kind'] != 'Shared'
            or reference['place']['kind']['Projection'][0]['kind'] != {'Local': 1}
            or reference['place']['kind']['Projection'][1] != 'Deref'):
        raise ValueError('byte call lost original argument/object')
    call_node = kinds[3]['Call']
    call = call_node['call']
    if (len(call['args']) != 1 or call['args'][0]['Move']['kind'] != {'Local': 2}
            or call['dest']['kind'] != {'Local': 0}):
        raise ValueError('byte argument/result forwarding changed')
    regular = call['func']['Regular']
    callee = functions[regular['kind']['Fun']]
    expected = 'serde_json::' + ('de::from_slice' if kind.endswith('decode') else 'ser::to_vec')
    if simple_name(callee['item_meta']['name']) != expected:
        raise ValueError('original JSON callee changed')
    if len(regular['generics']['types']) != 1 or len(regular['generics']['trait_refs']) != 1:
        raise ValueError('complete concrete type/derive dictionary missing')
    # No AST normalization here. Cleanup and argument metadata stay in snapshot;
    # the full regenerated LLBC is additionally hashed by the shared canonicalizer.
    return {'root': root, 'callee': callee, 'options': options}


def check_byte_registry(root, binding=None):
    environment = load_environment(root)
    binding = binding if binding is not None else environment['byte_binding']
    if (binding.get('schema') != 1 or binding.get('scope') != BINDING_SCOPE
            or set(binding['entries']) != set(KINDS)):
        raise ValueError('unreviewed byte binding surface')
    if sha(root / WRAPPER) != binding['wrapper_sha256']:
        raise ValueError('full direct wrapper source changed')
    source = (root / WRAPPER).read_text()
    for kind in KINDS:
        entry = binding['entries'][kind]
        if (source.count(direct_source(kind)) != 1
                or entry['snapshot']['root']['item_meta']['source_text'] != direct_source(kind)):
            raise ValueError('real byte API/concrete source changed')
        if not re.fullmatch('[0-9a-f]{64}', entry['full_llbc_sha256']):
            raise ValueError('invalid full byte LLBC digest')
    if (binding['leaf_fields'] != LEAVES or binding['drop_domain'] != DROP_DOMAIN
            or binding['budgets'] != {'request': 1024, 'response': 2048}
            or binding['encoding_bounds'] != {
                'request_fixed': 128, 'response_fixed': 512,
                'per_allowed_identifier_utf8_byte': 1, 'identifiers': 2}
            or binding['server_bounded_writer'] != 'unproved-public-V0-gap-not-a-B4-assumption'
            or binding['response_encode_role'] != 'ordinary-Vec-reference-not-production-server'):
        raise ValueError('unreviewed leaf/budget/Drop or server Writer scope')
    expected_inputs = set(environment['source_files']) | {
        'Cargo.toml', 'Cargo.lock', WRAPPER,
        'verification/extract-schema-domains/Cargo.toml',
        'verification/extract-schema-domains/Cargo.lock'}
    if set(binding['input_sources']) != expected_inputs:
        raise ValueError('incomplete byte production/input source inventory')
    expected_packages = {'serde-1.0.228', 'serde_core-1.0.228',
                         'serde_derive-1.0.228', 'serde_json-1.0.142'}
    if set(binding['library_packages']) != expected_packages:
        raise ValueError('incomplete fixed JSON package inventory')
    for path, digest in binding['input_sources'].items():
        if sha(root / path) != digest:
            raise ValueError('byte production/input source changed: ' + path)
    if set(binding['library_sources']) != LIBRARY_FILES:
        raise ValueError('incomplete fixed JSON library source inventory')
    if set(binding['configured_features']) != {'production_workspace', 'extraction'}:
        raise ValueError('incomplete configured feature scope')
    return binding


def check_byte_actual(root, directory, binding=None):
    binding = check_byte_registry(root, binding)
    for kind in KINDS:
        llbc = json.loads((directory / (kind + '.llbc')).read_text())
        snapshot = byte_snapshot(llbc, kind)
        entry = binding['entries'][kind]
        if snapshot != entry['snapshot'] or llbc_digest(llbc) != entry['full_llbc_sha256']:
            raise ValueError('complete byte AST/type/dictionary/cleanup changed: ' + kind)
        file_id = snapshot['root']['item_meta']['span']['Untagged']['data']['file_id']
        original = llbc['translated']['files'][file_id]['contents']
        if (not isinstance(original, str)
                or hashlib.sha256(original.encode()).hexdigest() != binding['wrapper_sha256']):
            raise ValueError('full original byte wrapper source missing')
        for function in llbc['translated']['fun_decls']:
            if not function:
                continue
            first = function['item_meta']['name'][0].get('Ident', [''])[0]
            if first in {'sandproof_internal', 'sandproof_protocol'}:
                if (function['item_meta']['opacity'] != 'Transparent'
                        or not isinstance(function['body'], dict)
                        or set(function['body']) != {'Structured'}):
                    raise ValueError('own byte leaf/derive body hidden')
    return len(KINDS)


def check_library(directory, binding):
    for relative, expected in binding['library_sources'].items():
        if sha(directory / relative) != expected:
            raise ValueError('fixed JSON library source changed: ' + relative)
    for package, record in binding['library_packages'].items():
        folder = directory / package
        actual = {str(path.relative_to(folder)): sha(path)
                  for path in folder.rglob('*') if path.is_file()
                  and (path.suffix in {'.rs', '.toml'} or path.name == '.cargo-checksum.json')}
        if actual != record['members']:
            raise ValueError('fixed JSON package closure changed: ' + package)


def check_features(root, binding):
    for role, manifest in [('production_workspace', 'Cargo.toml'),
                           ('extraction', 'verification/extract-schema-domains/Cargo.toml')]:
        data = json.loads(subprocess.check_output([
            'cargo', 'metadata', '--format-version', '1', '--locked', '--offline',
            '--manifest-path', str(root / manifest)], cwd=root, text=True))
        packages = {p['id']: p for p in data['packages']}
        observed = {}
        for node in data['resolve']['nodes']:
            package = packages[node['id']]
            if package['name'] in {'serde', 'serde_core', 'serde_derive', 'serde_json'}:
                if package['name'] in observed:
                    raise ValueError('duplicate JSON library package version')
                observed[package['name']] = {'version': package['version'],
                                            'features': sorted(node['features'])}
        if observed != binding['configured_features'][role]:
            raise ValueError('configured JSON features changed: ' + role)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--llbc-dir', type=Path)
    parser.add_argument('--library-root', type=Path)
    parser.add_argument('--check-features', action='store_true')
    args = parser.parse_args()
    binding = check_byte_registry(args.root)
    if args.llbc_dir:
        check_byte_actual(args.root, args.llbc_dir, binding)
    if args.library_root:
        check_library(args.library_root, binding)
    if args.check_features:
        check_features(args.root, binding)
    print('Four fixed Status SliceRead/Vec bindings admitted conditionally; server Writer remains unproved.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, IndexError, subprocess.SubprocessError) as error:
        print('Status byte binding failed: ' + str(error), file=sys.stderr)
        sys.exit(1)
