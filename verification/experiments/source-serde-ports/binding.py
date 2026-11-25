"""Exact actual thin-call B4 admission, keeping full typed dictionary and MIR roots."""

TYPE_TAGS = {'Adt','Ref','Scalar','Slice','Tuple','Array','TypeVar','TraitType','RawPtr','DynTrait','Arrow','Never','Str','Literal','FnPtr','AssociatedType'}
BASE = 'sandproof_controller_service::controller::lifecycle_inbox::source_work'
RECORD = BASE + '::SourceWorkRecord'
ANCHOR = BASE + '::SourceAnchor'
CODEC = BASE + '::codec::'
REQUIRED = BASE + '::required_option'
OPTION = ('core::option::Option', ((RECORD, ()),))
BYTE = ('u8', ())
ERROR = ('serde_json::error::Error', ())
CALLS = {'decode_source_payload': ('serde_json::de::from_slice', OPTION, 'de', True),
         'encode_source_payload': ('serde_json::ser::to_vec', OPTION, 'ser', True),
         'encode_record_json': ('serde_json::ser::to_vec', (RECORD, ()), 'ser', False),
         'encode_anchor_json': ('serde_json::ser::to_vec', (ANCHOR, ()), 'ser', False)}


def name(item):
    return '::'.join(p['Ident'][0] if 'Ident' in p else '<impl>' for p in item['item_meta']['name'])


def traverse(node):
    yield node
    if isinstance(node,dict):
        for item in node.values(): yield from traverse(item)
    elif isinstance(node,list):
        for item in node: yield from traverse(item)


def index_values(translated):
    types, traits = {}, {}
    for node in traverse(translated):
        if not isinstance(node,dict) or set(node) != {'Value'}: continue
        pair = node['Value']
        if not isinstance(pair,list) or len(pair)!=2 or not isinstance(pair[1],dict): continue
        identifier, body = pair
        table = (types if len(body)==1 and set(body) <= TYPE_TAGS else
                 traits if {'kind','trait_decl_ref'} == set(body) else None)
        if table is not None:
            if identifier in table and table[identifier]!=body:
                raise ValueError('inconsistent deduplication table')
            table[identifier] = body
    return types, traits


def resolve(table, value):
    if set(value)=={'Value'}: return value['Value'][1]
    if set(value)=={'Deduplicated'}: return table[value['Deduplicated']]
    raise ValueError('unreviewed deduplicated type or trait reference')


def describe(type_table, decls, reference):
    value = resolve(type_table, reference)
    if set(value)=={'Adt'}:
        a=value['Adt']; return name(decls[a['id']]), tuple(describe(type_table, decls,x) for x in a['generics']['types'])
    if set(value)=={'Scalar'} and value['Scalar']=={'Integer':{'Unsigned':'U8'}}: return BYTE
    if set(value)=={'Slice'}: return ('slice', (describe(type_table, decls,value['Slice'][0]),))
    if set(value)=={'Ref'}:
        _, inner, mutability = value['Ref']; return ('ref-'+mutability, (describe(type_table, decls,inner),))
    raise ValueError('unreviewed type at actual thin-call boundary: '+str(value.keys()))


def one(items,label):
    if len(items)!=1: raise ValueError('expected exactly one '+label)
    return items[0]


def check_trait(impls, traits, type_table, trait_table, decls, reference, kind, wanted):
    body = resolve(trait_table,reference)
    tagged = body['kind']
    if set(tagged)!={'TraitImpl'}: raise ValueError('the original concrete standard/derive dictionary is required')
    info=tagged['TraitImpl']; impl=impls[info['id']]
    trait=body['trait_decl_ref']['skip_binder']; label='serde_core::'+kind+'::'+('Deserialize' if kind=='de' else 'Serialize')
    if (name(traits[trait['id']])!=label or impl['impl_trait']['id']!=trait['id']
            or tuple(describe(type_table,decls,x) for x in trait['generics']['types'])!=(wanted,)):
        raise ValueError('the trait dictionary is not for the exact original concrete value type')
    if wanted==OPTION:
        if not name(impl).startswith('serde_core::'+kind+'::impls::'):
            raise ValueError('not the original standard Option dictionary')
        sub=one(info['generics']['trait_refs'],'original inner record dictionary')
        child, _=check_trait(impls,traits,type_table,trait_table,decls,sub,kind,(RECORD,()))
        return info['id'], child
    if not name(impl).startswith(BASE+'::') or info['generics']['trait_refs']:
        raise ValueError('not the original concrete source-record/anchor derived dictionary')
    if tuple(describe(type_table,decls,x) for x in impl['impl_trait']['generics']['types'])!=(wanted,):
        raise ValueError('original concrete derived dictionary type differs')
    return info['id'], None


