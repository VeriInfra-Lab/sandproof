import SandboxModel.RustRefinement.Values
import SandboxModel.Monitor

/-!
Refinement proof for the trace monitor extracted by Aeneas from
`controller-core/src/recovery_monitor.rs`.

Unlike controller transitions, the monitor is read-only.  The only boundary
conversions are the bounded generation counter, the optional observation, and
the finite Rust slice.  The loop theorem below starts at an arbitrary valid
slice index; the public `monitor_from` and `monitor` theorems are immediate
instances at index zero.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result ControlFlow

/-- The observation-value boundary used by the Rust monitor. -/
def monitorSandboxView
    (value : SandboxControllerRust.recovery_monitor.SandboxView) :
    SandboxModel.Monitor.SandboxView where
  phase := phase value.phase
  terminal := value.terminal
  generation := u64ToNat value.generation

/-- Lift the observation boundary through absence. -/
def monitorOptionView
    (value : Option SandboxControllerRust.recovery_monitor.SandboxView) :
    Option SandboxModel.Monitor.SandboxView :=
  value.map monitorSandboxView

/-- Lift the observation boundary through the concrete finite slice. -/
def monitorViewList
    (values : Slice (Option SandboxControllerRust.recovery_monitor.SandboxView)) :
    List (Option SandboxModel.Monitor.SandboxView) :=
  values.val.map monitorOptionView

@[simp] theorem monitorOptionView_none : monitorOptionView none = none := rfl

@[simp] theorem monitorOptionView_some value :
    monitorOptionView (some value) = some (monitorSandboxView value) := rfl

/-- One concrete monitor observation has exactly the acceptance bit and latch
    update encoded by the corresponding abstract `monitorFrom` head step. -/
@[step] theorem generated_monitorObservation_refines
    (seen : Bool)
    (observation : Option SandboxControllerRust.recovery_monitor.SandboxView) :
    SandboxControllerRust.recovery_monitor.monitor_observation seen observation
      ⦃ state =>
        ∀ rest,
          SandboxModel.Monitor.monitorFrom seen
              (monitorOptionView observation :: rest) =
            (if state.accepted then
              SandboxModel.Monitor.monitorFrom state.seen_terminal rest
            else false) ⦄ := by
  cases observation with
  | none =>
      cases seen <;>
        simp [SandboxControllerRust.recovery_monitor.monitor_observation,
          monitorOptionView, SandboxModel.Monitor.monitorFrom]
  | some observation =>
      cases observation with
      | mk concretePhase terminal generation =>
          cases seen <;> cases terminal <;> cases concretePhase <;>
            simp [SandboxControllerRust.recovery_monitor.monitor_observation,
              SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase.eq,
              SandboxControllerRust.model.Phase.read_discriminant,
              core.cmp.PartialEq.ne.default,
              core.cmp.PartialEq.ne.trait_default,
              monitorOptionView, monitorSandboxView, phase,
              SandboxModel.Monitor.monitorFrom]

/-- The generated Rust `view` and the model projection agree, including both
    the missing-sandbox and present-sandbox branches. -/
