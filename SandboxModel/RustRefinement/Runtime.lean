import SandboxModel.RustRefinement.Values
import SandboxModel.RustRefinement.Clone
import SandboxModel.RuntimeController
import SandboxModel.Monitor

/-!
The representation bridge for controller-process lifecycle and trace-monitor
values.  These definitions consume the Aeneas output of the real Rust module;
the evaluation lemmas below are the generated-code half of the later complete
world commuting diagram.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance runtimeOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance runtimeLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance runtimeEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

abbrev ConcreteWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

/-- Toggling runtime availability is the same field update on both sides of
    the representation boundary; every durable store remains unchanged. -/
theorem worldRep_setControllerUp
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : ConcreteWorldRep rust model) (available : Bool) :
    ConcreteWorldRep { rust with controller_up := available }
      { model with controllerUp := available } := by
  exact {
    sandboxes := represented.sandboxes
    vms := represented.vms
    filesystems := represented.filesystems
    snapshots := represented.snapshots
    netSlots := represented.netSlots
    appliedFilePolicies := represented.appliedFilePolicies
    workerFences := represented.workerFences
    operations := represented.operations
    operationPlans := represented.operationPlans
    resourceIndexes := represented.resourceIndexes
    logicalEffects := represented.logicalEffects
    effects := represented.effects
    effectKeys := represented.effectKeys
    effectKeysUnique := represented.effectKeysUnique
    inflight := represented.inflight
    inflightSupport := represented.inflightSupport
    inflightUnique := represented.inflightUnique
    nextVmId := represented.nextVmId
    nextSlotId := represented.nextSlotId
    nextSnapshotId := represented.nextSnapshotId
    nextManifestDigest := represented.nextManifestDigest
    controllerUp := rfl }

def runtimeInput : SandboxControllerRust.recovery_monitor.RuntimeInput →
    SandboxModel.RuntimeController.Input
  | .Crash => .crash
  | .Recover => .recover
  | .DuplicateOrLateDelivery effect =>
      .duplicateOrLateDelivery (effectId effect)

def runtimeEvent : SandboxControllerRust.recovery_monitor.RuntimeEvent → Event
  | .ControllerCrash => .controllerCrash
  | .ControllerRecover => .controllerRecover
  | .DuplicateOrLateDelivery effect =>
      .duplicateOrLateDelivery (effectId effect)

def runtimeReject : SandboxControllerRust.recovery_monitor.RuntimeReject →
    SandboxModel.RuntimeController.Reject
  | .ControllerAlreadyDown => .controllerAlreadyDown
  | .ControllerAlreadyUp => .controllerAlreadyUp
  | .EffectNotFound => .effectNotFound

def sandboxView : SandboxControllerRust.recovery_monitor.SandboxView →
    SandboxModel.Monitor.SandboxView
  | ⟨phaseValue, terminal, generation⟩ =>
      ⟨phase phaseValue, terminal, u64ToNat generation⟩

@[simp] theorem runtimeInput_event (input) :
    (runtimeInput input).event = runtimeEvent (match input with
      | .Crash => .ControllerCrash
      | .Recover => .ControllerRecover
      | .DuplicateOrLateDelivery effect => .DuplicateOrLateDelivery effect) := by
  cases input <;> rfl

theorem runtimeEvent_injective : Function.Injective runtimeEvent := by
  intro left right same
  cases left <;> cases right <;> simp_all [runtimeEvent, effectId_injective.eq_iff]

theorem generated_handleCrash_down
    (world : SandboxControllerRust.model.World)
    (down : world.controller_up = false) :
    SandboxControllerRust.recovery_monitor.handle_controller_crash world =
      ok (core.result.Result.Err
        SandboxControllerRust.recovery_monitor.RuntimeReject.ControllerAlreadyDown) := by
  simp [SandboxControllerRust.recovery_monitor.handle_controller_crash, down]

theorem generated_handleCrash_up
    (world cloned : SandboxControllerRust.model.World)
    (up : world.controller_up = true)
    (clonedWorld :
      SandboxControllerRust.model.World.Insts.CoreCloneClone.clone world =
        ok cloned) :
    SandboxControllerRust.recovery_monitor.handle_controller_crash world =
      ok (core.result.Result.Ok
        ({
          event :=
            SandboxControllerRust.recovery_monitor.RuntimeEvent.ControllerCrash
          next := { cloned with controller_up := false }
        } : SandboxControllerRust.recovery_monitor.RuntimeDecision)) := by
  simp [SandboxControllerRust.recovery_monitor.handle_controller_crash, up,
    clonedWorld]

