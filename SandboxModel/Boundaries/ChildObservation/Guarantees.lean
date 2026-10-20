import SandboxModel.Boundaries.ChildObservation.Refinement

open Aeneas Aeneas.Std
open SandproofChildObservationRust
namespace SandboxModel.Boundaries.ChildObservation.Guarantees

variable {I C L S E T K : Type}

theorem tail_member {history suffix : List α} (tail : Tail history suffix)
    {value : α} (member : value ∈ suffix) : value ∈ history := by
  rcases tail with ⟨before, same⟩
  rw [same]
  exact List.mem_append.mpr (Or.inr member)

/-- The primitive meanings are per-call A2/B4 evidence, not a promise that the
whole program or global reaper is correct. Both facts are historical. -/
structure Meaning (events : List (Event K L S E T))
    (reaped : K → S → Prop) (transferred : K → L → Prop) : Prop where
  poll : ∀ key status, Event.poll key (.Ok (some status)) ∈ events → reaped key status
  handoff : ∀ key slot, Event.handedOff key slot ∈ events → transferred key slot

theorem normal_return_accounted (ops : Operations I C L S E T) (identity : C → K)
    (history : I → List (Event K L S E T)) (unknown : E)
    (calls : NormalCalls ops identity history unknown) (child : C) (slot : L) (io : I)
    (outcome : Outcome C L S E I)
    (returned : child_process.observation_core.stop ops (some child) (some slot) io = .ok outcome) :
    Accounted identity history unknown (identity child) slot outcome := by
  exact WP.dspec_imp_forall (stop_accounted ops identity history unknown calls child slot io)
    outcome returned

theorem success_has_exact_observation (identity : C → K)
    (history : I → List (Event K L S E T)) (unknown : E) (key : K) (slot : L)
    (status : S) (child : Option C) (owner : Option L) (io : I)
    (safe : Accounted identity history unknown key slot (.Ok status, child, owner, io)) :
    Event.poll key (.Ok (some status)) ∈ history io ∧ owner = some slot := by
  rcases safe with ⟨_, _, _, sameSlot, tail⟩
  exact ⟨tail_member tail (by simp), sameSlot⟩

theorem unknown_has_original_handoff (identity : C → K)
    (history : I → List (Event K L S E T)) (unknown : E) (key : K) (slot : L)
    (error : E) (child : Option C) (owner : Option L) (io : I)
    (safe : Accounted identity history unknown key slot (.Err error, child, owner, io)) :
    Event.handedOff key slot ∈ history io ∧ child = none ∧ owner = none ∧ error = unknown := by
  rcases safe with ⟨sameChild, sameOwner, sameError, tail⟩
  exact ⟨tail_member tail (by simp), sameChild, sameOwner, sameError⟩

theorem success_under_primitive_meaning (identity : C → K)
    (history : I → List (Event K L S E T)) (unknown : E) (key : K) (slot : L)
    (status : S) (child : Option C) (owner : Option L) (io : I)
    (reaped : K → S → Prop) (transferred : K → L → Prop)
    (meaning : Meaning (history io) reaped transferred)
    (safe : Accounted identity history unknown key slot (.Ok status, child, owner, io)) :
    reaped key status := by
  exact meaning.poll key status
    (success_has_exact_observation identity history unknown key slot status child owner io safe).1

theorem unknown_under_primitive_meaning (identity : C → K)
    (history : I → List (Event K L S E T)) (unknown : E) (key : K) (slot : L)
    (error : E) (child : Option C) (owner : Option L) (io : I)
    (reaped : K → S → Prop) (transferred : K → L → Prop)
    (meaning : Meaning (history io) reaped transferred)
    (safe : Accounted identity history unknown key slot (.Err error, child, owner, io)) :
    transferred key slot := by
  exact meaning.handoff key slot
    (unknown_has_original_handoff identity history unknown key slot error child owner io safe).1

/-- A kill acknowledgment alone does not contain any exit observation. -/
theorem kill_ack_is_not_poll (key : K) (status : S) :
    Event.poll (L := L) (E := E) (T := T) key (.Ok (some status)) ∉
      [Event.kill key (.Ok ())] := by simp

/-- Pipe-field cleanup is not process-exit evidence. -/
theorem pipe_cleanup_is_not_poll (key : K) (status : S) :
    Event.poll (L := L) (E := E) (T := T) key (.Ok (some status)) ∉
      [Event.closeStdin key, Event.closeStdout key, Event.closeStderr key] := by simp

end SandboxModel.Boundaries.ChildObservation.Guarantees
