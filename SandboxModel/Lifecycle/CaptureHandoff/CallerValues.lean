import CaptureCaller.Funs
import SandboxModel.Lifecycle.CaptureHandoff.Conditions
import SandboxModel.ProofAuditCommand

open SandboxModel.Lifecycle.CaptureHandoffProof
open Aeneas Aeneas.Std CaptureCallerRust
open CaptureCallerRust.sandbox_controller_core
open CaptureCallerRust.sandbox_controller_core.capture

namespace SandboxModel.Lifecycle.CaptureHandoffCaller

abbrev State := CaptureCallerRust.sandbox_controller_core.capture.CaptureHandoff

def useView (state : State) : Bool × Bool := (state.capture_work, state.unload_source)
def revoked (state : State) : Prop := state.eligibility = RestoreEligibility.Revoked

/-- Composition of the two actual Rust getters; no synthetic business entry. -/
theorem material_projection (state : State) :
    (do let work ← CaptureHandoff.has_capture_work state
        let source ← CaptureHandoff.has_unload_source state
        .ok (work, source)) = .ok (useView state) := by
  simp [useView, CaptureHandoff.has_capture_work, CaptureHandoff.has_unload_source]

theorem failure_preserves_uses : FailurePreservesUses CaptureHandoff.ordinary_failure useView := by
  intro state
  rcases state with ⟨commit, eligibility, work, source, continuation, drain⟩
  cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp [CaptureHandoff.ordinary_failure, useView,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq, CaptureCommit.read_discriminant]

theorem late_success_safe : LateSuccessSafe CaptureHandoff.observe_bundle revoked useView CaptureHandoff.may_stop_original := by
  intro state after wasRevoked observed
  rcases state with ⟨commit, eligibility, work, source, continuation, drain⟩
  cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp_all [revoked, useView, CaptureHandoff.observe_bundle, CaptureHandoff.may_stop_original,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq, CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]
    <;> subst after <;> simp

theorem unknown_failure_preserves_unknown (state : State)
    (unknown : state.commit = CaptureCommit.Unknown) :
    ∃ after, CaptureHandoff.ordinary_failure state = .ok after ∧
      after.commit = CaptureCommit.Unknown ∧ revoked after ∧ useView after = useView state := by
  rcases state with ⟨commit, eligibility, work, source, continuation, drain⟩
  cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp_all [CaptureHandoff.ordinary_failure, revoked, useView,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq, CaptureCommit.read_discriminant]

theorem normal_handoff_accepts :
    ∃ initial pending complete : State,
      CaptureHandoff.accepted = .ok initial ∧ CaptureHandoff.registration_dispatched initial = .ok pending ∧
      CaptureHandoff.register_bundle pending = .ok (some complete) ∧ CaptureHandoff.may_stop_original complete = .ok true ∧
      useView complete = (true, true) := by
  let initial : State := ⟨.NotRegistered, .Capturing, true, false, none, none⟩
  let pending : State := ⟨.Unknown, .Capturing, true, false, none, none⟩
  let complete : State := ⟨.Registered, .Recoverable, true, true, none, none⟩
  refine ⟨initial, pending, complete, ?_, ?_, ?_, ?_, rfl⟩ <;>
    simp [initial, pending, complete, CaptureHandoff.accepted, CaptureHandoff.registration_dispatched, CaptureHandoff.register_bundle, CaptureHandoff.may_stop_original, CaptureHandoff.accepts_registration,
      CaptureHandoff.observe_bundle,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]

theorem revoked_late_success_accepts_material :
    ∃ pending cancelled material : State,
      pending.commit = CaptureCommit.Unknown ∧
      CaptureHandoff.ordinary_failure pending = .ok cancelled ∧ CaptureHandoff.observe_bundle cancelled = .ok (some material) ∧
      revoked material ∧ CaptureHandoff.may_stop_original material = .ok false ∧
      useView material = (true, true) := by
  let pending : State := ⟨.Unknown, .Capturing, true, false, none, none⟩
  let cancelled : State := ⟨.Unknown, .Revoked, true, false, none, none⟩
  let material : State := ⟨.Registered, .Revoked, true, true, none, none⟩
  refine ⟨pending, cancelled, material, rfl, ?_, ?_, rfl, ?_, rfl⟩ <;>
    simp [pending, cancelled, material, CaptureHandoff.ordinary_failure, CaptureHandoff.observe_bundle, CaptureHandoff.may_stop_original,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]


