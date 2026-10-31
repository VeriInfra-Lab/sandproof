"""Current actual Create and real mutants under one complete contract; no acceptance seal."""
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
HERE=Path('verification/experiments/stage1-protocol/create-attachment-flow')


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


support=module('actual_create_flow_support',ROOT/HERE/'support.py')
load,sha,runtime=support.load,support.sha,support.runtime


def run(output,lock_path,selected_variant=None,prepare_only=False,lock_fd=None):
    output.mkdir(mode=0o700,parents=True)
    record=dict(kind='actual-create-complete-flow-development',complete=False,commands=[],variants={},
        production_adopted=True,private_business_replacement=False,standard_instances_proved=False,
        same_transaction_or_recovery_proved=False,native_checks_run=False,formal_acceptance=False,
        shared_acceptance=False,full_v3=False)
    env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',CARGO_NET_OFFLINE='true',LEAN_NUM_THREADS='4',LEAN_STACK_SIZE_KB='65536')

    def command(project,folder,label,argv):
        log=folder/(label+'.log');started=time.monotonic()
        with log.open('xb') as stream:
            done=subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
        record['commands'].append(dict(folder=str(folder),cwd=str(project),label=label,argv=argv,exit_code=done.returncode,
            seconds=time.monotonic()-started,log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
        if done.returncode:raise ValueError('command failed; not a property rejection: '+label)

    def capture(project,folder,label,roots,include,opaque,namespace,primitive=False):
        target=folder/('generated-'+label);target.mkdir();ir=project/'artifacts'/(label+'.llbc')
        support.load.admission.ROOTS=support.ROOTS
        support.load.admission.INCLUDE=support.INCLUDE
        support.load.admission.OPAQUE=opaque
        args=[str(runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'),'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default']
        for name in include:args+=['--include',name]
        for name in opaque:args+=['--opaque',name]
        for name in roots:args+=['--start-from',name]
        args+=['--dest-file',str(runtime.VIRTUAL/'artifacts'/(label+'.llbc')),'--','--manifest-path',
            'verification/experiments/stage1-protocol/inbox-record/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']]
        command(project,folder,label+'-charon',support.wrapper(project)+args)
        raw=json.loads(ir.read_text());admitted=load.admit_ir(raw,opaque=opaque,primitive=primitive)
        bindings={}
        if not primitive:
            bindings['whole_original_object']=support.capacity.binding.admit(raw)
            bindings['all_parent_arguments']=support.binding.admit(raw)
            bindings['complete_paths_and_map']=support.path_binding.admit(raw,support.error_binding)
        if label=='original':
            support.owner.ROOTS=support.ROOTS;support.owner.INCLUDE=support.INCLUDE
            bindings['original_owner']=support.owner.admit_ir(raw,opaque)
            original=support.error_binding.contract_error(raw)
            foundation=json.loads((ROOT/'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
            if original!=support.error_binding.contract_error(foundation):raise ValueError('full original ContractError differs')
            bindings['complete_original_contract_error']=original
        command(project,folder,label+'-aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),'-backend','lean',
            '-abort-on-error','-warnings-as-errors','-split-files','-emit-json','-namespace',namespace+'Rust',
            '-subdir',namespace,'-dest',str(target),str(ir)])
        translated=json.loads((target/'translation.json').read_text())
        external={f['lean_name'] for f in translated['functions'] if f['is_opaque']}
        if primitive:load.admit_generated(target,'primitive')
        else:
            if external!=set(data['external_functions']):raise ValueError('changed finite external function inventory')
            required=['create_attachment_install','create_owner_present','create_attachment_selection','codec.load_using',
                'check_existing_create_owner','check_new_create_owner','create_owner_successor','owner_for',
                'attach_create_using','codec.save_using']
            for name in required:
                suffix=support.PREFIX.replace('::','.')+name
                found=[f for f in translated['functions'] if f['lean_name'].endswith(suffix)]
                if len(found)!=1 or found[0]['is_opaque']:raise ValueError('business function absent or external: '+name)
            converted=[f for f in translated['functions'] if f['lean_name'].endswith('I64.Insts.CoreConvertTryFromU64TryFromIntError.try_from')]
            if len(converted)!=1 or converted[0]['is_opaque']:raise ValueError('actual Std conversion not transparent')
        return dict(ir_sha256=sha(ir),admission=admitted,bindings=bindings,
            generated_files={str(p.relative_to(target)):sha(p) for p in target.rglob('*') if p.is_file()},
            translation=translated)

    def path_capture(project,folder):
        recipe=data['path_provider'];ir=project/'artifacts/std-path.llbc';target=folder/'generated-std-path';target.mkdir()
        args=[str(runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'),'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default']
        for name in recipe['capture']['include']:args+=['--include',name]
        for name in recipe['capture']['roots']:args+=['--start-from',name]
        args+=['--dest-file',str(runtime.VIRTUAL/'artifacts/std-path.llbc'),'--','--manifest-path',recipe['capture']['manifest'],'--lib','--locked','--offline','--target',fixed['target']]
        command(project,folder,'std-path-charon',support.wrapper(project)+args)
        raw=json.loads(ir.read_text());opts=raw['translated']['options']
        if (raw['has_errors'] is not False or opts['start_from']!=recipe['capture']['roots']
                or opts['include']!=recipe['capture']['include'] or opts['opaque'] or opts['exclude']
                or opts['skip_borrowck'] or opts['no_typecheck'] or opts['monomorphize'] or opts['remove_unused_clauses']):
            raise ValueError('changed original Std path capture')
        command(project,folder,'std-path-aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),'-backend','lean',
            '-abort-on-error','-warnings-as-errors','-split-files','-emit-json','-namespace','SandproofUnixPathRust',
            '-subdir','SandproofUnixPath','-dest',str(target),str(ir)])
        actual={str(p.relative_to(target)):sha(p) for p in target.rglob('*.lean')}
        if actual!=recipe['generated_files']:raise ValueError('unchanged Std provider generation differs')
        return dict(generated_files=actual,ir_sha256=sha(ir),original_mathematics_reused=True)

    waiting=time.monotonic()
    try:
        with (lock_path.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd),'r+')) as lock:
            expected,actual=lock_path.stat(),os.fstat(lock.fileno())
            if (expected.st_dev,expected.st_ino)!=(actual.st_dev,actual.st_ino):
                raise ValueError('inherited lock is not the exact coordination lock')
            fcntl.flock(lock,fcntl.LOCK_EX | (fcntl.LOCK_NB if lock_fd is not None else 0))
            record['coordination_lock']=dict(inherited=lock_fd is not None,device=actual.st_dev,inode=actual.st_ino)
            record['lock_wait_seconds']=time.monotonic()-waiting;started=time.monotonic()
            disk=os.statvfs(ROOT);available=disk.f_bavail*disk.f_frsize;required=64*2**30+2*858993459
            record['resource_admission']=dict(available_bytes=available,original_floor_bytes=64*2**30,declared_budget_bytes=2*858993459,required_bytes=required,admitted=available>=required)
            if available<required:raise ValueError('original resource floor refused')
            data=support.inventory();fixed=json.loads((ROOT/'verification/foundations-environment.json').read_text())
            for name,suffix in [('charon','charon/bin/charon'),('aeneas','bin/aeneas')]:
                if sha(ROOT/'.tools/aeneas'/suffix)!=fixed[name]['binary_sha256']:raise ValueError('fixed tool differs: '+name)
            # Fixed exact use-point sources are conditions, never a whole library oracle.
            standard={}
            for kind,toolchain in [('production','1.97.0'),('extraction',fixed['extraction_frontend'])]:
                sysroot=Path(subprocess.check_output(['rustup','run',toolchain,'rustc','--print','sysroot'],text=True).strip())
                standard[kind]={}
                for n,h in data['standard_sources'][kind].items():
                    path=sysroot/'lib/rustlib/src/rust/library'/n
                    if sha(path)!=h:raise ValueError('fixed selected Std source differs: '+kind+'/'+n)
                    standard[kind][n]=dict(path=str(path),sha256=h)
            inputs=load.base.snapshot(ROOT);modes={n:('100755' if (ROOT/n).stat().st_mode&0o111 else '100644') for n in inputs}
            record.update(source_commit=subprocess.check_output(['git','rev-parse','HEAD'],text=True,cwd=ROOT).strip(),inputs=inputs,input_modes=modes,standard_sources=standard)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE',str(output/'runtime-cache'))
            load.resolver.ROOT,load.resolver.MODULES=ROOT,('stage1-protocol',);tools=load.resolver.runtime()
            record['fixed_runtime_before']=tools
            for variant in ([selected_variant] if selected_variant else ['actual','skip-save','eager-world']):
                folder=output/variant;project=folder/'source';project.mkdir(mode=0o700,parents=True);expected=dict(inputs)
                for n,h in inputs.items():
                    target=project/n;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/n,target)
                    if sha(target)!=h:raise ValueError('source changed while copying: '+n)
                (project/'.tools').symlink_to(runtime.tool_root(ROOT),target_is_directory=True);(project/'.lake').mkdir();(project/'.lake/packages').symlink_to(runtime.package_root(ROOT),target_is_directory=True);(project/'artifacts/extract-target').mkdir(parents=True)
                derivation,changed=support.prepare(project,variant,data);expected.update(changed)
                row=dict(complete=False,derivation=derivation,prepared_inputs=changed,captures={},property_rejected=False)
                record['variants'][variant]=row;(output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
                for label,roots,include,opaque,namespace,primitive in [
                    ('original',support.ROOTS,support.INCLUDE,['rusqlite::error::Error'],'SandproofStage1Request',False),
                    ('request',support.ROOTS,support.INCLUDE,['rusqlite::error::Error','sandproof_protocol::ContractError'],'SandproofStage1Request',False),
                    ('primitive',load.admission.PRIMITIVE_ROOTS,load.admission.PRIMITIVE_INCLUDE,['rusqlite::error::Error'],'SandproofStage1Graphic',True)]:
                    row['captures'][label]=capture(project,folder,label,roots,include,opaque,namespace,primitive)
                if variant=='actual' or selected_variant is not None:
                    record['std_path_capture']=path_capture(project,folder);path_provider=folder/'generated-std-path/SandproofUnixPath'
                else:
                    # Independent unchanged standalone crate and exact fixed Std inputs.
                    record['variants'][variant]['std_path_reuse']=dict(from_variant='actual',original_execution_identity_preserved=True)
                installed=support.materialize(project,variant,data)
                for origin,destination in [(folder/'generated-request/SandproofStage1Request',support.GEN),(folder/'generated-primitive/SandproofStage1Graphic',support.GRAPHIC),(path_provider,support.PATH_GEN)]:
                    for name in ['Types.lean','Funs.lean']:
                        target=project/destination/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(origin/name,target);installed[str(destination/name)]=sha(target)
                expected.update(installed)
                if any(sha(project/n)!=h for n,h in expected.items()):raise ValueError('prepared input changed after capture/model installation')
                seeds=data['variants'][variant]['seed_files'];files=sorted(load.resolver.closure(project,seeds,active_view='lakefile.toml'))
                audits=[x for n in files for x in re.findall(r'^#audit_controller_proof (\S+)$',(project/n).read_text(),re.M)]
                if sorted(audits)!=sorted(data['variants'][variant]['audits']) or len(audits)!=len(set(audits)):raise ValueError('actual proof audit inventory changed')
                row.update(installed=installed,files={n:sha(project/n) for n in files},audits=audits)
                command(project,folder,'build',['lake','build']+[n[:-5].replace('/','.') for n in files if n.startswith('SandboxModel/')])
                if prepare_only:
                    row.update(prepared=True,complete=False)
                    continue
                physical=folder/'actual-files';physical.mkdir();row['actual']=load.checker.actual_files(project,files,physical,env,command)
                if row['actual']['files']!=row['files'] or row['actual']['audits']!=audits:raise ValueError('actual physical inventory differs')
                row.update(complete=True,property_rejected=variant!='actual');(output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
            completed_runtime=load.resolver.runtime()
            if completed_runtime!=tools or load.base.snapshot(ROOT)!=inputs:raise ValueError('source or fixed runtime changed')
            record['fixed_runtime_after']=completed_runtime
            for kind,values in standard.items():
                for n,info in values.items():
                    if sha(Path(info['path']))!=info['sha256']:raise ValueError('Std source changed during proof')
            if any(('100755' if (ROOT/n).stat().st_mode&0o111 else '100644')!=m for n,m in modes.items()):raise ValueError('source execution mode changed')
            record.update(complete=not prepare_only,prepared=prepare_only,execution_seconds=time.monotonic()-started,selected_variant=selected_variant,all_three_variants_complete=not prepare_only and selected_variant is None)
    except BaseException as error:
        record.update(error=repr(error),traceback=traceback.format_exc())
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--lock',type=Path,required=True)
    parser.add_argument('--lock-fd',type=int,help='Inherited open description of the exact already-held outer coordination lock')
    parser.add_argument('--variant',choices=['actual','skip-save','eager-world'])
    parser.add_argument('--prepare-only',action='store_true',help='Diagnostic compile/extract/build only; never a complete proof result')
    args=parser.parse_args();result=run(args.output.absolute(),args.lock.resolve(strict=True),args.variant,args.prepare_only,args.lock_fd)
    print(json.dumps({k:result.get(k) for k in ['complete','execution_seconds','error']}))
    raise SystemExit(not (result['complete'] or (args.prepare_only and result.get('prepared'))))
