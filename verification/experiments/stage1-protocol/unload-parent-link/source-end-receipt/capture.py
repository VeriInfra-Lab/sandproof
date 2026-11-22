"""Precheck the original saved-End receipt and its exact typed library calls."""
import argparse
from hashlib import sha256
import importlib.util,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('source_end_receipt_inputs',HERE/'inputs.py')
inputs=importlib.util.module_from_spec(spec);spec.loader.exec_module(inputs)
parent,retire=inputs.parent,inputs.retire
ROOT,NAMESPACE,OPTIONS,STAGED,END,VARIANTS=inputs.ROOT,inputs.NAMESPACE,inputs.OPTIONS,inputs.STAGED,inputs.END,inputs.VARIANTS
reconstruct,mutate=inputs.reconstruct,inputs.mutate
LIBRARIES=set(parent.LIBRARIES)|{'controller.lifecycle_inbox.sha256.digest', 'controller.lifecycle_inbox.source_work.full.end.encode_receipt_end'}
BUSINESS=(*parent.BUSINESS,
 'controller.lifecycle_inbox.source_work.full.end.receipt',
 'controller.lifecycle_inbox.source_work.full.end.seal_end',
 'controller.lifecycle_inbox.source_work.digest')

TRANSPARENT_STANDARD=('core.option.Option.map','core.option.OptionResult.transpose')

def digest(path):return sha256(path.read_bytes()).hexdigest()
def admit(ir,generated,target):
 tree=retire.checked_translation(ir,OPTIONS,target,ROOT,parent.ROOT)
 mapping=json.loads((generated/'translation.json').read_bytes())['functions'];report=dict(business={},libraries=[])
 for name in (*BUSINESS,*TRANSPARENT_STANDARD):
  rows=[r for r in mapping if r['lean_name']==NAMESPACE+'.'+name]
  if len(rows)!=1 or rows[0]['is_opaque']:raise ValueError('whole original transparent business/clone/standard function required: '+name)
  bodies=[f for f in tree['fun_decls'] if f and f['def_id']==rows[0]['def_id']]
  if len(bodies)!=1 or 'Structured' not in (bodies[0]['body'] or {}):raise ValueError('actual structured body required: '+name)
  report['business'][name]=rows[0]['def_id']
 allowed={p.replace('sandproof_controller_service::','').replace('::','.') for p in (*parent.inputs.parent.inputs.PORTS,parent.inputs.PORT,inputs.SHA,inputs.PORT)}
 for row in mapping:
  if row['is_opaque']:
   if row['source']['file'].startswith(('controller-service/','controller-core/','sandbox-protocol/')) and row['lean_name'] not in allowed:raise ValueError('no extra business opacity: '+row['lean_name'])
   report['libraries'].append(row['lean_name'])
 report['libraries'].sort()
 if len(report['libraries'])!=29 or set(report['libraries'])!=LIBRARIES:raise ValueError('only exact29 existing per-symbol interfaces including the registered SHA primitive permitted: '+str(report['libraries']))
 return report

def capture(output,tools,lock,variant):
 retire.ROOT,retire.NAMESPACE,retire.OPTIONS,retire.STAGED=ROOT,NAMESPACE,OPTIONS,STAGED
 retire.reconstruct,retire.admit=reconstruct,admit
 try:retire.capture(output,tools,lock,variant)
 finally:
  path=output/'result.json'
  if path.is_file():
   row=json.loads(path.read_bytes());row.update(base_driver=row['driver'],driver=digest(Path(__file__)),staging_driver=digest(HERE/'inputs.py'),parent_driver=digest(Path(parent.__file__)),capture_root=ROOT,direct_original_end_receipt=True,original_option_map_and_transpose_preserved=True,two_exact_original_end_encode_sites_isolated=True,exact_original_sha_primitive=inputs.SHA,exact_receipt_encoder=inputs.PORT,original_map_transpose_transparent=True,whole_receipt_proved=False,mutation=None if variant=='original' else 'change only NoRequest receipt tag to Returned')
   path.write_text(json.dumps(row,indent=2)+'\n')
if __name__=='__main__':
 cli=argparse.ArgumentParser(description=__doc__);cli.add_argument('--variant',choices=VARIANTS,required=True)
 for name in ('output','tools','lock'):cli.add_argument('--'+name,type=Path,required=True)
 a=cli.parse_args();capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
