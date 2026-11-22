"""Check the complete conditional development closure; never public admission."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE=Path(__file__).resolve().parent
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
prepare=module('continue_binding_check_prepare',HERE/'prepare.py')
v=prepare.v
parent=module('continue_binding_complete_check_parent',HERE.parent/'source-remove-retired/check.py')
KEYS={'captures','reads','reads_audit','retention','retention_audit'}

def check(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal':raise ValueError('formal proof machine role required')
    os.umask(0o077);os.sched_setaffinity(0,sorted(os.sched_getaffinity(0))[:4])
    os.environ.update(LEAN_NUM_THREADS='4',LEAN_STACK_SIZE_KB='65536')
    saved=args.inputs.read_bytes();locations=json.loads(saved)
    if set(locations)!=KEYS or any(not isinstance(p,str) for p in locations.values()):
        raise ValueError('all five exact original input locations required')
    locations={n:Path(p).resolve(strict=True) for n,p in locations.items()}
    args.output.mkdir(mode=0o700);private=args.output/'diagnostics.lock';private.touch(mode=0o600)
    report=dict(complete=False,development_only=True,conditional_interface_model=True,
        interface_adoption_complete=False,ordinary_native_complete=False,public_acceptance=False,
        source=v.capture.retire.SOURCE,tree=v.capture.retire.TREE,commands=[])
    started=time.monotonic()
    try:
        with args.lock.open('r+') as guard:
            fcntl.flock(guard,fcntl.LOCK_EX|fcntl.LOCK_NB)
            report['dependencies']=v.inputs.dependencies(args.lake_packages,v.REPO/'lake-manifest.json')
            project=args.output/'project'
            options=argparse.Namespace(**locations,output=project,lake_packages=args.lake_packages,lean_mcp=args.lean_mcp)
            prepare.prepare(options)
            plan=json.loads((project/'plan.json').read_bytes())
            if (locations['reads']/'.lake/build').is_dir():
                subprocess.run(['cp','-a','--reflink=auto',str(locations['reads']/'.lake/build'),str(project/'.lake/build')],check=True)
            parent.completed(report,['lake','build',plan['root']],project,args.output/'build.log',1800)
            print('Complete build passed; checking all220 real files via same Lean service',flush=True)
            audit=args.output/'audit'
            parent.completed(report,[sys.executable,str(HERE.parent/'diagnose.py'),'--project',str(project),
                '--output',str(audit),'--lock',str(private)],v.REPO,args.output/'diagnostics.log',3700)
            view=dict(physical_files=220,full_audits=1222,own_audits=plan['own_audits'])
            report['audit']=parent.audit(project,audit,view)
            fresh=args.output/'reproduced';options.output=fresh;prepare.prepare(options)
            if (fresh/'plan.json').read_bytes()!=(project/'plan.json').read_bytes():
                raise ValueError('whole final plan must reproduce byte for byte')
            for name in (*plan['files'],*v.parent.SUPPORT,'lakefile.toml'):
                if (fresh/name).read_bytes()!=(project/name).read_bytes():
                    raise ValueError('whole final actual closure must reproduce: '+name)
            if v.inputs.dependencies(args.lake_packages,v.REPO/'lake-manifest.json')!=report['dependencies']:
                raise ValueError('fixed Lean dependencies changed')
        if saved!=args.inputs.read_bytes():raise ValueError('input locations changed')
        report.update(complete=True,all_actual_views_diagnosed=True,fresh_full_plan_and_files_reproduced=True)
    except BaseException as error:report['error']=repr(error)
    finally:
        report['seconds']=time.monotonic()-started;report['inputs_sha256']=v.digest(args.inputs)
        (args.output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2),flush=True)
    return not report['complete']

if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    for name in ('inputs','output','lock','lake-packages','lean-mcp'):cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args()
    for name,value in vars(args).items():setattr(args,name,value.resolve(strict=name!='output'))
    raise SystemExit(check(args))
