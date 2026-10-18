import Permit.Funs
import Policy
open Aeneas Aeneas.Std PermitPolicy Permit.permit_guards

namespace PermitProofs

theorem register_exact (bounded present : Bool) :
    register_gate_allows bounded present = .ok (!bounded || present) := by
  cases bounded <;> cases present <;> simp [register_gate_allows]

theorem read_exact (has : Bool) : signal_needs_revocation_read has = .ok (!has) := by
  cases has <;> simp [signal_needs_revocation_read]

theorem retain_exact (revoked : Bool) : retain_after_revocation revoked = .ok (!revoked) := by
  cases revoked <;> simp [retain_after_revocation]

theorem create_exact (last : Option Bool) :
    create_signal_required last = .ok (expectedCreate last) := by
  cases last <;> simp [create_signal_required, core.option.Option.unwrap_or, expectedCreate]

theorem first_exact (revoked : Bool) : dispatch_not_revoked revoked = .ok (!revoked) := by
  cases revoked <;> simp [dispatch_not_revoked]

theorem config_exact (bounded : Bool) : dispatch_configured bounded = .ok bounded := by
  simp [dispatch_configured]

theorem target_exact (current : Option U64) (vm generation expected : U64) :
    dispatch_target_matches current vm generation expected =
      .ok (expectedTarget current vm generation expected) := by
  cases current with
  | none => simp [dispatch_target_matches, expectedTarget]
  | some actual =>
    by_cases eq : actual = vm <;> simp [dispatch_target_matches, expectedTarget, eq]

theorem source_semantics : Semantics register_gate_allows signal_needs_revocation_read
    retain_after_revocation create_signal_required dispatch_not_revoked dispatch_configured
    dispatch_target_matches :=
  ⟨register_exact, read_exact, retain_exact, create_exact, first_exact, config_exact, target_exact⟩

theorem registration_iff (bounded present : Bool) :
    register_gate_allows bounded present = .ok true ↔ bounded = false ∨ present = true := by
  rw [register_exact]
  cases bounded <;> cases present <;> simp

theorem retain_signal_iff (has revoked : Bool) :
    retainSignal signal_needs_revocation_read retain_after_revocation has revoked = .ok true ↔
      has = true ∨ revoked = false := by
  rw [stages_retain_exact source_semantics]
  cases has <;> cases revoked <;> simp

theorem live_receiver_ignores_watch (retain : Bool → Result Bool) (revoked : Bool) :
    retainSignal signal_needs_revocation_read retain true revoked = .ok true := by
  simp [retainSignal, signal_needs_revocation_read]

theorem create_signal_iff (last : Option Bool) :
    create_signal_required last = .ok true ↔ last = none ∨ last = some true := by
  rw [create_exact]
  cases last with
  | none => simp [expectedCreate]
  | some revoked => cases revoked <;> simp [expectedCreate]

theorem reuse_signal_iff (last : Option Bool) :
    create_signal_required last = .ok false ↔ last = some false := by
  rw [create_exact]
  cases last with
  | none => simp [expectedCreate]
  | some revoked => cases revoked <;> simp [expectedCreate]

theorem dispatch_exact (revoked bounded : Bool) (current : Option U64)
    (vm generation expected : U64) :
    canDispatch dispatch_not_revoked dispatch_configured dispatch_target_matches
      revoked bounded current vm generation expected =
      .ok (mayDispatch revoked bounded current vm generation expected) :=
  stages_dispatch_exact source_semantics revoked bounded current vm generation expected

theorem dispatch_iff (revoked bounded : Bool) (current : Option U64)
    (vm generation expected : U64) :
    canDispatch dispatch_not_revoked dispatch_configured dispatch_target_matches
      revoked bounded current vm generation expected = .ok true ↔
      revoked = false ∧ bounded = true ∧ current = some vm ∧ generation = expected := by
  rw [dispatch_exact]
  constructor
  · intro allowed
    exact (dispatch_policy_iff revoked bounded current vm generation expected).mp
      (Result.ok_injective allowed)
  · intro conditions
    rw [(dispatch_policy_iff revoked bounded current vm generation expected).mpr conditions]

theorem dispatch_safe :
    DispatchSafe dispatch_not_revoked dispatch_configured dispatch_target_matches :=
  semantics_implies_safe source_semantics

theorem revoked_ignores_later_functions (config : Bool → Result Bool) (target : TargetGuard)
    (bounded : Bool) (current : Option U64) (vm generation expected : U64) :
    canDispatch dispatch_not_revoked config target true bounded current vm generation expected =
      .ok false := by
  simp [canDispatch, dispatch_not_revoked]

theorem unconfigured_ignores_target (target : TargetGuard) (current : Option U64)
    (vm generation expected : U64) :
    canDispatch dispatch_not_revoked dispatch_configured target false false
      current vm generation expected = .ok false := by
  simp [canDispatch, dispatch_not_revoked, dispatch_configured]

end PermitProofs
