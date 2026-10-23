import SandboxModel.Boundaries.TokioMutex.Core

namespace SandboxModel.Boundaries.TokioMutexContract.Examples
def start : State Nat := initial 7 5
def guard : Guard := TokioRwLockContract.fresh start
def held : State Nat := acquired start
def updated : State Nat := { held with payload := 8 }

theorem nonempty_legal_history : History (fun _ _ : Nat => True) start (released updated) :=
  .next (.next (.next (.refl _) (.returned start rfl))
    (.writeValue held guard 8 (returned_guard_is_original start) trivial))
    (.guardDrop updated guard (returned_guard_is_original start))

theorem own_mutation_and_release_do_not_rollback : (released updated).payload = 8 := rfl

theorem retained_use_is_legal : ProtectedUse held guard := acquired_use_is_protected start

theorem detached_early_release_is_rejected : ¬ ProtectedUse (released held) guard :=
  early_release_rejected_by_same_protected_use held guard

end SandboxModel.Boundaries.TokioMutexContract.Examples