theorem generated_view_refines
    [DecidableEq SandboxControllerRust.model.OperationKey]
    [DecidableEq SandboxControllerRust.model.LogicalEffectId]
    [DecidableEq SandboxControllerRust.model.EffectId]
    (relations : WorldValueRelations)
    (projectionCompatible :
      ∀ concreteRecord modelRecord,
        relations.sandboxRecord concreteRecord modelRecord →
          monitorSandboxView
              {
                phase := concreteRecord.phase
                terminal := concreteRecord.terminal
                generation := concreteRecord.generation
              } =
            ({
              phase := modelRecord.phase
              terminal := modelRecord.terminal
              generation := modelRecord.generation
            } : SandboxModel.Monitor.SandboxView))
    {rustWorld : SandboxControllerRust.model.World} {modelWorld : World}
    (represented : WorldRep relations rustWorld modelWorld)
    (sandbox : SandboxControllerRust.ids.SandboxId) :
    SandboxControllerRust.recovery_monitor.view rustWorld sandbox
      ⦃ observation =>
        observation.map monitorSandboxView =
          SandboxModel.Monitor.view modelWorld (sandboxId sandbox) ⦄ := by
  unfold SandboxControllerRust.recovery_monitor.view
  apply Aeneas.Std.WP.spec_bind
    (generated_get_refines
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful rustWorld.sandboxes sandbox)
  intro found foundPost
  subst found
  have agrees := represented.sandboxes.lookupAgrees sandbox
  cases concreteLookup : storeLookup rustWorld.sandboxes sandbox with
  | none =>
      rw [concreteLookup] at agrees
      cases modelLookup : modelWorld.sandboxes (sandboxId sandbox) with
      | none =>
          simp [modelLookup, SandboxModel.Monitor.view]
      | some record =>
          simp [OptionRel, modelLookup] at agrees
  | some concreteRecord =>
      rw [concreteLookup] at agrees
      cases modelLookup : modelWorld.sandboxes (sandboxId sandbox) with
      | none =>
          simp [OptionRel, modelLookup] at agrees
      | some modelRecord =>
          have recordsRelated :
              relations.sandboxRecord concreteRecord modelRecord := by
            simpa [OptionRel, modelLookup] using agrees
          simp only [WP.spec_ok, Option.map_some,
            SandboxModel.Monitor.view, modelLookup]
          exact congrArg some
            (projectionCompatible concreteRecord modelRecord recordsRelated)

/-- The complete concrete value relation preserves exactly the three fields
    exported by the monitor projection. -/
theorem valueRelations_monitorProjectionCompatible :
    ∀ concreteRecord modelRecord,
      valueRelations.sandboxRecord concreteRecord modelRecord →
        monitorSandboxView
            {
              phase := concreteRecord.phase
              terminal := concreteRecord.terminal
              generation := concreteRecord.generation
            } =
          ({
            phase := modelRecord.phase
            terminal := modelRecord.terminal
            generation := modelRecord.generation
          } : SandboxModel.Monitor.SandboxView) := by
  intro concreteRecord modelRecord related
  change sandboxRecord concreteRecord = modelRecord at related
  subst modelRecord
  rfl

/-- Concrete specialization of `generated_view_refines` for the actual
    persistent-world relation used by all controller commuting diagrams. -/
theorem generated_view_valueRelations_refines
    {rustWorld : SandboxControllerRust.model.World} {modelWorld : World}
    (represented :
      @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
        effectIdDecidableEq valueRelations rustWorld modelWorld)
    (sandbox : SandboxControllerRust.ids.SandboxId) :
    SandboxControllerRust.recovery_monitor.view rustWorld sandbox
      ⦃ observation =>
        observation.map monitorSandboxView =
          SandboxModel.Monitor.view modelWorld (sandboxId sandbox) ⦄ := by
  letI : DecidableEq SandboxControllerRust.model.OperationKey :=
    operationKeyDecidableEq
  letI : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
    logicalEffectIdDecidableEq
  letI : DecidableEq SandboxControllerRust.model.EffectId :=
    effectIdDecidableEq
  exact generated_view_refines valueRelations
    valueRelations_monitorProjectionCompatible represented sandbox

/-- From any valid concrete index, the Aeneas loop computes the abstract
    monitor over exactly the translated suffix beginning at that index. -/
