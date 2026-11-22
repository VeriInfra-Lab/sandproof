"""Reproduce all three complete actual getter/constructor development views."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('source_end_accessor_validation',HERE/'validation.py')
validation=importlib.util.module_from_spec(spec)
spec.loader.exec_module(validation)
capture=validation.capture
CORE=('SourceEndAccessorTypes','SourceEndAccessorView','SourceEndAccessorModel',
      'SourceEndAccessorProof','SourceEndAccessorValues','SourceEndAccessor')
WRONG={
    'receipt-no-request-as-returned':('SourceEndAccessorReceiptWrong','SourceEndAccessorReceiptNegative'),
    'swap-receipt-bindings':('SourceEndAccessorConstructorWrong','SourceEndAccessorConstructorNegative')}


def replace(source,marker,body):
    if source.count(marker)!=1:
        raise ValueError('one complete original declaration insertion required')
    return source.replace(marker,body+b'\n')


def prepare(args):
    original,reference=validation.current(args.reference,'original')
    parent,evidence,additions=validation.parent(args.reads,args.reads_audit,
        args.retention,args.retention_audit,reference)
    wrong_parent=None
    if args.variant=='original':
        if args.actual!=args.reference:
            raise ValueError('one exact original correct capture is required')
        actual,generated,difference=original,reference,None
    elif args.variant in WRONG:
        # The fixed wrong input is a store containing both named complete
        # captures. This is not a parameter for choosing fewer obligations.
        actual,generated=validation.current(args.actual/(args.variant+'-01'),args.variant)
        if args.variant=='receipt-no-request-as-returned':
            wrong_parent,wrong_plan,wrong_evidence=validation.receipt_wrong_parent(args.reads)
            evidence=dict(evidence,receipt_wrong_parent=wrong_evidence)
        difference=validation.same_interfaces(reference,generated,args.variant,wrong_parent)
    else:
        raise ValueError('all three exact registered variants are required')
    args.output.mkdir(mode=0o700)
    for name,target in(('.lake/packages',args.lake_packages),('.tools/lean-mcp',args.lean_mcp)):
        p=args.output/name;p.parent.mkdir(exist_ok=True);p.symlink_to(target,target_is_directory=True)
    for name,wanted in parent['files'].items():
        p=args.output/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes((args.reads/name).read_bytes())
        if validation.digest(p)!=wanted:
            raise ValueError('all complete inherited files must remain unchanged: '+name)
    for name in validation.SUPPORT:
        content=(validation.REPO/name).read_bytes()
        if content!=(args.reads/name).read_bytes():
            raise ValueError('the exact common support must remain unchanged: '+name)
        p=args.output/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(content)
    inherited_wrong=()
    if wrong_parent is not None:
        inherited_wrong=('SourceEndReceiptWrong','SourceEndReceiptNegative')
        for name in inherited_wrong:
            n=name+'.lean';content=(wrong_parent/n).read_bytes()
            if validation.digest(wrong_parent/n)!=wrong_plan['files'][n]:
                raise ValueError('the actual checked parent wrong leaf changed')
            (args.output/n).write_bytes(content)
    modules=(*CORE,*WRONG.get(args.variant,()))
    lake=(args.reads/'lakefile.toml').read_text()
    for name in modules:
        source=(HERE/(name+'.lean.in')).read_bytes()
        if name=='SourceEndAccessorTypes':
            source=replace(source,b'-- ACTUAL COMPLETE CONTINUATION QUESTION TYPE\n',
                b'\n\n'.join(validation.declaration(reference,'Types',n,b)for n,b in additions['Types'].items()))
        elif name in('SourceEndAccessorView','SourceEndAccessorReceiptWrong','SourceEndAccessorConstructorWrong'):
            ref=reference if name=='SourceEndAccessorView'else generated
            marker=(b'-- ACTUAL COMPLETE ACCESSOR FUNCTIONS\n'if name=='SourceEndAccessorView'
                    else b'-- ACTUAL COMPLETE WRONG ACCESSOR FUNCTIONS\n')
            rows=validation.parsed(ref['Funs'])
            source=replace(source,marker,b'\n\n'.join(validation.declaration(ref,'Funs',n,rows[n])
                for n in rows if n in additions['Funs']))
        (args.output/(name+'.lean')).write_bytes(source)
    for name in(*modules,*inherited_wrong):
        if 'name = "'+name+'"'not in lake:
            lake+='\n[[lean_lib]]\nname = "'+name+'"\n'
    (args.output/'lakefile.toml').write_text(lake)
    root='SourceEndAccessor'if args.variant=='original'else WRONG[args.variant][-1]
    files=validation.original_inputs.closure(args.output,[root+'.lean'],active_view='lakefile.toml')
    expected=set(parent['files'])|{n+'.lean'for n in(*modules,*inherited_wrong)}
    if set(files)!=expected or any(files[n]!=h for n,h in parent['files'].items()):
        raise ValueError('all parent and current actual files are mandatory')
    audits=[]
    for name in files:
        audits.extend(re.findall(r'^#audit_controller_proof (\S+)$',(args.output/name).read_text(),re.M))
    if len(audits)!=len(set(audits)):
        raise ValueError('the complete audit inventory must be unique')
    inputs=[p for p in sorted(HERE.iterdir())if p.is_file()]
    inputs+=[validation.REPO/n for n in validation.extra_inputs()]
    plan=dict(root=root,variant=args.variant,source=capture.retire.SOURCE,tree=capture.retire.TREE,
        reference=original,actual=actual,parent=evidence,wrong_view=difference,
        selected_declarations={g:list(rows)for g,rows in additions.items()},
        templates={str(p.relative_to(validation.REPO)):validation.digest(p)for p in inputs},
        files=files,strict_files=sorted(files),audits=audits,
        modules={n:dict(sha256=h)for n,h in files.items()},build_order=[n.removesuffix('.lean')for n in files],
        development_only=True,public_acceptance=False,ordinary_native_complete=False,
        complete_original_receipt_property=True,constructor_fields=6,exact_standard_interfaces=29)
    (args.output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print(root,len(files),'complete actual files;',len(audits),'unique audit obligations')


if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=capture.VARIANTS,required=True)
    for name in('actual','reference','reads','reads-audit','retention','retention-audit',
                'output','lake-packages','lean-mcp'):
        cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args()
    for name,value in vars(args).items():
        if isinstance(value,Path):setattr(args,name,value.resolve(strict=name!='output'))
    prepare(args)
