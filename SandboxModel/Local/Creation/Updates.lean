import SandboxModel.Local.Creation.Containers
import SandboxModel.Local.Creation.GuardRefinement

/-! Actual creation bookkeeping on the complete current state.
    Each table's append headroom is an explicit caller obligation. This does
    not yet connect work commitment, enqueueing or the whole create update. -/
namespace SandboxModel.Local.Creation
open Aeneas Aeneas.Std SandproofNetworkRust SandboxModel.Local.State SandboxModel.Foundations

local instance : DecidableEq sandbox_controller_core.model.OperationKey := fun a b =>
  decidable_of_iff (State.model.OperationKey.view a = State.model.OperationKey.view b)
    State.model.OperationKey.view_injective.eq_iff

def recordedActual (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) : sandbox_controller_core.model.World :=
  {world with maintenance := world.maintenance.map (fun state =>
    {state with sandbox_frontier := some input.sandbox, workspace_frontier := some input.workspace})}

theorem record_identity_exact (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.maintenance.record_identity world input.sandbox input.workspace =
      .ok (recordedActual world input) := by
  cases maintenance : world.maintenance with
  | none =>
    simp [sandbox_controller_core.maintenance.record_identity, recordedActual, maintenance]
    cases world; simp_all
  | some state =>
    simp [sandbox_controller_core.maintenance.record_identity, recordedActual, maintenance]

theorem record_identity_view (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) :
    State.model.World.view (recordedActual world input) =
      recordIdentity (State.model.World.view world) (State.request.CreateRequest.view input) := by
  cases maintenance : world.maintenance <;>
    simp [recordedActual, recordIdentity, State.model.World.view,
      State.request.CreateRequest.view, State.maintenance.MaintenanceState.view,
      ValueView.view, maintenance]

theorem record_identity_refines (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.maintenance.record_identity world input.sandbox input.workspace
      ⦃ next => State.model.World.view next =
        recordIdentity (State.model.World.view world) (State.request.CreateRequest.view input) ⦄ := by
  rw [record_identity_exact]
  simp only [WP.spec_ok]
  exact record_identity_view world input

def resourceActual (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) : sandbox_controller_core.model.ResourceIndex :=
  (tableLookup sandbox world.resource_indexes.entries.val).getD
    ⟨alloc.vec.Vec.new _, alloc.vec.Vec.new _, alloc.vec.Vec.new _⟩

theorem resource_index_exact (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) :
    sandbox_controller_core.model.World.resource_index world sandbox =
      .ok (resourceActual world sandbox) := by
  unfold sandbox_controller_core.model.World.resource_index
  rw [SandboxModel.Access.store_get_refines _ SandboxModel.Access.sandbox_equality_lawful]
  cases found : tableLookup sandbox world.resource_indexes.entries.val <;>
    simp [core.option.OptionShared0T.cloned, core.option.Option.unwrap_or_default,
      sandbox_controller_core.model.ResourceIndex.Insts.CoreDefaultDefault.default,
      resourceActual, found]

theorem resource_index_view (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) :
    State.model.ResourceIndex.view (resourceActual world sandbox) =
      resourceIndex (State.model.World.view world) sandbox.val := by
  unfold resourceIndex
  change State.model.ResourceIndex.view (resourceActual world sandbox) =
    (tableLookup sandbox.val (world.resource_indexes.entries.val.map (fun p =>
      (p.1.val, State.model.ResourceIndex.view p.2)))).getD ⟨[], [], []⟩
  rw [SandboxModel.Access.table_lookup_map UScalar.val State.model.ResourceIndex.view
    (fun _ _ same => UScalar.eq_of_val_eq same)]
  cases found : tableLookup sandbox world.resource_indexes.entries.val <;>
    simp [resourceActual, State.model.ResourceIndex.view, ValueView.view,
      alloc.vec.Vec.new, found]

def operationActual (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.model.Operation :=
  ⟨input.operation, 1#u64, .Create, .Pending⟩

def operationInputActual (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.model.OperationInput :=
  .Create input.workspace input.policy input.file_policy

def planActual (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) : sandbox_controller_core.model.OperationPlan := {
  key := ⟨input.sandbox, input.operation⟩, epoch := 1#u64, generation := 1#u64,
  vm := none, slot := none, restore_from := none, target_snapshot := none,
  stop_vms := (resourceActual world input.sandbox).vms,
  close_slots := (resourceActual world input.sandbox).slots,
  retry_limit := 3#u64, sealed := false
}

theorem initial_plan_exact (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.transition.initial_operation_plan world input.sandbox
      (operationActual input) (operationInputActual input) = .ok (planActual world input) := by
  unfold sandbox_controller_core.transition.initial_operation_plan
  rw [resource_index_exact]
  simp [operationActual, operationInputActual, planActual,
    sandbox_controller_core.transition.operation_key,
    sandbox_controller_core.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
    sandbox_controller_core.model.OperationKind.read_discriminant,
    sandbox_controller_core.transition.DEFAULT_RETRY_LIMIT]

theorem initial_plan_view (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest) :
    State.model.OperationPlan.view (planActual world input) =
      plan (State.model.World.view world) (State.request.CreateRequest.view input) := by
  have resources := resource_index_view world input.sandbox
  have vms := congrArg State.model.ResourceIndex.vms resources
  have slots := congrArg State.model.ResourceIndex.slots resources
  simp only [State.model.ResourceIndex.view, ValueView.view] at vms slots
  simp [planActual, plan, operationKey, State.model.OperationPlan.view,
    State.model.OperationKey.view, State.request.CreateRequest.view, ValueView.view, vms, slots]

def ledgerActual (input : sandbox_controller_core.request.CreateRequest) :
    sandbox_controller_core.model.OperationLedgerRecord :=
  ⟨input.sandbox, operationActual input, operationInputActual input, none⟩

theorem ledger_view (input : sandbox_controller_core.request.CreateRequest) :
    State.model.OperationLedgerRecord.view (ledgerActual input) =
      ledger (State.request.CreateRequest.view input) := by
  rfl

theorem store_new_operation_refines (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.request.CreateRequest)
    (operationCapacity : TableWriteCapacity world.operations.entries.val
      (⟨input.sandbox, input.operation⟩ : sandbox_controller_core.model.OperationKey))
    (planCapacity : TableWriteCapacity world.operation_plans.entries.val
      (⟨input.sandbox, input.operation⟩ : sandbox_controller_core.model.OperationKey)) :
    sandbox_controller_core.transition.store_new_operation world input.sandbox
      (operationActual input) (operationInputActual input)
      ⦃ next => State.model.World.view next =
        storeOperation (State.model.World.view world) (State.request.CreateRequest.view input) ⦄ := by
  unfold sandbox_controller_core.transition.store_new_operation
  simp only [operationActual, sandbox_controller_core.transition.operation_key, bind_tc_ok]
  change (do
    let prepared ← sandbox_controller_core.transition.initial_operation_plan world input.sandbox
      (operationActual input) (operationInputActual input)
    let records ← sandbox_controller_core.store.Store.put
      sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      world.operations ⟨input.sandbox, input.operation⟩ (ledgerActual input)
    let plans ← sandbox_controller_core.store.Store.put
      sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      world.operation_plans ⟨input.sandbox, input.operation⟩ prepared
    .ok {world with operations := records, operation_plans := plans}) ⦃ _ ⦄
  rw [initial_plan_exact]
  simp only [bind_tc_ok]
  apply WP.spec_bind (SandboxModel.Local.Containers.put_refines _ operation_equality_lawful
    world.operations ⟨input.sandbox, input.operation⟩ (ledgerActual input) operationCapacity)
  intro records recordsPost
  apply WP.spec_bind (SandboxModel.Local.Containers.put_refines _ operation_equality_lawful
    world.operation_plans ⟨input.sandbox, input.operation⟩ (planActual world input) planCapacity)
  intro plans plansPost
  simp only [WP.spec_ok]
  have recordView := table_put_view State.model.OperationKey.view
    State.model.OperationKey.view_injective State.model.OperationLedgerRecord.view
    (⟨input.sandbox, input.operation⟩ : sandbox_controller_core.model.OperationKey)
    (ledgerActual input) world.operations.entries.val
  have planView := table_put_view State.model.OperationKey.view
    State.model.OperationKey.view_injective State.model.OperationPlan.view
    (⟨input.sandbox, input.operation⟩ : sandbox_controller_core.model.OperationKey)
    (planActual world input) world.operation_plans.entries.val
  simp only [ledger_view] at recordView
  simp only [initial_plan_view] at planView
  simp [State.model.World.view, storeOperation, ValueView.view, recordsPost, plansPost,
    tableValueView, operationKey, State.request.CreateRequest.view,
    State.model.OperationKey.view] at recordView planView ⊢
  exact ⟨recordView, planView⟩

theorem initial_operation_store_realizable (input : sandbox_controller_core.request.CreateRequest) :
    ∃ actual next, SandproofNetworkRust.initial_world = .ok actual ∧
      sandbox_controller_core.transition.store_new_operation actual input.sandbox
      (operationActual input) (operationInputActual input) = .ok next ∧
      State.model.World.view next =
        storeOperation State.initialModel (State.request.CreateRequest.view input) := by
  obtain ⟨next, executed, represented⟩ := WP.spec_imp_exists
    (store_new_operation_refines SandboxModel.Access.emptyWorld input
      (Or.inr (by simp [SandboxModel.Access.emptyWorld, SandboxModel.Access.emptyStore,
        alloc.vec.Vec.new]; scalar_tac))
      (Or.inr (by simp [SandboxModel.Access.emptyWorld, SandboxModel.Access.emptyStore,
        alloc.vec.Vec.new]; scalar_tac)))
  exact ⟨SandboxModel.Access.emptyWorld, next, SandboxModel.Access.initial_world_root_exact,
    executed, represented⟩

end SandboxModel.Local.Creation
