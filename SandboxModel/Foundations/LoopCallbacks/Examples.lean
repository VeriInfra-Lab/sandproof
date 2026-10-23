import SandboxModel.Foundations.LoopCallbacks.Refinement

/-! Finite legal nonempty witnesses. Duplicate rows are permitted by the
query contract; their first value is preserved, not overwritten or normalized. -/
namespace SandboxModel.Foundations.LoopCallbacks
open Aeneas Aeneas.Std SandproofLoopCallbacksRust
open SandboxModel.Foundations

def duplicateExample : sandbox_controller_core.store.Store U64 U64 :=
  ⟨alloc.vec.Vec.from [(7#u64, 11#u64), (7#u64, 12#u64), (2#u64, 20#u64)] (by scalar_tac)⟩

theorem duplicate_first_example : lookup_u64 duplicateExample 7#u64 = .ok (some 11#u64) := by
  simpa [duplicateExample, tableLookup] using u64_query_contract duplicateExample 7#u64

theorem nonempty_membership_example : contains_u64 duplicateExample 7#u64 = .ok true := by
  simpa [duplicateExample, tableLookup] using u64_membership_contract duplicateExample 7#u64

theorem absent_key_example : lookup_u64 duplicateExample 9#u64 = .ok none := by
  simpa [duplicateExample, tableLookup] using u64_query_contract duplicateExample 9#u64

end SandboxModel.Foundations.LoopCallbacks
