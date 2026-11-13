"""Retain/check complete direct-call AST evidence, not a parser correctness proof."""
import hashlib
import json

ROOTS = ('accepted_decode', 'accepted_encode', 'recorded_decode', 'recorded_encode')


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def simple(parts):
    if any(set(p) != {'Ident'} or p['Ident'][1] != 0 for p in parts):
        raise ValueError('unexpected direct-call declaration name')
    return '::'.join(p['Ident'][0] for p in parts)


def snapshot(raw, name, wrapper_sha256):
    if name not in ROOTS or raw.get('has_errors') is not False:
        raise ValueError('incomplete original byte wrapper LLBC')
    translated = raw['translated']
    options = translated['options']
    if (options['start_from'] != ['crate::' + name] or options['include'] != ['sandproof_protocol']
            or options['opaque'] or options['exclude'] or options['monomorphize']
            or options['dest_file'] != '../../../../artifacts/bytes-' + name + '.llbc'
            or options['skip_borrowck'] or options['no_typecheck']):
        raise ValueError('unreviewed byte binding options')
    if {t['key'] for t in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('wrong byte binding target')
    functions = {d['def_id']: d for d in translated['fun_decls'] if d}
    roots = [d for d in functions.values() if d['item_meta']['name'] == [
        {'Ident': ['sandproof_envelope_bytes_check', 0]}, {'Ident': [name, 0]}]]
    if len(roots) != 1:
        raise ValueError('direct original wrapper missing/duplicated')
    root = roots[0]
    if (root['item_meta']['opacity'] != 'Transparent' or not root['item_meta']['is_local']
            or root['signature']['is_unsafe'] or root['signature']['is_variadic']
            or root['signature']['abi'] != 'Rust' or set(root['body']) != {'Structured'}):
        raise ValueError('wrapper body is not the original transparent Rust')
    structured = root['body']['Structured']
    kinds = [s['kind'] for s in structured['body']['statements']]
    tags = [next(iter(k)) if isinstance(k, dict) else k for k in kinds]
    if structured['locals']['arg_count'] != 1 or tags != [
            'StorageLive', 'StorageLive', 'Assign', 'Call', 'StorageDead', 'StorageDead', 'Return']:
        raise ValueError('wrapper is not one direct call')
    assigned, reference = kinds[2]['Assign']
    if assigned['kind'] != {'Local': 2} or set(reference) != {'Ref'}:
        raise ValueError('wrapper replaced argument')
    reference = reference['Ref']
    projection = reference['place']['kind']['Projection']
    if reference['kind'] != 'Shared' or projection[0]['kind'] != {'Local': 1} or projection[1] != 'Deref':
        raise ValueError('wrapper does not borrow original complete input')
    call = kinds[3]['Call']['call']
    if len(call['args']) != 1 or call['args'][0]['Move']['kind'] != {'Local': 2} or call['dest']['kind'] != {'Local': 0}:
        raise ValueError('original argument/result forwarding changed')
    regular = call['func']['Regular']
    callee = functions[regular['kind']['Fun']]
    expected = 'serde_json::' + ('de::from_slice' if name.endswith('decode') else 'ser::to_vec')
    if simple(callee['item_meta']['name']) != expected:
        raise ValueError('wrong original JSON callee')
    generics = regular['generics']
    if len(generics['types']) != 1 or len(generics['trait_refs']) != 1:
        raise ValueError('original type/derive dictionary missing')
    # Charon serializes each interning category independently; inspect its actual
    # tagged payload rather than confusing span/type/trait IDs with each other.
    types, traits = {}, {}
    def visit(node):
        if isinstance(node, dict):
            if (set(node) == {'Value'} and isinstance(node['Value'], list)
                    and len(node['Value']) == 2 and isinstance(node['Value'][0], int)):
                index, value = node['Value']
                if isinstance(value, dict) and set(value) == {'Adt'}:
                    types[index] = value
                if isinstance(value, dict) and set(value) == {'kind', 'trait_decl_ref'}:
                    traits[index] = value
            for value in node.values(): visit(value)
        elif isinstance(node, list):
            for value in node: visit(value)
    visit(raw)
    def resolve(value, table):
        if set(value) == {'Value'}: return value['Value'][1]
        if set(value) == {'Deduplicated'}: return table[value['Deduplicated']]
        raise ValueError('unreviewed Charon interned reference')
    actual_type = resolve(generics['types'][0], types)
    actual_trait = resolve(generics['trait_refs'][0], traits)
    type_decl = next(d for d in translated['type_decls'] if d and d['def_id'] == actual_type['Adt']['id'])
    envelope = 'AcceptedLifecycleIntent' if name.startswith('accepted') else 'RecordedLifecycleIntent'
    if simple(type_decl['item_meta']['name']) != 'sandproof_protocol::lifecycle::authority::' + envelope:
        raise ValueError('wrong complete envelope type')
    impl_id = actual_trait['kind']['TraitImpl']['id']
    impl = next(d for d in translated['trait_impls'] if d and d['def_id'] == impl_id)
    if impl['item_meta']['name'][:3] != [{'Ident':[p,0]} for p in ['sandproof_protocol','lifecycle','authority']]:
        raise ValueError('derive dictionary is not from original production authority')
    span = root['item_meta']['span']['Untagged']
    contents = translated['files'][span['data']['file_id']]['contents']
    if hashlib.sha256(contents.encode()).hexdigest() != wrapper_sha256:
        raise ValueError('embedded full wrapper source changed')
    return dict(root=root, callee=callee, type_decl=type_decl, trait_impl=impl,
                original_type=actual_type, original_dictionary=actual_trait, options=options)
