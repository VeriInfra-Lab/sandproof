import SandproofCallbackState.Funs
import SandboxModel.Foundations.CallbackState.Contracts
import SandboxModel.ProofAuditCommand
namespace SandboxModel.Foundations.CallbackState.WrongTrace
open Aeneas Aeneas.Std

def emptyTrace : TraceValue := alloc.vec.Vec.from [] (by simp)

theorem observed_double_record :
    SandproofCallbackStateRust.trace_step emptyTrace 0#u64 1#u64 0#u64
      ⦃ result => result.2.val = [(0#u64, 1#u64), (0#u64, 1#u64)] ⦄ := by
  have emptyRoom : emptyTrace.val.length < Usize.max := by
    simp [emptyTrace]
    scalar_tac
  unfold SandproofCallbackStateRust.trace_step
  step as ⟨first, firstPost⟩
  have room : first.val.length < Usize.max := by
    simp [firstPost, emptyTrace]
    scalar_tac
  step as ⟨second, secondPost⟩
  simp [secondPost, firstPost, emptyTrace]

theorem spec_return_witness {run : Result α} {post : α → Prop}
    (completed : run ⦃ post ⦄) : ∃ output, run = .ok output ∧ post output := by
  cases completed with
  | ret property => exact ⟨_, rfl, property⟩

theorem same_trace_contract_rejects :
    ¬ TraceContract SandproofCallbackStateRust.trace_step := by
  intro contract
  have expected := contract emptyTrace 0#u64 1#u64 0#u64 (by simp [emptyTrace]; scalar_tac)
  obtain ⟨output, evaluated, actual⟩ := spec_return_witness observed_double_record
  rw [evaluated] at expected
  simp only [WP.spec_ok] at expected
  have same := expected.2
  rw [actual] at same
  simp [emptyTrace] at same
end SandboxModel.Foundations.CallbackState.WrongTrace

#audit_controller_proof SandboxModel.Foundations.CallbackState.WrongTrace.observed_double_record
#audit_controller_proof SandboxModel.Foundations.CallbackState.WrongTrace.spec_return_witness
#audit_controller_proof SandboxModel.Foundations.CallbackState.WrongTrace.same_trace_contract_rejects
