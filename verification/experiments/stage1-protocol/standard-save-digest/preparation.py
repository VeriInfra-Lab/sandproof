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
NAMED='    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {\n        super::sha256::digest(bytes)\n    }'
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
 'save-input-empty':NAMED.replace('super::sha256::digest(bytes)','super::sha256::digest(&bytes[..0])'),
 'save-tail-zero':'''    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        let mut digest = super::sha256::digest(bytes);
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
 spec=importlib.util.spec_from_file_location('save_adopted_unique_SHA_source',
  ROOT/'verification/experiments/stage1-protocol/standard-digest/preparation.py')
 parent=importlib.util.module_from_spec(spec);spec.loader.exec_module(parent)
 data,text,_,_=parent.admitted_source(project)
 if variant not in ('original',*VARIANTS,'restored') or text.count(DELEGATE)!=1:
  raise ValueError('not the exact adopted Save method and original delegation')
 start=text.index("impl InboxSaveEffects for StandardInboxSave<'_>")
 end=text.index('// The existing connection carrier',start)
 block=text[start:end]
 if block.count(NAMED)!=1:
  raise ValueError('the unique actual StandardInboxSave digest changed')
 method=VARIANTS['actual' if variant in ('original','restored') else variant]
 prepared=text[:start]+block.replace(NAMED,method,1)+text[end:]
 actual_delegate=DELEGATE
 if variant=='delegate-input-empty':
  actual_delegate=DELEGATE.replace('            bytes,','            &bytes[..0],')
  prepared=prepared.replace(DELEGATE,actual_delegate,1)
 restored=prepared.replace(actual_delegate,DELEGATE,1)
 changed_block=restored[start:start+len(block)-len(NAMED)+len(method)]
 if changed_block.count(method)!=1:
  raise ValueError('actual Save method span changed')
 restored=restored[:start]+changed_block.replace(method,NAMED,1)+restored[start+len(changed_block):]
 if restored!=text:
  raise ValueError('an unrelated current method or receiver field changed')
 restoration=None
 if variant=='restored':
  wrong=text[:start]+block.replace(NAMED,VARIANTS['save-tail-zero'],1)+text[end:]
  path.write_text(wrong)
  restoration=dict(actual_wrong_sha256=sha(path),exact_adopted_source_restored=True)
 path.write_text(prepared)
 return dict(variant=variant,source=SOURCE,prepared_sha256=sha(path),
  production_original_sha256=data['adopted_source_sha256'],
  unrelated_production_bytes_preserved=True,production_adopted=True,
  actual_unique_leaf=parent.LEAF,private_business_replacement=False,
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
