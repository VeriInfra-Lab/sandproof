import Aeneas

/-! Concrete callback-core contracts. These specify actual generated functions,
not a trusted predicate. The surrounding retain traversal has separate A6/B4
bindings. Ordinary value/valid safe borrows and normal allocation are required;
a failed computation does not expose a rolled-back external heap. -/
namespace SandboxModel.Foundations.CallbackState
open Aeneas Aeneas.Std

abbrev Row := U64 × U64
abbrev TraceValue := alloc.vec.Vec Row
abbrev ToggleFn := Bool → U64 → U64 → Result (Bool × Bool)
abbrev TraceFn := TraceValue → U64 → U64 → U64 → Result (Bool × TraceValue)

def ToggleContract (run : ToggleFn) : Prop :=
  ∀ state key value, run state key value = .ok (!state, !state)

def TraceContract (run : TraceFn) : Prop :=
  ∀ visited key value floor, visited.val.length < Usize.max →
    run visited key value floor
      ⦃ result => result.1 = decide (floor.val < value.val) ∧
        result.2.val = visited.val ++ [(key, value)] ⦄

end SandboxModel.Foundations.CallbackState
