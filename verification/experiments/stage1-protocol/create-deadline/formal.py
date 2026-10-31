"""Re-extract the adopted real deadline helper and check the same full positive/negative contract."""
from pathlib import Path
import argparse
import fcntl
import importlib.util
import json
import os
import re
import shutil
import subprocess
import time
import traceback

ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/create-deadline')


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


support=module('deadline_current_source',ROOT/HERE/'support.py')
binding=module('deadline_full_actual_binding',ROOT/HERE/'binding.py')
checker,resolver,sha=support.load.checker,support.load.resolver,support.sha


def run(output,variant,data,inputs,modes):
    output.mkdir(mode=0o700);started=time.monotonic()
    rec=dict(kind='actual-create-deadline-development',variant=variant,complete=False,capture_complete=False,proof_complete=False,property_rejected=False,production_adopted=True,private_business_replacement=False,generated_bodies_modified=False,new_axioms=False,native_checks_run=False,formal_acceptance=False,shared_acceptance=False,full_parent_proved=False,full_v3=False,commands=[])
    env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',CARGO_NET_OFFLINE='true',LEAN_NUM_THREADS='4',LEAN_STACK_SIZE_KB='65536')
    def command(project,destination,label,argv):
        now=time.monotonic();log=destination/(label+'.log')
        with log.open('xb') as stream:done=subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
        rec['commands'].append(dict(label=label,argv=argv,exit_code=done.returncode,seconds=time.monotonic()-now,log_sha256=sha(log)));(output/'progress.json').write_text(json.dumps(rec,indent=2)+'\n')
        if done.returncode:raise ValueError('compiler/kernel/service failure, not property rejection: '+label)
    try:
        if support.inventory()!=data:raise ValueError('reviewed input changed')
        rec['fixed_library_review']=support.library_review(data)
        disk=os.statvfs(ROOT);available=disk.f_bavail*disk.f_frsize;required=64*2**30+2*858993459
        rec['resource_admission']=dict(available_bytes=available,original_floor_bytes=64*2**30,declared_budget_bytes=2*858993459,required_bytes=required,admitted=available>=required)
        if available<required:raise ValueError('original resource floor refused before compiler')
        rec.update(inputs=inputs,input_modes=modes,source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),source_tree=subprocess.check_output(['git','rev-parse','HEAD^{tree}'],cwd=ROOT,text=True).strip())
        project=output/'source';project.mkdir(mode=0o700);expected=dict(inputs)
        for n,h in inputs.items():
            target=project/n;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/n,target)
            if sha(target)!=h:raise ValueError('source changed during copy: '+n)
        (project/'.tools').symlink_to(support.runtime.tool_root(ROOT),target_is_directory=True);(project/'.lake').mkdir();(project/'.lake/packages').symlink_to(support.runtime.package_root(ROOT),target_is_directory=True);(project/'artifacts/extract-target').mkdir(parents=True)
        recipe=data['variants'][variant];rec['private_source_derivation']=support.mutate(project,recipe['mutation']);expected[support.SOURCE]=sha(project/support.SOURCE)
        for target,provider in [(support.SOURCE,'root.rs.in'),(support.LIB_ROOT,'stdlib-root.rs.in')]:
            with (project/target).open('a') as stream:stream.write((ROOT/HERE/provider).read_text())
            expected[target]=sha(project/target)
        fixed=json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for key,suffix in [('charon','charon/bin/charon'),('aeneas','bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix)!=fixed[key]['binary_sha256']:raise ValueError('fixed tool changed: '+key)
        rec['tools']=fixed
        for label,opaque in [('original',['rusqlite::error::Error']),('request',['rusqlite::error::Error','sandproof_protocol::ContractError'])]:
            ir=Path('artifacts')/(label+'.llbc');args=[str(support.runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'),'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default']
            for n in support.INCLUDE:args+=['--include',n]
            for n in opaque:args+=['--opaque',n]
            for n in support.ROOTS:args+=['--start-from',n]
            args+=['--dest-file',str(support.runtime.VIRTUAL/ir),'--','--manifest-path','verification/experiments/stage1-protocol/inbox-record/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']]
            command(project,output,label+'-charon',support.wrapper(project)+args)
            raw=json.loads((project/ir).read_text());admitted=binding.admit(raw,support.ROOTS,support.INCLUDE,opaque)
            if label=='original':
                enum=support.error_binding.contract_error(raw);old=json.loads((project/'verification/aeneas/llbc/sandproof_foundations.llbc').read_text());assert enum==support.error_binding.contract_error(old);rec['complete_original_contract_error']=enum
            rec.setdefault('captures',{})[label]=dict(path=str(ir),sha256=sha(project/ir),binding=admitted)
            generated=output/('generated-'+label);generated.mkdir();namespace='SandproofStage1Request'
            command(project,output,label+'-aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),'-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json','-namespace',namespace+'Rust','-subdir',namespace,'-dest',str(generated),str(project/ir)])
            translated=json.loads((generated/'translation.json').read_text());rec.setdefault('translation',{})[label]=translated;rec.setdefault('raw_generated',{})[label]={str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()}
            if label=='request':
                actual=[{k:f[k] for k in ('lean_name','rust_name','is_opaque','lean_file')} for f in translated['functions']]
                if actual!=recipe['generated_functions']:raise ValueError('actual generated function/opaque closure changed')
                external={f['lean_name'].removeprefix('SandproofStage1RequestRust.') for f in translated['functions'] if f['is_opaque']};assert external==set(support.SYMBOLS)
                for leaf in ['Types.lean','Funs.lean']:
                    target=project/support.GEN/leaf;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(generated/namespace/leaf,target);expected[str(support.GEN/leaf)]=sha(target)
        rec['capture_complete']=True;installed=support.install(project,variant,data);expected.update(installed)
        seeds=[str(support.PROOFS/(n+'.lean')) for n in recipe['seeds']];files=sorted(resolver.closure(project,seeds,active_view='lakefile.toml'));audits=[a for n in files for a in re.findall(r'^#audit_controller_proof (\S+)$',(project/n).read_text(),re.M)]
        if files!=recipe['expected_files'] or audits!=recipe['expected_audits'] or len(audits)!=len(set(audits)):raise ValueError('actual file/audit inventory differs')
        if variant!='actual' and str(support.PROOFS/'CreateDeadline.lean') in files:raise ValueError('wrong view imports correct helper correspondence')
        rec.update(files={n:sha(project/n) for n in files},audits=audits,common_contract_sha256=sha(project/support.PROOFS/'CreateDeadlineModel.lean'))
        command(project,output,'build',['lake','build',*['SandboxModel.Lifecycle.Protocol.'+n for n in recipe['seeds']]])
        physical=output/'actual-files';physical.mkdir();rec['actual']=checker.actual_files(project,files,physical,env,command)
        assert rec['actual']['files']==rec['files'] and rec['actual']['audits']==audits
        if any(sha(project/n)!=h for n,h in expected.items()):raise ValueError('private source/generation/proof changed')
        if any(('100755' if (project/n).stat().st_mode&0o111 else '100644')!=m for n,m in modes.items()):raise ValueError('copied source mode changed')
        rec.update(private_derived_inputs={n:h for n,h in expected.items() if inputs.get(n)!=h},complete=True,proof_complete=True,property_rejected=variant!='actual')
    except BaseException as error:rec.update(error=repr(error),traceback=traceback.format_exc())
    finally:
        rec['original_inputs_unchanged']=support.load.base.snapshot(ROOT)==inputs;rec['original_modes_unchanged']=all(('100755' if (ROOT/n).stat().st_mode&0o111 else '100644')==m for n,m in modes.items())
        if not rec['original_inputs_unchanged'] or not rec['original_modes_unchanged']:rec.update(complete=False,proof_complete=False,property_rejected=False)
        rec['execution_seconds']=time.monotonic()-started;(output/'result.json').write_text(json.dumps(rec,indent=2)+'\n')
    return rec


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--lock',type=Path,required=True);parser.add_argument('--variant',choices=('all','actual','unchecked-cast','wrong-error'),default='all');args=parser.parse_args();os.umask(0o077)
    output=args.output.absolute();output.mkdir(parents=True,mode=0o700);batch=dict(kind='actual-create-deadline-development-batch',complete=False,variants={},native_checks_run=False,formal_acceptance=False,shared_acceptance=False,full_parent_proved=False,full_v3=False);waiting=time.monotonic()
    try:
        with args.lock.open('r+') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX);batch['lock_wait_seconds']=time.monotonic()-waiting;started=time.monotonic();data=support.inventory();resolver.ROOT,resolver.MODULES=ROOT,('stage1-protocol',);os.environ.setdefault('SANDPROOF_FORMAL_CACHE',str(output/'runtime-cache'));runtime=resolver.runtime();registry=support.load.native.registry_module.registry();inputs=support.load.base.snapshot(ROOT);modes={n:('100755' if (ROOT/n).stat().st_mode&0o111 else '100644') for n in inputs};contracts=set()
            for variant in data['variants'] if args.variant=='all' else [args.variant]:
                result=run(output/variant,variant,data,inputs,modes);batch['variants'][variant]=dict(proof_complete=result['proof_complete'],property_rejected=result['property_rejected'],result_sha256=sha(output/variant/'result.json'))
                if not result['proof_complete']:raise ValueError('actual development view failed: '+variant)
                contracts.add(result['common_contract_sha256'])
            assert len(contracts)==1
            if resolver.runtime()!=runtime or support.load.native.registry_module.registry()!=registry:raise ValueError('fixed runtime/library changed')
            batch.update(complete=True,actual_runtime=runtime,registry=registry,common_contract_sha256=next(iter(contracts)),execution_seconds=time.monotonic()-started)
    except BaseException as error:batch.update(error=repr(error),traceback=traceback.format_exc())
    (output/'result.json').write_text(json.dumps(batch,indent=2)+'\n');print(json.dumps({k:batch.get(k) for k in ['complete','lock_wait_seconds','execution_seconds','error']}),flush=True);raise SystemExit(not batch['complete'])