theorem generated_handleRecover_up
    (world : SandboxControllerRust.model.World)
    (up : world.controller_up = true) :
    SandboxControllerRust.recovery_monitor.handle_controller_recover world =
      ok (core.result.Result.Err
        SandboxControllerRust.recovery_monitor.RuntimeReject.ControllerAlreadyUp) := by
  simp [SandboxControllerRust.recovery_monitor.handle_controller_recover, up]

theorem generated_handleRecover_down
    (world cloned : SandboxControllerRust.model.World)
    (down : world.controller_up = false)
    (clonedWorld :
      SandboxControllerRust.model.World.Insts.CoreCloneClone.clone world =
        ok cloned) :
    SandboxControllerRust.recovery_monitor.handle_controller_recover world =
      ok (core.result.Result.Ok
        ({
          event :=
            SandboxControllerRust.recovery_monitor.RuntimeEvent.ControllerRecover
          next := { cloned with controller_up := true }
        } : SandboxControllerRust.recovery_monitor.RuntimeDecision)) := by
  simp [SandboxControllerRust.recovery_monitor.handle_controller_recover, down,
    clonedWorld]

theorem generated_handleDuplicate_missing
    (world : SandboxControllerRust.model.World)
    (effect : SandboxControllerRust.model.EffectId)
    (missing : SandboxControllerRust.store.Store.contains_key
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effects effect = ok false) :
    SandboxControllerRust.recovery_monitor.handle_duplicate_or_late_delivery
        world effect =
      ok (core.result.Result.Err
        SandboxControllerRust.recovery_monitor.RuntimeReject.EffectNotFound) := by
  simp [SandboxControllerRust.recovery_monitor.handle_duplicate_or_late_delivery,
    missing]

theorem generated_handleDuplicate_present
    (world cloned : SandboxControllerRust.model.World)
    (effect : SandboxControllerRust.model.EffectId)
    (present : SandboxControllerRust.store.Store.contains_key
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effects effect = ok true)
    (clonedWorld :
      SandboxControllerRust.model.World.Insts.CoreCloneClone.clone world =
        ok cloned) :
    SandboxControllerRust.recovery_monitor.handle_duplicate_or_late_delivery
        world effect =
      ok (core.result.Result.Ok
        ({
          event := SandboxControllerRust.recovery_monitor.RuntimeEvent.DuplicateOrLateDelivery
            effect
          next := cloned
        } : SandboxControllerRust.recovery_monitor.RuntimeDecision)) := by
  simp [SandboxControllerRust.recovery_monitor.handle_duplicate_or_late_delivery,
    present, clonedWorld]

theorem model_handleCrash_down (world : World)
    (down : world.controllerUp = false) :
    SandboxModel.RuntimeController.handleCrash world =
      .error .controllerAlreadyDown := by
  simp [SandboxModel.RuntimeController.handleCrash, down]

theorem model_handleRecover_up (world : World)
    (up : world.controllerUp = true) :
    SandboxModel.RuntimeController.handleRecover world =
      .error .controllerAlreadyUp := by
  simp [SandboxModel.RuntimeController.handleRecover, up]

/-! ## End-to-end crash/recovery commuting diagrams -/

theorem handleCrash_reject_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (down : rust.controller_up = false) :
    SandboxControllerRust.recovery_monitor.handle_controller_crash rust =
        ok (core.result.Result.Err
          SandboxControllerRust.recovery_monitor.RuntimeReject.ControllerAlreadyDown) ∧
      SandboxModel.RuntimeController.handleCrash model =
        .error .controllerAlreadyDown := by
  have modelDown : model.controllerUp = false := by
    rw [← represented.controllerUp]
    exact down
  exact ⟨generated_handleCrash_down rust down,
    model_handleCrash_down model modelDown⟩