/-- Edges are successful returns of the actual extracted value methods.
Continuation/drain preparation and World/SQL transitions are outside this relation. -/
inductive Step : State → State → Prop where
  | dispatch {before after} : CaptureHandoff.registration_dispatched before = .ok after → Step before after
  | register {before after} : CaptureHandoff.register_bundle before = .ok (some after) → Step before after
  | observe {before after} : CaptureHandoff.observe_bundle before = .ok (some after) → Step before after
  | failure {before after} : CaptureHandoff.ordinary_failure before = .ok after → Step before after
  | terminate {before after} : CaptureHandoff.terminate before = .ok after → Step before after

inductive History : State → State → Prop where
  | refl (state) : History state state
  | snoc {first before after} : History first before → Step before after → History first after

/-- The two new responsibility records are retained, not erased by projection. -/
def SameResponsibilities (before after : State) : Prop :=
  after.original_continuation = before.original_continuation ∧
  after.terminal_drain = before.terminal_drain ∧
  after.capture_work = before.capture_work ∧
  (before.unload_source = true → after.unload_source = true)

theorem step_preserves_responsibilities {before after : State} (step : Step before after) :
    SameResponsibilities before after ∧ (revoked before → revoked after) := by
  cases step <;>
    rcases before with ⟨commit, eligibility, work, source, continuation, drain⟩ <;>
    cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp_all [SameResponsibilities, revoked, CaptureHandoff.registration_dispatched,
      CaptureHandoff.register_bundle, CaptureHandoff.observe_bundle,
      CaptureHandoff.ordinary_failure, CaptureHandoff.terminate, CaptureHandoff.accepts_registration,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]
    <;> subst after <;> simp

theorem history_preserves_responsibilities {first after : State}
    (history : History first after) : SameResponsibilities first after := by
  induction history with
  | refl => exact ⟨rfl, rfl, rfl, id⟩
  | snoc history step ih =>
    obtain ⟨hc, hd, hw, hs⟩ := (step_preserves_responsibilities step).1
    exact ⟨hc.trans ih.1, hd.trans ih.2.1, hw.trans ih.2.2.1,
      fun h => hs (ih.2.2.2 h)⟩

theorem history_preserves_revocation {first after : State}
    (history : History first after) (wasRevoked : revoked first) : revoked after := by
  induction history with
  | refl => exact wasRevoked
  | snoc history step ih => exact (step_preserves_responsibilities step).2 ih

theorem stop_requires_complete_registration (state : State) :
    CaptureHandoff.may_stop_original state = .ok true ↔
      state.commit = .Registered ∧ state.eligibility = .Recoverable ∧
      state.unload_source = true := by
  rcases state with ⟨commit, eligibility, work, source, continuation, drain⟩
  cases commit <;> cases eligibility <;> cases source <;>
    simp [CaptureHandoff.may_stop_original,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]

theorem revoked_history_never_grants_stop {first after : State}
    (history : History first after) (wasRevoked : revoked first) :
    CaptureHandoff.may_stop_original after ≠ .ok true := by
  intro granted
  have eligibility := ((stop_requires_complete_registration after).1 granted).2.1
  have remains := history_preserves_revocation history wasRevoked
  simp [revoked, eligibility] at remains

theorem normal_history_accepts :
    ∃ initial complete : State, CaptureHandoff.accepted = .ok initial ∧ History initial complete ∧
      CaptureHandoff.may_stop_original complete = .ok true ∧
      complete.original_continuation = none ∧ complete.terminal_drain = none := by
  obtain ⟨initial, pending, complete, ha, hd, hr, hp, _⟩ := normal_handoff_accepts
  have path := History.snoc (History.snoc (History.refl initial) (Step.dispatch hd))
    (Step.register hr)
  have fields := history_preserves_responsibilities path
  have initialFields : initial.original_continuation = none ∧ initial.terminal_drain = none := by
    simp [CaptureHandoff.accepted] at ha
    subst initial
    exact ⟨rfl, rfl⟩
  exact ⟨initial, complete, ha, path, hp, fields.1.trans initialFields.1,
    fields.2.1.trans initialFields.2⟩

end SandboxModel.Lifecycle.CaptureHandoffCaller

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.material_projection
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.failure_preserves_uses
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.late_success_safe
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.unknown_failure_preserves_unknown
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.normal_handoff_accepts
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.revoked_late_success_accepts_material

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.step_preserves_responsibilities

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.history_preserves_responsibilities

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.history_preserves_revocation

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.stop_requires_complete_registration

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.revoked_history_never_grants_stop

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.normal_history_accepts
