"""Check original/adopted/wrong/restored actual Rust with one unchanged full-field assertion set."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback
ROOT=Path(__file__).resolve().parents[4]
HERE=Path(__file__).resolve().parent
BASE='9c50da7cc9391518514f14405e6c779e5c5cbe2c'
def module(n,p):
 s=importlib.util.spec_from_file_location(n,p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
support=module('pause_projection_native_support',HERE/'support.py')
sha,require,mode=support.sha,support.require,support.mode
SOURCE=support.SOURCE
FRAGMENTS={'controller-service/src/controller/lifecycle_inbox/source_work/tests.rs':'native-tests.rs.in','controller-service/src/lib.rs':'probe.rs.in'}
FAILURES={'baseline':set(),'correct':set(),'origin-binding':{'proof_pause_request_full','proof_pause_request_operation_binding'},'native-incarnation':{'proof_pause_request_full','proof_pause_request_wire_numbers','proof_pause_request_native_domain'},'options-binding':{'proof_pause_request_full','proof_pause_request_work_binding'},'restored':set()}

def run(output):
 output.mkdir(parents=True,mode=0o700);started=time.monotonic()
 record=dict(kind='actual-pause-request-native-development',complete=False,commands=[],views={},production_source_adopted_in_branch=True,general_old_new_equivalence_proved=False,ning_acceptance=False,shared_acceptance=False,full_v3=False)
 try:
  require(1<=len(os.sched_getaffinity(0))<=4,'native CPU allocation differs')
  head=support.source_guard();inputs=support.base.snapshot(ROOT);modes={n:mode(ROOT/n)for n in inputs};registry=support.registry.registry()
  tests=set(re.findall(r'^#\[test\]\nfn (proof_pause_request_\w+)\(\)',(HERE/'native-tests.rs.in').read_text(),re.M));require(len(tests)==10,'original native assertions lost')
  record.update(inputs=inputs,input_modes=modes,registry=registry,source_head=head,source_base=BASE,test_names=sorted(tests),tools={})
  for n in ['rustc','cargo','rustfmt','clippy-driver']:
   p=Path(subprocess.check_output(['rustup','which',n],text=True).strip());record['tools'][n]=dict(path=str(p),sha256=sha(p))
  original=subprocess.check_output(['git','show',BASE+':'+SOURCE],cwd=ROOT)
  for view,failures in FAILURES.items():
   folder=output/view;project=folder/'source';project.mkdir(parents=True,mode=0o700);expected=dict(inputs)
   for n,digest in inputs.items():
    p=project/n;p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/n,p);require(sha(p)==digest,'input changed during copy')
   selected=original if view=='baseline'else support.mutate((ROOT/SOURCE).read_text(),'correct'if view=='restored'else view).encode()
   (project/SOURCE).write_bytes(selected);expected[SOURCE]=sha(project/SOURCE)
   for n,fragment in FRAGMENTS.items():
    p=project/n;before=p.read_bytes();extra=(HERE/fragment).read_bytes();require(extra not in before,'fixture/bridge already present');p.write_bytes(before+extra);expected[n]=sha(p)
   target=folder/'target';require(not target.exists(),'variant target already exists')
   env=dict(os.environ,CARGO_NET_OFFLINE='true',CARGO_BUILD_JOBS='4',CARGO_TARGET_DIR=str(target),CARGO_TERM_COLOR='never',PYTHONDONTWRITEBYTECODE='1')
   entry=dict(expected_failures=sorted(failures),source_inputs=expected,source_input_modes=modes,target_initially_absent=True)
   for label,argv in [('format',['rustfmt','--edition','2024','--check',SOURCE,*FRAGMENTS]),('test',['cargo','test','-p','sandproof-controller-service','--locked','--offline','--lib','--message-format=json','proof_pause_request_','--','--test-threads=1']),('clippy',['cargo','clippy','-p','sandproof-controller-service','--locked','--offline','--lib','--tests','--','-D','warnings'])]:
    log=folder/(label+'.log');begin=time.monotonic()
    with log.open('xb')as stream:code=subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,stdout=stream,stderr=subprocess.STDOUT,timeout=1800).returncode
    record['commands'].append(dict(view=view,label=label,argv=argv,cwd=str(project),exit_code=code,seconds=time.monotonic()-begin,log=str(log),log_sha256=sha(log)))
    (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
    require(code==(101 if label=='test'and failures else 0),'compiler/format failure is not a property rejection: '+view+'/'+label)
    if label!='test':continue
    text=log.read_text();outcomes=re.findall(r'^test [\w:]+::(proof_pause_request_\w+) \.\.\. (?:(?!^test ).)*?\b(ok|FAILED)\n',text,re.M|re.S)
    require(len(outcomes)==10 and{n for n,s in outcomes}==tests and{n for n,s in outcomes if s=='FAILED'}==failures,'assertion set or exact failures differ')
    require(f'{10-len(failures)} passed; {len(failures)} failed; 0 ignored;'in text,'test process did not reach original assertions')
    sites={}
    for failed in failures:
     block=text.split('::'+failed+' stdout ----',1)[1].split('\n---- ',1)[0]
     found=re.search(r'panicked at ([^\n:]+):(\d+):(\d+):\n(.*?)(?=\nnote:|\nstack backtrace:|\n\nfailures:|\Z)',block,re.S);require(found is not None,'missing target assertion')
     source,line,column,message=found.groups();require(source in FRAGMENTS,'failure outside original test');enclosing=re.findall(r'^fn (proof_pause_request_\w+)\(','\n'.join((project/source).read_text().splitlines()[:int(line)-1]),re.M)
     require(enclosing and enclosing[-1]==failed and message.startswith('assertion `left == right` failed'),'fixture failure is not the full field contract refusal')
     sites[failed]=dict(source=source,line=int(line),column=int(column),message=message,source_sha256=sha(project/source))
    artifacts=[]
    for line in text.splitlines():
     try:item=json.loads(line)
     except ValueError:continue
     if item.get('reason')=='compiler-artifact'and item['target']['name']in['sandproof_protocol','sandproof_controller_service']:artifacts.append(item)
    require(len(artifacts)==2 and all(not x['fresh']for x in artifacts),'actual libraries/program were not freshly compiled')
    for item in artifacts:
     package='sandbox-protocol'if item['target']['name']=='sandproof_protocol'else'controller-service'
     require(item['manifest_path']==str(project/package/'Cargo.toml')and item['target']['src_path']==str(project/package/'src/lib.rs')and item['profile']['test']==(package=='controller-service'),'wrong source/manifest/program')
    protocol,=[item for item in artifacts if item['target']['name']=='sandproof_protocol'];linked=list(target.glob('debug/deps/libsandproof_protocol-*.rlib'));require(len(linked)==1 and str(linked[0])in protocol['filenames'],'different protocol library linked')
    entry.update(assertion_sites=sites,native_assertions_checked=True,property_rejected=bool(failures),artifacts=[dict(message=item,files={n:sha(Path(n))for n in item['filenames']if Path(n).is_file()})for item in artifacts])
   require(all(sha(project/n)==digest and mode(project/n)==modes[n]for n,digest in expected.items()),'native source/mode changed')
   entry['complete']=True;record['views'][view]=entry
  require(support.base.snapshot(ROOT)==inputs and support.registry.registry()==registry and all(mode(ROOT/n)==value for n,value in modes.items()),'root source/modes/libraries changed')
  require(all(sha(Path(v['path']))==v['sha256']for v in record['tools'].values()),'native tool changed')
  require(all(sha(Path(v['log']))==v['log_sha256']for v in record['commands']),'original log changed')
  require((output/'correct/source'/SOURCE).read_bytes()==(output/'restored/source'/SOURCE).read_bytes()==(ROOT/SOURCE).read_bytes(),'restored Rust differs')
  record.update(complete=True,source_tools_library_and_log_postguards_complete=True)
 except BaseException as error:record.update(complete=False,error=repr(error),traceback=traceback.format_exc())
 record['seconds']=time.monotonic()-started;(output/'result.json').write_text(json.dumps(record,indent=2)+'\n');return record
if __name__=='__main__':
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args();os.umask(0o077)
 cpus=sorted(os.sched_getaffinity(0))
 if len(cpus)>4:os.sched_setaffinity(0,set(cpus[:4]))
 result=run(args.output.resolve());print(json.dumps({n:result.get(n)for n in ['complete','seconds','error']}));raise SystemExit(0 if result['complete']else 1)
