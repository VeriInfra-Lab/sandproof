import SandboxModel.Lifecycle.CaptureHandoff.CallerGuards

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- A value view of precisely the identity checks performed here. VM and target
are intentionally not claimed: this function does not check them. -/
noncomputable def currentCaptureView (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId)
    (record : sandbox_controller_core.model.SandboxRecord) :
    Option sandbox_controller_core.capture.CaptureHandoff := by
  classical
  exact do
    let operation ← record.current_operation
    if operation.kind = .Pause ∧ operation.epoch = record.epoch then
      let plan ← tableLookup ⟨sandbox, operation.id⟩ world.operation_plans.entries.val
      if plan.key = ⟨sandbox, operation.id⟩ ∧ plan.epoch = record.epoch ∧
          plan.generation = record.generation then plan.capture else none
    else none

theorem current_capture_exact (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId)
    (record : sandbox_controller_core.model.SandboxRecord) :
    sandbox_controller_core.transition.current_capture world sandbox record =
      .ok (currentCaptureView world sandbox record) := by
  classical
  cases operation : record.current_operation with
  | none =>
    simp [currentCaptureView, sandbox_controller_core.transition.current_capture, operation,
      core.option.Option.Insts.CoreOpsTry_traitTry.branch,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual]
  | some op =>
    have query := get_exact
      sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operation_key_lawful world.operation_plans ⟨sandbox, op.id⟩
    cases kind : op.kind <;>
      simp [currentCaptureView, sandbox_controller_core.transition.current_capture, operation,
        kind, sandbox_controller_core.transition.operation_key,
        core.option.Option.Insts.CoreOpsTry_traitTry.branch,
        core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual,
        sandbox_controller_core.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
        sandbox_controller_core.model.OperationKind.read_discriminant,
        core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        query, operation_key_eq_exact, ite_and, ← UScalar.eq_equiv]
    cases lookup : tableLookup ⟨sandbox, op.id⟩ world.operation_plans.entries.val <;>
      simp_all
    split_ifs <;> rfl

end SandboxModel.Lifecycle.CaptureHandoffCaller
