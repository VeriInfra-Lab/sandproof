from pathlib import Path
import sys,hashlib,io,json,os,re,shutil,subprocess,tarfile,time,traceback
import argparse
from contextlib import nullcontext
from check import check_inputs
ROOT=Path(__file__).resolve().parents[3]
parser=argparse.ArgumentParser(description='Actual original Mutex/Session native checks; run on ning under the coordination lock')
parser.add_argument('--output',type=Path,required=True)
out=parser.parse_args().output.resolve()
assert os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')=='native', 'ordinary tests run on the native host only'
assert 'CARGO_TARGET_DIR' in os.environ, 'provide a dedicated, coordinated native target'
check_inputs()
out.mkdir(mode=0o700,parents=True,exist_ok=False)
with (out/'input-regressions.log').open('xb') as stream:
    checked=subprocess.run([sys.executable,str(ROOT/'verification/experiments/tokio-mutex/test_checks.py')],cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT)
    assert checked.returncode==0, 'input admission regressions failed'
appendix=ROOT/'verification/experiments/tokio-mutex/session-tests.rs'
metadata=dict(base=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
    appendix_sha256=hashlib.sha256(appendix.read_bytes()).hexdigest(),
    original_session_sha256=hashlib.sha256((ROOT/'sandbox-guest-service/src/session.rs').read_bytes()).hexdigest(),
    tokio_mutex_sha256='f5ca224054d774db86f735e5400fde606aa6acb6e84a711598678082f562af85')
(out/'session-tests.rs').write_bytes(appendix.read_bytes())
(out/'input.json').write_text(json.dumps(metadata,indent=2)+'\n')
cargo_home=Path(os.environ.get('CARGO_HOME',str(Path.home()/'.cargo')))
start=time.monotonic();os.umask(0o077)
source=None;env=None
record=dict(kind='actual Tokio API and isolated real Session mutation; not async Rust refinement',complete=False,variants={})
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def state(phase,**extra):
 (out/'state.json').write_text(json.dumps(dict(phase=phase,pid=os.getpid(),seconds=time.monotonic()-start,**extra),indent=2)+'\n')
def run(args,cwd,env,name):
 with (out/name).open('xb') as log:return subprocess.run(args,cwd=cwd,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=900).returncode
