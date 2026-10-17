import SandproofDataDrain.Types
open Aeneas Aeneas.Std

/-! Three read providers only. Other orderings are rejected as outside this
model's supported subset, not classified as errors of the real standard library.
Extraction auditing requires that the actual call uses Acquire. -/
def core.sync.atomic.AtomicBool.load
    (value : core.sync.atomic.Atomic Bool (core.sync.atomic.private.Align1 U8))
    (ordering : SandproofDataDrainRust.core.sync.atomic.Ordering) : Result Bool :=
  match ordering with
  | .Acquire => .ok value
  | _ => .fail .panic

def alloc.sync.Arc.strong_count {T : Type} (_A : Type)
    (view : alloc.sync.Arc T) : Result Usize :=
  .ok view.observedStrongCount

def alloc.sync.Arc.Insts.CoreOpsDerefDeref.deref {T : Type} (_A : Type)
    (view : alloc.sync.Arc T) : Result T :=
  .ok view.observedValue