theorem handleCrash_success_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (up : rust.controller_up = true) :
    ∃ rustDecision modelDecision,
      SandboxControllerRust.recovery_monitor.handle_controller_crash rust =
        ok (core.result.Result.Ok rustDecision) ∧
      SandboxModel.RuntimeController.handleCrash model = .ok modelDecision ∧
      runtimeEvent rustDecision.event = modelDecision.event ∧
      ConcreteWorldRep rustDecision.next modelDecision.next := by
  have modelUp : model.controllerUp = true := by
    rw [← represented.controllerUp]
    exact up
  let rustDecision : SandboxControllerRust.recovery_monitor.RuntimeDecision :=
    { event := .ControllerCrash
      next := { rust with controller_up := false } }
  let modelDecision : SandboxModel.RuntimeController.Decision model :=
    ⟨.controllerCrash, { model with controllerUp := false },
      Step.controllerCrash modelUp⟩
  refine ⟨rustDecision, modelDecision, ?_, ?_, rfl, ?_⟩
  · simpa [rustDecision] using
      generated_handleCrash_up rust rust up (world_clone_eq rust)
  · simp [SandboxModel.RuntimeController.handleCrash, modelUp, modelDecision]
  · simpa [rustDecision, modelDecision] using
      worldRep_setControllerUp represented false

theorem handleRecover_reject_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (up : rust.controller_up = true) :
    SandboxControllerRust.recovery_monitor.handle_controller_recover rust =
        ok (core.result.Result.Err
          SandboxControllerRust.recovery_monitor.RuntimeReject.ControllerAlreadyUp) ∧
      SandboxModel.RuntimeController.handleRecover model =
        .error .controllerAlreadyUp := by
  have modelUp : model.controllerUp = true := by
    rw [← represented.controllerUp]
    exact up
  exact ⟨generated_handleRecover_up rust up,
    model_handleRecover_up model modelUp⟩

theorem handleRecover_success_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (down : rust.controller_up = false) :
    ∃ rustDecision modelDecision,
      SandboxControllerRust.recovery_monitor.handle_controller_recover rust =
        ok (core.result.Result.Ok rustDecision) ∧
      SandboxModel.RuntimeController.handleRecover model = .ok modelDecision ∧
      runtimeEvent rustDecision.event = modelDecision.event ∧
      ConcreteWorldRep rustDecision.next modelDecision.next := by
  have modelDown : model.controllerUp = false := by
    rw [← represented.controllerUp]
    exact down
  let rustDecision : SandboxControllerRust.recovery_monitor.RuntimeDecision :=
    { event := .ControllerRecover
      next := { rust with controller_up := true } }
  let modelDecision : SandboxModel.RuntimeController.Decision model :=
    ⟨.controllerRecover, { model with controllerUp := true },
      Step.controllerRecover modelDown⟩
  refine ⟨rustDecision, modelDecision, ?_, ?_, rfl, ?_⟩
  · simpa [rustDecision] using
      generated_handleRecover_down rust rust down (world_clone_eq rust)
  · simp [SandboxModel.RuntimeController.handleRecover, modelDown, modelDecision]
  · simpa [rustDecision, modelDecision] using
      worldRep_setControllerUp represented true

theorem handleDuplicate_missing_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (effect : SandboxControllerRust.model.EffectId)
    (missing : model.effects (effectId effect) = none) :
    SandboxControllerRust.recovery_monitor.handle_duplicate_or_late_delivery
          rust effect =
        ok (core.result.Result.Err
          SandboxControllerRust.recovery_monitor.RuntimeReject.EffectNotFound) ∧
      SandboxModel.RuntimeController.handleDuplicateOrLateDelivery model
          (effectId effect) = .error .effectNotFound := by
  have presence := represented.effects.lookup_isSome_eq effect
  change (storeLookup rust.effects effect).isSome =
    (model.effects (effectId effect)).isSome at presence
  rw [missing] at presence
  simp at presence
  have contains :
      SandboxControllerRust.store.Store.contains_key
          SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
          rust.effects effect = ok false := by
    calc
      _ = ok (storeLookup rust.effects effect).isSome :=
        generated_contains_key_eq
          SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
          effectId_partialEq_lawful rust.effects effect
      _ = ok false := by simp [presence]
  constructor
  · exact generated_handleDuplicate_missing rust effect contains
  · unfold SandboxModel.RuntimeController.handleDuplicateOrLateDelivery
    split
    · rfl
    · rename_i unexpected
      simp [missing] at unexpected

