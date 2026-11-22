"""Reproduce the full parent and all actual policy forwarding proof views."""
import argparse
import importlib.util
import json
from pathlib import Path
import re

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('continue_network_prepare_validation',HERE/'validation.py')
v=importlib.util.module_from_spec(spec);spec.loader.exec_module(v)
MODULES=('ContinueNetworkForwardingView','ContinueNetworkForwardingWrong','ContinueNetworkForwardingInline',
    'ContinueNetworkForwardingProof','ContinueNetworkForwarding')
ACTUAL={
    'ContinueNetworkForwardingView':(('shared','original'),(v.INPUT,v.ROOT)),
    'ContinueNetworkForwardingWrong':(('shared','network-policy-as-deny-all'),(v.INPUT,v.ROOT)),
    'ContinueNetworkForwardingInline':(('inline','network-policy-as-deny-all'),(v.ROOT,)),
}

def prepare(args):
    saved=args.captures.read_bytes();locations=json.loads(saved)
    if set(locations)!={'/'.join(key) for key in v.VIEWS} or any(not isinstance(p,str) for p in locations.values()):
        raise ValueError('all three original real caller/shared-policy capture locations required')
    views={key:v.current(Path(locations['/'.join(key)]).resolve(strict=True),*key) for key in v.VIEWS}
    original,reference=views[('shared','original')]
    parent,evidence=v.inherited(args.reads,args.reads_audit,reference)
    inventory=v.interfaces_and_bodies(views)
    args.output.mkdir(mode=0o700)
    for name,target in (('.lake/packages',args.lake_packages),('.tools/lean-mcp',args.lean_mcp)):
        path=args.output/name;path.parent.mkdir(exist_ok=True);path.symlink_to(target,target_is_directory=True)
    for name,wanted in parent['files'].items():
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes((args.reads/name).read_bytes())
        if v.digest(path)!=wanted:raise ValueError('whole parent source changed: '+name)
    for name in v.SUPPORT:
        source=v.REPO/name
        if source.read_bytes()!=(args.reads/name).read_bytes():raise ValueError('fixed common support changed: '+name)
        path=args.output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(source.read_bytes())
    for module in MODULES:
        source=(HERE/(module+'.lean.in')).read_bytes()
        if module in ACTUAL:
            key,names=ACTUAL[module];_,raw=views[key];rows=v.parsed(raw['Funs']);bodies=[]
            for name in names:
                body=rows[name]
                if raw['Funs'].count(body)!=1 or re.search(rb'\b(axiom|sorry|admit|opaque)\b',body):
                    raise ValueError('whole original transparent actual declaration required: '+name)
                bodies.append(body)
            marker=b'-- ACTUAL COMPLETE POLICY FORWARDING FUNCTIONS\n'
            if source.count(marker)!=1:raise ValueError('one complete actual declaration insertion required')
            source=source.replace(marker,b'\n\n'.join(bodies)+b'\n')
        (args.output/(module+'.lean')).write_bytes(source)
    lake=(args.reads/'lakefile.toml').read_text()
    for module in MODULES:lake+='\n[[lean_lib]]\nname = "'+module+'"\n'
    (args.output/'lakefile.toml').write_text(lake)
    files=v.inputs.closure(args.output,['ContinueNetworkForwarding.lean'],active_view='lakefile.toml')
    if set(files)!=set(parent['files'])|{n+'.lean' for n in MODULES} or any(files[n]!=h for n,h in parent['files'].items()):
        raise ValueError('all220 whole parent files and all five own actual modules mandatory')
    audits=[name for file in files for name in re.findall(r'^#audit_controller_proof (\S+)$',(args.output/file).read_text(),re.M)]
    own=[name for module in MODULES for name in re.findall(r'^#audit_controller_proof (\S+)$',(args.output/(module+'.lean')).read_text(),re.M)]
    if len(files)!=225 or len(audits)!=1243 or len(set(audits))!=1243 or len(own)!=21:
        raise ValueError('all225 actual files/1243 unique audits/21 new obligations required')
    templates={name:v.digest(v.REPO/name) for name in v.extra_inputs()}
    engine=v.capture.configure('shared')
    plan=dict(root='ContinueNetworkForwarding',source=engine.retire.SOURCE,tree=engine.retire.TREE,
        reference=original,actual_views={'/'.join(key):views[key][0] for key in v.VIEWS},parent=evidence,
        inventory=inventory,templates=templates,files=files,strict_files=sorted(files),audits=audits,own_audits=own,
        modules={n:dict(sha256=h) for n,h in files.items()},build_order=[n.removesuffix('.lean') for n in files],
        development_only=True,conditional_interface_model=True,interface_adoption_complete=False,
        ordinary_native_complete=False,public_acceptance=False,
        same_parent_complete_contract=True,actual_callsite_inputs_connected=True,unfactored_wrong_preserved=True)
    if args.captures.read_bytes()!=saved:raise ValueError('actual source locations changed')
    (args.output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print('Conditional forwarding development:225 actual files/1243 audits; adoption/native/public pending')

if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    for name in ('captures','reads','reads-audit','output','lake-packages','lean-mcp'):cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args()
    for name,value in vars(args).items():setattr(args,name,value.resolve(strict=name!='output'))
    prepare(args)
