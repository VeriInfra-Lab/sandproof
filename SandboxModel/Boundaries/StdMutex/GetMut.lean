import SandboxModel.Boundaries.StdMutex.BorrowModel
import SandproofStdMutex.Funs
open Aeneas Aeneas.Std

namespace SandboxModel.Boundaries.StdMutexContract.Borrow
theorem original_serial_wrapper_is_original_call (s : Cell T) :
    SandproofStdMutexRust.application.diagnostics.SerialMutex.get_mut s =
      std.sync.poison.mutex.Mutex.get_mut s := by
  simp [SandproofStdMutexRust.application.diagnostics.SerialMutex.get_mut,
    std.sync.poison.mutex.Mutex.get_mut]

theorem original_serial_get_mut_contract :
    GetMutContract (T := T)
      SandproofStdMutexRust.application.diagnostics.SerialMutex.get_mut := by
  intro s
  refine ⟨StdMutexExternal.view s, StdMutexExternal.restore s, ?_, ?_, ?_⟩
  · rw [original_serial_wrapper_is_original_call]
    rfl
  · cases h : s.poisoned <;> simp [BorrowView, StdMutexExternal.view, h]
  · intro updated
    cases updated <;> simp [BackView, StdMutexExternal.restore, UpdatedPayload]

theorem unchanged_borrow_returns_original_cell (s : Cell T) :
    StdMutexExternal.restore s (StdMutexExternal.view s) = s := by
  cases s with
  | mk identity payload poison rawLocked =>
    cases poison <;> rfl

theorem updated_borrow_returns_updated_value (s : Cell T) (v : View T)
    (compatible : SameArm s v) :
    StdMutexExternal.view (StdMutexExternal.restore s v) = v := by
  cases h : s.poisoned <;> cases v <;>
    simp_all [SameArm, StdMutexExternal.view, StdMutexExternal.restore]

theorem write_back_keeps_original_identity_poison_and_lock (s : Cell T) (v : View T) :
    BackView s (StdMutexExternal.restore s v) v := by
  cases v <;> simp [BackView, StdMutexExternal.restore, UpdatedPayload]

theorem poisoned_call_keeps_original_payload (identity : Nat) (value : T) (held : Bool) :
    BorrowView (⟨identity, value, true, held⟩ : Cell T) (.Err ⟨value⟩) := by rfl

theorem nonempty_poisoned_abandoned_lock_witness :
    SandproofStdMutexRust.application.diagnostics.SerialMutex.get_mut
      (⟨7, 19, true, true⟩ : Cell Nat) =
      .ok (.Err ⟨19⟩, StdMutexExternal.restore ⟨7, 19, true, true⟩) ∧
    StdMutexExternal.restore (⟨7, 19, true, true⟩ : Cell Nat) (.Err ⟨31⟩) =
      ⟨7, 31, true, true⟩ := by
  constructor
  · rw [original_serial_wrapper_is_original_call]
    rfl
  · rfl
end SandboxModel.Boundaries.StdMutexContract.Borrow
