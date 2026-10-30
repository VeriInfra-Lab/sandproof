"""Check information loss in the unchanged fixed format-argument model.

This is a library-model adequacy proof, not an attachment or identity proof.
"""
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
HERE = Path('verification/experiments/stage1-protocol/create-attachment')
PROOF = 'SandboxModel/Lifecycle/Protocol/FixedFmtArguments.lean'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name,path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


parent = module('format_review_existing_load',ROOT/HERE.parent/'inbox-load/formal.py')
mapping = module('format_review_existing_mapping',ROOT/HERE.parent/'formal_runtime.py')
checker = module('format_review_original_checker',ROOT/HERE.parent/'formal.py')
resolver = module('format_review_original_closure',ROOT/'tools/proof_batch.py')
sha = parent.sha


def run(output, lock_path):
    output.mkdir(parents=True,mode=0o700)
    project = output/'source'
    rec = dict(kind='fixed-format-model-information-loss',complete=False,commands=[],
        original_attachment_proved=False,original_identity_proved=False,
        tool_changed=False,generated_bodies_modified=False,new_axioms=False,
        generated_axiom_templates_imported=False,native_checks_run=False,
        formal_acceptance=False,ning_acceptance=False,shared_acceptance=False,full_v3=False)
    env = dict(os.environ,PYTHONDONTWRITEBYTECODE='1',LEAN_NUM_THREADS='4',LEAN_STACK_SIZE_KB='65536')

    def command(project, folder, label, argv):
        log = folder/(label+'.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            done = subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,
                stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
        rec['commands'].append(dict(label=label,argv=argv,cwd=str(project),
            exit_code=done.returncode,seconds=time.monotonic()-begin,log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec,indent=2)+'\n')
        if done.returncode:
            raise ValueError('complete proof command failed: '+str(log))

    waiting = time.monotonic()
    try:
        with lock_path.open('r+') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX)
            rec['lock_wait_seconds'] = time.monotonic()-waiting
            started = time.monotonic()
            inv = json.loads((ROOT/HERE/'inventory.json').read_text())['format_review']
            if sha(Path(__file__))!=inv['driver_sha256'] or sha(ROOT/HERE/'FixedFmtArguments.lean.in')!=inv['proof_sha256']:
                raise ValueError('reviewed model-adequacy driver/proof changed')
            for name,digest in inv['fixed_models'].items():
                paths = [ROOT/'.tools/aeneas/backends/lean/Aeneas/Std'/name,
                         ROOT/'.lake/packages/aeneas/backends/lean/Aeneas/Std'/name]
                if any(sha(path)!=digest for path in paths):
                    raise ValueError('actual fixed model/package changed: '+name)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE',str(output/'runtime-cache'))
            resolver.ROOT,resolver.MODULES = ROOT,('stage1-protocol',)
            original_runtime = resolver.runtime()
            inputs = parent.base.snapshot(ROOT)
            rec.update(inputs=inputs,
                input_modes={n:('100755' if (ROOT/n).stat().st_mode&0o111 else '100644') for n in inputs},
                actual_runtime=original_runtime,
                source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
                source_tree=subprocess.check_output(['git','rev-parse','HEAD^{tree}'],cwd=ROOT,text=True).strip())
            project.mkdir(mode=0o700)
            for name,digest in inputs.items():
                path = project/name
                path.parent.mkdir(parents=True,exist_ok=True)
                shutil.copy2(ROOT/name,path)
                if sha(path)!=digest:
                    raise ValueError('source changed during copy: '+name)
            (project/'.tools').symlink_to(mapping.tool_root(ROOT),target_is_directory=True)
            (project/'.lake').mkdir()
            (project/'.lake/packages').symlink_to(mapping.package_root(ROOT),target_is_directory=True)
            (project/'artifacts').mkdir(exist_ok=True)
            shutil.copy2(ROOT/HERE/'FixedFmtArguments.lean.in',project/PROOF)
            expected = dict(inputs)
            expected[PROOF] = sha(project/PROOF)
            files = sorted(resolver.closure(project,[PROOF],active_view='lakefile.toml'))
            audits = [a for name in files for a in re.findall(r'^#audit_controller_proof (\S+)$',
                (project/name).read_text(),re.M)]
            if files!=inv['expected_files'] or audits!=inv['expected_audits'] or len(set(audits))!=len(audits):
                raise ValueError('changed/missing physical proof or audit inventory')
            rec.update(files={n:sha(project/n) for n in files},audits=audits)
            command(project,output,'build',['lake','build','SandboxModel.Lifecycle.Protocol.FixedFmtArguments'])
            actual = output/'actual-files'
            actual.mkdir()
            rec['actual'] = checker.actual_files(project,files,actual,env,command)
            if rec['actual']['files']!=rec['files'] or rec['actual']['audits']!=audits:
                raise ValueError('actual complete checker closure differs')
            if any(sha(project/n)!=h for n,h in expected.items()):
                raise ValueError('private proof/source changed')
            if resolver.runtime()!=original_runtime:
                raise ValueError('actual fixed runtime changed')
            rec['complete'] = True
    except BaseException as error:
        rec.update(error=repr(error),traceback=traceback.format_exc())
    finally:
        try:
            if 'inputs' in locals():
                rec['original_inputs_unchanged'] = inputs==parent.base.snapshot(ROOT)
                rec['original_modes_unchanged'] = all(
                    ('100755' if (ROOT/n).stat().st_mode&0o111 else '100644')==m
                    for n,m in rec['input_modes'].items())
                if not rec['original_inputs_unchanged'] or not rec['original_modes_unchanged']:
                    raise ValueError('original bytes/modes changed')
        except BaseException as error:
            rec.update(complete=False,final_input_error=repr(error))
        rec['execution_seconds'] = time.monotonic()-started if 'started' in locals() else None
        (output/'result.json').write_text(json.dumps(rec,indent=2)+'\n')
    return rec


if __name__=='__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--lock',type=Path,required=True,help='existing shared heavy-proof lock')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(),args.lock)
    print(json.dumps({k:result.get(k) for k in ('complete','lock_wait_seconds','execution_seconds','error')}))
    raise SystemExit(not result['complete'])
