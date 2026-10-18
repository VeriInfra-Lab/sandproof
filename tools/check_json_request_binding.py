"""Check the explicitly trusted JSON call binding; this is not a Rust proof."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

REGISTRY = 'verification/json-request-binding.json'
TRUSTED_ROOTS = {'json_request::decode', 'json_request::encode'}
DIRECT_SOURCES = {
    'decode': 'pub fn decode(bytes: &[u8]) -> Result<LifecycleRequest, serde_json::Error> {\n'
              '    serde_json::from_slice::<LifecycleRequest>(bytes)\n}',
    'encode': 'pub fn encode(request: &LifecycleRequest) -> Result<Vec<u8>, serde_json::Error> {\n'
              '    serde_json::to_vec(request)\n}',
}
LIBRARY_FILES = {
    'serde-1.0.228/src/private/de.rs',
    'serde_derive-1.0.228/src/de/enum_internally.rs',
    'serde_derive-1.0.228/src/de/struct_.rs',
    'serde_derive-1.0.228/src/ser.rs',
    'serde_json-1.0.142/src/de.rs',
    'serde_json-1.0.142/src/ser.rs',
    'serde_json-1.0.142/src/read.rs',
    'serde_json-1.0.142/src/error.rs',
    'serde_core-1.0.228/src/de/impls.rs',
    'serde_core-1.0.228/src/ser/impls.rs',
}
PRODUCTION_FILES = {
    'sandbox-protocol/src/lib.rs', 'sandbox-protocol/src/internal.rs',
    'sandbox-protocol/src/internal/lifecycle.rs',
    'sandbox-internal/src/client.rs', 'sandbox-internal/src/server.rs',
}


def name_of(declaration):
    return declaration['item_meta']['name']


def simple_name(parts):
    if any(set(part) != {'Ident'} or part['Ident'][1] != 0 for part in parts):
        raise ValueError('unreviewed binding name')
    return '::'.join(part['Ident'][0] for part in parts)


def snapshot(llbc, kind):
    """Keep the complete wrapper AST and the selected original callee metadata."""
    if kind not in {'decode', 'encode'} or llbc.get('has_errors') is not False:
        raise ValueError('invalid binding root or Charon errors')
    translated = llbc['translated']
    options = translated['options']
    if (options['start_from'] != ['crate::json_request::' + kind]
            or options['include'] != ['sandproof_protocol']
            or options['opaque'] or options['exclude'] or options['monomorphize']
            or options['dest_file'] != '../../artifacts/json-request-binding/' + kind + '.llbc'):
        raise ValueError('unreviewed binding extraction options')
    if {item['key'] for item in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('unreviewed binding target')
    functions = {item['def_id']: item for item in translated['fun_decls'] if item is not None}
    candidates = [item for item in functions.values() if name_of(item) == [
        {'Ident': [part, 0]} for part in ('sandproof_foundations_extract', 'json_request', kind)]]
    if len(candidates) != 1:
        raise ValueError('binding root missing or duplicated')
    root = candidates[0]
    if (root['item_meta']['opacity'] != 'Transparent' or not root['item_meta']['is_local']
            or root['signature']['is_unsafe'] or root['signature']['is_variadic']
            or root['signature']['abi'] != 'Rust'
            or not isinstance(root['body'], dict) or set(root['body']) != {'Structured'}):
        raise ValueError('real transparent binding body missing')
    structured = root['body']['Structured']
    if structured['locals']['arg_count'] != 1:
        raise ValueError('binding must forward its one complete argument')
    statements = structured['body']['statements']
    kinds = [statement['kind'] for statement in statements]
    tags = [next(iter(item)) if isinstance(item, dict) else item for item in kinds]
    if tags != ['StorageLive', 'StorageLive', 'Assign', 'Call', 'StorageDead', 'StorageDead', 'Return']:
        raise ValueError('binding is no longer a single direct call')
    assigned, reference = kinds[2]['Assign']
    if assigned['kind'] != {'Local': 2} or set(reference) != {'Ref'}:
        raise ValueError('binding changed forwarded argument')
    reference = reference['Ref']
    if (reference['kind'] != 'Shared'
            or reference['place']['kind']['Projection'][0]['kind'] != {'Local': 1}
            or reference['place']['kind']['Projection'][1] != 'Deref'):
        raise ValueError('binding must reborrow the original argument')
    call_node = kinds[3]['Call']
    call = call_node['call']
    if (call['args'][0]['Move']['kind'] != {'Local': 2} or len(call['args']) != 1
            or call['dest']['kind'] != {'Local': 0}):
        raise ValueError('binding changed argument or result forwarding')
    regular = call['func']['Regular']
    callee = functions[regular['kind']['Fun']]
    expected = 'serde_json::' + ('de::from_slice' if kind == 'decode' else 'ser::to_vec')
    if simple_name(name_of(callee)) != expected:
        raise ValueError('binding changed the original JSON target')
    if len(regular['generics']['types']) != 1 or len(regular['generics']['trait_refs']) != 1:
        raise ValueError('binding type or derive dictionary missing')
    # The entire body, signature, concrete type references, dictionary and unwind
    # path are kept, rather than hashing only the call's display name.
    return {'root': root, 'callee': callee, 'options': options}


def check_registry(root, registry=None):
    registry = registry if registry is not None else json.loads((root / REGISTRY).read_text())
    if (registry.get('schema') != 1 or registry.get('contracts') != ['A6-json-lifecycle-derive', 'B4-json-lifecycle-call']
            or set(registry['entries']) != TRUSTED_ROOTS):
        raise ValueError('unreviewed trusted binding surface')
    if set(registry['production_sources']) != PRODUCTION_FILES or set(registry['library_sources']) != LIBRARY_FILES:
        raise ValueError('incomplete binding source inventory')
    for relative, expected in registry['production_sources'].items():
        if hashlib.sha256((root / relative).read_bytes()).hexdigest() != expected:
            raise ValueError('production binding source changed: ' + relative)
    for expected in registry['library_sources'].values():
        if not isinstance(expected, str) or not re.fullmatch('[0-9a-f]{64}', expected):
            raise ValueError('malformed fixed library source hash')
    internal = (root / 'sandbox-protocol/src/internal.rs').read_text()
    lifecycle = (root / 'sandbox-protocol/src/internal/lifecycle.rs').read_text()
    expected_structs = {'NodeIdentity', 'ControlIdentity', 'Resources', 'CreateBinding', 'LifecycleRequest'}
    if set(registry['structs']) != expected_structs:
        raise ValueError('complete schema type missing')
    for name, schema in registry['structs'].items():
        text = internal if name in {'NodeIdentity', 'ControlIdentity'} else lifecycle
        match = re.search(r'#\[serde\(deny_unknown_fields\)\]\s*pub struct ' + name + r'\s*\{([^}]+)\}', text)
        if match is None:
            raise ValueError('original standard struct or attributes changed: ' + name)
        body = re.sub(r'//[^\n]*', '', match[1])
        fields = [[field, re.sub(r'\s+', '', ty)] for field, ty in
                  re.findall(r'pub\s+(\w+)\s*:\s*([^,\n]+)', body)]
        if fields != schema['fields_in_declaration_order']:
            raise ValueError('schema fields/types/order disagree with real Rust: ' + name)
        if {key: value for key, value in schema.items() if key != 'fields_in_declaration_order'} != {
            'object_unknown_fields': 'reject', 'duplicates': 'reject', 'missing': 'reject',
            'input_shapes': ['object', 'sequence in declaration order'],
            'output_shape': 'object in declaration order',
        }:
            raise ValueError('unreviewed standard derive policy: ' + name)
    if registry['leaf_bindings']['strings'] != {
        'identity.node.deployment_id': 'DeploymentId', 'identity.node.node_id': 'NodeId',
        'identity.tenant_id': 'TenantId', 'identity.sandbox_id': 'SandboxId',
        'identity.operation_id': 'OperationId', 'create.template_revision': 'ResourceId',
    } or registry['leaf_bindings']['counters'] != [
        'identity.node.node_generation', 'identity.intent_version', 'identity.request_window',
    ]:
        raise ValueError('custom field path or original type missing')
    entry = (root / 'verification/extract-foundations/src/json_request.rs').read_text().split('#[cfg(test)]', 1)[0]
    if not re.search(r'^use sandproof_protocol::internal::lifecycle::LifecycleRequest;$', entry, re.M):
        raise ValueError('original concrete lifecycle type import changed')
    for kind in ('decode', 'encode'):
        registered = registry['entries']['json_request::' + kind]
        if (registered['trust'] != 'B4: direct call binding only; not machine refinement'
                or registered['snapshot']['root']['item_meta']['source_text'] != DIRECT_SOURCES[kind]
                or entry.count(DIRECT_SOURCES[kind]) != 1):
            raise ValueError('verification root no longer calls the real API directly')
    if registry['drop_domain'] != {
        'owned': ['String', 'Vec<u8>', 'serde_json::Error from SliceRead/Vec without custom IO error source'],
        'borrowed': ['&[u8]', '&LifecycleRequest', '&mut serde_json::Deserializer<SliceRead>'],
        'excluded': ['custom Drop', 'FD/permit/lock ownership', 'panic/unwind cleanup', 'OOM/abort', 'async cancellation'],
    }:
        raise ValueError('unreviewed Drop scope')
    contract = registry['formal_contract']
    if (contract['operations'] != 'SandboxModel.Boundaries.JsonRequest.LifecycleJsonLibrary'
            or contract['condition'] != 'SandboxModel.Boundaries.JsonRequest.SliceDeriveContract'
            or contract['encoding_bound'] != {
                'fixed_schema_allowance': 2048, 'escape_per_utf8_byte': 6,
                'options_per_byte': 4, 'body_bytes': 270336,
            }):
        raise ValueError('unreviewed concrete byte contract or size bound')
    model = (root / 'SandboxModel/Boundaries/JsonRequest/SliceBinding.lean').read_text()
    if contract['own_theorems'] != re.findall(r'^theorem ([A-Za-z0-9_]+)', model, re.M):
        raise ValueError('concrete byte theorem inventory changed')
    return registry


def check_actual(root, directory, registry=None):
    registry = check_registry(root, registry)
    try:
        from tools.check_foundations_extraction import llbc_digest
    except ModuleNotFoundError:
        from check_foundations_extraction import llbc_digest
    for kind in ('decode', 'encode'):
        llbc = json.loads((directory / (kind + '.llbc')).read_text())
        actual = snapshot(llbc, kind)
        registered = registry['entries']['json_request::' + kind]
        if actual != registered['snapshot'] or llbc_digest(llbc) != registered['full_llbc_sha256']:
            raise ValueError('binding AST/type/dictionary/cleanup changed: ' + kind)
    return 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--llbc-dir', type=Path)
    parser.add_argument('--library-root', type=Path)
    args = parser.parse_args()
    try:
        registry = check_registry(args.root)
        if args.llbc_dir is not None:
            check_actual(args.root, args.llbc_dir, registry)
        if args.library_root is not None:
            for relative, expected in registry['library_sources'].items():
                if hashlib.sha256((args.library_root / relative).read_bytes()).hexdigest() != expected:
                    raise ValueError('fixed JSON library source changed: ' + relative)
    except (OSError, ValueError, KeyError, TypeError, IndexError) as error:
        print('JSON request binding failed: ' + str(error), file=sys.stderr)
        return 1
    print('Two JSON direct-call bindings admitted under explicit A6/B4 conditions; not a Rust or library proof.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