theorem handleDuplicate_success_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (effect : SandboxControllerRust.model.EffectId)
    (status : EffectStatus)
    (found : model.effects (effectId effect) = some status) :
    ∃ rustDecision modelDecision,
      SandboxControllerRust.recovery_monitor.handle_duplicate_or_late_delivery
          rust effect = ok (core.result.Result.Ok rustDecision) ∧
      SandboxModel.RuntimeController.handleDuplicateOrLateDelivery model
          (effectId effect) = .ok modelDecision ∧
      runtimeEvent rustDecision.event = modelDecision.event ∧
      ConcreteWorldRep rustDecision.next modelDecision.next := by
  have presence := represented.effects.lookup_isSome_eq effect
  change (storeLookup rust.effects effect).isSome =
    (model.effects (effectId effect)).isSome at presence
  rw [found] at presence
  simp at presence
  have contains :
      SandboxControllerRust.store.Store.contains_key
          SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
          rust.effects effect = ok true := by
    calc
      _ = ok (storeLookup rust.effects effect).isSome :=
        generated_contains_key_eq
          SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
          effectId_partialEq_lawful rust.effects effect
      _ = ok true := by rw [presence]
  let rustDecision : SandboxControllerRust.recovery_monitor.RuntimeDecision :=
    { event := .DuplicateOrLateDelivery effect
      next := rust }
  let modelDecision : SandboxModel.RuntimeController.Decision model :=
    ⟨.duplicateOrLateDelivery (effectId effect), model,
      Step.duplicateOrLateDelivery (effectId effect) found⟩
  refine ⟨rustDecision, modelDecision, ?_, ?_, rfl, ?_⟩
  · simpa [rustDecision] using generated_handleDuplicate_present rust rust effect
      contains (world_clone_eq rust)
  · unfold SandboxModel.RuntimeController.handleDuplicateOrLateDelivery
    split
    · rename_i unexpected
      simp [found] at unexpected
    · rfl
  · simpa [rustDecision, modelDecision] using represented

/-! ## Unified runtime entry point -/

