import BadPermit.Funs
import Policy
open Aeneas Aeneas.Std PermitPolicy BadPermit.permit_guards

namespace NegativeProofs

-- The revoked watch was the only failing guard. All later finite guards pass.
theorem bad_source_witness (vm generation : U64) :
    canDispatch dispatch_not_revoked dispatch_configured dispatch_target_matches
      true true (some vm) vm generation generation = .ok true := by
  simp [canDispatch, dispatch_not_revoked, dispatch_configured, dispatch_target_matches]

-- Exactly the predicate proved for the correct actual Rust guards.
theorem rejected_same_dispatch_safe (vm generation : U64) :
    ¬ DispatchSafe dispatch_not_revoked dispatch_configured dispatch_target_matches := by
  intro safe
  have contradiction := safe true true (some vm) vm generation generation
    (bad_source_witness vm generation)
  simp at contradiction

theorem rejected_same_semantics (vm generation : U64) :
    ¬ Semantics register_gate_allows signal_needs_revocation_read retain_after_revocation
      create_signal_required dispatch_not_revoked dispatch_configured dispatch_target_matches := by
  intro contract
  exact rejected_same_dispatch_safe vm generation (semantics_implies_safe contract)

end NegativeProofs
