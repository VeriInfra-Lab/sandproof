import SandboxModel.RustRefinement.Values
import SandboxModel.Controller

/-!
Refinement facts for the admission control of `handle_create`.

The theorems in this file mention the function emitted by Aeneas, rather than
an independently rewritten Rust specification.  They establish the first
observable agreement points while the full successor-world representation
relation is built out store by store.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

/-- Rust contains one machine-specific rejection that is outside the
    unbounded Lean model.  All shared rejection constructors map exactly;
    counter exhaustion remains explicitly unmapped instead of being disguised
    as an unrelated model error. -/
def reject? : SandboxControllerRust.request.Reject →
    Option SandboxModel.Controller.Reject
  | .ControllerDown => some .controllerDown
  | .SandboxExists => some .sandboxExists
  | .SandboxNotFound => some .sandboxNotFound
  | .WorkspaceExists => some .workspaceExists
  | .OperationExists => some .operationExists
  | .OperationPlanExists => some .operationPlanExists
  | .Unauthorized => some .unauthorized
  | .WrongPhase => some .wrongPhase
  | .Terminal => some .terminal
  | .NoCurrentVm => some .noCurrentVm
  | .SnapshotUnavailable => some .snapshotUnavailable
  | .OperationUnavailable => some .operationUnavailable
  | .OperationNotCompensable => some .operationNotCompensable
  | .OperationAlreadyFinal => some .operationAlreadyFinal
  | .LedgerMismatch => some .ledgerMismatch
  | .FailureUnjustified => some .failureUnjustified
  | .CounterExhausted => none

@[simp] theorem reject_controllerDown :
    reject? .ControllerDown = some .controllerDown := rfl

/-- Minimal world relation needed before any store is inspected. -/
def ControllerAvailabilityRep
    (rust : SandboxControllerRust.model.World) (model : World) : Prop :=
  rust.controller_up = model.controllerUp

/-- Direct evaluation of the controller-down branch of the Rust function
    translated by Aeneas. -/
theorem generated_handleCreate_controllerDown
    (world : SandboxControllerRust.model.World)
    (input : SandboxControllerRust.request.CreateRequest)
    (down : world.controller_up = false) :
    SandboxControllerRust.request.handle_create world input =
      ok (core.result.Result.Err
        SandboxControllerRust.request.Reject.ControllerDown) := by
  simp [SandboxControllerRust.request.handle_create, down]

/-- Direct evaluation of the corresponding executable Lean reducer branch. -/
theorem model_handleCreate_controllerDown
    (world : World) (actor : Actor) (owner : TenantId) (sid : SandboxId)
    (workspace : FsId) (filePolicy : FilePolicyRef) (policy : NetworkPolicy)
    (operation : OperationId) (down : world.controllerUp = false) :
    SandboxModel.Controller.handleCreate world actor owner sid workspace
      filePolicy policy operation = .error .controllerDown := by
  simp [SandboxModel.Controller.handleCreate, down]

/-- First end-to-end refinement theorem: related Rust and Lean worlds return
    the corresponding rejection without inspecting or changing any store. -/
theorem handleCreate_controllerDown_refines
    (rustWorld : SandboxControllerRust.model.World) (modelWorld : World)
    (input : SandboxControllerRust.request.CreateRequest)
    (related : ControllerAvailabilityRep rustWorld modelWorld)
    (down : modelWorld.controllerUp = false) :
    SandboxControllerRust.request.handle_create rustWorld input =
        ok (core.result.Result.Err
          SandboxControllerRust.request.Reject.ControllerDown) ∧
      SandboxModel.Controller.handleCreate modelWorld
        (actor input.actor) (tenantId input.owner) (sandboxId input.sandbox)
        (fsId input.workspace) (filePolicyRef input.file_policy)
        (networkPolicy input.policy) (operationId input.operation) =
          .error .controllerDown := by
  constructor
  · apply generated_handleCreate_controllerDown
    simpa [ControllerAvailabilityRep, down] using related
  · exact model_handleCreate_controllerDown _ _ _ _ _ _ _ _ down

/-- If every ordered admission check and both fallible helper calls succeed,
    the generated Rust handler returns the concrete accepted decision shown in
    its source.  Later world-refinement theorems discharge these premises from
    the store representation invariant. -/
theorem generated_handleCreate_accepts
    (world : SandboxControllerRust.model.World)
    (input : SandboxControllerRust.request.CreateRequest)
    (clonedPolicy : SandboxControllerRust.model.NetworkPolicy)
    (next : SandboxControllerRust.model.World)
    (up : world.controller_up = true)
    (sandboxFresh :
      SandboxControllerRust.store.Store.contains_key
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
        world.sandboxes input.sandbox = ok false)
    (workspaceFresh :
      SandboxControllerRust.store.Store.contains_key
        SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId
        world.filesystems input.workspace = ok false)
    (operationFresh :
      SandboxControllerRust.store.Store.contains_key
        SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
        world.operations
        { sandbox := input.sandbox, operation := input.operation } = ok false)
    (planFresh :
      SandboxControllerRust.store.Store.contains_key
        SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
        world.operation_plans
        { sandbox := input.sandbox, operation := input.operation } = ok false)
    (authorized :
      SandboxControllerRust.transition.authorized input.actor input.owner =
        ok true)
    (cloned :
      SandboxControllerRust.model.NetworkPolicy.Insts.CoreCloneClone.clone
        input.policy = ok clonedPolicy)
    (begun :
      SandboxControllerRust.transition.begin_create world input.sandbox
        input.owner input.workspace input.file_policy input.policy
        input.operation = ok next) :
    SandboxControllerRust.request.handle_create world input =
      ok (core.result.Result.Ok
        ({
          event := SandboxControllerRust.model.Event.AcceptCreate
            input.actor input.owner input.sandbox input.workspace
            input.file_policy clonedPolicy input.operation
          next := next
        } : SandboxControllerRust.request.Decision)) := by
  simp [SandboxControllerRust.request.handle_create, up, sandboxFresh,
    workspaceFresh, SandboxControllerRust.transition.operation_key,
    operationFresh, planFresh, authorized, cloned, begun]

/-- The executable model accepts under the matching abstract checks.  This is
    the model-side half of the accepted-path commuting diagram. -/
theorem model_handleCreate_accepts
    (world : World) (actor : Actor) (owner : TenantId) (sid : SandboxId)
    (workspace : FsId) (filePolicy : FilePolicyRef) (policy : NetworkPolicy)
    (operation : OperationId)
    (up : world.controllerUp = true)
    (sandboxFresh : world.sandboxes sid = none)
    (workspaceFresh : world.filesystems workspace = none)
    (operationFresh :
      world.operations (SandboxModel.operationKey sid operation) = none)
    (planFresh :
      world.operationPlans (SandboxModel.operationKey sid operation) = none)
    (auth : authorized actor owner) :
    ∃ decision, SandboxModel.Controller.handleCreate world actor owner sid
      workspace filePolicy policy operation = .ok decision := by
  unfold SandboxModel.Controller.handleCreate
  split <;> simp_all
  all_goals split <;> simp_all
  all_goals split <;> simp_all
  all_goals split <;> simp_all
  all_goals split <;> simp_all

end SandboxModel.RustRefinement
