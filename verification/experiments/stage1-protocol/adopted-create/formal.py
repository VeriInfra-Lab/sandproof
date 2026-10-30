"""Check the three adopted V3 functions in two independent, real generated views."""
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

ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/adopted-create')
SOURCE='controller-service/src/controller/lifecycle_inbox.rs'
BASE=HERE.parent

def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value

request=module('adopted_request_binding',ROOT/BASE/'create-selection/support.py')
successor=module('adopted_successor_binding',ROOT/BASE/'create-owner-successor/support.py')
owner=module('adopted_original_owner_binding',ROOT/HERE/'reuse/admission.py')
checker=module('adopted_original_physical_checker',ROOT/BASE/'formal.py')
resolver=module('adopted_original_closure',ROOT/'tools/proof_batch.py')
sha=request.sha

def inventory():
 data=json.loads((ROOT/HERE/'inventory.json').read_text())
 for name,digest in data['inputs'].items():
  if sha(ROOT/name)!=digest:raise ValueError('adopted input differs '+name)
 if sha(ROOT/SOURCE)!=data['production_source_sha256']:raise ValueError('production source differs')
 return data

def borrow_successor(raw):
 functions=[f for f in raw['translated']['fun_decls'] if f]
 prefix='sandproof_controller_service::controller::lifecycle_inbox::'
 def select(name):
  values=[f for f in functions if f['item_meta']['name']==[{'Ident':[n,0]} for n in (prefix+name).split('::')]]
  if len(values)!=1 or values[0]['item_meta']['opacity']!='Transparent' or 'Structured' not in values[0]['body']:raise ValueError('business root missing or external '+name)
  return values[0]
 root,actual=select('proof_create_owner_successor'),select('create_owner_successor')
 calls,assignments=[],[]
 def walk(x):
  if isinstance(x,dict):
   if 'Call'in x:calls.append(x['Call']['call'])
   if 'Assign'in x:assignments.append(x['Assign'])
   for y in x.values():walk(y)
  elif isinstance(x,list):
   for y in x:walk(y)
 walk(root['body'])
 if len(calls)!=1 or len(assignments)!=3 or len(calls[0]['args'])!=3 or root['body']['Structured']['locals']['arg_count']!=3 or calls[0]['dest']['kind']!={'Local':0} or calls[0]['func']['Regular']['kind']['Fun']!=actual['def_id']:raise ValueError('changed actual successor call')
 for arg,(destination,expression),n in zip(calls[0]['args'][:2],assignments[:2],[1,2]):
  if arg.get('Move',{}).get('kind')!=destination['kind'] or expression.get('Ref',{}).get('kind')!='Shared':raise ValueError('changed full borrow')
  projection=expression['Ref']['place']['kind']['Projection']
  if projection[0]['kind']!={'Local':n} or projection[1]!='Deref':raise ValueError('projected or reordered input')
 destination,expression=assignments[2]
 if calls[0]['args'][2].get('Move',{}).get('kind')!=destination['kind'] or expression.get('Use')!=[{'Copy':{'kind':{'Local':3},'ty':destination['ty']}},'Yes']:raise ValueError('changed original sandbox value')
 return dict(root=root['def_id'],actual=actual['def_id'],two_original_full_shared_borrows=True,original_sandbox_copy=True)

