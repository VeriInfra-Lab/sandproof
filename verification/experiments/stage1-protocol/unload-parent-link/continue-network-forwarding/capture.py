"""Extract the real caller and transparent shared-policy tuple; never admit."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path

HERE=Path(__file__).resolve().parent
PARENT=HERE.parent/'continue-work-binding'
VARIANTS=('original','network-policy-as-deny-all')
HELPER='binding_input_with_policy'
NAMESPACE='SandproofContinueNetworkForwarding'

def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value

def factor(source,ports):
    path=source/ports.SOURCE;text=path.read_text()
    start,stop=ports.region(text);body=text[start:stop]
    prefix='encode_continue_work_binding(&('
    if body.count(prefix)!=1:raise ValueError('one actual original tuple encoding call required')
    begin=body.index(prefix)+len('encode_continue_work_binding(&')
    depth,end=0,None
    for at in range(begin,len(body)):
        if body[at]=='(':depth+=1
        elif body[at]==')':
            depth-=1
            if depth==0:end=at+1;break
    if end is None:raise ValueError('complete exact actual tuple expression required')
    expression=body[begin:end]
    for name in ('self.identity','&self.full','&self.source','self.end_receipt','self.file_policy'):
        if expression.count(name)!=1:raise ValueError('all other complete fields must remain: '+name)
    choices=('&self.network_policy','&sandbox_controller_core::NetworkPolicy::DenyAll')
    found=[value for value in choices if expression.count(value+',')==1]
    if len(found)!=1:raise ValueError('exact original or original wrong policy argument required')
    policy=found[0];forwarded=expression.replace(policy+',','network_policy,',1)
    body=body[:begin-1]+'&self.'+HELPER+'('+policy+')'+body[end:]
    helper='''
impl OriginalContinueWorkRequest {
    fn binding_input_with_policy<'a>(
        &'a self,
        network_policy: &'a sandbox_controller_core::NetworkPolicy,
    ) -> (
        OriginalContinueIdentity,
        &'a FullCaptureWorkRequest,
        &'a SourceObservationRequirement,
        CaptureWorkEndReceipt,
        &'a sandbox_controller_core::NetworkPolicy,
        sandbox_controller_core::FilePolicyRef,
    ) {
'''+forwarded+'''
    }
}
'''
    result=text[:start]+body+text[stop:]+helper
    if result.count(forwarded)!=1 or result.count('self.'+HELPER+'('+policy+')')!=1:
        raise ValueError('one moved original expression and one call with the original shared argument required')
    path.write_text(result)

def configure(view):
    if view not in ('inline','shared'):raise ValueError('exact original inline/shared views required')
    base=module('continue_network_parent_capture',PARENT/'capture.py')
    base.NAMESPACE=NAMESPACE
    if view=='shared':
        original=base.reconstruct
        def reconstruct(archive,source,variant):
            if variant not in VARIANTS:raise ValueError('exact original network-policy mutant required')
            original(archive,source,variant);factor(source,base.ports)
        base.reconstruct=reconstruct
        base.ROOTS=(base.ROOT,base.TYPE+'::'+HELPER)
        base.METHOD_ROOTS=(*base.METHOD_ROOTS,(base.MODULE,HELPER))
        base.RETAINED_ROOTS=list(dict.fromkeys((*base.RETAINED_ROOTS,base.MODULE+'::*::'+HELPER)))
        base.OPTIONS=dict(base.OPTIONS,roots=list(base.ROOTS),retained_roots=base.RETAINED_ROOTS)
    return base

def capture(output,tools,lock,variant,view):
    base=configure(view)
    try:base.capture(output,tools,lock,variant)
    finally:
        path=output/'result.json'
        if path.is_file():
            row=json.loads(path.read_bytes())
            row.update(network_forwarding_driver=sha256(Path(__file__).read_bytes()).hexdigest(),
                network_forwarding_view=view,actual_forwarding_proved=False,
                property_refutation=False,interface_adoption_complete=False)
            path.write_text(json.dumps(row,indent=2)+'\n')

if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--view',choices=('inline','shared'),required=True)
    cli.add_argument('--variant',choices=VARIANTS,required=True)
    for name in ('output','tools','lock'):cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args();capture(args.output.resolve(),args.tools.resolve(strict=True),args.lock.resolve(strict=True),args.variant,args.view)
