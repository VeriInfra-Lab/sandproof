import SaveLoadCollectorComposition
import SaveLoadCollectorWrong

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadCollectorWitness
open SaveLoadCollectorView SaveLoadCollectorProof SaveLoadCursorView
noncomputable section

def sampleRecord : LoadedRecord := SaveEncoder.Witness.row
def sampleAnchor : Bytes := alloc.vec.Vec.from [7#u8] (by scalar_tac)
def storedKey : Bytes := alloc.vec.Vec.from (SaveEncoder.keyOf sampleRecord).val (by scalar_tac)
def foreignBlob : Bytes := alloc.vec.Vec.from
  (Array.repeat 32#usize (0#u8)).val (by scalar_tac)

def samplePorts : Ports where
  checked _ _ := .ok (.Ok (some sampleRecord))
  anchor _ := .ok (.Ok sampleAnchor)
  digest _ := .ok sampleRecord.work.requirement.work_binding

theorem foreign_key_differs_in_full_content :
    foreignBlob.val ≠ (SaveEncoder.keyOf sampleRecord).val := by
  intro same
  apply SaveEncoder.Witness.nonzero
  exact Aeneas.Std.Array.ext _ _ same

theorem normal_original_load_preserves_record :
    decode samplePorts sampleAnchor.deref sampleAnchor.deref (some storedKey) =
      .ok (.Ok (some sampleRecord)) := by
  exact original_decode_accepts_exact_key_and_digest samplePorts _ _ storedKey sampleAnchor
    sampleRecord rfl rfl rfl rfl

theorem original_rejects_foreign_full_key :
    decode samplePorts sampleAnchor.deref sampleAnchor.deref (some foreignBlob) =
      .ok (.Err .CorruptStore) :=
  foreign_blob_is_rejected_before_anchor samplePorts _ _ foreignBlob sampleRecord rfl
    foreign_key_differs_in_full_content

theorem length_only_accepts_same_record_under_foreign_key :
    SaveLoadCollectorWrong.decode samplePorts sampleAnchor.deref sampleAnchor.deref (some foreignBlob) =
      .ok (.Ok (some sampleRecord)) := by
  simp [SaveLoadCollectorWrong.decode,
    SaveLoadCollectorWrong.controller.lifecycle_inbox.source_work.codec.decode_loaded_record,
    samplePorts, foreignBlob, alloc.vec.Vec.len, SourceWorkMatch.Scalars.wire_content_digest_eq,
    core.result.Result.Insts.CoreOpsTry.branch]

theorem length_only_violates_same_full_key_contract :
    ¬ FullStoredKeyContract SaveLoadCollectorWrong.decode := by
  intro contract
  obtain ⟨blob, equal, full⟩ := contract samplePorts sampleAnchor.deref sampleAnchor.deref
    (some foreignBlob) sampleRecord length_only_accepts_same_record_under_foreign_key
  simp at equal
  subst blob
  exact foreign_key_differs_in_full_content full

end
end SandboxModel.Stage1Protocol.SaveLoadCollectorWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorWitness.foreign_key_differs_in_full_content
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorWitness.normal_original_load_preserves_record
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorWitness.original_rejects_foreign_full_key
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorWitness.length_only_accepts_same_record_under_foreign_key
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorWitness.length_only_violates_same_full_key_contract
