"""Check the actual V3 owner guards, complete value contracts, and real Rust mutants."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-owner-check')


def module(name,path):
    spec = importlib.util.spec_from_file_location(name,path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


support = module('selection_support',ROOT/HERE/'support.py')
checker = module('selection_original_actual_files',ROOT/HERE.parent/'formal.py')
resolver = module('selection_original_closure',ROOT/'tools/proof_batch.py')
sha = support.sha


def run(output,variant,data,inputs,modes):
    output.mkdir(mode=0o700)
    started = time.monotonic()
    rec = dict(kind='actual-create-owner-check-development',variant=variant,complete=False,
        capture_complete=False,translation_complete=False,proof_complete=False,property_rejected=False,
        production_modified=False,production_adopted=True,original_general_equivalence_proved=False,
        original_full_attachment_proved=False,generated_bodies_modified=False,new_axioms=False,
        native_checks_run=False,formal_acceptance=False,ning_acceptance=False,shared_acceptance=False,
        full_v3=False,commands=[])
    env = dict(os.environ,PYTHONDONTWRITEBYTECODE='1',CARGO_NET_OFFLINE='true',
               LEAN_NUM_THREADS='4',LEAN_STACK_SIZE_KB='65536')

    def command(project,folder,label,argv):
        begin = time.monotonic()
        log = folder/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,
                stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
        rec['commands'].append(dict(label=label,argv=argv,cwd=str(project),exit_code=done.returncode,
            seconds=time.monotonic()-begin,log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec,indent=2)+'\n')
        if done.returncode: raise ValueError('command failed: '+str(log))

    try:
        if support.inventory()!=data: raise ValueError('reviewed actual inputs changed')
        capacity = os.statvfs(ROOT)
        available = capacity.f_bavail * capacity.f_frsize
        required = 64 * 2**30 + 858993459
        rec['resource_admission'] = dict(available_bytes=available, minimum_retained_bytes=64*2**30,
            this_view_budget_bytes=858993459, required_bytes=required, admitted=available>=required)
        if available < required:
            raise ValueError('resource admission refused before capture; original floor and view budget retained')
        rec.update(inputs=inputs,input_modes=modes,
            source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
            source_tree=subprocess.check_output(['git','rev-parse','HEAD^{tree}'],cwd=ROOT,text=True).strip())
        project = output/'source'
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name,digest in inputs.items():
            path = project/name
            path.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(ROOT/name,path)
            if sha(path)!=digest: raise ValueError('source changed while copying: '+name)
        (project/'.tools').symlink_to(support.mapping.tool_root(ROOT),target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(support.mapping.package_root(ROOT),target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        rec['private_source_derivation'] = support.prepare(project,variant,data)
        expected[support.SOURCE] = sha(project/support.SOURCE)
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name,suffix in [('charon','charon/bin/charon'),('aeneas','bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix)!=fixed[name]['binary_sha256']:
                raise ValueError('fixed tool changed: '+name)
        rec['tools'] = fixed
        rec['fixed_library_review'] = support.legacy.fixed_library_review(data)
        raw,ir,binding = support.capture(project,output,command,fixed)
        rec.update(capture_complete=True,original_capture=dict(path=str(ir),sha256=sha(project/ir),binding=binding))
        generated = output/'generated'
        generated.mkdir()
        command(project,output,'aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),'-backend','lean',
            '-abort-on-error','-warnings-as-errors','-split-files','-emit-json',
            '-namespace',support.NAMESPACE,'-subdir',support.LIBRARY,
            '-dest',str(generated),str(project/ir)])
        support.admit_generated(generated,data)
        rec['admission_rejection_audits'] = support.admission_rejections(raw,generated,data)
        del raw
        recipe = data['variants'][variant]
        for name,digest in recipe['generated_raw_files'].items():
            if sha(generated/support.LIBRARY/name)!=digest:
                raise ValueError('unmodified actual generated bytes differ: '+name)
        rec['translation_complete'] = True
        (project/support.GEN).mkdir(parents=True,exist_ok=True)
        for name in ('Types.lean','Funs.lean'):
            shutil.copy2(generated/support.LIBRARY/name,project/support.GEN/name)
            expected[str(support.GEN/name)] = sha(project/support.GEN/name)
        installed = []
        for destination,provider in data['common_proofs'].items():
            (project/destination).parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(ROOT/provider,project/destination)
            expected[destination] = sha(project/destination)
            installed.append(destination)
        for name in data['common_templates']+recipe['templates']:
            target = 'CreateOwnerCheckCounter' if name.endswith('Counter') else name
            destination = str(support.PROOFS/(target+'.lean'))
            shutil.copy2(ROOT/HERE/(name+'.lean.in'),project/destination)
            expected[destination] = sha(project/destination)
            installed.append(destination)
        lake = project/'lakefile.toml'
        lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofStage1OwnerMapping"\nsrcDir = "verification/aeneas/stage1-protocol/owner-mapping"\n')
        expected['lakefile.toml'] = sha(lake)
        seeds = installed+[str(support.GEN/name) for name in
            ('Types.lean','Funs.lean','TypesExternal.lean','FunsExternal.lean')]
        files = sorted(resolver.closure(project,seeds,active_view='lakefile.toml'))
        audits = [a for name in files for a in re.findall(r'^#audit_controller_proof (\S+)$',
            (project/name).read_text(),re.M)]
        if files!=recipe['expected_files'] or audits!=recipe['expected_audits'] or len(audits)!=len(set(audits)):
            raise ValueError('missing, duplicate or changed actual-file/audit inventory')
        rec.update(files={n:sha(project/n) for n in files},audits=audits)
        command(project,output,'build',['lake','build',*['SandboxModel.Lifecycle.Protocol.'+n for n in recipe['seeds']]])
        physical = output/'actual-files'
        physical.mkdir()
        rec['actual'] = checker.actual_files(project,files,physical,env,command)
        if rec['actual']['files']!=rec['files'] or rec['actual']['audits']!=audits:
            raise ValueError('complete checker inventory differs')
        if any(sha(project/n)!=h for n,h in expected.items()):
            raise ValueError('checked private source/proof changed')
        rec['private_derived_inputs'] = {n:h for n,h in expected.items() if inputs.get(n)!=h}
        rec.update(complete=True,proof_complete=True,property_rejected=variant!='actual')
    except BaseException as error:
        rec.update(error=repr(error),traceback=traceback.format_exc())
    finally:
        try:
            rec['original_inputs_unchanged'] = support.parent.base.snapshot(ROOT)==inputs
            rec['original_modes_unchanged'] = all(('100755' if (ROOT/n).stat().st_mode&0o111 else '100644')==m
                for n,m in modes.items())
            if not rec['original_inputs_unchanged'] or not rec['original_modes_unchanged']:
                raise ValueError('original bytes/modes changed')
        except BaseException as error:
            rec.update(complete=False,proof_complete=False,property_rejected=False,final_input_error=repr(error))
        rec['generated_files'] = {str(p.relative_to(output/'generated')):sha(p)
            for p in (output/'generated').rglob('*') if p.is_file()}
        rec['execution_seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(rec,indent=2)+'\n')
    return rec


if __name__=='__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant',choices=('all','actual','omit-local','skip-first-conflict','exclude-owner','retag-exhaustion'),default='all')
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--lock',type=Path,required=True,help='existing shared heavy-proof lock')
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.absolute()
    output.mkdir(parents=True,mode=0o700)
    batch = dict(kind='actual-create-owner-check-development-batch',complete=False,variants={},
        production_adopted=True,original_full_attachment_proved=False,original_general_equivalence_proved=False,
        native_checks_run=False,formal_acceptance=False,ning_acceptance=False,shared_acceptance=False,full_v3=False)
    waiting = time.monotonic()
    try:
        with args.lock.open('r+') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX)
            batch['lock_wait_seconds'] = time.monotonic()-waiting
            started = time.monotonic()
            data = support.inventory()
            resolver.ROOT,resolver.MODULES = ROOT,('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE',str(output/'runtime-cache'))
            profile = resolver.runtime()
            libraries = support.parent.native.registry_module.registry()
            inputs = support.parent.base.snapshot(ROOT)
            modes = {n:('100755' if (ROOT/n).stat().st_mode&0o111 else '100644') for n in inputs}
            variants = list(data['variants']) if args.variant=='all' else [args.variant]
            for variant in variants:
                result = run(output/variant,variant,data,inputs,modes)
                batch['variants'][variant] = dict(proof_complete=result['proof_complete'],
                    property_rejected=result['property_rejected'],result_sha256=sha(output/variant/'result.json'))
                if not result['proof_complete']: raise ValueError('development view failed: '+variant)
            if resolver.runtime()!=profile or support.parent.native.registry_module.registry()!=libraries:
                raise ValueError('fixed runtime/Cargo sources changed')
            batch.update(complete=True,actual_runtime=profile,registry=libraries,
                execution_seconds=time.monotonic()-started)
    except BaseException as error:
        batch.update(error=repr(error),traceback=traceback.format_exc())
    (output/'result.json').write_text(json.dumps(batch,indent=2)+'\n')
    print(json.dumps({k:batch.get(k) for k in ('complete','lock_wait_seconds','execution_seconds','error')}))
    raise SystemExit(not batch['complete'])
