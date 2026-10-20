import SandboxModel.Boundaries.OwnedMessage.Mailbox

/-! The normal data-bearing Frame destructor has observable stages. This does not prove
that dropping a body/receiver stops a command. Notice means the send attempt returned (not that a peer received it). It occurs after the encoded
buffer and original output have been destroyed, but before automatic field
destruction of the retained lease; other Arc owners can also still exist.
No actual atomic strong_count snapshot or prompt wake-up is assumed.
-/
namespace SandboxModel.Boundaries.OwnedMessage

structure FrameDrop where
  encoded : Bool
  output : Bool
  notice : Bool
  leaseRefs : Nat
  deriving DecidableEq, Repr

def DropSafe (s : FrameDrop) : Prop := s.notice = true → s.encoded = false ∧ s.output = false

inductive DropStep : FrameDrop → FrameDrop → Prop
  | encoded (n) : DropStep ⟨true, true, false, n⟩ ⟨false, true, false, n⟩
  | output (n) : DropStep ⟨false, true, false, n⟩ ⟨false, false, false, n⟩
  | notify (n) : DropStep ⟨false, false, false, n⟩ ⟨false, false, true, n⟩
  | fieldDrop (n) : DropStep ⟨false, false, true, n + 1⟩ ⟨false, false, true, n⟩

theorem frame_initial_safe (n : Nat) : DropSafe ⟨true, true, false, n⟩ := by simp [DropSafe]

theorem frame_drop_step_safe {s t} (step : DropStep s t) : DropSafe t := by
  cases step <;> simp [DropSafe]

theorem frame_notice_requires_original_output_drop {s} (safe : DropSafe s) (notice : s.notice = true) :
    s.encoded = false ∧ s.output = false := safe notice

theorem frame_notice_can_precede_lease_drop :
    DropSafe ⟨false, false, true, 1⟩ ∧ DropStep ⟨false, false, false, 1⟩ ⟨false, false, true, 1⟩ := by
  exact ⟨by simp [DropSafe], .notify 1⟩

theorem early_frame_notice_breaks_same_property (n : Nat) :
    ¬ DropSafe ⟨false, true, true, n⟩ := by simp [DropSafe]

end SandboxModel.Boundaries.OwnedMessage
