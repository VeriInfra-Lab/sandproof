import SandboxModel.Local.State.Model
import SandboxModel.Foundations.Model

/-! Creation of the current product, over the complete independent state.
    Ordered tables retain duplicate keys. Rejection is separate from updating;
    no physical VM success, persistence or allocation is assumed here. -/
namespace SandboxModel.Local.Creation
open SandboxModel.Local.State SandboxModel.Foundations

inductive Reject where
  | controllerDown | sandboxExists | workspaceExists | operationExists
  | operationPlanExists | unauthorized | retiredIdentity
  deriving DecidableEq, Repr

def operationKey (input : request.CreateRequest) : model.OperationKey :=
  ⟨input.sandbox, input.operation⟩

def authorized (input : request.CreateRequest) : Bool :=
  decide (input.actor = .Tenant input.owner ∨ input.actor = .SystemController)

def freshIdentity (world : model.World) (input : request.CreateRequest) : Bool :=
  match world.maintenance with
  | none => true
  | some state =>
      (match state.sandbox_frontier with
       | none => true
       | some last => decide (last < input.sandbox)) &&
      (match state.workspace_frontier with
       | none => true
       | some last => decide (last < input.workspace))

/-- The first failed check is the observable rejection. -/
def check (world : model.World) (input : request.CreateRequest) : Except Reject Unit :=
  if !world.controller_up then .error .controllerDown
  else if (tableLookup input.sandbox world.sandboxes.entries).isSome then .error .sandboxExists
  else if (tableLookup input.workspace world.filesystems.entries).isSome then .error .workspaceExists
  else if (tableLookup (operationKey input) world.operations.entries).isSome then .error .operationExists
  else if (tableLookup (operationKey input) world.operation_plans.entries).isSome then .error .operationPlanExists
  else if !authorized input then .error .unauthorized
  else if !freshIdentity world input then .error .retiredIdentity
  else .ok ()

/-- No guards are hidden in a general-purpose "valid state" premise. -/
def Admitted (world : model.World) (input : request.CreateRequest) : Prop :=
  world.controller_up = true ∧
  tableLookup input.sandbox world.sandboxes.entries = none ∧
  tableLookup input.workspace world.filesystems.entries = none ∧
  tableLookup (operationKey input) world.operations.entries = none ∧
  tableLookup (operationKey input) world.operation_plans.entries = none ∧
  authorized input = true ∧ freshIdentity world input = true

def recordIdentity (world : model.World) (input : request.CreateRequest) : model.World :=
  {world with maintenance := world.maintenance.map (fun state =>
    {state with
      sandbox_frontier := some input.sandbox
      workspace_frontier := some input.workspace})}

def operation (input : request.CreateRequest) : model.Operation :=
  ⟨input.operation, 1, .Create, .Pending⟩

def sandboxRecord (input : request.CreateRequest) : model.SandboxRecord := {
  owner := input.owner, phase := .Starting, terminal := false, epoch := 1,
  generation := 1, current_vm := none, workspace := input.workspace,
  workspace_access := .ManagedReadWrite, file_policy := input.file_policy,
  durable_snapshot := none, restore_from := none, net_slot := none,
  policy := input.policy, desired := .Closed, effective := .Closed,
  current_operation := some (operation input)
}

def ledger (input : request.CreateRequest) : model.OperationLedgerRecord :=
  ⟨input.sandbox, operation input,
    .Create input.workspace input.policy input.file_policy, none⟩

def resourceIndex (world : model.World) (sandbox : Nat) : model.ResourceIndex :=
  (tableLookup sandbox world.resource_indexes.entries).getD ⟨[], [], []⟩

def plan (world : model.World) (input : request.CreateRequest) : model.OperationPlan := {
  key := operationKey input, epoch := 1, generation := 1, vm := none, slot := none,
  restore_from := none, target_snapshot := none,
  stop_vms := (resourceIndex world input.sandbox).vms,
  close_slots := (resourceIndex world input.sandbox).slots,
  retry_limit := 3, sealed := false
}

/-- Register the complete creation operation and its original resource plan.
    Other fields are unchanged; existing keys use ordered first replacement. -/
def storeOperation (world : model.World) (input : request.CreateRequest) : model.World :=
  {world with
    operations := ⟨tablePut (operationKey input) (ledger input) world.operations.entries⟩,
    operation_plans := ⟨tablePut (operationKey input) (plan world input) world.operation_plans.entries⟩}

def initialEffect (input : request.CreateRequest) : model.EffectId :=
  ⟨⟨.InstallFence input.sandbox 1 false⟩, 0⟩

def insertUnique [DecidableEq T] (value : T) (values : List T) : List T :=
  if value ∈ values then values else value :: values

/-- Existing attempts and logical results are preserved, not reset on enqueue. -/
def commitFence (world : model.World) (input : request.CreateRequest) : model.World :=
  let effect := initialEffect input
  if (tableLookup effect world.effects.entries).isSome then world
  else {
    world with
    effects := ⟨tablePut effect .Committed world.effects.entries⟩,
    effect_keys := insertUnique effect world.effect_keys,
    logical_effects :=
      let key : model.LogicalEffectId := effect.logical
      if (tableLookup key world.logical_effects.entries).isSome then world.logical_effects
      else ⟨tablePut key ⟨.Operation (operationKey input), 0, 3, none⟩
        world.logical_effects.entries⟩
  }

def beginCreate (world : model.World) (input : request.CreateRequest) : model.World :=
  let recorded := recordIdentity world input
  let accepted := {
    recorded with
    filesystems := ⟨tablePut input.workspace ⟨input.owner, input.sandbox, 1⟩
      world.filesystems.entries⟩,
    sandboxes := ⟨tablePut input.sandbox (sandboxRecord input) world.sandboxes.entries⟩,
    operations := ⟨tablePut (operationKey input) (ledger input) world.operations.entries⟩,
    operation_plans := ⟨tablePut (operationKey input) (plan world input) world.operation_plans.entries⟩
  }
  let committed := commitFence accepted input
  {committed with inflight := insertUnique (initialEffect input) committed.inflight}

/-- Success records the complete accepted request and complete successor state. -/
structure Decision where
  request : request.CreateRequest
  next : model.World
  deriving DecidableEq, Repr

def transition (world : model.World) (input : request.CreateRequest) : Except Reject Decision :=
  match check world input with
  | .error reject => .error reject
  | .ok () => .ok ⟨input, beginCreate world input⟩

end SandboxModel.Local.Creation
