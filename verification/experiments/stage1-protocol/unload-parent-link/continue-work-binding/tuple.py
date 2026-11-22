"""Move the actual tuple expression once; prove the transparent helper too."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('continue_binding_tuple_base',HERE/'capture.py')
base=importlib.util.module_from_spec(spec);spec.loader.exec_module(base)
ORIGINAL_RECONSTRUCT=base.reconstruct
VARIANTS=('original','file-policy-as-zero')
base.ROOTS=(base.ROOT,base.TYPE+'::binding_input')
base.METHOD_ROOTS=(*base.METHOD_ROOTS,(base.MODULE,'binding_input'))
base.NAMESPACE='SandproofContinueWorkBindingTuple'
base.RETAINED_ROOTS=list(dict.fromkeys((*base.RETAINED_ROOTS,base.MODULE+'::*::binding_input')))
base.OPTIONS=dict(base.OPTIONS,roots=list(base.ROOTS),retained_roots=base.RETAINED_ROOTS)


def factor(source):
    path=source/base.ports.SOURCE
    text=path.read_text()
    start,stop=base.ports.region(text)
    body=text[start:stop]
    prefix='encode_continue_work_binding(&('
    if body.count(prefix)!=1:
        raise ValueError('one exact original typed encoding call required')
    begin=body.index(prefix)+len('encode_continue_work_binding(&')
    depth,end=0,None
    for at in range(begin,len(body)):
        if body[at]=='(':
            depth+=1
        elif body[at]==')':
            depth-=1
            if depth==0:
                end=at+1
                break
    if end is None:
        raise ValueError('complete original tuple expression required')
    expression=body[begin:end]
    if (not expression.startswith('(\n') or any(expression.count(name)!=1 for name in
            ('self.identity','&self.full','&self.source','self.end_receipt','&self.network_policy'))):
        raise ValueError('all five original tuple positions required')
    original='                self.file_policy,\n'
    wrong='                sandbox_controller_core::FilePolicyRef { digest: sandbox_controller_core::PolicyDigest(0) },\n'
    if expression.count(original)+expression.count(wrong)!=1 or text.count(expression)!=1:
        raise ValueError('exact original or sole Copy-field mutation; move once without duplication')
    body=body[:begin-1]+'&self.binding_input()'+body[end:]
    helper='''
impl OriginalContinueWorkRequest {
    fn binding_input(&self) -> (
        OriginalContinueIdentity,
        &FullCaptureWorkRequest,
        &SourceObservationRequirement,
        CaptureWorkEndReceipt,
        &sandbox_controller_core::NetworkPolicy,
        sandbox_controller_core::FilePolicyRef,
    ) {
'''+expression+'''
    }
}
'''
    result=text[:start]+body+text[stop:]+helper
    if result.count(expression)!=1 or result.count('self.binding_input()')!=1:
        raise ValueError('one moved original expression and one actual helper call required')
    path.write_text(result)


def reconstruct(archive,source,variant):
    if variant not in VARIANTS:
        raise ValueError('only two actual Copy-field tuple views are supported')
    ORIGINAL_RECONSTRUCT(archive,source,variant)
    factor(source)


def capture(output,tools,lock,variant):
    base.reconstruct=reconstruct
    try:
        base.capture(output,tools,lock,variant)
    finally:
        path=output/'result.json'
        if path.is_file():
            row=json.loads(path.read_bytes())
            row.update(tuple_driver=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                transparent_tuple_factor=True,tuple_factor_proved=False)
            path.write_text(json.dumps(row,indent=2)+'\n')


if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=VARIANTS,required=True)
    for name in ('output','tools','lock'):
        cli.add_argument('--'+name,type=Path,required=True)
    a=cli.parse_args()
    capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
