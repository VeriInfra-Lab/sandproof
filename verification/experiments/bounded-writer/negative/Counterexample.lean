import SandboxModel.ProofAuditCommand
import SandboxModel.Boundaries.BoundedWriter.Contract
import SandboxModel.Boundaries.BoundedWriter.Library
import SandproofBoundedWriter.Funs

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.BoundedWriter.Negative
open SandproofBoundedWriterRust
open sandproof_controller_service.template_registry.evidence
open SandboxModel.Boundaries.BoundedWriter.Refinement
open SandboxModel.Boundaries.BoundedWriter.Refinement.Examples

/-- Computed from the newly extracted wrong Rust, not a hand-written wrong algorithm. -/
theorem wrong_actual_write_accepts_past_zero_budget :
    write_bounded (ordinaryOps false) (initial 0#usize) oneByte =
      .ok (.Ok 1#usize, ⟨withByte, 0#usize, false⟩) := by
  have oneBound : 1 ≤ Usize.max := by scalar_tac
  simp only [write_bounded, ordinaryOps, initial, bind_tc_ok, empty_one_sum, lift]
  simp only [one_count]
  simp [emptyBuffer, oneByte, withByte, reserveBuffer, appendBuffer, oneBound,
    core.result.Result.map_err,
    write_bounded.closure.Insts.CoreOpsFunctionFnOnceTupleClause0_ReserveErrorError.call_once,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    core.convert.FromSame.from]
  apply UScalar.eq_of_val_eq
  scalar_tac

theorem wrong_actual_write_fails_same_contract :
    ¬ WriteContract ordinaryView (write_bounded (ordinaryOps false)) := by
  intro contract
  have post := contract (initial 0#usize) oneByte (.Ok 1#usize)
    ⟨withByte, 0#usize, false⟩ wrong_actual_write_accepts_past_zero_budget
  have bound := post.content.2.2.1
  simp [writerView, IsBounded, ordinaryView, withByte] at bound

theorem wrong_rejected_with_satisfied_library_conditions :
    BufferLaws (ordinaryOps false) ordinaryView ∧
      ¬ WriteContract ordinaryView (write_bounded (ordinaryOps false)) :=
  ⟨ordinary_operations_satisfy_laws false, wrong_actual_write_fails_same_contract⟩

end SandboxModel.Boundaries.BoundedWriter.Negative

#audit_controller_proof SandboxModel.Boundaries.BoundedWriter.Negative.wrong_actual_write_accepts_past_zero_budget
#audit_controller_proof SandboxModel.Boundaries.BoundedWriter.Negative.wrong_actual_write_fails_same_contract
#audit_controller_proof SandboxModel.Boundaries.BoundedWriter.Negative.wrong_rejected_with_satisfied_library_conditions
