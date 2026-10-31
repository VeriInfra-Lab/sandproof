"""Reextract current Rust and check all files for six complete digest uses."""
from pathlib import Path
import argparse
import fcntl
import importlib.util
import json
import os
import re
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/digest-uses')
spec = importlib.util.spec_from_file_location('current_digest_uses_support',ROOT/HERE/'support.py')
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)
sha = support.sha


def run(output,variant,data,prepare_only):
    output.mkdir(mode=0o700)
    started = time.monotonic()
    record = dict(kind='current-adopted-six-digest-uses',variant=variant,complete=False,
        prepared=False,commands=[],property_rejected=False,native_checks_run=False,
        ordinary_checks_run=False,shared_acceptance=False,full_v3=False,
        library_implementation_refinement=False,public_dependency_adopted=False)
    env = dict(os.environ,PYTHONDONTWRITEBYTECODE='1',LEAN_NUM_THREADS='4',
        LEAN_STACK_SIZE_KB='65536',CARGO_NET_OFFLINE='true')

    def command(project,folder,label,argv):
        begin = time.monotonic()
        log = folder/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,
                stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
        record['commands'].append(dict(label=label,argv=argv,cwd=str(project),
            exit_code=done.returncode,seconds=time.monotonic()-begin,log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
        if done.returncode:
            raise ValueError('compile/kernel/service failure is not a counterexample: '+label)

    try:
        assert support.inventory() == data
        assert os.sched_getaffinity(0) <= {0,1,2,3}
        disk = os.statvfs(ROOT)
        assert disk.f_bavail*disk.f_frsize >= 64*2**30+2*4*2**30
        captured = support.capture.run(output/'capture',variant)
        record.update(capture_result_sha256=sha(output/'capture/result.json'),
            capture_complete=captured['capture_complete'],
            translation_complete=captured['translation_complete'],
            source_commit=captured['source_commit'],inputs=captured['inputs'],
            input_modes=captured['input_modes'])
        if not captured['capture_complete'] or not captured['translation_complete']:
            raise ValueError('actual adopted Rust capture failed')
        project = output/'capture/source'
        generated = output/'capture/generated'
        recipe = data['variants'][variant]
        support.generated_inventory(generated,recipe)
        expected = dict(captured['inputs'])
        for row in captured['instrumentation']:
            expected[row['source']] = row['derived_sha256']
        if captured['source_mutation']:
            mutation = captured['source_mutation']
            expected[mutation['mutation']['source']] = mutation['wrong_sha256']
            # The source_work thin root is appended after this mutation.
            for row in captured['instrumentation']:
                expected[row['source']] = row['derived_sha256']
        expected.update(support.install(project,variant,data,generated))
        types = (project/support.GEN/'Types.lean').read_text()
        for name,fields in data['own_structure_fields'].items():
            begin = types.index('structure '+name)
            end = types.find('\n/--',begin)
            found = re.findall(r'^  (\w+) :',types[begin:end if end>=0 else len(types)],re.M)
            if found != fields:
                raise ValueError('actual receiver carrier lost fields: '+name)
        files,audits = support.proof_inventory(project,recipe['seeds'])
        if files != recipe['expected_files'] or audits != recipe['expected_audits']:
            raise ValueError('complete current file/audit closure differs')
        if variant != 'actual' and any(Path(n).stem in {'DigestUses','DigestUsesWitnesses'} for n in files):
            raise ValueError('wrong view imports correct correspondence')
        record.update(project=str(project),files={n:sha(project/n) for n in files},
            audits=audits,generated_bodies_modified=False,no_axiom_template_imported=True,
            common_contract_sha256=sha(project/support.PROOFS/'DigestUsesModel.lean'),
            common_values_sha256=sha(project/support.PROOFS/'DigestUsesValues.lean'),
            common_library_sha256=sha(project/support.GEN/'FunsExternal.lean'),
            common_carriers_sha256=sha(project/support.GEN/'TypesExternal.lean'))
        command(project,output,'build',['lake','build',*[
            'SandboxModel.Lifecycle.Protocol.'+name for name in recipe['seeds']]])
        record['prepared'] = True
        if not prepare_only:
            actual = output/'actual-files'
            actual.mkdir()
            record['actual'] = support.load.checker.actual_files(project,files,actual,env,command)
            assert record['actual']['files'] == record['files']
            assert record['actual']['audits'] == audits
            support.reader._saved_physical_view(record['actual'],project,
                actual/'physical-files',record['commands'])
            record.update(complete=True,property_rejected=variant!='actual',
                original_kernel_service_dependencies_read_back=True)
        if any(sha(project/n) != value for n,value in expected.items()):
            raise ValueError('private current inputs or generated bodies changed')
        if any(('100755' if (project/n).stat().st_mode & 0o111 else '100644') != mode
                for n,mode in captured['input_modes'].items()):
            raise ValueError('private source mode changed')
        record['private_derived_inputs'] = {n:h for n,h in expected.items()
            if captured['inputs'].get(n) != h}
    except BaseException as error:
        record.update(complete=False,property_rejected=False,error=repr(error),traceback=traceback.format_exc())
    record['execution_seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


def batch(output,prepare_only):
    started = time.monotonic()
    record = dict(kind='current-adopted-digest-uses-complete-development-batch',complete=False,
        prepared=False,variants={},prepare_only=prepare_only,native_checks_run=False,
        ordinary_checks_run=False,shared_acceptance=False,full_v3=False,
        public_dependency_adopted=False,library_implementation_refinement=False)
    try:
        data = support.inventory()
        resolver = support.load.resolver
        resolver.ROOT,resolver.MODULES = ROOT,('stage1-protocol',)
        os.environ.setdefault('SANDPROOF_FORMAL_CACHE',str(output/'runtime-cache'))
        before = resolver.runtime()
        registry = support.load.native.registry_module.registry()
        inputs = support.base.snapshot(ROOT)
        record.update(fixed_runtime_before=before,inputs=inputs)
        leaf = support.capture.run(output/'leaf-binding','leaf-binding')
        if not leaf['capture_complete'] or not leaf['translation_complete']:
            raise ValueError('current fixed leaf capture failed')
        ir = output/'leaf-binding/source/artifacts/digest-array32.llbc'
        assert ir.stat().st_size <= 16*2**20
        record['source_binding'] = support.binding.admit(json.loads(ir.read_text()),
            json.loads((ROOT/HERE/'fixed-interfaces.json').read_text()))
        record['source_binding_llbc_sha256'] = sha(ir)
        contracts,values,libraries,carriers = set(),set(),set(),set()
        for variant in data['variants']:
            view = run(output/variant,variant,data,prepare_only)
            record['variants'][variant] = dict(view,result_sha256=sha(output/variant/'result.json'))
            if not (view['prepared'] if prepare_only else view['complete']):
                raise ValueError('current digest view failed: '+variant)
            for collection,key in [(contracts,'common_contract_sha256'),(values,'common_values_sha256'),
                (libraries,'common_library_sha256'),(carriers,'common_carriers_sha256')]:
                collection.add(view[key])
        assert len(contracts) == len(values) == len(libraries) == len(carriers) == 1
        assert support.base.snapshot(ROOT) == inputs
        assert resolver.runtime() == before
        assert support.load.native.registry_module.registry() == registry
        record.update(complete=not prepare_only,prepared=True,fixed_runtime_after=before,
            same_complete_contract_and_value_library_carriers=True,
            source_worktree_inputs_unchanged=True,registry=registry)
    except BaseException as error:
        record.update(complete=False,error=repr(error),traceback=traceback.format_exc())
    record['execution_seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--lock',type=Path,required=True)
    parser.add_argument('--lock-fd',type=int)
    parser.add_argument('--prepare-only',action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.absolute()
    output.mkdir(parents=True,mode=0o700)
    inherited = args.lock_fd is not None
    lock = os.fdopen(os.dup(args.lock_fd),'r+') if inherited else args.lock.open('r+')
    with lock:
        observed,expected = os.fstat(lock.fileno()),args.lock.stat()
        if (observed.st_dev,observed.st_ino) != (expected.st_dev,expected.st_ino):
            raise ValueError('inherited digest lock is a different file')
        fcntl.flock(lock,fcntl.LOCK_EX | (fcntl.LOCK_NB if inherited else 0))
        result = batch(output,args.prepare_only)
    print(json.dumps({k:result.get(k) for k in ['complete','prepared','execution_seconds','error']}),flush=True)
    raise SystemExit(not (result['prepared'] if args.prepare_only else result['complete']))