theorem generated_monitorFrom_loop_refines
    (seen : Bool)
    (views : Slice (Option SandboxControllerRust.recovery_monitor.SandboxView))
    (start : Std.Usize) (startBound : start.val ≤ views.val.length) :
    SandboxControllerRust.recovery_monitor.monitor_from_loop seen views start
      ⦃ accepted =>
        accepted = SandboxModel.Monitor.monitorFrom seen
          ((views.val.drop start.val).map monitorOptionView) ⦄ := by
  unfold SandboxControllerRust.recovery_monitor.monitor_from_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : Bool × Std.Usize => views.val.length - state.2.val)
    (fun state : Bool × Std.Usize =>
      state.2.val ≤ views.val.length ∧
      SandboxModel.Monitor.monitorFrom state.1
          ((views.val.drop state.2.val).map monitorOptionView) =
        SandboxModel.Monitor.monitorFrom seen
          ((views.val.drop start.val).map monitorOptionView))
    (fun accepted =>
      accepted = SandboxModel.Monitor.monitorFrom seen
        ((views.val.drop start.val).map monitorOptionView))
  · intro state invariant
    rcases state with ⟨seenNow, index⟩
    rcases invariant with ⟨indexUpper, monitorSame⟩
    unfold SandboxControllerRust.recovery_monitor.monitor_from_loop.body
    dsimp only
    split
    · rename_i inBounds
      step as ⟨ observation, observationPost ⟩
      have indexBound : index.val < views.val.length := by
        simpa using inBounds
      have dropEq : views.val.drop index.val =
          observation :: views.val.drop (index.val + 1) := by
        rw [List.drop_eq_getElem_cons indexBound]
        rw [← observationPost]
      step as ⟨ next, nextSpec ⟩
      have headStep := nextSpec (views.val.drop (index.val + 1) |>.map monitorOptionView)
      split
      · rename_i accepted
        step as ⟨ nextIndex, nextIndexPost ⟩
        constructor
        · scalar_tac
        · constructor
          · rw [nextIndexPost]
            rw [dropEq, List.map_cons] at monitorSame
            rw [headStep, if_pos accepted] at monitorSame
            exact monitorSame
          · omega
      · rename_i rejected
        simp only [WP.spec_ok]
        rw [dropEq, List.map_cons] at monitorSame
        rw [headStep, if_neg rejected] at monitorSame
        exact monitorSame
    · rename_i outOfBounds
      simp only [WP.spec_ok]
      have indexLower : views.val.length ≤ index.val := by
        simpa using outOfBounds
      have dropEmpty : views.val.drop index.val = [] :=
        List.drop_eq_nil_of_le indexLower
      rw [dropEmpty] at monitorSame
      cases seenNow <;>
        simpa [SandboxModel.Monitor.monitorFrom] using monitorSame.symm
  · exact ⟨startBound, rfl⟩

/-- Public Rust `monitor_from` exactly implements the abstract monitor after
    translating the concrete slice. -/
theorem generated_monitorFrom_refines
    (seen : Bool)
    (views : Slice (Option SandboxControllerRust.recovery_monitor.SandboxView)) :
    SandboxControllerRust.recovery_monitor.monitor_from seen views
      ⦃ accepted =>
        accepted = SandboxModel.Monitor.monitorFrom seen (monitorViewList views) ⦄ := by
  unfold SandboxControllerRust.recovery_monitor.monitor_from monitorViewList
  simpa using generated_monitorFrom_loop_refines seen views 0#usize (by simp)

/-- Equality form of `generated_monitorFrom_refines`: the generated function
    terminates normally and returns exactly the model boolean. -/
theorem generated_monitorFrom_eq
    (seen : Bool)
    (views : Slice (Option SandboxControllerRust.recovery_monitor.SandboxView)) :
    SandboxControllerRust.recovery_monitor.monitor_from seen views =
      ok (SandboxModel.Monitor.monitorFrom seen (monitorViewList views)) := by
  obtain ⟨accepted, evaluation, same⟩ :=
    Aeneas.Std.WP.spec_imp_exists (generated_monitorFrom_refines seen views)
  rw [same] at evaluation
  exact evaluation

/-- The top-level Rust monitor is extensionally equal to the model monitor. -/
theorem generated_monitor_refines
    (views : Slice (Option SandboxControllerRust.recovery_monitor.SandboxView)) :
    SandboxControllerRust.recovery_monitor.monitor views
      ⦃ accepted =>
        accepted = SandboxModel.Monitor.monitor (monitorViewList views) ⦄ := by
  unfold SandboxControllerRust.recovery_monitor.monitor SandboxModel.Monitor.monitor
  exact generated_monitorFrom_refines false views

/-- Direct equality for the public top-level Rust monitor. -/
theorem generated_monitor_eq
    (views : Slice (Option SandboxControllerRust.recovery_monitor.SandboxView)) :
    SandboxControllerRust.recovery_monitor.monitor views =
      ok (SandboxModel.Monitor.monitor (monitorViewList views)) := by
  obtain ⟨accepted, evaluation, same⟩ :=
    Aeneas.Std.WP.spec_imp_exists (generated_monitor_refines views)
  rw [same] at evaluation
  exact evaluation

end SandboxModel.RustRefinement
