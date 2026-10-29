import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- The original late-entry checks hand exactly the queried original plan to
commit. logicalRead is an actual generated lookup equation, not an assertion
that the outer handler is correct or an external business-function stub. -/
theorem record_inner_commit_connects
    (world : sandbox_controller_core.model.World)
    (id : sandbox_controller_core.model.EffectId)
    (row : sandbox_controller_core.model.SandboxRecord)
    (logical : sandbox_controller_core.model.LogicalEffectRecord)
    (key : sandbox_controller_core.model.OperationKey)
    (plan : sandbox_controller_core.model.OperationPlan)
    (current : sandbox_controller_core.model.Operation)
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (command : id.logical.command = .CommitSnapshot sandbox epoch generation snapshot manifest)
    (logicalRead : sandbox_controller_core.model.World.logical_effect world id.logical = .ok (some logical))
    (origin : logical.origin = .Operation key)
    (storedPlan : tableLookup key world.operation_plans.entries.val = some plan)
    (planKey : plan.key = key) (session : key.sandbox = sandbox)
    (currentOperation : row.current_operation = some current)
    (differentOperation : key.operation ≠ current.id)
    (planGeneration : plan.generation = row.generation)
    (planEpoch : epoch = plan.epoch)
    (hasCapture : plan.capture = some capture)
    (cancelled : capture.eligibility = .Revoked) (working : capture.capture_work = true) :
    sandbox_controller_core.worker.late_capture.record_inner world id row =
      sandbox_controller_core.worker.late_capture.commit world row plan
        (.CommitSnapshot sandbox epoch generation snapshot manifest) := by
  have getPlan := get_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operation_plans key
  simp [sandbox_controller_core.worker.late_capture.record_inner,
    sandbox_controller_core.model.EffectId.command, command,
    sandbox_controller_core.model.Command.sandbox, sandbox_controller_core.model.Command.epoch,
    logicalRead, origin, sandbox_controller_core.model.World.operation_plan,
    getPlan, storedPlan, planKey, session, currentOperation, differentOperation,
    planGeneration, planEpoch, hasCapture, cancelled, working,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
    core.option.Option.Insts.CoreCmpPartialEqOption.eq,
    core.option.Option.is_some_and, SandproofFoundationsRust.core.option.Option.is_some_and,
    sandbox_controller_core.worker.late_capture.record_inner.closure.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once,
    sandbox_controller_core.capture.CaptureHandoff.impl.eligibility,
    sandbox_controller_core.capture.CaptureHandoff.has_capture_work,
    sandbox_controller_core.capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
    sandbox_controller_core.capture.RestoreEligibility.read_discriminant]

/-- All five binding guards reject before material adoption. This does not
claim a VM check for CommitSnapshot, whose command has no VM argument. -/
theorem record_inner_commit_rejects_bad_binding
    (world : sandbox_controller_core.model.World)
    (id : sandbox_controller_core.model.EffectId)
    (row : sandbox_controller_core.model.SandboxRecord)
    (logical : sandbox_controller_core.model.LogicalEffectRecord)
    (key : sandbox_controller_core.model.OperationKey)
    (plan : sandbox_controller_core.model.OperationPlan)
    (current : sandbox_controller_core.model.Operation)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (command : id.logical.command = .CommitSnapshot sandbox epoch generation snapshot manifest)
    (logicalRead : sandbox_controller_core.model.World.logical_effect world id.logical = .ok (some logical))
    (origin : logical.origin = .Operation key)
    (storedPlan : tableLookup key world.operation_plans.entries.val = some plan)
    (currentOperation : row.current_operation = some current)
    (bad : plan.key ≠ key ∨ key.sandbox ≠ sandbox ∨ key.operation = current.id ∨
      plan.generation ≠ row.generation ∨ epoch ≠ plan.epoch) :
    sandbox_controller_core.worker.late_capture.record_inner world id row = .ok (.Err .PlanMismatch) := by
  have getPlan := get_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operation_plans key
  rcases bad with wrongKey | wrongSession | sameOperation | wrongGeneration | wrongEpoch <;>
    simp [sandbox_controller_core.worker.late_capture.record_inner,
      sandbox_controller_core.model.EffectId.command,
      sandbox_controller_core.model.Command.sandbox, sandbox_controller_core.model.Command.epoch,
      sandbox_controller_core.model.World.operation_plan,
      *, core.result.Result.Insts.CoreOpsTry.branch,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
      core.option.Option.Insts.CoreCmpPartialEqOption.eq, ← UScalar.eq_equiv]

end SandboxModel.Lifecycle.CaptureHandoffCaller
