"""Original set_end with explicit proposed-End JSON boundary; no rollback promise."""
import importlib.util,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'source-remove-retired'))
spec=importlib.util.spec_from_file_location('source_set_end_parent',HERE.parent/'source-end-receipt/capture.py')
parent=importlib.util.module_from_spec(spec);spec.loader.exec_module(parent)
retire=parent.retire
END=parent.END
ROOT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::set_end'
NAMESPACE='SandproofSourceSetEnd'
VARIANTS=('original','allow-completed-no-request')
PORT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::encode_proposed_end'
STAGED=parent.STAGED
OPTIONS=dict(parent.OPTIONS,roots=[ROOT],retained_roots=[parent.parent.ROOT],opaque=[*parent.OPTIONS['opaque'],PORT])
REJECT='        if capture.completed.is_some()\n            && matches!(observation.outcome, CaptureRequestEndOutcome::NoRequest)\n        {\n            return Err(JournalError::InvalidHostEvidence);\n        }\n'

STAGED_REJECT='        if saved_completed_no_request(capture.completed.is_some(), &observation) {\n            return Err(JournalError::InvalidHostEvidence);\n        }\n'
def mutate(text,variant):
 if variant not in VARIANTS:raise ValueError('unknown original set_end view')
 begin=text.index('fn set_end(');body=text[begin:]
 found=[guard for guard in (REJECT,STAGED_REJECT)if body.count(guard)==1]
 if len(found)!=1:raise ValueError('one exact original/staged early completed/NoRequest rejection required')
 return text[:begin]+(body if variant=='original'else body.replace(found[0],'',1))

def reconstruct(archive,source,variant):
 try:parent.reconstruct(archive,source,'original')
 finally:retire.STAGED=STAGED
 path=source/END;text=mutate(path.read_text(),variant)
 begin=text.index('fn set_end(');end=text.index('\n}\n',begin)+3;body=text[begin:end]
 call='serde_json::to_vec(&proposed)?.len()'
 if body.count(call)!=1:raise ValueError('one original proposed-End byte bound required')
 body=body.replace(call,'encode_proposed_end(&proposed)?.len()',1)
 allocation='capture.action.clone().map(Box::new)'
 if body.count(allocation)!=1:raise ValueError('one original Option/Box function-value call required')
 body=body.replace(allocation,'capture.action.clone().map(|action| Box::new(action))',1)
 if body.count(REJECT)!=(1 if variant=='original'else 0):raise ValueError('exact original guard presence required')
 if variant=='original':body=body.replace(REJECT,STAGED_REJECT,1)
 start=body.index('    let capture = record\n');stop=body.index('    if let Some(old) = &capture.end')
 prefix=body[start:stop]
 if not prefix.endswith('    };\n'):raise ValueError('entire original proposal prefix required')
 helper=('\nfn propose_end(record: &SourceWorkRecord, observation: Option<CaptureRequestEndObservation>) -> Result<CaptureEndRecord, JournalError> {\n'
         +prefix+'    Ok(proposed)\n}\n')
 root=(body[:start]+'    let proposed = propose_end(record, observation)?;\n'
       +'    let capture = record.capture.as_ref().ok_or(JournalError::UnresolvedAction)?;\n'+body[stop:])
 old="""        record
            .capture
            .as_mut()
            .ok_or(JournalError::CorruptStore)?
            .end = Some(proposed);"""
 new="""        let capture = match &mut record.capture {
            Some(capture) => capture,
            None => return Err(JournalError::CorruptStore),
        };
        capture.end = Some(proposed);"""
 if root.count(old)!=1:raise ValueError('one original mutable End assignment required')
 root=root.replace(old,new,1)
 encoder='\nfn encode_proposed_end(value: &CaptureEndRecord) -> serde_json::Result<Vec<u8>> {\n    serde_json::to_vec(value)\n}\n'
 path.write_text(text[:begin]+root+text[end:]+helper+encoder)
