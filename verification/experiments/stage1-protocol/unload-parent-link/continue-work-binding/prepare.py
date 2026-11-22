"""Reproduce every conditional actual view; public admission remains false."""
import argparse
import importlib.util
import json
from pathlib import Path
import re

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('continue_binding_materialization_validation',HERE/'validation.py')
v=importlib.util.module_from_spec(spec);spec.loader.exec_module(v)
MODULES=('ContinueWorkBindingModel','ContinueWorkBindingView','ContinueWorkBindingTupleView',
    'ContinueWorkBindingFileWrong','ContinueWorkBindingFileInline','ContinueWorkBindingProof','ContinueWorkBinding')
ACTUAL={
    'ContinueWorkBindingView':(('inline','original'),(v.ROOT,)),
    'ContinueWorkBindingTupleView':(('tuple','original'),(v.INPUT,v.ROOT)),
    'ContinueWorkBindingFileWrong':(('tuple','file-policy-as-zero'),(v.INPUT,v.ROOT)),
    'ContinueWorkBindingFileInline':(('inline','file-policy-as-zero'),(v.ROOT,)),
}

def prepare(args):
    locations_bytes=args.captures.read_bytes()
    locations=json.loads(locations_bytes)
    wanted={view+'/'+variant for view,variant in v.VIEWS}
    if set(locations)!=wanted or any(not isinstance(p,str) for p in locations.values()):
        raise ValueError('exact four real inline/tuple captures required')
    views={key:v.current(Path(locations['/'.join(key)]).resolve(strict=True),*key) for key in v.VIEWS}
    original,reference,engine=views[('inline','original')]
    parent,evidence=v.inherited(args.reads,args.reads_audit,args.retention,args.retention_audit,reference,engine)
    inventory=v.interfaces_and_bodies(views)
    args.output.mkdir(mode=0o700)
    for name,target in (('.lake/packages',args.lake_packages),('.tools/lean-mcp',args.lean_mcp)):
        path=args.output/name;path.parent.mkdir(exist_ok=True);path.symlink_to(target,target_is_directory=True)
    for name,wanted in parent['files'].items():
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes((args.reads/name).read_bytes())
        if v.digest(path)!=wanted:raise ValueError('whole parent source changed: '+name)
    for name in v.parent.SUPPORT:
        source=v.REPO/name
        if source.read_bytes()!=(args.reads/name).read_bytes():raise ValueError('fixed shared support changed: '+name)
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(source.read_bytes())
    for module in MODULES:
        source=(HERE/(module+'.lean.in')).read_bytes()
        if module in ACTUAL:
            key,names=ACTUAL[module];_,raw,engine=views[key]
            decls=v.parsed(raw['Funs'],engine)
            marker=b'-- ACTUAL COMPLETE BINDING FUNCTIONS\n'
            bodies=[]
            for name in names:
                body=decls[name]
                if raw['Funs'].count(body)!=1 or re.search(rb'\b(axiom|sorry|admit|opaque)\b',body):
                    raise ValueError('whole transparent generated body required: '+name)
                bodies.append(body)
            if source.count(marker)!=1:raise ValueError('one actual declaration insertion required')
            source=source.replace(marker,b'\n\n'.join(bodies)+b'\n')
        (args.output/(module+'.lean')).write_bytes(source)
    lake=(args.reads/'lakefile.toml').read_text()
    for module in MODULES:lake+='\n[[lean_lib]]\nname = "'+module+'"\n'
    (args.output/'lakefile.toml').write_text(lake)
    files=v.inputs.closure(args.output,['ContinueWorkBinding.lean'],active_view='lakefile.toml')
    if set(files)!=set(parent['files'])|{n+'.lean' for n in MODULES} or any(files[n]!=h for n,h in parent['files'].items()):
        raise ValueError('all213 parent and seven complete own physical files mandatory')
    audits=[name for file in files for name in re.findall(r'^#audit_controller_proof (\S+)$',(args.output/file).read_text(),re.M)]
    if len(files)!=220 or len(audits)!=1222 or len(set(audits))!=1222:
        raise ValueError('all220 physical files and1222 unique audits required')
    own=[name for module in MODULES for name in re.findall(r'^#audit_controller_proof (\S+)$',(args.output/(module+'.lean')).read_text(),re.M)]
    if len(own)!=23:raise ValueError('all23 model/actual/counter/control audits required')
    inputs={name:v.digest(v.REPO/name) for name in v.extra_inputs()}
    plan=dict(root='ContinueWorkBinding',source=v.capture.retire.SOURCE,tree=v.capture.retire.TREE,
        reference=original,actual_views={'/'.join(key):views[key][0] for key in v.VIEWS},parent=evidence,
        inventory=inventory,templates=inputs,files=files,strict_files=sorted(files),audits=audits,own_audits=own,
        modules={n:dict(sha256=h) for n,h in files.items()},build_order=[n.removesuffix('.lean') for n in files],
        development_only=True,conditional_interface_model=True,interface_adoption_complete=False,
        public_acceptance=False,ordinary_native_complete=False,
        full_tuple_and_result_contract=True,actual_unfactored_roots_preserved=True)
    if locations_bytes!=args.captures.read_bytes():raise ValueError('actual source locations changed')
    (args.output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print('Conditional development:220 actual files/1222 unique audits; admission/native/public remain false')

if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    for name in ('captures','reads','reads-audit','retention','retention-audit','output','lake-packages','lean-mcp'):
        cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args()
    for name,value in vars(args).items():setattr(args,name,value.resolve(strict=name!='output'))
    prepare(args)
