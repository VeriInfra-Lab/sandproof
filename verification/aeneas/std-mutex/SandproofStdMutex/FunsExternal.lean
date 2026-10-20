import SandproofStdMutex.Types
open Aeneas Aeneas.Std

namespace StdMutexExternal
abbrev Cell (T : Type) := std.sync.poison.mutex.Mutex T
abbrev View (T : Type) := core.result.Result T (std.sync.poison.PoisonError T)

def view (s : Cell T) : View T :=
  if s.poisoned then .Err ⟨s.payload⟩ else .Ok s.payload

def restore (s : Cell T) (updated : View T) : Cell T :=
  { s with payload := match updated with
      | .Ok value => value
      | .Err error => error.data }
end StdMutexExternal

/-- Only the original exclusive get_mut call. The continuation represents
writes through the returned original borrow; it does not perform physical
Drop, imply a repaired invariant, clear poison or unlock abandoned locks. -/
@[rust_fun "std::sync::poison::mutex::{std::sync::poison::mutex::Mutex<@T>}::get_mut"]
def std.sync.poison.mutex.Mutex.get_mut {T : Type}
    (s : std.sync.poison.mutex.Mutex T) :
    Result ((core.result.Result T (std.sync.poison.PoisonError T)) ×
      (core.result.Result T (std.sync.poison.PoisonError T) →
        std.sync.poison.mutex.Mutex T)) :=
  .ok (StdMutexExternal.view s, StdMutexExternal.restore s)
