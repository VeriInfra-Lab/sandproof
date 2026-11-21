"""Compose the unchanged reviewed staging views, rejecting conflicting edits."""
import importlib.util
from pathlib import Path
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'source-remove-retired'))
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path)
 value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
parent=module('source_drain_saved_parent',HERE.parent/'source-saved-question/capture.py')
terminal=module('source_drain_terminal_parent',HERE.parent/'source-terminal-match/capture.py')
prior=module('source_drain_prior_parent',HERE.parent/'source-prior-end-match/capture.py')
retire=parent.retire
END=Path('controller-service/src/controller/lifecycle_inbox/source_work/full/drain.rs')
VARIANTS=('original','allow-mismatched-prior')
ROOT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::drain::writer_closed'
VALIDATE='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::drain::validate_record'
NAMESPACE='SandproofSourceDrainValidation'
STAGED=tuple(dict.fromkeys((*parent.STAGED,*terminal.STAGED,END)))
OPTIONS=dict(parent.OPTIONS,roots=[ROOT],retained_roots=[VALIDATE],
 include=parent.OPTIONS['include'],
 opaque=[*parent.OPTIONS['opaque'],*terminal.PORTS])
PORTS=terminal.PORTS

def mutate(text,variant):
 if variant not in VARIANTS:raise ValueError('unknown original drain validation view')
 start=text.index('pub(super) fn validate_record(')
 stop=text.index('pub(super) fn writer_closed(',start)
 body=text[start:stop]
 marker='    if let Some(CaptureEndRecord::Terminated {\n        prior: Some(prior), ..\n    }) = record\n        .capture\n        .as_ref()\n        .and_then(|capture| capture.end.as_ref())\n        && !prior_matches(record, prior)?\n    {\n        return Err(JournalError::CorruptStore);\n    }\n'
 if body.count(marker)!=1:raise ValueError('one exact original prior rejection branch required')
 replacement=marker.replace('        return Err(JournalError::CorruptStore);\n','',1)
 return text[:start]+(body if variant=='original' else body.replace(marker,replacement,1))+text[stop:]

def reconstruct(archive,source,variant):
 try:parent.reconstruct(archive,source,'original')
 finally:retire.STAGED=STAGED
 with tarfile.open(archive) as frozen:
  with TemporaryDirectory(prefix='drain-original-ports-',dir=source.parent) as temporary:
   other=Path(temporary)
   terminal.reconstruct(archive,other,'original')
   for relative in terminal.STAGED:
    left,right=source/relative,other/relative
    original=frozen.extractfile(str(relative)).read()
    a,b=left.read_bytes(),right.read_bytes()
    if a!=b:
     if a==original:left.write_bytes(b)
     elif b!=original:raise ValueError('reviewed staging views conflict: '+str(relative))
 path=source/END
 text=path.read_text()
 start=text.index('fn prior_matches(');stop=text.index('\nfn terminal_end(',start)
 body=text[start:stop]
 if body.count(prior.CLAUSE)!=1:raise ValueError('exact original late prior clause required')
 text=text[:start]+body.replace(prior.CLAUSE,'prior_completed_no_request(capture.completed.is_some(), observation)',1)+text[stop:]+prior.HELPER
 text=mutate(text,variant)
 start=text.index('pub(super) fn validate_record(');stop=text.index('pub(super) fn writer_closed(',start)
 body=text[start:stop]
 old='    if let Some(CaptureEndRecord::Terminated {\n        prior: Some(prior), ..\n    }) = record\n        .capture\n        .as_ref()\n        .and_then(|capture| capture.end.as_ref())'
 new='    if let Some(capture) = record.capture.as_ref()\n        && let Some(CaptureEndRecord::Terminated {\n            prior: Some(prior), ..\n        }) = capture.end.as_ref()'
 if body.count(old)!=1:raise ValueError('one complete original Option selector required')
 text=text[:start]+body.replace(old,new,1)+text[stop:]
 path.write_text(text)
 retire.STAGED=STAGED
