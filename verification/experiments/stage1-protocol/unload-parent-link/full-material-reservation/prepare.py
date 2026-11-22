"""Reproduce both complete actual Full material reservation views."""
import argparse
import importlib.util
import json
from pathlib import Path
import re

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('full_material_validation',HERE/'validation.py')
validation=importlib.util.module_from_spec(spec)
spec.loader.exec_module(validation)
capture=validation.capture
CORE=('FullMaterialReservationTypes','FullMaterialReservationView','FullMaterialReservationModel',
      'FullMaterialReservationSteps','FullMaterialReservationProof','FullMaterialReservationValues','FullMaterialReservation')
WRONG=('FullMaterialReservationWrong','FullMaterialReservationNegative')


def insert(source,marker,body):
    if source.count(marker)!=1:
        raise ValueError('one exact original declaration insertion required')
    return source.replace(marker,body+b'\n')


def prepare(args):
    original,reference=validation.current(args.reference,'original')
    parent,evidence,additions=validation.parent(args.reads,args.reads_audit,args.retention,args.retention_audit,reference)
    if args.variant=='original':
        if args.actual!=args.reference:
            raise ValueError('one exact original correct capture required')
        actual,generated,difference=original,reference,None
    elif args.variant=='omit-second-material-copy':
        actual,generated=validation.current(args.actual,args.variant)
        difference=validation.same_interfaces(reference,generated)
    else:
        raise ValueError('both exact registered original/wrong variants required')
    args.output.mkdir(mode=0o700)
    for name,target in(('.lake/packages',args.lake_packages),('.tools/lean-mcp',args.lean_mcp)):
        path=args.output/name;path.parent.mkdir(exist_ok=True);path.symlink_to(target,target_is_directory=True)
    for name,wanted in parent['files'].items():
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes((args.reads/name).read_bytes())
        if validation.digest(path)!=wanted:
            raise ValueError('all207 complete inherited files must remain unchanged: '+name)
    for name in validation.SUPPORT:
        body=(validation.REPO/name).read_bytes()
        if body!=(args.reads/name).read_bytes():
            raise ValueError('exact common support must remain unchanged: '+name)
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(body)
    modules=(*CORE,*(WRONG if args.variant!='original'else()))
    lake=(args.reads/'lakefile.toml').read_text()
    for name in modules:
        body=(HERE/(name+'.lean.in')).read_bytes()
        if name=='FullMaterialReservationTypes':
            body=insert(body,b'-- ACTUAL COMPLETE MATERIAL CLOSURE TYPES\n',b'\n\n'.join(
                validation.declaration(reference,'Types',n,b)for n,b in additions['Types'].items()))
        elif name in('FullMaterialReservationView','FullMaterialReservationWrong'):
            ref=reference if name=='FullMaterialReservationView'else generated
            marker=b'-- ACTUAL COMPLETE MATERIAL FUNCTIONS\n'if name=='FullMaterialReservationView'else b'-- ACTUAL COMPLETE WRONG MATERIAL FUNCTIONS\n'
            rows=validation.parsed(ref['Funs'])
            body=insert(body,marker,b'\n\n'.join(validation.declaration(ref,'Funs',n,rows[n])for n in rows if n in additions['Funs']))
        (args.output/(name+'.lean')).write_bytes(body)
        if 'name = "'+name+'"'not in lake:lake+='\n[[lean_lib]]\nname = "'+name+'"\n'
    (args.output/'lakefile.toml').write_text(lake)
    root='FullMaterialReservation'if args.variant=='original'else'FullMaterialReservationNegative'
    files=validation.original_inputs.closure(args.output,[root+'.lean'],active_view='lakefile.toml')
    expected=set(parent['files'])|{n+'.lean'for n in modules}
    if set(files)!=expected or any(files[n]!=h for n,h in parent['files'].items()):
        raise ValueError('every complete parent and current actual file is mandatory')
    audits=[]
    for name in files:audits.extend(re.findall(r'^#audit_controller_proof (\S+)$',(args.output/name).read_text(),re.M))
    if len(audits)!=len(set(audits)):
        raise ValueError('all complete proof audit names must be unique')
    inputs=[p for p in sorted(HERE.iterdir())if p.is_file()]
    inputs+=[validation.REPO/n for n in validation.extra_inputs()]
    plan=dict(root=root,variant=args.variant,source=capture.retire.SOURCE,tree=capture.retire.TREE,
        reference=original,actual=actual,parent=evidence,wrong_view=difference,
        selected_declarations={g:list(rows)for g,rows in additions.items()},
        templates={str(p.relative_to(validation.REPO)):validation.digest(p)for p in inputs},
        files=files,strict_files=sorted(files),audits=audits,modules={n:dict(sha256=h)for n,h in files.items()},
        build_order=[n.removesuffix('.lean')for n in files],development_only=True,public_acceptance=False,
        ordinary_native_complete=False,complete_original_full_arithmetic_property=True,
        exact_standard_interfaces=29,cost_formula='2*(state+memory+disk)+589824')
    (args.output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print(root,len(files),'complete actual files;',len(audits),'unique audit obligations')


if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant',choices=capture.VARIANTS,required=True)
    for name in('actual','reference','reads','reads-audit','retention','retention-audit','output','lake-packages','lean-mcp'):
        cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args()
    for name,value in vars(args).items():
        if isinstance(value,Path):setattr(args,name,value.resolve(strict=name!='output'))
    prepare(args)