def typed_calls(raw, selected):
    if raw['has_errors'] is not False: raise ValueError('incomplete actual Charon typed call extraction')
    translated=raw['translated']; funs={item['def_id']:item for item in translated['fun_decls'] if item is not None}
    decls={item['def_id']:item for item in translated['type_decls'] if item is not None}
    impls={item['def_id']:item for item in translated['trait_impls'] if item is not None}
    traits={item['def_id']:item for item in translated['trait_decls'] if item is not None}
    type_table, trait_table=index_values(translated)
    evidence={}
    for short in selected:
        target,wanted,kind,_=CALLS[short]
        root=one([f for f in funs.values() if name(f)==CODEC+short],'original typed wrapper '+short)
        sig=root['signature']; structured=root['body']['Structured']; statements=structured['body']['statements']; kinds=[s['kind'] for s in statements]
        tags=[next(iter(k)) if isinstance(k,dict) else k for k in kinds]
        if (root['item_meta']['opacity']!='Transparent' or root['item_meta']['is_local'] is not True or sig['is_unsafe'] or sig['is_variadic']
                or sig['abi']!='Rust' or structured['locals']['arg_count']!=1 or tags!=['StorageLive','StorageLive','Assign','Call','StorageDead','StorageDead','Return']):
            raise ValueError('actual wrapper is not the single original complete forwarding call')
        place, ref=kinds[2]['Assign']; r=ref['Ref']
        projection=r['place']['kind']['Projection']
        if (place['kind']!={'Local':2} or set(ref)!={'Ref'} or r['kind']!='Shared' or len(projection)!=2
                or projection[0]['kind']!={'Local':1} or projection[1]!='Deref'):
            raise ValueError('actual wrapper does not reborrow the full original input')
        call_node=kinds[3]['Call']; call=call_node['call']; regular=call['func']['Regular']; callee=funs[regular['kind']['Fun']]
        if (name(callee)!=target or len(call['args'])!=1 or call['args'][0]['Move']['kind']!={'Local':2}
                or call['dest']['kind']!={'Local':0}):
            raise ValueError('actual JSON function, parameters or return forwarding differ')
        unwrap=regular['generics']
        if tuple(describe(type_table,decls,x) for x in unwrap['types'])!=(wanted,):
            raise ValueError('original generic JSON value type differs')
        expected_in=('ref-Shared', (('slice',(BYTE,)),)) if kind=='de' else ('ref-Shared',(wanted,))
        expected_out=('core::result::Result',(wanted if kind=='de' else ('alloc::vec::Vec',(BYTE,)),ERROR))
        if len(sig['inputs'])!=1 or describe(type_table,decls,sig['inputs'][0])!=expected_in or describe(type_table,decls,sig['output'])!=expected_out:
            raise ValueError('original complete input/result type differs')
        dictionary,child=check_trait(impls,traits,type_table,trait_table,decls,one(unwrap['trait_refs'],'typed dictionary'),kind,wanted)
        unwind=[next(iter(s['kind'])) if isinstance(s['kind'],dict) else s['kind'] for s in call_node['on_unwind']['statements']]
        if unwind!=['StorageDead','UnwindResume']: raise ValueError('original wrapper unwind differs')
        evidence[short]={'root':root,'callee':callee,'dictionary':impls[dictionary], 'inner':None if child is None else impls[child]}
    if {'encode_source_payload','encode_record_json'} <= evidence.keys() and evidence['encode_source_payload']['inner']!=evidence['encode_record_json']['dictionary']:
        raise ValueError('Some and bare record must use the exact same derived Serialize dictionary')
    return evidence


def required_call(raw):
    if raw['has_errors'] is not False: raise ValueError('incomplete actual required_option')
    translated=raw['translated']; funs={item['def_id']:item for item in translated['fun_decls'] if item is not None}
    root=one([f for f in funs.values() if name(f)==REQUIRED],'actual required_option')
    if root['item_meta']['opacity']!='Transparent' or root['body']['Structured']['locals']['arg_count']!=1:
        raise ValueError('actual generic required_option not transparent')
    calls=[node['Call']['call'] for node in traverse(root['body']['Structured']['body']) if isinstance(node,dict) and 'Call' in node and isinstance(node['Call'],dict) and 'call' in node['Call']]
    call=one(calls,'exact original Option deserialize call'); regular=call['func']['Regular']; callee=funs[regular['kind']['Fun']]
    if (name(callee)!='serde_core::de::impls::<impl>::deserialize' or len(call['args'])!=1 or len(regular['generics']['trait_refs'])!=2):
        raise ValueError('original Option Deserialize and complete two dictionaries required')
    return {'root':root,'callee':callee}
