"""Capture the whole original saved question, identity, guard and constructor."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path

HERE=Path(__file__).resolve().parent
_spec=importlib.util.spec_from_file_location('source_saved_question_staging',HERE/'inputs.py')
inputs=importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(inputs)
parent=inputs.parent
retire=inputs.retire
ROOT,NAMESPACE,OPTIONS,STAGED,END,VARIANTS=(inputs.ROOT,inputs.NAMESPACE,inputs.OPTIONS,inputs.STAGED,inputs.END,inputs.VARIANTS)
reconstruct,mutate=inputs.reconstruct,inputs.mutate
LIBRARIES=set(parent.LIBRARIES)|{
 'Bool.Insts.CoreCmpPartialEqBool.ne',
 'alloc.string.String.Insts.CoreCloneClone.clone',
 'core.option.Option.Insts.CoreCloneClone.clone',
 'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
 'core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual',
 'core.option.Option.Insts.CoreOpsTry_traitTry.branch',
 'core.option.Option.as_ref',
}
BUSINESS=(
 'controller.lifecycle_inbox.source_work.full.drain.question.from_saved',
 'controller.lifecycle_inbox.source_work.full.drain.question.original_identity',
 'controller.lifecycle_inbox.source_work.full.drain.question.assemble',
 'controller.lifecycle_inbox.source_work.full.validate.full_binding',
 'controller.lifecycle_inbox.source_work.full.key',
 'controller.lifecycle_inbox.source_work.SourceWork.request',
 'controller.lifecycle_inbox.source_work.SourceWork.impl.requirement',
 'capture.terminated.TerminatedCaptureWorkRequest.from_source',
 'capture.work.FullCaptureWorkRequest.valid',
 'capture.work.PauseWorkRequest.valid',
 'sandproof_protocol.WorkAction.Insts.CoreCloneClone.clone',
 'sandbox_controller_core.model.Command.Insts.CoreCloneClone.clone',
 'capture.work.FullCaptureWorkRequest.Insts.CoreCloneClone.clone',
 'capture.source.SourceObservationRequirement.Insts.CoreCloneClone.clone',
 'capture.launch.CaptureWorkerBinding.Insts.CoreCloneClone.clone',
 'core.num.U8.is_ascii_graphic',
)

def digest(path):return sha256(path.read_bytes()).hexdigest()

def admit(ir,generated,target):
 tree=retire.checked_translation(ir,OPTIONS,target,ROOT,None)
 mapping=json.loads((generated/'translation.json').read_bytes())['functions']
 report=dict(business={},libraries=[])
 for name in BUSINESS:
  entries=[r for r in mapping if r['lean_name']==NAMESPACE+'.'+name]
  if len(entries)!=1 or entries[0]['is_opaque']:
   raise ValueError('the actual complete business/clone body is required: '+name)
  found=[f for f in tree['fun_decls'] if f and f['def_id']==entries[0]['def_id']]
  if len(found)!=1 or 'Structured' not in (found[0]['body'] or {}):
   raise ValueError('the entire actual structured body is required: '+name)
  report['business'][name]=entries[0]['def_id']
 for row in mapping:
  if row['is_opaque']:
   if row['source']['file'].startswith(('controller-service/','controller-core/','sandbox-protocol/')):
    raise ValueError('no complete owned business, validation or clone may be opaque')
   report['libraries'].append(row['lean_name'])
 report['libraries'].sort()
 if len(report['libraries'])!=21 or set(report['libraries'])!=LIBRARIES:
  raise ValueError('the exact21 existing typed per-symbol standard interfaces are required')
 return report

def capture(output,tools,lock,variant):
 retire.ROOT,retire.NAMESPACE,retire.OPTIONS,retire.STAGED=ROOT,NAMESPACE,OPTIONS,STAGED
 retire.reconstruct,retire.admit=reconstruct,admit
 try:retire.capture(output,tools,lock,variant)
 finally:
  p=output/'result.json'
  if p.is_file():
   row=json.loads(p.read_bytes())
   row.update(base_driver=row['driver'],driver=digest(Path(__file__)),staging_driver=digest(HERE/'inputs.py'),
    parent_driver=digest(Path(parent.__file__)),capture_root=ROOT,direct_original_saved_question=True,
    whole_receipt_proved=False,mutation=None if variant=='original' else 'omit only original strict stop epoch comparison')
   p.write_text(json.dumps(row,indent=2)+'\n')

if __name__=='__main__':
 cli=argparse.ArgumentParser(description=__doc__)
 cli.add_argument('--variant',choices=VARIANTS,required=True)
 for name in ('output','tools','lock'):cli.add_argument('--'+name,type=Path,required=True)
 a=cli.parse_args();capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
