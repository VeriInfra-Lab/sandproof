import SandboxModel.Boundaries.StdMutex.Guard
import SandboxModel.Boundaries.StdMutex.GetMut

namespace SandboxModel.Boundaries.StdMutexContract.Bridge
open Aeneas Aeneas.Std
open SandboxModel.Boundaries.StdMutexContract
open SandboxModel.Boundaries.StdMutexContract.Borrow

/- The exclusive Rust loan is a separate caller/borrow-checker condition.
Inactive alone does not grant unique access. No shared mutable heap snapshot,
resource destruction or whole business callback correctness is assumed here. -/
def project (s : State α) : Cell α :=
  ⟨s.mutex, s.payload, s.poisoned, s.holder.isSome⟩

def Represents (s : State α) (v : Cell α) : Prop :=
  v.identity = s.mutex ∧ v.payload = s.payload ∧
    v.poisoned = s.poisoned ∧ v.rawLocked = s.holder.isSome

theorem original_state_projection (s : State α) : Represents s (project s) := by
  simp [Represents, project]

theorem back_view_respects_same_borrow_arm (s t : Cell α) (updated : View α)
    (frame : BackView s t updated) (compatible : SameArm s updated) :
    StdMutexExternal.view t = updated := by
  obtain ⟨identity, poison, held, payload⟩ := frame
  cases h : s.poisoned <;> cases updated <;>
    simp_all [SameArm, UpdatedPayload, StdMutexExternal.view]

theorem original_get_mut_back_obeys_guard_model {writes : α -> α -> Prop}
    {unique : State α -> Prop} (s : State α) (updated : View α)
    (inactive : s.active = false) (loan : unique s)
    (compatible : SameArm (project s) updated)
    (body : writes s.payload (UpdatedPayload updated)) :
    ∃ original back,
      SandproofStdMutexRust.application.diagnostics.SerialMutex.get_mut (project s) =
        .ok (original, back) ∧
      BorrowView (project s) original ∧
      Step writes unique s { s with payload := (back updated).payload } ∧
      Represents { s with payload := (back updated).payload } (back updated) ∧
      StdMutexExternal.view (back updated) = updated := by
  obtain ⟨original, back, returned, view, frame⟩ :=
    original_serial_get_mut_contract (project s)
  refine ⟨original, back, returned, view, ?_, ?_, ?_⟩
  · have payload := (frame updated).2.2.2
    rw [payload]
    exact .uniqueWrite s (UpdatedPayload updated) inactive loan body
  · obtain ⟨identity, poison, held, payload⟩ := frame updated
    exact ⟨identity, rfl, poison, held⟩
  · exact back_view_respects_same_borrow_arm (project s) (back updated) updated
      (frame updated) compatible

theorem restored_value_retains_same_guard_owner (s : State α) (returned : Cell α) :
    ({ s with payload := returned.payload } : State α).holder = s.holder ∧
    ({ s with payload := returned.payload } : State α).active = s.active ∧
    ({ s with payload := returned.payload } : State α).issued = s.issued := by
  exact ⟨rfl, rfl, rfl⟩
end SandboxModel.Boundaries.StdMutexContract.Bridge
