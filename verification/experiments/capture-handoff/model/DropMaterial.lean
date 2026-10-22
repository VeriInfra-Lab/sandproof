import SandboxModel.Lifecycle.CaptureHandoff.Conditions
import SandboxModel.ProofAuditCommand
import CaptureHandoffDrop.Funs

open Aeneas Aeneas.Std
open CaptureHandoffDropRust.sandbox_controller_core.capture

namespace SandboxModel.Lifecycle.CaptureHandoffProof.DropRejected

abbrev State := CaptureHandoffDropRust.sandbox_controller_core.capture.CaptureHandoff
def useView (state : State) : Bool × Bool := (state.capture_work, state.unload_source)

theorem same_property_rejects :
    ¬ FailurePreservesUses CaptureHandoffDropRust.pause_failure useView := by
  intro law
  let original : State := ⟨.Registered, .Recoverable, true, true⟩
  let lost : State := ⟨.Registered, .Recoverable, false, false⟩
  have observed : CaptureHandoffDropRust.pause_failure original = .ok lost := by
    simp [original, lost, CaptureHandoffDropRust.pause_failure,
      CaptureHandoff.ordinary_failure, core.cmp.PartialEq.ne.trait_default,
      core.cmp.PartialEq.ne.default,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq, CaptureCommit.read_discriminant]
  rcases law original with ⟨after, run, preserved⟩
  rw [observed] at run
  simp only [Result.ok.injEq] at run
  subst after
  simp [useView, lost, original] at preserved

end SandboxModel.Lifecycle.CaptureHandoffProof.DropRejected

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.DropRejected.same_property_rejects
