import CaptureCaller.Funs

/-! Successful observation of a bundle is characterized from the actual
generated function, without assuming that registration was already dispatched.
Both NotRegistered and Unknown can succeed. This value-method connection is
not evidence that a bundle exists, was durably committed or belongs to a caller.
-/
namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust
open CaptureCallerRust.sandbox_controller_core
open CaptureCallerRust.sandbox_controller_core.capture

theorem caller_observation_success_iff (before after : CaptureHandoff) :
    CaptureHandoff.observe_bundle before = .ok (some after) ↔
      before.capture_work = true ∧ before.unload_source = false ∧
      before.commit ≠ .Registered ∧ before.eligibility ≠ .Recoverable ∧
      after = { before with
        commit := .Registered
        unload_source := true
        eligibility := match before.eligibility with
          | .Capturing => .Recoverable
          | eligibility => eligibility } := by
  rcases before with ⟨commit, eligibility, work, source, continuation, drain⟩
  cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp [CaptureHandoff.observe_bundle,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant, eq_comm]

theorem caller_observation_success_preconditions (before after : CaptureHandoff)
    (observed : CaptureHandoff.observe_bundle before = .ok (some after)) :
    before.capture_work = true ∧ before.unload_source = false ∧
      before.commit ≠ .Registered ∧ before.eligibility ≠ .Recoverable := by
  obtain ⟨work, source, commit, eligibility, _output⟩ :=
    (caller_observation_success_iff before after).1 observed
  exact ⟨work, source, commit, eligibility⟩

theorem caller_observation_success_output (before after : CaptureHandoff)
    (observed : CaptureHandoff.observe_bundle before = .ok (some after)) :
    after = { before with
      commit := .Registered
      unload_source := true
      eligibility := match before.eligibility with
        | .Capturing => .Recoverable
        | eligibility => eligibility } := by
  exact ((caller_observation_success_iff before after).1 observed).2.2.2.2

theorem caller_observation_success_fields (before after : CaptureHandoff)
    (observed : CaptureHandoff.observe_bundle before = .ok (some after)) :
    after.commit = .Registered ∧ after.unload_source = true ∧
      after.eligibility = (match before.eligibility with
        | .Capturing => .Recoverable
        | eligibility => eligibility) ∧
      after.capture_work = before.capture_work ∧
      after.original_continuation = before.original_continuation ∧
      after.terminal_drain = before.terminal_drain := by
  rw [caller_observation_success_output before after observed]
  exact ⟨rfl, rfl, rfl, rfl, rfl, rfl⟩

/-- Successful late observation preserves revoked eligibility and cannot
grant the original-VM stop permit, irrespective of the input commit tag. -/
theorem caller_observation_revoked_success (before after : CaptureHandoff)
    (observed : CaptureHandoff.observe_bundle before = .ok (some after))
    (wasRevoked : before.eligibility = .Revoked) :
    after.eligibility = .Revoked ∧ CaptureHandoff.may_stop_original after = .ok false ∧
      after.capture_work = before.capture_work ∧
      after.original_continuation = before.original_continuation ∧
      after.terminal_drain = before.terminal_drain := by
  obtain ⟨registered, _source, eligibility, work, continuation, drain⟩ :=
    caller_observation_success_fields before after observed
  have stillRevoked : after.eligibility = .Revoked := by
    simpa [wasRevoked] using eligibility
  refine ⟨stillRevoked, ?_, work, continuation, drain⟩
  simp [CaptureHandoff.may_stop_original, registered, stillRevoked,
    CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
    RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
    CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]

/-- A legal value-level success with NotRegistered demonstrates that the
general inverse rule has not silently strengthened its input to Unknown. -/
theorem caller_observation_not_registered_accepts :
    ∃ before after : CaptureHandoff, before.commit = .NotRegistered ∧
      before.eligibility = .Revoked ∧
      CaptureHandoff.observe_bundle before = .ok (some after) ∧
      after.eligibility = .Revoked ∧ CaptureHandoff.may_stop_original after = .ok false := by
  let before : CaptureHandoff := ⟨.NotRegistered, .Revoked, true, false, none, none⟩
  let after : CaptureHandoff := ⟨.Registered, .Revoked, true, true, none, none⟩
  have observed : CaptureHandoff.observe_bundle before = .ok (some after) := by
    apply (caller_observation_success_iff before after).2
    exact ⟨rfl, rfl, by simp [before], by simp [before], rfl⟩
  exact ⟨before, after, rfl, rfl, observed, rfl,
    (caller_observation_revoked_success before after observed rfl).2.1⟩

end SandboxModel.Lifecycle.CaptureHandoffCaller
