"""Capture nine original readonly record methods; reuse exact29 interfaces."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('source_record_projections_parent',HERE.parent/'source-end-receipt/capture.py')
parent=importlib.util.module_from_spec(spec);spec.loader.exec_module(parent)
retire=parent.retire
FULL=Path('controller-service/src/controller/lifecycle_inbox/source_work/full.rs')
MODULE='sandproof_controller_service::controller::lifecycle_inbox::source_work::full'
TYPE='sandproof_controller_service::controller::lifecycle_inbox::source_work::SourceWorkRecord'
METHODS=('source_sandbox','capture_has_end','full_capture','capture_worker','capture_action','capture_request_end','captured_manifest','captured_material_bytes','capture_operation')
ROOTS=tuple(TYPE+'::'+m for m in METHODS)
ROOT=ROOTS[0]
NAMESPACE='SandproofSourceRecordProjections'
STAGED=(*parent.STAGED,FULL)
VARIANTS=('original','capture-presence-as-ended','reserved-as-material-bytes')
RETAINED_ROOTS=list(dict.fromkeys((*parent.OPTIONS['retained_roots'],parent.ROOT,*(MODULE+'::*::'+m for m in METHODS))))
OPTIONS=dict(parent.OPTIONS,roots=list(ROOTS),retained_roots=RETAINED_ROOTS,
 include=[*parent.OPTIONS['include'],'core::option::_::is_some','core::option::_::is_some_and'])
LIBRARIES=parent.LIBRARIES

def digest(path):return sha256(path.read_bytes()).hexdigest()

def mutate(text,variant):
 if variant not in VARIANTS:raise ValueError('unknown original record projection variant')
 start=text.index('impl SourceWorkRecord {');stop=text.index('impl super::SourceWork {',start)
 body=text[start:stop]
 end='.is_some_and(|capture| capture.end.is_some())'
 material='self.capture.as_ref()?.material_bytes'
 if body.count(end)!=1 or body.count(material)!=1:raise ValueError('exact original End and observed-material projections required')
 if variant=='capture-presence-as-ended':body=body.replace(end,'.is_some_and(|capture| { let _ = &capture.end; true })',1)
 if variant=='reserved-as-material-bytes':body=body.replace(material,'Some(self.capture.as_ref()?.reserved_material_bytes)',1)
 return text[:start]+body+text[stop:]

def reconstruct(archive,source,variant):
 try:parent.reconstruct(archive,source,'original')
 finally:retire.STAGED=STAGED
 path=source/FULL;path.write_text(mutate(path.read_text(),variant))

def admit(ir,generated,target):
 raw=json.loads(ir.read_bytes());tree=raw['translated'];options=tree['options']
 if (raw.get('has_errors')is not False or any(options[n]is not False for n in('skip_borrowck','no_typecheck','remove_unused_clauses','monomorphize'))
  or options['exclude']!=[] or any(options[k]!=OPTIONS[g]for k,g in(('start_from','roots'),('include','include'),('opaque','opaque')))
  or OPTIONS['roots']!=list(ROOTS)or OPTIONS['retained_roots']!=RETAINED_ROOTS or options['start_from_if_exists']!=RETAINED_ROOTS
  or{r['key']for r in tree['target_information']}!={target}):raise ValueError('all nine exact roots, retained parent closure, safe options and fixed target required')
 mapping=json.loads((generated/'translation.json').read_bytes())['functions'];report=dict(roots=[],business={},libraries=[])
 from prepare_transaction_capture import method
 for member in METHODS:
  function=method(tree['fun_decls'],MODULE,member);rows=[r for r in mapping if r['def_id']==function['def_id']]
  if len(rows)!=1 or rows[0]['is_opaque']or'Structured'not in(function['body']or{}):raise ValueError('entire original method must remain transparent: '+member)
  report['roots'].append(dict(member=member,lean=rows[0]['lean_name'],def_id=function['def_id']))
 for name in(*parent.BUSINESS,*parent.TRANSPARENT_STANDARD,'core.option.Option.is_some_and'):
  rows=[r for r in mapping if r['lean_name']==NAMESPACE+'.'+name]
  if len(rows)!=1 or rows[0]['is_opaque']:raise ValueError('one original transparent body required: '+name)
  functions=[f for f in tree['fun_decls']if f and f['def_id']==rows[0]['def_id']]
  if len(functions)!=1 or'Structured'not in(functions[0]['body']or{}):raise ValueError('complete original structured body required: '+name)
  report['business'][name]=rows[0]['def_id']
 for row in mapping:
  if row['is_opaque']:
   if row['source']['file'].startswith(('controller-service/','controller-core/','sandbox-protocol/'))and row['lean_name']not in LIBRARIES:raise ValueError('no additional owned opacity: '+row['lean_name'])
   report['libraries'].append(row['lean_name'])
 # is_some is the existing fixed Aeneas.Std builtin, already used by the
 # identical retained parent bodies; it has no generated function-map row.
 # is_some_and and its actual predicate/dictionary remain fully transparent.
 if any(row['lean_name'] in ('core.option.Option.is_some',NAMESPACE+'.core.option.Option.is_some')for row in mapping):
  raise ValueError('the exact existing builtin Option.is_some mapping must remain unchanged')
 if b'core.option.Option.is_some' not in(generated/NAMESPACE/'Funs.lean').read_bytes():
  raise ValueError('the actual retained builtin Option.is_some uses must remain')
 report['builtin_standard']=['core.option.Option.is_some']
 report['libraries'].sort()
 if len(report['libraries'])!=29 or set(report['libraries'])!=LIBRARIES:raise ValueError('only the exact29 inherited per-symbol interfaces allowed')
 return report

def capture(output,tools,lock,variant):
 retire.ROOT,retire.NAMESPACE,retire.OPTIONS,retire.STAGED=ROOT,NAMESPACE,OPTIONS,STAGED
 retire.reconstruct,retire.admit=reconstruct,admit
 try:retire.capture(output,tools,lock,variant)
 finally:
  path=output/'result.json'
  if path.is_file():
   row=json.loads(path.read_bytes());row.update(base_driver=row['driver'],driver=digest(Path(__file__)),parent_driver=digest(Path(parent.__file__)),capture_roots=list(ROOTS),readonly_record_methods=9,behavior_proved=False,mutation=None if variant=='original'else variant)
   path.write_text(json.dumps(row,indent=2)+'\n')

if __name__=='__main__':
 cli=argparse.ArgumentParser(description=__doc__);cli.add_argument('--variant',choices=VARIANTS,required=True)
 for name in('output','tools','lock'):cli.add_argument('--'+name,type=Path,required=True)
 a=cli.parse_args();capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
