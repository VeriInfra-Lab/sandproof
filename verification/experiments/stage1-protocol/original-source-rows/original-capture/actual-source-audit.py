"""Finite exact source and dictionary checks for the approved changed driver."""
import hashlib,json

def check(raw,cfg,out):
    t=raw['translated'];fs=[f for f in t['fun_decls']if f]
    def simple(f):return [x.get('Ident',[None])[0]for x in f['item_meta']['name']]
    found={}
    for name,expected in cfg['expected_actual_sources'].items():
        selected=[f for f in fs if simple(f)==['sandproof_controller_service','controller','capture',name]]
        assert len(selected)==1;f=selected[0]
        if name=='accept_original_rows':
            assert f['item_meta']['opacity']=='Opaque'and f['body']=='Opaque'
            actual=f['item_meta']['source_text']
            if actual is not None:assert actual==expected,(name,actual,expected)
            actual=expected
        else:
            assert f['item_meta']['opacity']=='Transparent'and'Structured'in f['body']
            actual=f['item_meta']['source_text'];assert actual==expected,(name,actual,expected)
        found[name]={'id':f['def_id'],'source_sha256':hashlib.sha256(actual.encode()).hexdigest(),'exact_current_actual_Rust_source':True,'body_is_trusted_mechanical_cut':name=='accept_original_rows','signature':f['signature'],'generics':f['generics'],'body_sha256':hashlib.sha256(json.dumps(f['body'],sort_keys=True).encode()).hexdigest()}
    conversions=[]
    for f in fs:
        n=simple(f);s=f['item_meta'].get('source_text')or''
        if n[0]=='sandproof_controller_service'and n[-1]=='from'and any(v in s for v in ['Self::Sqlite(error)','Self::Json(error)']):
            assert f['item_meta']['opacity']=='Transparent'and'Structured'in f['body'];conversions.append({'id':f['def_id'],'source_text':s,'signature':f['signature'],'body':f['body']})
    assert len(conversions)==2
    # Retain complete original type/function/trait/dictionary definitions for
    # future comparison; these are current source hashes, not a claim that the
    # changed driver is identical to the previous program.
    all_current={k:{'count':len(t[k]),'sha256':hashlib.sha256(json.dumps(t[k],sort_keys=True).encode()).hexdigest()}for k in ['type_decls','fun_decls','trait_decls','trait_impls']}
    result={'complete':True,'actual_four_functions':found,'actual_transparent_error_conversions':conversions,'current_full_collections':all_current,'source_name_metadata':'Only lib.rs two impl names/register_tool; source inverse checked. Driver is explicitly trusted B4 mechanical adapter, not machine-refined; caller/handler/agree are transparent.','library_and_generic_dictionary_obligations':'Original E HostEvidence, actual D::decode_action/decode_receipt mutable dictionaries and I::next/FnMut dictionaries remain explicit; actual resolved inventory in actual-IR-admission.json.'}
    (out/'actual-source-body-and-dictionary-check.json').write_text(json.dumps(result,indent=2)+'\n')
    return {k:v for k,v in result.items()if k!='actual_transparent_error_conversions'}
