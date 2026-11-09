"""This finite two-root original IR audit is preparation, not a generic frontend."""
import hashlib,json
OWNED=('sandproof_controller_service','sandproof_protocol','sandbox_controller_core')
PREFIX='sandproof_controller_service::controller::capture::'
TRUSTED={PREFIX+'accept_original_rows'}
VERIFIED={'sandproof_protocol::valid_identifier'}
SHIMS=TRUSTED|VERIFIED

def walk(x):
    yield x
    if isinstance(x,dict):
        for k,v in x.items():
            if k not in ['source_text','comments_before']:yield from walk(v)
    elif isinstance(x,list):
        for v in x:yield from walk(v)

def name(meta):
    return '::'.join(x['Ident'][0] if 'Ident'in x else '*' for x in meta['name'])

def audit(raw,cfg,out):
    t=raw['translated'];opts=t['options'];assert raw['has_errors']is False
    assert opts['start_from']==cfg['roots']and opts['include']==cfg['include']and opts['opaque']==cfg['opaque']
    assert not any(opts[k]for k in ['exclude','skip_borrowck','no_typecheck','monomorphize','remove_unused_clauses'])
    assert {x['key']for x in t['target_information']}=={'x86_64-unknown-linux-gnu'}
    fs={f['def_id']:f for f in t['fun_decls']if f};ims={f['def_id']:f for f in t['trait_impls']if f};ts={f['def_id']:f for f in t['trait_decls']if f}
    trmemo={}
    for x in walk(t):
        if isinstance(x,dict)and set(x)=={'Value'}:
            v=x['Value']
            if isinstance(v,list)and len(v)==2 and isinstance(v[1],dict)and 'trait_decl_ref'in v[1]:
                if v[0]in trmemo:assert trmemo[v[0]]==v[1]
                trmemo[v[0]]=v[1]
    def ref(v):
        if 'Value'in v:return v['Value'][1]
        assert set(v)=={'Deduplicated'};return trmemo[v['Deduplicated']]
    def brief(f):
        m=f['item_meta'];b=f['body'];return dict(id=f['def_id'],name=name(m),name_ast=m['name'],opacity=m['opacity'],src=f['src'],body_kind=list(b)[0]if isinstance(b,dict)else b,span=m['span'],arg_count=b.get('Structured',{}).get('locals',{}).get('arg_count')if isinstance(b,dict)else None)
    owned=[brief(f)for f in fs.values()if f['item_meta']['name'][0].get('Ident',[None])[0]in OWNED]
    opaque=[f for f in owned if f['opacity']!='Transparent' or f['body_kind']!='Structured']
    unknown=[f for f in opaque if f['name']not in SHIMS]
    owned_types=[dict(id=f['def_id'],name=name(f['item_meta']),opacity=f['item_meta']['opacity'],kind=list(f['kind'])[0]if isinstance(f['kind'],dict)else f['kind'])for f in t['type_decls']if f and f['item_meta']['name'][0].get('Ident',[None])[0]in OWNED]
    badtypes=[f for f in owned_types if f['opacity']!='Transparent' or f['kind']=='Opaque']
    roots=[]
    for n in cfg['roots']:
        selected=[f for f in owned if f['name']==n];assert len(selected)==1
        roots.append(selected[0])
    assert {r['name'].split('::')[-1]:r['arg_count']for r in roots}=={'accept_original_source_row':7,'original_source_using':5}
    edges=[];traitcalls=[];unresolved=[]
    for f in fs.values():
        for x in walk(f['body']):
            if not isinstance(x,dict)or 'Call'not in x:continue
            call=x['Call']['call'];v=call['func']
            if 'Regular'not in v:
                unresolved.append(dict(caller=f['def_id'],func=v,reason='non-Regular function operand'));continue
            k=v['Regular']['kind']
            if 'Fun'in k:
                j=k['Fun'];assert j in fs;edges.append(dict(caller=f['def_id'],callee=j,name=name(fs[j]['item_meta']),dispatch='direct',generics=v['Regular']['generics']));continue
            if 'Trait'not in k:
                unresolved.append(dict(caller=f['def_id'],func=v,reason='unrecognized regular function kind'));continue
            rr,index=k['Trait'];r=ref(rr);rk=r['kind'];declaration=r['trait_decl_ref']['skip_binder']['id'];entry=dict(caller=f['def_id'],trait=name(ts[declaration]['item_meta']),method_index=index,trait_reference=r)
            if isinstance(rk,dict)and'TraitImpl'in rk:
                im=ims[rk['TraitImpl']['id']];m=im['methods'][index];j=m['skip_binder']['id'];assert j in fs
                entry.update(dispatch='concrete implementation',impl=im['def_id'],callee=j,name=name(fs[j]['item_meta']));edges.append(entry)
            elif any(isinstance(n,dict)and any('Clause'in str(key)for key in n)for n in walk(rk)):
                entry.update(dispatch='original generic dictionary parameter; concrete caller instantiation remains obligation')
            elif any(s in str(rk)for s in ['SelfId','Self','Clause']):
                entry.update(dispatch='original generic/self dictionary parameter; concrete caller instantiation remains obligation')
            else:
                entry.update(dispatch='UNRESOLVED');unresolved.append(entry)
            traitcalls.append(entry)
    # Keep the full compiler-selected dictionary method inventory, including closure
    # FnOnce/FnMut bridges. This distinguishes direct runtime edges from compiler
    # dictionary members; no claim that every selected member is called at runtime.
    dictionaries=[]
    for i,im in ims.items():
        methods=[]
        for ix,m in enumerate(im['methods']):
            if m is None:continue
            j=m['skip_binder']['id'];assert j in fs
            methods.append(dict(item_index=ix,function=brief(fs[j]),instantiation=m))
        dictionaries.append(dict(id=i,trait=im['impl_trait'],name=name(im['item_meta']),methods=methods))
    result=dict(kind='finite-original-source-driver-cut-IR-audit',complete=not(unknown or badtypes or unresolved),roots=roots,owned_functions=owned,planned_owned_library_opaque=sorted(SHIMS),trusted_owned_opaque=sorted(TRUSTED),separately_verified_owned_callees=sorted(VERIFIED),unknown_owned_opaque=unknown,owned_types=owned_types,unknown_owned_opaque_types=badtypes,actual_direct_and_concrete_trait_call_edges=edges,actual_trait_calls=traitcalls,unresolved_calls=unresolved,compiler_selected_dictionaries=dictionaries,closure_functions=[f for f in owned if any('closure'in str(n).lower()for n in f['name_ast'])],scope='actual source row/helper/driver/caller functions only; not full current or production E instantiation; no Lean/kernel/public acceptance')
    (out/'actual-IR-admission.json').write_text(json.dumps(result,indent=2)+'\n')
    assert not unknown,('unknown concrete owned opaque functions',[(x['id'],x['name'])for x in unknown])
    assert not badtypes,('unknown opaque owned carriers',badtypes)
    assert not unresolved,('unresolved actual function/trait dispatch',unresolved)
    return result
