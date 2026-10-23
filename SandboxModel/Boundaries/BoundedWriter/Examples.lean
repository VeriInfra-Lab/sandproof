import SandboxModel.Boundaries.BoundedWriter.Refinement
import SandboxModel.Boundaries.BoundedWriter.Library

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.BoundedWriter.Refinement
open SandproofBoundedWriterRust
open sandproof_controller_service.template_registry.evidence
namespace Examples

theorem nonempty_actual_write_is_accepted :
    write_bounded (ordinaryOps false) (initial 1#usize) oneByte =
      .ok (.Ok 1#usize, ⟨withByte, 1#usize, false⟩) := by
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

theorem actual_zero_budget_refuses_nonempty :
    write_bounded (ordinaryOps false) (initial 0#usize) oneByte =
      .ok (.Err ⟨0⟩, ⟨emptyBuffer, 0#usize, true⟩) := by
  simp only [write_bounded, ordinaryOps, initial, bind_tc_ok, empty_one_sum, lift]
  simp [emptyBuffer]

theorem actual_allocation_refusal_is_not_a_size_fault :
    write_bounded (ordinaryOps true) (initial 1#usize) oneByte =
      .ok (.Err ⟨1⟩, initial 1#usize) := by
  simp only [write_bounded, ordinaryOps, initial, bind_tc_ok, empty_one_sum, lift]
  simp [emptyBuffer, oneByte, reserveBuffer, appendBuffer,
    core.result.Result.map_err,
    write_bounded.closure.Insts.CoreOpsFunctionFnOnceTupleClause0_ReserveErrorError.call_once,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    core.convert.FromSame.from]


end Examples
end SandboxModel.Boundaries.BoundedWriter.Refinement
