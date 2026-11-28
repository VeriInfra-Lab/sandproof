"""Exact declared existing provider, source, signature and formula link only."""
from pathlib import Path
import hashlib,json,re

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def walk(x):
    yield x
    if isinstance(x,dict):
        for v in x.values():yield from walk(v)
    elif isinstance(x,list):
        for v in x:yield from walk(v)
def names(x):return [a.get('Ident',[None])[0]for a in x['item_meta']['name']]
def signature(raw):
    t=raw['translated'];f=next(x for x in t['fun_decls']if x and names(x)==['sandproof_protocol','valid_identifier']);m={}
    for x in walk(t):
        v=x.get('Value')if isinstance(x,dict)else None
        if not isinstance(v,list)or len(v)!=2:continue
        p=v[1]
        valid=isinstance(p,dict)and len(p)==1 and next(iter(p))in ['Ref','Scalar','Adt','TypeVar','Slice','Array','RawPtr','FnPtr','FnDef','DynTrait']
        if valid:
            if v[0]in m:assert m[v[0]]==p
            m[v[0]]=p
    def dec(x):
        if isinstance(x,dict):
            if set(x)=={'Deduplicated'}:return dec(m[x['Deduplicated']])
            if set(x)=={'Value'}:return dec(x['Value'][1])
            if 'Adt'in x and x['Adt']['builtin']=='Str':
                assert x['Adt']['generics']=={'regions':[],'types':[],'const_generics':[],'trait_refs':[]};return 'Str'
            return {k:dec(v)for k,v in x.items()}
        if isinstance(x,list):return [dec(v)for v in x]
        return x
    sig=dec(f['signature']);expected={'is_unsafe':False,'abi':'Rust','is_variadic':False,'inputs':[{'Ref':[{'Var':{'Free':0}},'Str','Shared']}],'output':{'Scalar':'Bool'}}
    assert sig==expected and not f['generics']['types']and not f['generics']['trait_clauses']
    return f,sig

def fn_source(s):
    start=s.index('fn valid_identifier(');pos=s.index('{',start);depth=0
    for j in range(pos,len(s)):
        if s[j]=='{':depth+=1
        if s[j]=='}':
            depth-=1
            if depth==0:return s[start:j+1]
    raise ValueError('unterminated original source')

def check(raw,S,I,O,V):
    b=json.loads((I/'proof-link-binding.json').read_text());assert b['rust_name']=='sandproof_protocol::valid_identifier'and b['role']=='separately-verified owned callee; NOT trusted-owned cut'
    assert b['provider_theorem']=='SandboxModel.Foundations.identifier_validator_refines'
    for n,h in b['provider_inputs'].items():assert sha(S/n)==h,('provider input changed',n)
    frozen=json.loads(json.dumps(b['source_identity']));f=(I/'provider-frozen-protocol-lib.rs').read_text();current=(S/'sandbox-protocol/src/lib.rs').read_text()
    assert sha(I/'provider-frozen-protocol-lib.rs')==frozen['frozen_provider_full_file_sha256']
    manifest={line.split('  ',1)[1]:line.split('  ',1)[0]for line in(S/'verification/foundations-inputs.sha256').read_text().splitlines()if'  'in line}
    assert manifest['sandbox-protocol/src/lib.rs']==frozen['frozen_provider_full_file_sha256']
    a=fn_source(f);c=fn_source(current);assert a==c and hashlib.sha256(c.encode()).hexdigest()==frozen['function_sha256']
    theory=(S/'SandboxModel/Foundations/Identifiers.lean').read_text();assert 'theorem identifier_validator_refines (value : Str) :\n    sandproof_protocol.valid_identifier value = .ok (decide (opaqueBytesValid value.val)) := by' in theory
    assert '#audit_controller_proof SandboxModel.Foundations.identifier_validator_refines'in(S/'SandboxModel/Foundations/Audit.lean').read_text()
    assert sha(S/'SandboxModel/Foundations/Identifiers.lean')==manifest['SandboxModel/Foundations/Identifiers.lean']
    assert sha(I/'SourceIdentifierFormula.lean')==b['leaf_sha256']and sha(I/'SourceIdentifierProvider.lean')==b['provider_bridge_sha256']
    assert 'SandproofFoundations.Funs'not in(I/'SourceIdentifierFormula.lean').read_text()
    for stem,cur in [('valid-identifier','current-row14-valid-identifier'),('call_mut','current-call_mut'),('call_once','current-call_once')]:
        original=(I/('provider-'+stem+'.lean.fragment')).read_bytes();actual=(I/(cur+'.lean.fragment')).read_bytes();assert original==actual
        # The provider fragment must be literally from the delivered actual Funs.
        assert original.decode()in(S/'verification/aeneas/foundations/SandproofFoundations/Funs.lean').read_text()
    provider=json.loads((S/'verification/aeneas/llbc/sandproof_foundations.llbc').read_text());old_current=json.loads((V/'std-mutex-persistent-tmp/v3-original-source-driver-cut-precheck-16/original_source_generic.llbc').read_text())
    pf,ps=signature(provider);cf,cs=signature(old_current);nf,ns=signature(raw);assert ps==cs==ns
    critical=b['semantic_options_equal'];assert all(provider['translated']['options'][k]==raw['translated']['options'][k]for k in critical)
    assert {x['key']for x in provider['translated']['target_information']}=={x['key']for x in raw['translated']['target_information']}=={b['target']}
    assert pf['item_meta']['opacity']=='Transparent'and cf['item_meta']['opacity']=='Transparent'and nf['item_meta']['opacity']=='Opaque'and nf['body']=='Opaque'
    its=[x for x in raw['translated']['trait_decls']if x and names(x)==['core','iter','traits','iterator','Iterator']];assert len(its)==1
    methods=[m['skip_binder']['name']for m in its[0]['methods']if m];assert 'next'in methods and'all'not in methods,('all still selected despite exact proved-module link',methods)
    # Reuse the independently sealed actual provider bridge, not a new proof.
    evidence=b['independent_bridge_evidence']
    proof=V/'std-mutex-persistent-tmp/v3-source-provider-kernel-20'
    assert sha(proof/'result.json')==evidence['result_sha256']
    sealed=json.loads((proof/'result.json').read_text())
    assert sealed['complete']and sealed['kernel_checked']and sealed['original_actual_file_service']
    assert sha(proof/'actual-files/physical-files/service.log')==evidence['service_log_sha256']==sealed['actual_files']['service_log_sha256']
    provider_source=V/'std-mutex-persistent-tmp/v3-source-provider-kernel-18/source'
    for n,h in sealed['closure'].items():assert sha(provider_source/n)==h
    assert sha(provider_source/'SourceIdentifierProvider.lean')==b['provider_bridge_sha256']
    assert sha(provider_source/'SourceIdentifierFormula.lean')==b['leaf_sha256']
    result={'complete_source_signature_existing_provider_binding':True,'rust_name':b['rust_name'],'role':b['role'],'existing_provider_theorem':b['provider_theorem'],'source_full_function_equal':True,'unrelated_whole_protocol_source_differs':f!=current,'actual_signature_equal':ns,'critical_options_equal':critical,'actual_Iterator_methods':methods,'independent_bridge_kernel_and_original_service_reused':True,'independent_bridge_result_sha256':evidence['result_sha256'],'bridge_status':'independent exact provider bridge previously sealed; actual new consumer assembly/property still requires kernel/service/audit','formula_leaf_no_generated_Funs_import':True}
    (O/'verified-identifier-provider-link-admission.json').write_text(json.dumps(result,indent=2)+'\n');return result
