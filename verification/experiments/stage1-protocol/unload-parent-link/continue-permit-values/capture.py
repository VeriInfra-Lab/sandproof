"""Capture the original two-field continuation permit value interface only."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('continue_permit_values_parent',HERE.parent/'source-end-accessor/capture.py')
parent=importlib.util.module_from_spec(spec);spec.loader.exec_module(parent)
retire=parent.retire
CONTINUATION_SOURCE=parent.CONTINUATION_SOURCE
MODULE=parent.CONTINUATION
TYPE=MODULE+'::OriginalContinuePermit'
METHODS=('after_dispatch_commit','action','work_binding')
ROOTS=tuple(TYPE+'::'+m for m in METHODS)
ROOT=ROOTS[0]
METHOD_ROOTS=(*parent.METHOD_ROOTS,*((MODULE,m)for m in METHODS))
RETAINED_ROOTS=list(dict.fromkeys((*parent.OPTIONS['retained_roots'],*parent.ROOTS,*(MODULE+'::*::'+m for m in METHODS))))
NAMESPACE='SandproofContinuePermitValues'
STAGED=parent.STAGED
OPTIONS=dict(parent.OPTIONS,roots=list(ROOTS),retained_roots=RETAINED_ROOTS)
LIBRARIES=parent.LIBRARIES
VARIANTS=('original','discard-work-binding')

def digest(path):return sha256(path.read_bytes()).hexdigest()

def mutate(text,variant):
 if variant not in VARIANTS:raise ValueError('unknown continuation permit value input')
 begin=text.index('    pub(crate) const fn after_dispatch_commit(')
 end=text.index('    #[must_use]',begin)
 body=text[begin:end]
 field='            work_binding,\n'
 header='    ) -> Self {\n'
 if body.count(field)!=1 or body.count(header)!=1:raise ValueError('one exact original work binding move required')
 if variant=='discard-work-binding':
  body=body.replace(header,header+'        let _ = work_binding;\n',1)
  body=body.replace(field,'            work_binding: ContentDigest::from_sha256([0; 32]),\n',1)
 return text[:begin]+body+text[end:]

def reconstruct(archive,source,variant):
 try:parent.reconstruct(archive,source,'original')
 finally:retire.STAGED=STAGED
 path=source/CONTINUATION_SOURCE;path.write_text(mutate(path.read_text(),variant))

def admit(ir,generated,target):
 raw=json.loads(ir.read_bytes());tree=raw['translated'];options=tree['options']
 if(raw.get('has_errors')is not False or any(options[n]is not False for n in('skip_borrowck','no_typecheck','remove_unused_clauses','monomorphize'))
  or options['exclude']!=[] or any(options[k]!=OPTIONS[g]for k,g in(('start_from','roots'),('include','include'),('opaque','opaque')))
  or OPTIONS['roots']!=list(ROOTS)or OPTIONS['retained_roots']!=RETAINED_ROOTS or options['start_from_if_exists']!=RETAINED_ROOTS
  or{r['key']for r in tree['target_information']}!={target}):raise ValueError('exact three permit roots,full eight parent methods,safe options and fixed target required')
 mapping=json.loads((generated/'translation.json').read_bytes())['functions'];report=dict(roots=[],business={},libraries=[])
 from prepare_transaction_capture import method
 for module,member in METHOD_ROOTS:
  function=method(tree['fun_decls'],module,member);rows=[r for r in mapping if r['def_id']==function['def_id']]
  if len(rows)!=1 or rows[0]['is_opaque']or'Structured'not in(function['body']or{}):raise ValueError('one entire transparent original method required: '+member)
  report['roots'].append(dict(member=member,lean=rows[0]['lean_name'],def_id=function['def_id']))
 for name in(*parent.parent.BUSINESS,*parent.parent.TRANSPARENT_STANDARD):
  rows=[r for r in mapping if r['lean_name']==NAMESPACE+'.'+name]
  if len(rows)!=1 or rows[0]['is_opaque']:raise ValueError('complete original parent body required: '+name)
  functions=[f for f in tree['fun_decls']if f and f['def_id']==rows[0]['def_id']]
  if len(functions)!=1 or'Structured'not in(functions[0]['body']or{}):raise ValueError('structured original body required: '+name)
  report['business'][name]=rows[0]['def_id']
 for row in mapping:
  if row['is_opaque']:
   if row['source']['file'].startswith(('controller-service/','controller-core/','sandbox-protocol/'))and row['lean_name']not in LIBRARIES:raise ValueError('no additional business opacity: '+row['lean_name'])
   report['libraries'].append(row['lean_name'])
 report['libraries'].sort()
 if len(report['libraries'])!=29 or set(report['libraries'])!=LIBRARIES:raise ValueError('only exact29 existing per-symbol interfaces allowed')
 return report

def capture(output,tools,lock,variant):
 retire.ROOT,retire.NAMESPACE,retire.OPTIONS,retire.STAGED=ROOT,NAMESPACE,OPTIONS,STAGED
 retire.reconstruct,retire.admit=reconstruct,admit
 try:retire.capture(output,tools,lock,variant)
 finally:
  path=output/'result.json'
  if path.is_file():
   row=json.loads(path.read_bytes());row.update(base_driver=row['driver'],driver=digest(Path(__file__)),parent_driver=digest(Path(parent.__file__)),capture_roots=list(ROOTS),complete_permit_fields=2,behavior_proved=False,mutation=None if variant=='original'else variant)
   path.write_text(json.dumps(row,indent=2)+'\n')

if __name__=='__main__':
 cli=argparse.ArgumentParser(description=__doc__);cli.add_argument('--variant',choices=VARIANTS,required=True)
 for name in('output','tools','lock'):cli.add_argument('--'+name,type=Path,required=True)
 a=cli.parse_args();capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
