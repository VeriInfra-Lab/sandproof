import SaveLoadCanonicalComposition
import SaveLoadCanonicalWrong

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadCanonicalWitness
open SaveLoadCanonicalView SaveLoadCanonicalProof SaveLoadCollectorView SaveLoadCursorView
noncomputable section

def sampleRecord : LoadedRecord := SaveEncoder.Witness.row
def canonical : Bytes := alloc.vec.Vec.from [7#u8] (by scalar_tac)
def alternate : Bytes := alloc.vec.Vec.from [8#u8] (by scalar_tac)
def checksum : Array U8 32#usize := Array.repeat 32#usize 0#u8

def codec : Codec where
  parse _ := .ok (.Ok (some sampleRecord))
  serialize _ := .ok (.Ok canonical)
  checksum _ := .ok checksum
  anchorJson _ := .ok (.Ok canonical)
  workHash _ := .ok sampleRecord.work.requirement.work_binding

theorem canonical_and_alternate_differ : canonical.val ≠ alternate.val := by decide

theorem original_accepts_normal_canonical_bytes :
    checked codec canonical.deref checksum.to_slice = .ok (.Ok (some sampleRecord)) := by
  rw [actual_complete_bytes_decide_normal_parse codec _ _ (some sampleRecord) checksum canonical
    rfl rfl (by simp [Array.val_to_slice]) rfl]
  have full : canonical.val = canonical.deref.val := rfl
  simp [full]

theorem original_rejects_noncanonical_reply :
    checked codec alternate.deref checksum.to_slice = .ok (.Err .CorruptStore) := by
  rw [actual_complete_bytes_decide_normal_parse codec _ _ (some sampleRecord) checksum canonical
    rfl rfl (by simp [Array.val_to_slice]) rfl]
  have different : canonical.val ≠ alternate.deref.val := canonical_and_alternate_differ
  simp [different]

theorem missing_comparison_accepts_same_noncanonical_reply :
    SaveLoadCanonicalWrong.checked codec alternate.deref checksum.to_slice = .ok (.Ok (some sampleRecord)) := by
  simp [SaveLoadCanonicalWrong.checked,
    SaveLoadCanonicalWrong.controller.lifecycle_inbox.source_work.codec.checked_source_payload,
    codec, core.result.Result.map_err, core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.impls.PartialEqShared.ne, SaveLoadSql.bytesCompare, lift, Array.val_to_slice]

theorem missing_comparison_violates_same_canonical_contract :
    ¬ FullCanonicalContract SaveLoadCanonicalWrong.checked := by
  intro contract
  obtain ⟨hash, written, _, _, _, serialized, exactBytes⟩ := contract codec alternate.deref checksum.to_slice
    (some sampleRecord) missing_comparison_accepts_same_noncanonical_reply
  have reply : canonical = written := by
    simpa [codec] using serialized
  subst written
  exact canonical_and_alternate_differ exactBytes

end
end SandboxModel.Stage1Protocol.SaveLoadCanonicalWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalWitness.canonical_and_alternate_differ
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalWitness.original_accepts_normal_canonical_bytes
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalWitness.original_rejects_noncanonical_reply
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalWitness.missing_comparison_accepts_same_noncanonical_reply
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalWitness.missing_comparison_violates_same_canonical_contract
