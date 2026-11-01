"""Next owned slice: original Save digest and its actual borrowed adapter.

Private source preparation only; no production or published input is written.
The Load SHA candidate supplies the same one pure library naming interface.
"""
from pathlib import Path
import hashlib
import importlib.util

ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/standard-save-digest')
SOURCE='controller-service/src/controller/lifecycle_inbox/codec.rs'
OLD='    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {\n        Sha256::digest(bytes).into()\n    }'
NAMED=OLD.replace('Sha256::digest(bytes).into()','sha256_bytes(bytes)')
DELEGATE='''    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        InboxSaveEffects::digest(
            &mut StandardInboxSave {
                connection: self.connection,
            },
            bytes,
        )
    }'''
VARIANTS={
 'actual':NAMED,
 'save-input-empty':NAMED.replace('sha256_bytes(bytes)','sha256_bytes(&bytes[..0])'),
 'save-tail-zero':'''    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        let mut digest = sha256_bytes(bytes);
        if !bytes.is_empty() {
            digest[31] = 0;
        }
        digest
    }''',
 'delegate-input-empty':NAMED,
}

def sha(path):
 return hashlib.sha256(path.read_bytes()).hexdigest()

def candidate(project,variant='actual'):
 path=project/SOURCE
 original=path.read_bytes()
 if variant=='original':
  return dict(variant=variant,source_unchanged=True,source_sha256=sha(path)),{}
 spec=importlib.util.spec_from_file_location('next_save_existing_pure_SHA_candidate',
  ROOT/'verification/experiments/stage1-protocol/standard-digest/preparation.py')
 parent=importlib.util.module_from_spec(spec);spec.loader.exec_module(parent)
 previous,changed=parent.candidate(project,'actual')
 text=path.read_text()
 if variant not in (*VARIANTS, 'restored') or text.count(OLD)!=1 or text.count(DELEGATE)!=1:
  raise ValueError('not the exact current Save method and original delegation')
 position=text.index(OLD)
 if not (text.index("impl InboxSaveEffects for StandardInboxSave<'_>") < position
   < text.index('// The existing connection carrier')):
  raise ValueError('method is not the original StandardInboxSave digest field')
 method=VARIANTS['actual' if variant=='restored' else variant]
 prepared=text.replace(OLD,method,1)
 actual_delegate=DELEGATE
 if variant=='delegate-input-empty':
  actual_delegate=DELEGATE.replace('            bytes,','            &bytes[..0],')
  prepared=prepared.replace(DELEGATE,actual_delegate,1)
 # Full inverse to the exact parent source, then the exact actual production.
 restored=prepared.replace(actual_delegate,DELEGATE,1)
 if restored[position:position+len(method)]!=method:
  raise ValueError('actual Save method span changed')
 restored=restored[:position]+OLD+restored[position+len(method):]
 if restored!=text:
  raise ValueError('an unrelated method or receiver field changed')
 restoration=None
 if variant=='restored':
  if prepared[position:position+len(NAMED)]!=NAMED:raise ValueError('actual Save restoration span changed')
  wrong=prepared[:position]+VARIANTS['save-tail-zero']+prepared[position+len(NAMED):]
  if wrong==prepared:raise ValueError('restoration requires an actual Save mutation')
  path.write_text(wrong);restoration=dict(actual_wrong_sha256=sha(path),exact_candidate_restored=True)
 path.write_text(prepared)
 return dict(variant=variant,source=SOURCE,parent_preparation=previous,
  prepared_sha256=sha(path),production_original_sha256=hashlib.sha256(original).hexdigest(),
  unrelated_parent_bytes_preserved=True,production_modified=False,
  kernel_or_native_result=False,restoration=restoration),{SOURCE:sha(path)}

def view(project,variant='actual'):
 derivation,changed=candidate(project,variant)
 path=project/SOURCE
 if 'fn proof_stage1_save_digest(' in path.read_text() or 'fn proof_stage1_read_save_digest(' in path.read_text():
  raise ValueError('transparent Save roots already present')
 path.write_bytes(path.read_bytes()+(ROOT/HERE/'root.rs.in').read_bytes());changed[SOURCE]=sha(path)
 return derivation,changed

def generic_view(project):
 # Same existing thin roots as the original Save/validation package. Their old
 # entire-tree freeze is not promoted to this current source. Only append-only
 # instrumentation is applied; every original production byte is preserved.
 spec=importlib.util.spec_from_file_location('save_existing_append_only_roots',
  ROOT/HERE.parent/'standard-blob/preparation.py')
 previous=importlib.util.module_from_spec(spec);spec.loader.exec_module(previous)
 providers=[(SOURCE,'inbox-save/generic-root.rs.in','proof_inbox_save_using'),
  ('controller-service/src/controller/lifecycle_inbox/model.rs','inbox-validation/generic-root.rs.in','proof_inbox_validate_using'),
  ('controller-service/src/capture.rs','inbox-validation/primitive-roots.rs.in','proof_standard_graphic_bytes'),
  ('verification/experiments/stage1-protocol/inbox-record/lib.rs','terminal-binding/roots.rs.in','terminal_matches')]
 changed={};before={}
 for relative,suffix,marker in providers:
  before[relative]=(project/relative).read_bytes()
  changed[relative]=previous.append(project,relative,HERE.parent/suffix,marker)
 relative='verification/experiments/stage1-protocol/inbox-record/lib.rs';source=project/relative
 fragment=__import__('json').loads((ROOT/HERE.parent/'inbox-renewal-session/fragments.json').read_text())['renewal_roots']
 if 'fn renewal_valid(' in source.read_text():raise ValueError('renewal roots already instrumented')
 source.write_text(source.read_text()+fragment);changed[relative]=sha(source)
 for relative,original in before.items():
  if not (project/relative).read_bytes().startswith(original):raise ValueError('original production bytes changed')
 return dict(only_existing_thin_roots_appended=True,original_generic_save_body_unchanged=True,
  old_freeze_or_execution_retagged=False,original_inputs={n:hashlib.sha256(b).hexdigest()for n,b in before.items()}),changed
