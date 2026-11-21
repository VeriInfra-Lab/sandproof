"""Extract original drain validation and writer status with all business bodies."""
import argparse
from hashlib import sha256
import importlib.util,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('source_drain_validation_inputs',HERE/'inputs.py')
inputs=importlib.util.module_from_spec(spec);spec.loader.exec_module(inputs)
parent,terminal,prior,retire=inputs.parent,inputs.terminal,inputs.prior,inputs.retire
ROOT,NAMESPACE,OPTIONS,STAGED,END,VARIANTS=(inputs.ROOT,inputs.NAMESPACE,inputs.OPTIONS,inputs.STAGED,inputs.END,inputs.VARIANTS)
reconstruct,mutate=inputs.reconstruct,inputs.mutate
LIBRARIES=set(parent.LIBRARIES)|set(terminal.LIBRARIES)|set(prior.LIBRARIES)
BUSINESS=(*parent.BUSINESS,
 'capture.terminated.TerminatedCaptureObservation.matches',
 'capture.terminated.TerminatedCaptureWorkRequest.binding_digest',
 'capture.work.FullCaptureWorkRequest.binding_digest',
 'capture.CaptureRequestEndObservation.matches',
 'controller.lifecycle_inbox.source_work.full.drain.prior_matches',
 'controller.lifecycle_inbox.source_work.full.drain.prior_completed_no_request',
 'controller.lifecycle_inbox.source_work.full.drain.validate_record',
 'controller.lifecycle_inbox.source_work.full.drain.writer_closed')
def digest(path):return sha256(path.read_bytes()).hexdigest()
def admit(ir,generated,target):
 tree=retire.checked_translation(ir,OPTIONS,target,ROOT,inputs.VALIDATE)
 mapping=json.loads((generated/'translation.json').read_bytes())['functions']
 report=dict(business={},libraries=[])
 for name in BUSINESS:
  entries=[r for r in mapping if r['lean_name']==NAMESPACE+'.'+name]
  if len(entries)!=1 or entries[0]['is_opaque']:raise ValueError('one transparent original body required: '+name)
  bodies=[f for f in tree['fun_decls'] if f and f['def_id']==entries[0]['def_id']]
  if len(bodies)!=1 or 'Structured' not in (bodies[0]['body'] or {}):raise ValueError('whole structured body required: '+name)
  report['business'][name]=entries[0]['def_id']
 for row in mapping:
  if row['is_opaque']:
   if row['source']['file'].startswith(('controller-service/','controller-core/','sandbox-protocol/')) and row['lean_name'] not in {p.replace('sandproof_controller_service::','').replace('::','.') for p in inputs.PORTS}:
    raise ValueError('no additional business opacity: '+row['lean_name'])
   report['libraries'].append(row['lean_name'])
 report['libraries'].sort()
 if len(report['libraries'])!=26 or set(report['libraries'])!=LIBRARIES:raise ValueError('only the exact26 inherited typed library interfaces are permitted: '+str(report['libraries']))
 return report
def capture(output,tools,lock,variant):
 retire.ROOT,retire.NAMESPACE,retire.OPTIONS,retire.STAGED=ROOT,NAMESPACE,OPTIONS,STAGED
 retire.reconstruct,retire.admit=reconstruct,admit
 try:retire.capture(output,tools,lock,variant)
 finally:
  path=output/'result.json'
  if path.is_file():
   row=json.loads(path.read_bytes())
   row.update(base_driver=row['driver'],driver=digest(Path(__file__)),staging_driver=digest(HERE/'inputs.py'),
    parent_driver=digest(Path(parent.__file__)),terminal_driver=digest(Path(terminal.__file__)),prior_driver=digest(Path(prior.__file__)),
    capture_root=ROOT,direct_original_drain_validation=True,whole_receipt_proved=False,
    original_saved_end_option_selection_expanded_to_nested_matches=True,
    mutation=None if variant=='original' else 'omit only CorruptStore return for an actual mismatched saved prior')
   path.write_text(json.dumps(row,indent=2)+'\n')
if __name__=='__main__':
 cli=argparse.ArgumentParser(description=__doc__)
 cli.add_argument('--variant',choices=VARIANTS,required=True)
 for name in ('output','tools','lock'):cli.add_argument('--'+name,type=Path,required=True)
 a=cli.parse_args();capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
