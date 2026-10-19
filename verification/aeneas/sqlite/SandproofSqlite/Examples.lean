import SandproofSqlite.Results

open Aeneas Aeneas.Std
open SandproofSqliteRust.Model
namespace SandproofSqliteRust.Examples

-- A4 allows either physical outcome after an unconfirmed reply. This relation
-- is a contract model, not a proof of SQLite, the filesystem or this connection.
def receiptAllows (applied : Bool) (reply : core.result.Result Unit rusqlite.error.Error) : Prop :=
  match reply with
  | .Ok _ => applied = true
  | .Err _ => True

def replies (reply : core.result.Result Unit rusqlite.error.Error) :
    core.ops.function.FnOnce Unit Unit (core.result.Result Unit rusqlite.error.Error) where
  call_once := fun _ _ => .ok reply

theorem unknown_before_is_allowed :
    receiptAllows false (.Err ⟨1⟩) := by trivial

theorem unknown_after_is_allowed :
    receiptAllows true (.Err ⟨1⟩) := by trivial

theorem confirmed_requires_effect (applied : Bool)
    (allowed : receiptAllows applied (.Ok ())) : applied = true := allowed

theorem unknown_invalidates_for_both_outcomes (applied : Bool) :
    receiptAllows applied (.Err ⟨1⟩) ∧
    controller.transaction.memo.commit_with (slotOps Nat) (replies (.Err ⟨1⟩))
      (some ()) ⟨some 7, some 9⟩ () =
        .ok (.Err (.CommitUncertain ⟨1⟩), none, (⟨none, some 9⟩ : Slots Nat)) := by
  constructor
  · trivial
  · exact Results.commit_unknown_exact _ _ _ _ _ rfl

theorem confirmed_publishes_original_candidate :
    receiptAllows true (.Ok ()) ∧
    controller.transaction.memo.commit_with (slotOps Nat) (replies (.Ok ()))
      (some ()) ⟨some 7, some 9⟩ () =
        .ok (.Ok (), none, (⟨some 9, none⟩ : Slots Nat)) := by
  constructor
  · rfl
  · exact Results.commit_success_exact _ _ _ _ rfl

end SandproofSqliteRust.Examples
