import SaveLoadDecision
import SaveLoadCollectorWitness

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadDecisionWitness
open SaveLoadDecision SaveLoadCollectorView SaveLoadCursorView
open SaveLoadCollectorWitness
noncomputable section

def emptyReply : Ports where
  checked _ _ := .ok (.Ok none)
  anchor _ := .fail .panic
  digest _ := .fail .panic

theorem normal_vacant_row_returns_none_without_anchor :
    SaveLoadCollectorView.decode emptyReply sampleAnchor.deref sampleAnchor.deref none = .ok (.Ok none) :=
  original_vacant_slot_requires_null_key emptyReply _ _ none rfl

theorem present_key_for_vacant_row_rejects_before_anchor :
    SaveLoadCollectorView.decode emptyReply sampleAnchor.deref sampleAnchor.deref (some storedKey) =
      .ok (.Err .CorruptStore) :=
  original_vacant_slot_requires_null_key emptyReply _ _ (some storedKey) rfl

theorem length_only_wrong_refutes_same_complete_decision :
    ¬ CompleteDecisionContract SaveLoadCollectorWrong.decode := by
  intro contract
  have expected := contract samplePorts sampleAnchor.deref sampleAnchor.deref (some foreignBlob)
  rw [← actual_decode_refines_complete_nullable_decision] at expected
  rw [length_only_accepts_same_record_under_foreign_key, original_rejects_foreign_full_key] at expected
  simp at expected

end
end SandboxModel.Stage1Protocol.SaveLoadDecisionWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecisionWitness.normal_vacant_row_returns_none_without_anchor
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecisionWitness.present_key_for_vacant_row_rejects_before_anchor
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecisionWitness.length_only_wrong_refutes_same_complete_decision