try:
 data=json.loads((out/'input.json').read_text());record['input']=data
 assert sha(out/'session-tests.rs')==data['appendix_sha256']
 library=list((cargo_home/'registry/src').glob('*/tokio-1.49.0/src/sync/mutex.rs'))
 assert len(library)==1 and sha(library[0])==data['tokio_mutex_sha256']
 assert sha(library[0].with_name('batch_semaphore.rs'))=='714cbd7f724101891200c0c588b384aad92d3815bc1ee94973088d1bc5c36a60'
 usage=shutil.disk_usage(out);assert usage.free>8*1024**3, 'Insufficient space for isolated native target'
 record['free_bytes_before']=usage.free
 # The outer batch/standalone supervisor owns the lock; do not nest it.
 with nullcontext():
  state('running-isolated-mutex-native-probe',terminal=False)
  target=Path(os.environ['CARGO_TARGET_DIR']).resolve();target.mkdir(mode=0o700,parents=True,exist_ok=True)
  env=dict(os.environ,PATH='/home/ning/.cargo/bin:/home/ning/.local/bin:/usr/local/bin:/usr/bin:/bin',CARGO_HOME=str(cargo_home),CARGO_TARGET_DIR=str(target),CARGO_NET_OFFLINE='true',CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='4',RUST_TEST_THREADS='1')
  archive=subprocess.check_output(['git','archive',data['base']],cwd=ROOT);record['original_archive_sha256']=hashlib.sha256(archive).hexdigest()
  (out/'original-source.tar').write_bytes(archive)
  marker='mutex_contract_trial::initialization_retains_original_mutex_until_metadata_finishes'
  for variant in ['correct','early-release-init']:
   source=out/variant;source.mkdir(mode=0o700)
   with tarfile.open(fileobj=io.BytesIO(archive)) as tree:tree.extractall(source,filter='data')
   original=source/'sandbox-guest-service/src/session.rs';assert sha(original)==data['original_session_sha256']
   suite=source/'sandbox-guest-service/tests/session.rs';suite.write_text(suite.read_text()+(out/'session-tests.rs').read_text())
   if variant!='correct':
    text=original.read_text();before='            .map_err(|_| ServiceError::ResourceExhausted)?;\n        let (current_hash, identity) = {'
    after='            .map_err(|_| ServiceError::ResourceExhausted)?;\n        drop(_init);\n        let (current_hash, identity) = {'
    assert text.count(before)==1;original.write_text(text.replace(before,after))
   assert run(['cargo','fmt','--all','--check'],source,env,variant+'-fmt.log')==0, 'Formatting precondition failed'
   assert run(['cargo','clean','--package','sandproof-guest-service'],source,env,variant+'-package-clean.log')==0
   buildlog=out/(variant+'-build.log');command=['cargo','test','--locked','--offline','-p','sandproof-guest-service','--test','session','--no-run','--message-format=json']
   assert run(command,source,env,variant+'-build.log')==0, 'Compile failure is not semantic rejection'
   rows=[]
   for line in buildlog.read_text().splitlines():
    try:item=json.loads(line)
    except ValueError:continue
    if item.get('reason')=='compiler-artifact':rows.append(item)
   test=[x for x in rows if x.get('target',{}).get('name')=='session' and x.get('executable')]
   lib=[x for x in rows if x.get('target',{}).get('name')=='sandproof_guest_service' and x['target']['kind']==['lib'] and not x.get('executable')]
   assert len(test)==len(lib)==1 and test[0]['fresh'] is False and lib[0]['fresh'] is False
   assert all(Path(x['manifest_path'])==source/'sandbox-guest-service/Cargo.toml' for x in [*test,*lib])
   binary=Path(test[0]['executable']).resolve();libraries=[Path(x).resolve() for x in lib[0]['filenames'] if x.endswith('.rlib')]
   assert len(libraries)==1 and all(x.is_relative_to(target) and x.is_file() for x in [binary,*libraries])
   retained=out/(variant+'-tests');retainedlib=out/(variant+'-library.rlib');shutil.copy2(binary,retained);shutil.copy2(libraries[0],retainedlib)
   row=dict(build_exit=0,business_library_fresh=True,binary_fresh=True,original_sha256=data['original_session_sha256'],tested_source_sha256=sha(original),binary_sha256=sha(retained),library_sha256=sha(retainedlib),checks=[])
   tests=['mutex_contract_trial::','cancellation_at_metadata_or_write_lock_preserves_old_session_and_releases_init_slot','rotation_waits_for_original_dispatch_and_rejects_old_credentials_after_commit','invalid_or_oversize_update_cannot_partially_rotate_token_or_defaults'] if variant=='correct' else [marker]
   for index,name in enumerate(tests):
    arguments=[str(retained),name,*([] if name.endswith('::') else ['--exact']),'--nocapture','--test-threads=1']
    logname=variant+'-test-'+str(index)+'.log';code=run(arguments,source,env,logname);text=(out/logname).read_text();count=7 if name.endswith('::') else 1
    if variant=='correct':assert code==0 and f'test result: ok. {count} passed; 0 failed; 0 ignored;' in text,text
    else:assert code==101 and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in text and 'Original initialization mutex was released before metadata finished' in text,text
    row['checks'].append(dict(marker=name,count=count,test_exit=code,log_sha256=sha(out/logname)))
   if variant=='correct':
    assert run(['cargo','clippy','--locked','--offline','-p','sandproof-guest-service','--all-targets','--','-D','warnings'],source,env,'clippy.log')==0
    record['clippy_exit']=0
   record['variants'][variant]=row;(out/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
  record.update(complete=True,exit_code=0,correct=10,wrong_same_native_assertion=1)
 state('terminal',terminal=True,exit_code=0)
except BaseException as error:
 record.update(exit_code=1,error=repr(error),traceback=traceback.format_exc());state('terminal',terminal=True,exit_code=1,error=repr(error))
finally:
 if source is not None and env is not None:
  try:
   cleanup=run(['cargo','clean','--package','sandproof-guest-service'],source,env,'final-mutant-package-clean.log')
   record['final_own_package_cleanup']=dict(exit_code=cleanup,log_sha256=sha(out/'final-mutant-package-clean.log'))
   if cleanup:record.update(complete=False,exit_code=1,error='own mutant package cleanup failed')
  except BaseException as cleanup_error:record.update(complete=False,exit_code=1,cleanup_error=repr(cleanup_error))
 state('terminal',terminal=True,exit_code=record['exit_code'])
record['seconds']=time.monotonic()-start;(out/'result.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record));raise SystemExit(record['exit_code'])
