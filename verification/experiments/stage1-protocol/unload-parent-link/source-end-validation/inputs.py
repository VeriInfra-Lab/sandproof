"""Retain the original End validator, bounds and repeated writer validation."""
import importlib.util
from pathlib import Path
import sys
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'source-remove-retired'))
spec=importlib.util.spec_from_file_location('source_end_original_parent',HERE.parent/'source-drain-validation/capture.py')
parent=importlib.util.module_from_spec(spec);spec.loader.exec_module(parent)
retire=parent.retire
END=Path('controller-service/src/controller/lifecycle_inbox/source_work/full/end.rs')
VARIANTS=('original','allow-oversized-end')
ROOT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::material_writer_closed'
VALIDATE='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::validate_record'
NAMESPACE='SandproofSourceEndValidation'
PORT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::encode_saved_end'
STAGED=(*parent.STAGED,END)
OPTIONS=dict(parent.OPTIONS,roots=[ROOT],retained_roots=[VALIDATE],opaque=[*parent.OPTIONS['opaque'],PORT])
BOUND='    if serde_json::to_vec(end)?.len() > END_RECORD_BYTES {\n        return Err(JournalError::CorruptStore);\n    }\n'
LATE='capture.completed.is_some()\n                    && matches!(observation.outcome, CaptureRequestEndOutcome::NoRequest)'

NEVER='            capture.worker.is_none()\n                && capture.completed.is_none()\n                && *full_work_binding == capture.full.binding_digest()?\n                && seal_action.as_deref() == capture.action.as_ref()'

def mutate(text,variant):
 if variant not in VARIANTS:raise ValueError('unknown original End validation view')
 start=text.index('pub(super) fn validate_record(');stop=text.index('\nfn receipt(',start)
 body=text[start:stop]
 if body.count(BOUND)!=1:raise ValueError('one exact original encoded End byte bound required')
 replacement=BOUND.replace('> END_RECORD_BYTES {','> END_RECORD_BYTES * 2 {',1)
 return text[:start]+(body if variant=='original' else body.replace(BOUND,replacement,1))+text[stop:]

def reconstruct(archive,source,variant):
 try:parent.reconstruct(archive,source,'original')
 finally:retire.STAGED=STAGED
 path=source/END;text=mutate(path.read_text(),variant)
 start=text.index('pub(super) fn validate_record(');stop=text.index('\nfn receipt(',start);body=text[start:stop]
 call='serde_json::to_vec(end)?.len()'
 if body.count(call)!=1 or body.count(LATE)!=1:raise ValueError('one original encoding and entire late completed clause required')
 body=body.replace(call,'encode_saved_end(end)?.len()',1).replace(LATE,'saved_completed_no_request(capture.completed.is_some(), observation)',1)
 if body.count(NEVER)!=1:raise ValueError('one complete original NeverReleased expression required')
 body=body.replace(NEVER,'            saved_never_released_matches(capture, full_work_binding, seal_action)?',1)
 worker_header='        CaptureEndRecord::WorkerEnded { observation } => {\n'
 worker_start=body.index(worker_header);worker_end=body.index('        CaptureEndRecord::Terminated {',worker_start)
 worker_arm=body[worker_start:worker_end]
 if not worker_arm.endswith('        }\n'):raise ValueError('complete original WorkerEnded arm required')
 worker_body=worker_arm[len(worker_header):-len('        }\n')]
 expression=worker_body.index('            observation.matches(')
 worker_setup,worker_expression=worker_body[:expression],worker_body[expression:]
 if worker_setup.count('let action =')!=1 or worker_setup.count('let worker =')!=1:raise ValueError('original action-before-worker lookup required')
 body=body[:worker_start]+worker_header+'            saved_worker_ended_matches(record, capture, observation)?\n        }\n'+body[worker_end:]

 helper=('\nfn encode_saved_end(value: &CaptureEndRecord) -> serde_json::Result<Vec<u8>> {\n    serde_json::to_vec(value)\n}\n'
  +'\nfn saved_completed_no_request(completed: bool, observation: &CaptureRequestEndObservation) -> bool {\n    '
  +LATE.replace('capture.completed.is_some()','completed',1)+'\n}\n'
  +'\nfn saved_never_released_matches(capture: &CaptureWork, full_work_binding: &ContentDigest, seal_action: &Option<Box<sandproof_protocol::WorkAction<core::EffectId>>>) -> Result<bool, JournalError> {\n    Ok(\n'
  +NEVER+'\n    )\n}\n'
  +'\nfn saved_worker_ended_matches(record: &SourceWorkRecord, capture: &CaptureWork, observation: &CaptureRequestEndObservation) -> Result<bool, JournalError> {\n'
  +worker_setup+'    Ok(\n'+worker_expression+'    )\n}\n')
 path.write_text(text[:start]+body+text[stop:]+helper)
