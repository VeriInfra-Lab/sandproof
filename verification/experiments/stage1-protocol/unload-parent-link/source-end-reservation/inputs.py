"""Original saved-capture reservation with one exact typed encoding boundary."""
import importlib.util,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'source-remove-retired'))
spec=importlib.util.spec_from_file_location('source_end_reservation_parent',HERE.parent/'source-end-receipt/capture.py')
parent=importlib.util.module_from_spec(spec);spec.loader.exec_module(parent)
retire=parent.retire
END=parent.END
ROOT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::reserved_record_bytes'
NAMESPACE='SandproofSourceEndReservation'
VARIANTS=('original','skip-future-end-budget')
PORT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::encode_reserved_capture'
STAGED=parent.STAGED
OPTIONS=dict(parent.OPTIONS,roots=[ROOT],retained_roots=[parent.ROOT],opaque=[*parent.OPTIONS['opaque'],PORT])

def mutate(text,variant):
 if variant not in VARIANTS:raise ValueError('unknown reserved capture view')
 a=text.index('pub(super) fn reserved_record_bytes(');b=text.index('\npub(super) fn validate_record(',a);body=text[a:b]
 guard='            END_RECORD_BYTES\n'
 if body.count(guard)!=1:raise ValueError('one original future End reservation required')
 if variant!='original':body=body.replace(guard,'            0\n',1)
 return text[:a]+body+text[b:]

def reconstruct(archive,source,variant):
 try:parent.reconstruct(archive,source,'original')
 finally:retire.STAGED=STAGED
 path=source/END;text=mutate(path.read_text(),variant)
 a=text.index('pub(super) fn reserved_record_bytes(');b=text.index('\npub(super) fn validate_record(',a);body=text[a:b]
 call='serde_json::to_vec(capture)?.len()'
 if body.count(call)!=1:raise ValueError('one original complete CaptureWork encoding required')
 body=body.replace(call,'encode_reserved_capture(capture)?.len()',1)
 helper='\nfn encode_reserved_capture(value: &CaptureWork) -> serde_json::Result<Vec<u8>> {\n    serde_json::to_vec(value)\n}\n'
 path.write_text(text[:a]+body+text[b:]+helper)
