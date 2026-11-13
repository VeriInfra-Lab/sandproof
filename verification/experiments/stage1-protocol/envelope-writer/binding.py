"""Narrow original call/type checks; these support B4, not Serde correctness."""
import hashlib
import json


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def simple(parts):
    if any(set(p) != {'Ident'} or p['Ident'][1] != 0 for p in parts):
        raise ValueError('expected original direct name')
    return '::'.join(p['Ident'][0] for p in parts)


def interned(raw):
    types, traits = {}, {}
    def visit(node):
        if isinstance(node, dict):
            if (set(node) == {'Value'} and isinstance(node['Value'], list)
                    and len(node['Value']) == 2 and isinstance(node['Value'][0], int)):
                index, value = node['Value']
                if isinstance(value, dict) and set(value) in ({'Adt'}, {'Scalar'}):
                    types[index] = value
                if isinstance(value, dict) and set(value) == {'kind', 'trait_decl_ref'}:
                    traits[index] = value
            for value in node.values(): visit(value)
        elif isinstance(node, list):
            for value in node: visit(value)
    visit(raw)
    return types, traits


def resolve(value, table):
    if set(value) == {'Value'}: return value['Value'][1]
    if set(value) == {'Deduplicated'}: return table[value['Deduplicated']]
    raise ValueError('unknown interned reference')


def contract_error(raw):
    types, _ = interned(raw)
    name = [{'Ident': [n, 0]} for n in ['sandproof_protocol', 'ContractError']]
    matches = [d for d in raw['translated']['type_decls'] if d and d['item_meta']['name'] == name]
    if len(matches) != 1 or matches[0]['item_meta']['opacity'] != 'Transparent':
        raise ValueError('original transparent ContractError required')
    declaration = matches[0]
    shape = []
    for variant in declaration['kind']['Enum']:
        fields = []
        for field in variant['fields']:
            ty = resolve(field['ty'], types)
            if ty != {'Scalar': {'Integer': {'Unsigned': 'U16'}}}:
                raise ValueError('unreviewed ContractError field')
            fields.append(dict(name=field['name'], positional=field['is_positional'],
                               attributes=field['attr_info'], type=ty))
        shape.append(dict(id=variant['id'], name=variant['name'], attributes=variant['attr_info'],
                          fields=fields, discriminant=variant['discriminant']))
    return dict(shape=shape, generics=declaration['generics'], layout=declaration['layout'])


def snapshot(raw, wrapper_sha256):
    if raw.get('has_errors') is not False:
        raise ValueError('incomplete original bounded calls')
    translated = raw['translated']
    options = translated['options']
    if (options['start_from'] != ['crate::accepted_bounded', 'crate::recorded_bounded']
            or options['include'] != ['sandproof_controller_service', 'sandproof_internal', 'sandproof_protocol']
            or options['opaque'] or options['exclude'] or options['monomorphize']
            or options['skip_borrowck'] or options['no_typecheck']):
        raise ValueError('unreviewed original bounded-call options')
    if {t['key'] for t in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('wrong original target')
    functions = {d['def_id']: d for d in translated['fun_decls'] if d}
    types, traits = interned(raw)
    records = {}
    for name in ['accepted_bounded', 'recorded_bounded']:
        declarations = [d for d in functions.values() if d['item_meta']['name'] == [
            {'Ident': ['sandproof_envelope_writer_check', 0]}, {'Ident': [name, 0]}]]
        if len(declarations) != 1: raise ValueError('missing or duplicate bounded root')
        root = declarations[0]
        if (root['item_meta']['opacity'] != 'Transparent' or not root['item_meta']['is_local']
                or root['signature']['is_unsafe'] or root['signature']['is_variadic']
                or root['signature']['abi'] != 'Rust' or set(root['body']) != {'Structured'}):
            raise ValueError('unreviewed bounded root body')
        body = root['body']['Structured']
        kinds = [s['kind'] for s in body['body']['statements']]
        tags = [next(iter(k)) if isinstance(k, dict) else k for k in kinds]
        if body['locals']['arg_count'] != 2 or tags != [
                'StorageLive', 'StorageLive', 'Assign', 'StorageLive', 'Assign', 'Call',
                'StorageDead', 'StorageDead', 'StorageDead', 'StorageDead', 'Return']:
            raise ValueError('not the original single complete-value/limit forwarding call')
        destination, reference = kinds[2]['Assign']
        reference = reference['Ref']
        projection = reference['place']['kind']['Projection']
        if (destination['kind'] != {'Local': 3} or reference['kind'] != 'Shared'
                or projection[0]['kind'] != {'Local': 1} or projection[1] != 'Deref'):
            raise ValueError('complete original input was not forwarded')
        destination, use = kinds[4]['Assign']
        if (destination['kind'] != {'Local': 4} or use['Use'][0]['Copy']['kind'] != {'Local': 2}
                or use['Use'][1] != 'Yes'):
            raise ValueError('original limit was not forwarded')
        call = kinds[5]['Call']['call']
        if ([a['Move']['kind'] for a in call['args']] != [{'Local': 3}, {'Local': 4}]
                or call['dest']['kind'] != {'Local': 0}):
            raise ValueError('original arguments/result were replaced')
        unwind = kinds[5]['Call']['on_unwind']['statements']
        if [s['kind'] for s in unwind] != [{'StorageDead': 2}, {'StorageDead': 1}, 'UnwindResume']:
            raise ValueError('unwind changed or extra business effects introduced')
        regular = call['func']['Regular']
        callee = functions[regular['kind']['Fun']]
        if simple(callee['item_meta']['name']) != 'sandproof_controller_service::template_registry::evidence::encode_bounded':
            raise ValueError('wrong original bounded encoder')
        generic = regular['generics']
        if len(generic['types']) != 1 or len(generic['trait_refs']) != 1:
            raise ValueError('missing full original type/derive dictionary')
        ty = resolve(generic['types'][0], types)
        dictionary = resolve(generic['trait_refs'][0], traits)
        declaration = next(d for d in translated['type_decls'] if d and d['def_id'] == ty['Adt']['id'])
        expected = 'AcceptedLifecycleIntent' if name.startswith('accepted') else 'RecordedLifecycleIntent'
        if simple(declaration['item_meta']['name']) != 'sandproof_protocol::lifecycle::authority::' + expected:
            raise ValueError('wrong original complete envelope type')
        impl = next(d for d in translated['trait_impls'] if d and d['def_id'] == dictionary['kind']['TraitImpl']['id'])
        if impl['item_meta']['name'][:3] != [{'Ident': [p, 0]} for p in ['sandproof_protocol', 'lifecycle', 'authority']]:
            raise ValueError('wrong production standard derive')
        source = translated['files'][root['item_meta']['span']['Untagged']['data']['file_id']]['contents']
        if hashlib.sha256(source.encode()).hexdigest() != wrapper_sha256:
            raise ValueError('embedded original wrapper source differs')
        records[name] = dict(root=root, callee=callee, type=declaration, dictionary=impl,
                             original_type=ty, original_dictionary=dictionary)
    return dict(roots=records, contract_error=contract_error(raw), options=options)
