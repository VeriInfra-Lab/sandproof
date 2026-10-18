import Aeneas

/-! A6-data-drain-read: one invocation's observations, not a shared heap.
`observedStrongCount` is sampled by strong_count; the boolean is sampled by
the later Acquire load, if reached. They need not be an atomic snapshot.
No model of Arc creation, clone, mutation, Weak, destruction or physical
exit is provided. Object identity and observation accuracy are boundary
conditions; exclusion of new pins and permission to release are client duties. -/
def core.sync.atomic.private.Align1 (_T : Type) : Type := Unit
def core.sync.atomic.Atomic (T : Type) (_Storage : Type) : Type := T

structure alloc.sync.Arc (T : Type) where
  objectId : Nat
  observedStrongCount : Aeneas.Std.Usize
  observedValue : T
