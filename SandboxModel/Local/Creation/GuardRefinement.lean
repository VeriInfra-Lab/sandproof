import SandboxModel.Local.Creation.Model
import SandboxModel.Local.Creation.Properties
import SandboxModel.Local.State.Properties
import SandboxModel.Local.State.Commands
import SandboxModel.Access.Lookup
import SandboxModel.Access.Cloning
namespace SandboxModel.Local.Creation
open Aeneas Aeneas.Std SandproofNetworkRust SandboxModel.Local.State SandboxModel.Foundations

local instance : DecidableEq sandbox_controller_core.model.OperationKey := fun a b =>
  decidable_of_iff (State.model.OperationKey.view a = State.model.OperationKey.view b)
    State.model.OperationKey.view_injective.eq_iff

theorem operation_equality_lawful : TableLawfulEq
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey := by
  intro a b
  change sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq a b =
    .ok (decide (a = b))
  simp only [← State.model.OperationKey.view_injective.eq_iff]
  simp [sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
    sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
    State.model.OperationKey.view, ValueView.view, ← UScalar.eq_equiv]
  split_ifs <;> simp_all

theorem authorization_exact (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.transition.authorized input.actor input.owner =
      .ok (authorized (State.request.CreateRequest.view input)) := by
  rw [SandboxModel.Access.authorized_refines]
  cases actor : input.actor <;>
    simp [SandboxModel.Access.authorized, SandboxModel.Access.actorView, authorized,
      State.request.CreateRequest.view, ValueView.view, State.model.Actor.view, actor]
  rename_i tenant
  by_cases same : tenant.val = input.owner.val <;> simp [same]

theorem workspace_equality_lawful : TableLawfulEq
    sandbox_controller_core.ids.FsId.Insts.CoreCmpPartialEqFsId := by
  intro a b
  rfl

theorem fresh_identity_exact (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.maintenance.fresh_identity world input.sandbox input.workspace =
      .ok (freshIdentity (State.model.World.view world) (State.request.CreateRequest.view input)) := by
  cases maintenance : world.maintenance with
  | none => simp [sandbox_controller_core.maintenance.fresh_identity,
      core.option.Option.as_ref, SandproofFoundationsRust.core.option.Option.as_ref,
      freshIdentity, State.model.World.view, ValueView.view, maintenance]
  | some state =>
    cases sandbox : state.sandbox_frontier <;> cases workspace : state.workspace_frontier <;>
      simp [sandbox_controller_core.maintenance.fresh_identity,
        core.option.Option.as_ref, SandproofFoundationsRust.core.option.Option.as_ref,
        sandbox_controller_core.maintenance.fresh_identity.closure.Insts.CoreOpsFunctionFnOnceTupleSharedMaintenanceStateBool.call_once,
        sandbox_controller_core.maintenance.fresh_identity.closure.closure.Insts.CoreOpsFunctionFnOnceTupleU64Bool.call_once,
        sandbox_controller_core.maintenance.fresh_identity.closure.closure_1.Insts.CoreOpsFunctionFnOnceTupleU64Bool.call_once,
        freshIdentity, State.model.World.view, State.request.CreateRequest.view, ValueView.view,
        State.maintenance.MaintenanceState.view, maintenance, sandbox, workspace]
    all_goals split_ifs <;> simp_all

def rejectValue : Reject → sandbox_controller_core.request.Reject
  | .controllerDown => .ControllerDown
  | .sandboxExists => .SandboxExists
  | .workspaceExists => .WorkspaceExists
  | .operationExists => .OperationExists
  | .operationPlanExists => .OperationPlanExists
  | .unauthorized => .Unauthorized
  | .retiredIdentity => .RetiredIdentity

/-- All real checks agree. The success update is deliberately still the
    actual extracted update, not an assumed mathematical successor. -/
theorem guards_exact (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) :
    SandproofNetworkRust.create world input =
      (match check (State.model.World.view world) (State.request.CreateRequest.view input) with
      | .error reason => .ok (.Err (rejectValue reason))
      | .ok () => do
          let next ← sandbox_controller_core.transition.begin_create world input.sandbox input.owner
            input.workspace input.file_policy input.policy input.operation
          .ok (.Ok ⟨.AcceptCreate input.actor input.owner input.sandbox input.workspace
            input.file_policy input.policy input.operation, next⟩)) := by
  have sandboxView :
      (tableLookup (State.request.CreateRequest.view input).sandbox
        (State.model.World.view world).sandboxes.entries).isSome =
        (tableLookup input.sandbox world.sandboxes.entries.val).isSome := by
    change (tableLookup input.sandbox.val
      (world.sandboxes.entries.val.map (fun p => (p.1.val, State.model.SandboxRecord.view p.2)))).isSome = _
    rw [SandboxModel.Access.table_lookup_map UScalar.val State.model.SandboxRecord.view
      (fun _ _ h => UScalar.eq_of_val_eq h)]
    simp
  have workspaceView :
      (tableLookup (State.request.CreateRequest.view input).workspace
        (State.model.World.view world).filesystems.entries).isSome =
        (tableLookup input.workspace world.filesystems.entries.val).isSome := by
    change (tableLookup input.workspace.val
      (world.filesystems.entries.val.map (fun p => (p.1.val, State.model.Filesystem.view p.2)))).isSome = _
    rw [SandboxModel.Access.table_lookup_map UScalar.val State.model.Filesystem.view
      (fun _ _ h => UScalar.eq_of_val_eq h)]
    simp
  let key : sandbox_controller_core.model.OperationKey := ⟨input.sandbox, input.operation⟩
  have operationView :
      (tableLookup (operationKey (State.request.CreateRequest.view input))
        (State.model.World.view world).operations.entries).isSome =
        (tableLookup key world.operations.entries.val).isSome := by
    change (tableLookup (State.model.OperationKey.view key)
      (world.operations.entries.val.map (fun p =>
        (State.model.OperationKey.view p.1, State.model.OperationLedgerRecord.view p.2)))).isSome = _
    rw [SandboxModel.Access.table_lookup_map State.model.OperationKey.view
      State.model.OperationLedgerRecord.view State.model.OperationKey.view_injective]
    simp
  have planView :
      (tableLookup (operationKey (State.request.CreateRequest.view input))
        (State.model.World.view world).operation_plans.entries).isSome =
        (tableLookup key world.operation_plans.entries.val).isSome := by
    change (tableLookup (State.model.OperationKey.view key)
      (world.operation_plans.entries.val.map (fun p =>
        (State.model.OperationKey.view p.1, State.model.OperationPlan.view p.2)))).isSome = _
    rw [SandboxModel.Access.table_lookup_map State.model.OperationKey.view
      State.model.OperationPlan.view State.model.OperationKey.view_injective]
    simp
  have upView : (State.model.World.view world).controller_up = world.controller_up := rfl
  simp only [SandproofNetworkRust.create, sandbox_controller_core.request.handle_create,
    sandbox_controller_core.store.Store.contains_key,
    SandboxModel.Access.store_get_refines _ SandboxModel.Access.sandbox_equality_lawful,
    SandboxModel.Access.store_get_refines _ workspace_equality_lawful,
    SandboxModel.Access.store_get_refines _ operation_equality_lawful,
    sandbox_controller_core.transition.operation_key, authorization_exact, fresh_identity_exact,
    SandboxModel.Access.model_NetworkPolicy_clone_exact, bind_tc_ok,
    core.option.Option.is_some, check, upView, sandboxView, workspaceView, operationView, planView]
  change (if world.controller_up then _ else _) = _
  split_ifs <;> simp_all [rejectValue]

theorem reject_value_injective : Function.Injective rejectValue := by
  intro a b same
  cases a <;> cases b <;> simp_all [rejectValue]

theorem root_rejection_iff (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) (reason : Reject) :
    SandproofNetworkRust.create world input = .ok (.Err (rejectValue reason)) ↔
      check (State.model.World.view world) (State.request.CreateRequest.view input) = .error reason := by
  rw [guards_exact]
  cases checked : check (State.model.World.view world) (State.request.CreateRequest.view input) with
  | error error => simp [reject_value_injective.eq_iff]
  | ok value =>
      cases value
      cases updated : sandbox_controller_core.transition.begin_create world input.sandbox input.owner
        input.workspace input.file_policy input.policy input.operation <;> simp

theorem root_rejection_has_model_reason (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) (error : sandbox_controller_core.request.Reject)
    (rejected : SandproofNetworkRust.create world input = .ok (.Err error)) :
    ∃ reason, error = rejectValue reason ∧
      check (State.model.World.view world) (State.request.CreateRequest.view input) = .error reason := by
  rw [guards_exact] at rejected
  cases checked : check (State.model.World.view world) (State.request.CreateRequest.view input) with
  | error reason =>
      simp only [checked, Aeneas.Std.Result.ok.injEq, core.result.Result.Err.injEq] at rejected
      exact ⟨reason, rejected.symm, rfl⟩
  | ok value =>
      cases value
      simp only [checked] at rejected
      cases updated : sandbox_controller_core.transition.begin_create world input.sandbox input.owner
        input.workspace input.file_policy input.policy input.operation <;> simp [updated] at rejected

theorem root_rejection_represented (world : sandbox_controller_core.model.World)
    (abstract : State.model.World) (worldRep : State.WorldRep world abstract)
    (input : sandbox_controller_core.request.CreateRequest) (abstractInput : State.request.CreateRequest)
    (inputRep : State.request.CreateRequest.view input = abstractInput) (reason : Reject) :
    SandproofNetworkRust.create world input = .ok (.Err (rejectValue reason)) ↔
      transition abstract abstractInput = .error reason := by
  rw [root_rejection_iff, rejects_iff, worldRep, inputRep]

/-- Conditional wiring only: the actual success update is a remaining V4
    obligation, not an environment assumption or completed handler proof. -/
theorem root_success_after_update (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) (next : sandbox_controller_core.model.World)
    (admitted : Admitted (State.model.World.view world) (State.request.CreateRequest.view input))
    (updated : sandbox_controller_core.transition.begin_create world input.sandbox input.owner
      input.workspace input.file_policy input.policy input.operation = .ok next) :
    SandproofNetworkRust.create world input = .ok (.Ok
      ⟨.AcceptCreate input.actor input.owner input.sandbox input.workspace input.file_policy
        input.policy input.operation, next⟩) := by
  rw [guards_exact, (check_accepts_iff _ _).mpr admitted]
  simp [updated]

theorem initial_admission_realizable (input : State.request.CreateRequest)
    (owned : authorized input = true) :
    Admitted State.initialModel input := by
  simp [Admitted, State.initialModel, tableLookup, freshIdentity, owned]

end SandboxModel.Local.Creation
