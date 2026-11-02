"""Six actual SQL-read views with one shared complete row-field native contract; development evidence."""
from pathlib import Path
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import time
import traceback

import argparse
HERE=Path(__file__).resolve().parent
r=HERE.parents[3];a=HERE
parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
out=args.output.resolve();out.mkdir(parents=True,mode=0o700)
source='controller-service/src/controller/admission.rs'
tests=str(HERE/'native-fixture.rs.in')
candidate=r/source
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
mode=lambda p:'100755' if p.stat().st_mode&0o111 else '100644'
def module(name,p):
    s=importlib.util.spec_from_file_location(name,p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
base=module('native_snapshot_native_inputs',r/'verification/experiments/stage1-protocol/schema-leaves/formal.py')
registry=module('native_snapshot_native_registry',r/'verification/experiments/stage1-protocol/envelope-writer/native.py');registry.HERE=Path('.')
record=dict(kind='lifetime-row-helper-real-SQL-native-regression-and-field-counterexamples',complete=False,commands=[],views={},candidate_adopted=True,production_modified=True,production_equivalence_proved=False,compiler_failure_counts_as_property_rejection=False,ning_acceptance=False,shared_acceptance=False,full_v3=False)
start=time.monotonic()
try:
    assert 1<=len(os.sched_getaffinity(0))<=4
    assert subprocess.run(['git','merge-base','--is-ancestor','9c50da7cc9391518514f14405e6c779e5c5cbe2c','HEAD'],cwd=r).returncode==0
    support=module('lifetime_row_native_support',HERE/'support.py');support.source_guard()
    inputs=base.snapshot(r);modes={n:mode(r/n)for n in inputs};libraries=registry.registry()
    names=set(re.findall(r'^    #\[test\]\n    fn (\w+)\(', (r/tests).read_text(),re.M));assert len(names)==14
    record.update(inputs=inputs,input_modes=modes,registry=libraries,test_names=sorted(names),tools={})
    for name in ['rustc','cargo','rustfmt','clippy-driver']:
        p=Path(subprocess.check_output(['rustup','which',name],text=True).strip());record['tools'][name]=dict(path=str(p),sha256=sha(p))
    failures={'baseline':set(),'correct':set(),'skip-owner':{'full_owner_is_compared'},'skip-startup':{'startup_deadline_must_be_nonnegative'},'lost-start':{'full_projection_is_preserved','none_start_and_inclusive_bounds_are_legal'},'restored':set()}
    record['expected_failures']={k:sorted(v) for k,v in failures.items()}
    for variant in failures:
        folder=out/variant;p=folder/'source';p.mkdir(parents=True,mode=0o700)
        expected=dict(inputs)
        for name,s in inputs.items():
            q=p/name;q.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(r/name,q);assert sha(q)==s
        text=subprocess.check_output(['git','show','9c50da7cc9391518514f14405e6c779e5c5cbe2c:'+source],cwd=r,text=True) if variant=='baseline' else support.mutate(candidate.read_text(),'correct' if variant=='restored' else variant)
        assert text.endswith('\n')
        text+=(r/tests).read_text()
        (p/source).write_text(text);expected[source]=sha(p/source)
        target=folder/'target';assert not target.exists()
        env=dict(os.environ,CARGO_NET_OFFLINE='true',CARGO_BUILD_JOBS='4',CARGO_TARGET_DIR=str(target),PYTHONDONTWRITEBYTECODE='1')
        rows=[]
        for label,argv in [
            ('format',['rustfmt','--edition','2024','--check',source]),
            ('test',['cargo','test','-p','sandproof-controller-service','--locked','--offline','--lib','--message-format=json','controller::admission::proof_lifetime_row_contract::','--','--test-threads=1']),
            ('clippy',['cargo','clippy','-p','sandproof-controller-service','--locked','--offline','--lib','--tests','--','-D','warnings']),
        ]:
            log=folder/(label+'.log');begin=time.monotonic()
            with log.open('xb')as stream:code=subprocess.run(argv,cwd=p,env=env,stdin=subprocess.DEVNULL,stdout=stream,stderr=subprocess.STDOUT,timeout=1800).returncode
            row=dict(variant=variant,label=label,argv=argv,exit_code=code,seconds=time.monotonic()-begin,log=str(log),log_sha256=sha(log));record['commands'].append(row)
            (out/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
            assert code==(101 if label=='test' and failures[variant] else 0),(variant,label,code)
            if label=='test':
                text=log.read_text();observed=re.findall(r'^test controller::admission::proof_lifetime_row_contract::(\w+) \.\.\. (?:(?!^test ).)*?\b(ok|FAILED)\n',text,re.M|re.S)
                assert len(observed)==14 and {n for n,s in observed}==names and {n for n,s in observed if s=='FAILED'}==failures[variant]
                assert f'{14-len(failures[variant])} passed; {len(failures[variant])} failed; 0 ignored;' in text
                artifacts=[]
                for line in text.splitlines():
                    try:value=json.loads(line)
                    except ValueError:continue
                    if value.get('reason')=='compiler-artifact'and value['target']['name']in ['sandproof_protocol','sandproof_controller_service']:artifacts.append(value)
                assert len(artifacts)==2 and all(ar['fresh']is False for ar in artifacts)
                for ar in artifacts:
                    own='sandbox-protocol'if ar['target']['name']=='sandproof_protocol'else'controller-service'
                    assert ar['manifest_path']==str(p/own/'Cargo.toml')and ar['target']['src_path']==str(p/own/'src/lib.rs')and ar['profile']['test']==(own=='controller-service')
                rows=[dict(message=ar,files={n:sha(Path(n))for n in ar['filenames']if Path(n).is_file()})for ar in artifacts]
        assert all(sha(p/n)==s and mode(p/n)==modes[n]for n,s in expected.items())
        record['views'][variant]=dict(complete=True,inputs=expected,input_modes=modes,artifacts=rows,tests_passed=14-len(failures[variant]),tests_failed=sorted(failures[variant]),target_initially_absent=True)
    assert base.snapshot(r)==inputs and registry.registry()==libraries
    assert all(mode(r/n)==value for n,value in modes.items())
    assert all(sha(Path(v['path']))==v['sha256']for v in record['tools'].values())
    assert all(sha(Path(v['log']))==v['log_sha256']for v in record['commands'])
    assert(out/'correct/source'/source).read_bytes()==(out/'restored/source'/source).read_bytes()==candidate.read_bytes()+(r/tests).read_bytes()
    assert(out/'baseline/source'/source).read_bytes()==subprocess.check_output(['git','show','9c50da7cc9391518514f14405e6c779e5c5cbe2c:'+source],cwd=r)+(r/tests).read_bytes()
    record.update(complete=True,source_modes_tools_libraries_logs_postguards_complete=True)
except BaseException as error:record.update(complete=False,error=repr(error),traceback=traceback.format_exc())
record['seconds']=time.monotonic()-start;(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({k:record.get(k)for k in ['complete','seconds','error']}))
raise SystemExit(0 if record['complete']else 1)