def RuntimeResultRep (model : World)
    (rustResult : core.result.Result
      SandboxControllerRust.recovery_monitor.RuntimeDecision
      SandboxControllerRust.recovery_monitor.RuntimeReject)
    (modelResult : Except SandboxModel.RuntimeController.Reject
      (SandboxModel.RuntimeController.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject =>
      runtimeReject rustReject = modelReject
  | .Ok rustDecision, .ok modelDecision =>
      runtimeEvent rustDecision.event = modelDecision.event ∧
        ConcreteWorldRep rustDecision.next modelDecision.next
  | _, _ => False

/-- The generated unified Rust runtime handler is total and returns exactly
    the corresponding model rejection or a related accepted successor. -/
theorem handleRuntime_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (input : SandboxControllerRust.recovery_monitor.RuntimeInput) :
    ∃ rustResult,
      SandboxControllerRust.recovery_monitor.handle_runtime rust input =
        ok rustResult ∧
      RuntimeResultRep model rustResult
        (SandboxModel.RuntimeController.handle model (runtimeInput input)) := by
  cases input with
  | Crash =>
      by_cases up : rust.controller_up = true
      · rcases handleCrash_success_refines rust model represented up with
          ⟨rustDecision, modelDecision, rustEvaluates, modelEvaluates,
            eventAgrees, nextRelated⟩
        refine ⟨.Ok rustDecision, ?_, ?_⟩
        · simpa [SandboxControllerRust.recovery_monitor.handle_runtime] using
            rustEvaluates
        · change RuntimeResultRep model (.Ok rustDecision)
            (SandboxModel.RuntimeController.handleCrash model)
          rw [modelEvaluates]
          exact ⟨eventAgrees, nextRelated⟩
      · have down : rust.controller_up = false := by
          simpa using up
        rcases handleCrash_reject_refines rust model represented down with
          ⟨rustEvaluates, modelEvaluates⟩
        refine ⟨.Err .ControllerAlreadyDown, ?_, ?_⟩
        · simpa [SandboxControllerRust.recovery_monitor.handle_runtime] using
            rustEvaluates
        · change RuntimeResultRep model (.Err .ControllerAlreadyDown)
            (SandboxModel.RuntimeController.handleCrash model)
          rw [modelEvaluates]
          rfl
  | Recover =>
      by_cases up : rust.controller_up = true
      · rcases handleRecover_reject_refines rust model represented up with
          ⟨rustEvaluates, modelEvaluates⟩
        refine ⟨.Err .ControllerAlreadyUp, ?_, ?_⟩
        · simpa [SandboxControllerRust.recovery_monitor.handle_runtime] using
            rustEvaluates
        · change RuntimeResultRep model (.Err .ControllerAlreadyUp)
            (SandboxModel.RuntimeController.handleRecover model)
          rw [modelEvaluates]
          rfl
      · have down : rust.controller_up = false := by
          simpa using up
        rcases handleRecover_success_refines rust model represented down with
          ⟨rustDecision, modelDecision, rustEvaluates, modelEvaluates,
            eventAgrees, nextRelated⟩
        refine ⟨.Ok rustDecision, ?_, ?_⟩
        · simpa [SandboxControllerRust.recovery_monitor.handle_runtime] using
            rustEvaluates
        · change RuntimeResultRep model (.Ok rustDecision)
            (SandboxModel.RuntimeController.handleRecover model)
          rw [modelEvaluates]
          exact ⟨eventAgrees, nextRelated⟩
  | DuplicateOrLateDelivery effect =>
      cases found : model.effects (effectId effect) with
      | none =>
          rcases handleDuplicate_missing_refines rust model represented effect
              found with ⟨rustEvaluates, modelEvaluates⟩
          refine ⟨.Err .EffectNotFound, ?_, ?_⟩
          · simpa [SandboxControllerRust.recovery_monitor.handle_runtime] using
              rustEvaluates
          · change RuntimeResultRep model (.Err .EffectNotFound)
              (SandboxModel.RuntimeController.handleDuplicateOrLateDelivery
                model (effectId effect))
            rw [modelEvaluates]
            rfl
      | some status =>
          rcases handleDuplicate_success_refines rust model represented effect
              status found with
            ⟨rustDecision, modelDecision, rustEvaluates, modelEvaluates,
              eventAgrees, nextRelated⟩
          refine ⟨.Ok rustDecision, ?_, ?_⟩
          · simpa [SandboxControllerRust.recovery_monitor.handle_runtime] using
              rustEvaluates
          · change RuntimeResultRep model (.Ok rustDecision)
              (SandboxModel.RuntimeController.handleDuplicateOrLateDelivery
                model (effectId effect))
            rw [modelEvaluates]
            exact ⟨eventAgrees, nextRelated⟩

/-- Every model-accepted runtime input has a generated Rust acceptance with
    the same event and a successor still related by the complete world
    representation. -/
theorem handleRuntime_accept_complete
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (input : SandboxControllerRust.recovery_monitor.RuntimeInput)
    (modelDecision : SandboxModel.RuntimeController.Decision model)
    (accepted : SandboxModel.RuntimeController.handle model
      (runtimeInput input) = .ok modelDecision) :
    ∃ rustDecision,
      SandboxControllerRust.recovery_monitor.handle_runtime rust input =
          ok (core.result.Result.Ok rustDecision) ∧
        runtimeEvent rustDecision.event = modelDecision.event ∧
        ConcreteWorldRep rustDecision.next modelDecision.next := by
  rcases handleRuntime_refines rust model represented input with
    ⟨rustResult, rustEvaluates, related⟩
  rw [accepted] at related
  cases rustResult with
  | Err rustReject => simp [RuntimeResultRep] at related
  | Ok rustDecision =>
      exact ⟨rustDecision, rustEvaluates, related⟩

/-- Shared model runtime rejections are also complete: the Rust handler
    returns the unique corresponding rejection rather than inventing a
    successful state. -/
theorem handleRuntime_reject_complete
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ConcreteWorldRep rust model)
    (input : SandboxControllerRust.recovery_monitor.RuntimeInput)
    (modelReject : SandboxModel.RuntimeController.Reject)
    (rejected : SandboxModel.RuntimeController.handle model
      (runtimeInput input) = .error modelReject) :
    ∃ rustReject,
      SandboxControllerRust.recovery_monitor.handle_runtime rust input =
          ok (core.result.Result.Err rustReject) ∧
        runtimeReject rustReject = modelReject := by
  rcases handleRuntime_refines rust model represented input with
    ⟨rustResult, rustEvaluates, related⟩
  rw [rejected] at related
  cases rustResult with
  | Err rustReject => exact ⟨rustReject, rustEvaluates, related⟩
  | Ok rustDecision => simp [RuntimeResultRep] at related

end SandboxModel.RustRefinement
