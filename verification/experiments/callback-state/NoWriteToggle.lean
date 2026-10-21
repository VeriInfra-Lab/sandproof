import SandproofCallbackState.Funs
import SandboxModel.Foundations.CallbackState.Contracts
import SandboxModel.ProofAuditCommand
namespace SandboxModel.Foundations.CallbackState.WrongToggle
open Aeneas Aeneas.Std
theorem observed_wrong_toggle :
    SandproofCallbackStateRust.toggle_step false 0#u64 0#u64 = .ok (true, false) := by rfl
theorem same_toggle_contract_rejects :
    ¬ ToggleContract SandproofCallbackStateRust.toggle_step := by
  intro contract
  have expected := contract false 0#u64 0#u64
  rw [observed_wrong_toggle] at expected
  simp at expected
end SandboxModel.Foundations.CallbackState.WrongToggle

#audit_controller_proof SandboxModel.Foundations.CallbackState.WrongToggle.observed_wrong_toggle
#audit_controller_proof SandboxModel.Foundations.CallbackState.WrongToggle.same_toggle_contract_rejects
