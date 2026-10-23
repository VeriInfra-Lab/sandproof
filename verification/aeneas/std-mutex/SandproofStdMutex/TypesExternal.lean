import Aeneas

/-! A view of the original cell during a valid exclusive Rust borrow.
The payload describes inline data and original handles, not an immutable
snapshot of aliased heaps. The raw-lock bit may remain set after a forgotten
guard. get_mut neither acquires nor releases that lock. -/
@[rust_type "std::sync::poison::mutex::Mutex"]
structure std.sync.poison.mutex.Mutex (T : Type) where
  identity : Nat
  payload : T
  poisoned : Bool
  rawLocked : Bool

@[rust_type "std::sync::poison::PoisonError"]
structure std.sync.poison.PoisonError (T : Type) where
  data : T
