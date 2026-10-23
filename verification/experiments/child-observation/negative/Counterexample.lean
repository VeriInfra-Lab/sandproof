import SandboxModel.Boundaries.ChildObservation.Model
import SandboxModel.Boundaries.ChildObservation.Library
import SandboxModel.Boundaries.ChildObservation.Fixtures
import SandproofChildObservation.Funs
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
open SandproofChildObservationRust
namespace SandboxModel.Boundaries.ChildObservation.Counterexample
open Interface Examples

def releasedIo : Io := {
  seeded := true
  schedule := {waitingChild.schedule with polls := [.Ok none], kills := [], errors := []}
  history := [.poll 37 (.Ok none), .kill 37 (.Ok ()), .timeoutError unknownExit]
}

theorem mutant_returns_normally :
    child_process.observation_core.stop Library.ops (some waitingChild) (some originalSlot) {} =
      .ok (.Err unknownExit, none, none, releasedIo) := by
  simp [child_process.observation_core.stop, Library.ops, Library.poll, Library.kill,
    Library.error, Library.seed, Library.takeReply, Library.completed,
    waitingChild, originalSlot, releasedIo, core.option.Option.take,
    core.mem.drop]

theorem mutant_has_no_handoff :
    Event.handedOff 37 originalSlot ∉ releasedIo.history := by
  simp [releasedIo]

theorem mutant_rejected_by_same_accounted :
    ¬ Accounted Child.binding Io.history unknownExit 37 originalSlot
      (.Err unknownExit, none, none, releasedIo) := by
  rintro ⟨_, _, _, before, same⟩
  apply mutant_has_no_handoff
  rw [same]
  simp

theorem mutant_violates_normal_contract :
    ¬ WP.dspec (child_process.observation_core.stop Library.ops
        (some waitingChild) (some originalSlot) {})
      (Accounted Child.binding Io.history unknownExit waitingChild.binding originalSlot) := by
  intro claimed
  have safe := WP.dspec_imp_forall claimed _ mutant_returns_normally
  exact mutant_rejected_by_same_accounted safe

end SandboxModel.Boundaries.ChildObservation.Counterexample

#audit_controller_proof SandboxModel.Boundaries.ChildObservation.Counterexample.mutant_returns_normally
#audit_controller_proof SandboxModel.Boundaries.ChildObservation.Counterexample.mutant_has_no_handoff
#audit_controller_proof SandboxModel.Boundaries.ChildObservation.Counterexample.mutant_rejected_by_same_accounted
#audit_controller_proof SandboxModel.Boundaries.ChildObservation.Counterexample.mutant_violates_normal_contract
