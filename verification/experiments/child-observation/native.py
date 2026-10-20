"""Original and candidate real Child/Slot regressions. Ordinary-test node only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from prepare import prepare,sha
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[2]))
from tools.check_child_observation import fresh_native_artifact, check_native

def command(args,source,output,tag,expected=0):
    log=output/(tag+'.log')
    started=time.monotonic()
    with log.open('xb') as stream:
        code=subprocess.run(args,cwd=source,stdout=stream,stderr=subprocess.STDOUT,timeout=600).returncode
    record={'command':args,'exit_code':code,'expected_exit_code':expected,
            'log_sha256':sha(log),'seconds':time.monotonic()-started}
    (output/(tag+'.json')).write_text(json.dumps(record,indent=2)+'\n')
    if code!=expected: raise ValueError('unexpected command result: '+str(log))
    return record

def attach_tests(source,variant):
    p=source/'sandbox-runtime/src/child_process.rs'
    p.write_text(p.read_text()+'\n#[cfg(test)]\nmod observation_native_tests;\n')
    files=[('native-os-tests.rs','sandbox-runtime/src/child_process/observation_native_tests.rs')]
    if variant!='original':
        p=source/'sandbox-runtime/src/child_process/reaper.rs'
        p.write_text(p.read_text()+'\n#[cfg(test)]\nmod observation_fault_tests;\n')
        files.append(('native-fault-tests.rs','sandbox-runtime/src/child_process/reaper/observation_fault_tests.rs'))
    for name,target in files:
        p=source/target;p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(HERE/name,p)
    return {p:sha(source/p) for _,p in files}

def run(archive,output):
    if output.exists() or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('fresh output and dedicated native target required')
    output.mkdir(parents=True,mode=0o700)
    start=time.monotonic();inputs={p.name:sha(p) for p in HERE.iterdir() if p.is_file()}
    record={'kind':'child-observation-native-module-not-full-acceptance','variants':{},
            'target_dir':str(Path(os.environ['CARGO_TARGET_DIR']).resolve())}
    record['runtime']={name:subprocess.check_output(args,text=True).strip() for name,args in {
        'rustc':['rustc','--version','--verbose'],'cargo':['cargo','--version'],
        'rustfmt':['rustfmt','--version']}.items()}
    source=None
    try:
        for variant in ['original','correct','kill-success-release']:
            case=output/variant;case.mkdir();source=case/'source'
            materialized=prepare(archive,source,variant)
            tests=attach_tests(source,variant)
            entry={'materialized':materialized,'test_sources':tests,'commands':[]}
            # Format only test attachments; production/candidate bytes are immutable.
            entry['commands'].append(command(['rustfmt','--edition','2024']+[str(source/p) for p in tests],source,case,'format-test-attachments'))
            entry['formatted_test_sources']={p:sha(source/p) for p in tests}
            entry['commands'].append(command(['cargo','clean','-p','sandbox-runtime'],source,case,'clean-owned-package'))
            args=['cargo','test','-p','sandbox-runtime','--lib','--no-run','--message-format=json','--locked','--offline']
            entry['commands'].append(command(args,source,case,'compile-real-source'))
            artifact=fresh_native_artifact(case/'compile-real-source.log',Path(record['target_dir']),source)
            entry['actual_artifact']=artifact
            binary=case/'actual-tests'
            shutil.copy2(artifact['executable'],binary)
            entry['binary_sha256']=sha(binary)
            if variant!='kill-success-release':
                entry['commands'].append(command([str(binary),'child_process::','--test-threads=1'],source,case,'actual-child-tests'))
                text=(case/'actual-child-tests.log').read_text()
                if 'actual_stop_observes_original_child_before_success ... ok' not in text or 'already_observed_exit_returns_exact_status_without_closing_pipes ... ok' not in text:
                    raise ValueError('actual OS positive path did not run')
                if 'zero_argument_observation_can_change_between_calls ... ok' not in text:
                    raise ValueError('effectful nullary observation contrast did not run')
                if variant=='correct' and text.count('preserves_exact_original_pair ... ok')!=2:
                    raise ValueError('controlled actual-pair cases did not run')
                entry['commands'].append(command(['cargo','clippy','-p','sandbox-runtime','--lib','--tests','--locked','--offline','--','-D','warnings'],source,case,'clippy'))
            else:
                for name in ['unknown_after_none_preserves_exact_original_pair',
                             'unknown_after_errors_preserves_exact_original_pair']:
                    tag='same-accounted-'+name
                    entry['commands'].append(command([str(binary),name,'--test-threads=1'],source,case,tag,101))
                    text=(case/(tag+'.log')).read_text()
                    if text.count('unknown result lost the original Child/Slot responsibility')!=1:
                        raise ValueError('mutant failed for another reason')
            record['variants'][variant]=entry
            (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
    finally:
        # Keep only the current package's mutant out of subsequent shared-target
        # tests. Dependencies, evidence binaries and Lean caches are untouched.
        if source is not None:
            record['mutant_cache_cleanup']=command(['cargo','clean','-p','sandbox-runtime'],source,output,'mutant-cache-cleanup')
            (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
    if any(sha(HERE/name)!=digest for name,digest in inputs.items()):
        raise ValueError('native test inputs changed while running')
    record['inputs']=inputs
    record.update(success=True,seconds=time.monotonic()-start)
    (output/'complete.json').write_text(json.dumps(record,indent=2)+'\n')
    try:
        check_native(output,HERE.parents[2])
    except Exception:
        (output/'complete.json').rename(output/'rejected-complete.json')
        raise
    return record

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(run(args.archive.resolve(strict=True),args.output.absolute()),indent=2))