def run(folder,name,data,inputs,modes):
 folder.mkdir(mode=0o700);start=time.monotonic();recipe=data['views'][name]
 rec=dict(kind='adopted-v3-create-positive-correspondence',view=name,complete=False,production_adopted=True,private_business_replacement=False,original_general_equivalence_proved=False,original_full_attachment_proved=False,native_checks_run=False,formal_acceptance=False,ning_acceptance=False,shared_acceptance=False,full_v3=False,commands=[],inputs=inputs,input_modes=modes,source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip())
 env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',CARGO_NET_OFFLINE='true',LEAN_NUM_THREADS='4',LEAN_STACK_SIZE_KB='65536')
 def command(project,out,label,argv):
  log=out/(label+'.log');begin=time.monotonic()
  with log.open('xb') as stream:done=subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
  rec['commands'].append(dict(label=label,argv=argv,cwd=str(project),exit_code=done.returncode,seconds=time.monotonic()-begin,log_sha256=sha(log)))
  (folder/'progress.json').write_text(json.dumps(rec,indent=2)+'\n')
  if done.returncode:raise ValueError('command failed '+str(log))
 try:
  if inventory()!=data:raise ValueError('inputs changed')
  project=folder/'source';project.mkdir(mode=0o700);expected=dict(inputs)
  for n,h in inputs.items():
   p=project/n;p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/n,p)
   if sha(p)!=h:raise ValueError('source changed during copy '+n)
  (project/'.tools').symlink_to(request.mapping.tool_root(ROOT),target_is_directory=True);(project/'.lake').mkdir();(project/'.lake/packages').symlink_to(request.mapping.package_root(ROOT),target_is_directory=True);(project/'artifacts/extract-target').mkdir(parents=True)
  for n in recipe['roots']:
   with (project/SOURCE).open('ab') as stream:stream.write((ROOT/n).read_bytes())
  expected[SOURCE]=sha(project/SOURCE);rec['private_source_derivation']=dict(path=SOURCE,production_sha256=data['production_source_sha256'],derived_sha256=expected[SOURCE],only_transparent_roots_appended=True)
  fixed=json.loads((ROOT/'verification/foundations-environment.json').read_text());rec['tools']=fixed
  for tool,suffix in [('charon','charon/bin/charon'),('aeneas','bin/aeneas')]:
   if sha(ROOT/'.tools/aeneas'/suffix)!=fixed[tool]['binary_sha256']:raise ValueError('fixed tool differs '+tool)
  if name=='owners':rec['fixed_library_review']=successor.fixed_library_review(json.loads((ROOT/BASE/'create-owner-successor/inventory.json').read_text()))
  def capture(label,opaque):
   ir=Path('artifacts')/(label+'.llbc');argv=[str(request.mapping.VIRTUAL/'.tools/aeneas/charon/bin/charon'),'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default']
   for n in request.INCLUDE:argv+=['--include',n]
   for n in opaque:argv+=['--opaque',n]
   for n in recipe['start_from']:argv+=['--start-from',n]
   argv+=['--dest-file',str(request.mapping.VIRTUAL/ir),'--','--manifest-path',str(BASE/'inbox-record/Cargo.toml'),'--lib','--locked','--offline','--target',fixed['target']]
   command(project,folder,label+'-charon',request.wrapper(project)+argv)
   raw=json.loads((project/ir).read_text());options=raw['translated']['options']
   if raw['has_errors'] or options['start_from']!=recipe['start_from'] or options['include']!=request.INCLUDE or options['opaque']!=opaque or options['exclude'] or options['skip_borrowck'] or options['no_typecheck'] or options['monomorphize'] or options['remove_unused_clauses'] or {t['key'] for t in raw['translated']['target_information']}!={fixed['target']}:raise ValueError('unreviewed capture options')
   request.journal.admit(raw)
   if name=='selection':binding=request.admit(raw,opaque)
   else:binding=dict(owner=owner.admit_ir(raw),successor=borrow_successor(raw))
   rec.setdefault('captures',{})[label]=dict(path=str(ir),sha256=sha(project/ir),binding=binding)
   return raw,ir
  raw,ir=capture('original-enum',['rusqlite::error::Error'])
  if name=='selection':
   carrier=module('adopted_complete_contract_error',ROOT/BASE/'envelope-writer/binding.py');enum=carrier.contract_error(raw);old=json.loads((ROOT/'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
   if enum!=carrier.contract_error(old):raise ValueError('complete ContractError carrier differs')
   rec['complete_contract_error']=enum;del raw,old
   raw,ir=capture('bound-enum',['rusqlite::error::Error','sandproof_protocol::ContractError'])
  del raw
  generated=folder/'generated';generated.mkdir();command(project,folder,'aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),'-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json','-namespace',recipe['namespace'],'-subdir',recipe['library'],'-dest',str(generated),str(project/ir)])
  meta=json.loads((generated/'translation.json').read_text());functions={f['lean_name']:{k:f[k] for k in ('rust_name','is_opaque','lean_file')} for f in meta['functions']}
  if functions!=recipe['functions'] or len(functions)!=len(meta['functions']):raise ValueError('changed actual function closure')
  for actual in recipe['actual_roots']:
   found=[f for f in meta['functions'] if f['lean_name']==actual]
   if len(found)!=1 or found[0]['is_opaque']:raise ValueError('actual generated business root missing')
   source=(generated/found[0]['lean_file']).read_text();short=actual.removeprefix(recipe['namespace']+'.')
   if len(re.findall(r'^def\s+'+re.escape(short)+r'\b',source,re.M))!=1 or len(re.findall(r'^namespace '+recipe['namespace']+'$',source,re.M))!=1 or len(re.findall(r'^end '+recipe['namespace']+'$',source,re.M))!=1:raise ValueError('actual generated body missing')
  gen=Path(recipe['generated']);(project/gen).mkdir(parents=True,exist_ok=True)
  for n in ('Types.lean','Funs.lean'):
   shutil.copy2(generated/recipe['library']/n,project/gen/n);expected[str(gen/n)]=sha(project/gen/n)
  installed=[]
  for target,provider in recipe['providers'].items():
   p=project/target;p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/provider,p);expected[target]=sha(p);installed.append(target)
  if name=='owners':
   p=project/'lakefile.toml';p.write_text(p.read_text()+'\n[[lean_lib]]\nname = "SandproofStage1OwnerMapping"\nsrcDir = "verification/aeneas/stage1-protocol/owner-mapping"\n');expected['lakefile.toml']=sha(p)
  files=sorted(resolver.closure(project,installed+[str(gen/n) for n in ('Types.lean','Funs.lean')],active_view='lakefile.toml'));audits=[a for n in files for a in re.findall(r'^#audit_controller_proof (\S+)$',(project/n).read_text(),re.M)]
  if files!=recipe['files'] or audits!=recipe['audits'] or len(audits)!=len(set(audits)):raise ValueError('incomplete actual file/audit inventory')
  rec.update(files={n:sha(project/n) for n in files},audits=audits,translation=meta,raw_generated={n:sha(generated/recipe['library']/n) for n in ('Types.lean','Funs.lean')})
  command(project,folder,'build',['lake','build',*recipe['seeds']]);physical=folder/'actual-files';physical.mkdir();rec['actual']=checker.actual_files(project,files,physical,env,command)
  if rec['actual']['files']!=rec['files'] or rec['actual']['audits']!=audits:raise ValueError('complete checker mismatch')
  if any(sha(project/n)!=h for n,h in expected.items()):raise ValueError('private source changed')
  rec.update(complete=True,private_derived_inputs={n:h for n,h in expected.items() if inputs.get(n)!=h})
 except BaseException as e:rec.update(error=repr(e),traceback=traceback.format_exc())
 finally:
  rec['original_inputs_unchanged']=request.parent.base.snapshot(ROOT)==inputs;rec['original_modes_unchanged']=all(('100755' if (ROOT/n).stat().st_mode&0o111 else '100644')==m for n,m in modes.items())
  if not rec['original_inputs_unchanged'] or not rec['original_modes_unchanged']:rec['complete']=False
  rec['execution_seconds']=time.monotonic()-start;(folder/'result.json').write_text(json.dumps(rec,indent=2)+'\n')
 return rec

if __name__=='__main__':
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--lock',type=Path,required=True);args=parser.parse_args();os.umask(0o077);out=args.output.absolute();out.mkdir(mode=0o700,parents=True)
 record=dict(kind='adopted-v3-create-positive-correspondence-batch',complete=False,views={},production_adopted=True,original_full_attachment_proved=False,native_checks_run=False,formal_acceptance=False,ning_acceptance=False,shared_acceptance=False,full_v3=False);waiting=time.monotonic()
 try:
  with args.lock.open('r+') as lock:
   fcntl.flock(lock,fcntl.LOCK_EX);record['lock_wait_seconds']=time.monotonic()-waiting;start=time.monotonic();data=inventory();resolver.ROOT,resolver.MODULES=ROOT,('stage1-protocol',);os.environ.setdefault('SANDPROOF_FORMAL_CACHE',str(out/'runtime-cache'));runtime=resolver.runtime();registry=request.parent.native.registry_module.registry();inputs=request.parent.base.snapshot(ROOT);modes={n:('100755' if (ROOT/n).stat().st_mode&0o111 else '100644') for n in inputs}
   for name in data['views']:
    r=run(out/name,name,data,inputs,modes);record['views'][name]=dict(complete=r['complete'],result_sha256=sha(out/name/'result.json'))
    if not r['complete']:raise ValueError('adopted view failed '+name)
   if resolver.runtime()!=runtime or request.parent.native.registry_module.registry()!=registry:raise ValueError('fixed runtime or registry changed')
   record.update(complete=True,runtime=runtime,registry=registry,execution_seconds=time.monotonic()-start)
 except BaseException as e:record.update(error=repr(e),traceback=traceback.format_exc())
 (out/'result.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps({k:record.get(k) for k in ('complete','execution_seconds','error')}),flush=True);raise SystemExit(not record['complete'])
