import Std

/-!
A single owned message, rather than an interpreter for Rust/Tokio. Primitive
moves retain the original payload and lease identity. An accepted send may be
received or discarded before its caller observes success. Normal destruction
is separate from receiver closure, cancellation of a wait and interruption.
The actual async wiring must supply this history under the registered A6/B3/B4
conditions; these theorems do not establish that Rust-to-model refinement.
-/
namespace SandboxModel.Boundaries.OwnedMessage

structure Key where
  message : Nat
  bytePool : Nat
  bytePermit : Nat
  subscriber : Nat
  record : Nat
  deriving DecidableEq, Repr

structure Message (α : Type) where
  key : Key
  payload : α
  units : Nat
  deriving Repr

inductive Place | local | offered | channel | received | frame | dropping | released | interrupted
  deriving DecidableEq, Repr

structure Ownership (α : Type) where
  message : Message α
  place : Place
  byteHeld : Bool
  handed : Bool
  waiter : Bool
  acknowledged : Bool
  drops : Nat
  deriving Repr

def ownsBytes (p : Place) : Prop :=
  p = .local ∨ p = .offered ∨ p = .channel ∨ p = .received ∨ p = .frame ∨
    p = .dropping ∨ p = .interrupted

def Safe (s : Ownership α) : Prop :=
  s.drops ≤ 1 ∧ (ownsBytes s.place → s.byteHeld = true ∧ s.drops = 0) ∧
    (s.place = .released → s.byteHeld = false ∧ s.drops = 1) ∧
    (s.acknowledged = true → s.handed = true)

def initial (m : Message α) : Ownership α := ⟨m, .local, true, false, true, false, 0⟩

inductive Move : Ownership α → Ownership α → Prop
  | offer (m w a) : Move ⟨m, .local, true, false, w, a, 0⟩ ⟨m, .offered, true, false, w, a, 0⟩
  | accepted (m w a) : Move ⟨m, .offered, true, false, w, a, 0⟩ ⟨m, .channel, true, true, w, a, 0⟩
  | rejected (m w a) : Move ⟨m, .offered, true, false, w, a, 0⟩ ⟨m, .local, true, false, w, a, 0⟩
  | received (m w a) : Move ⟨m, .channel, true, true, w, a, 0⟩ ⟨m, .received, true, true, w, a, 0⟩
  | framed (m w a) : Move ⟨m, .received, true, true, w, a, 0⟩ ⟨m, .frame, true, true, w, a, 0⟩
  -- Closure or a cancelled pending receive does not destroy a queued value.
  | stopWaiting (m p b h a d) : Move ⟨m, p, b, h, true, a, d⟩ ⟨m, p, b, h, false, a, d⟩
  -- Only a successful original offer may acknowledge. This flag intentionally
  -- says nothing about the holder at observation time (including released).
  | observeSuccess (m p b w d)
      (accepted : p = .channel ∨ p = .received ∨ p = .frame ∨ p = .dropping ∨ p = .released) :
      Move ⟨m, p, b, true, w, false, d⟩ ⟨m, p, b, true, w, true, d⟩
  | beginDrop (m p h w a) (owned : ownsBytes p) :
      Move ⟨m, p, true, h, w, a, 0⟩ ⟨m, .dropping, true, h, w, a, 0⟩
  | normalDrop (m h w a) :
      Move ⟨m, .dropping, true, h, w, a, 0⟩ ⟨m, .released, false, h, w, a, 1⟩
  -- No invented destructor or refund on panic/abort/crash. Some normal drops
  -- may already have happened; those remain separately recorded in history.
  | interrupt (m p h w a) (owned : ownsBytes p) :
      Move ⟨m, p, true, h, w, a, 0⟩ ⟨m, .interrupted, true, h, false, a, 0⟩

inductive History : Ownership α → Ownership α → Prop
  | refl (s) : History s s
  | next : History s t → Move t u → History s u

theorem initial_safe (m : Message α) : Safe (initial m) := by
  simp [Safe, initial, ownsBytes]

theorem move_safe {s t : Ownership α} (move : Move s t) (safe : Safe s) : Safe t := by
  cases move <;> simp_all [Safe, ownsBytes]

theorem history_safe {s t : Ownership α} (history : History s t) (safe : Safe s) : Safe t := by
  induction history with
  | refl => exact safe
  | next _ move ih => exact move_safe move ih

theorem move_preserves_original {s t : Ownership α} (move : Move s t) :
    t.message = s.message := by
  cases move <;> rfl

theorem history_preserves_original {s t : Ownership α} (history : History s t) :
    t.message = s.message := by
  induction history with
  | refl => rfl
  | next _ move ih => exact (move_preserves_original move).trans ih

theorem held_output_keeps_bytes {s : Ownership α} (safe : Safe s)
    (held : s.place = .received ∨ s.place = .frame) : s.byteHeld = true :=
  (safe.2.1 (by rcases held with h | h <;> simp [ownsBytes, h])).1

theorem success_requires_original_handoff {s t : Ownership α} (history : History s t)
    (safe : Safe s) (success : t.acknowledged = true) : t.handed = true :=
  (history_safe history safe).2.2.2 success

theorem normal_refund_at_most_once {s t : Ownership α} (history : History s t)
    (safe : Safe s) : t.drops ≤ 1 := (history_safe history safe).1

theorem cancel_wait_keeps_message (m : Message α) (p b h a d) :
    Move ⟨m, p, b, h, true, a, d⟩ ⟨m, p, b, h, false, a, d⟩ := .stopWaiting ..

theorem rejected_send_returns_original (m : Message α) (w a) :
    Move ⟨m, .offered, true, false, w, a, 0⟩ ⟨m, .local, true, false, w, a, 0⟩ := .rejected ..

theorem interrupted_owner_has_no_refund (m : Message α) (w a) :
    Move ⟨m, .frame, true, true, w, a, 0⟩ ⟨m, .interrupted, true, true, false, a, 0⟩ :=
  .interrupt _ _ _ _ _ (by simp [ownsBytes])

theorem success_can_be_observed_after_drop (m : Message α) :
    Move ⟨m, .released, false, true, true, false, 1⟩ ⟨m, .released, false, true, true, true, 1⟩ :=
  .observeSuccess _ _ _ _ _ (by simp)

theorem correct_delivery_reaches_held_frame (m : Message α) :
    History (initial m) ⟨m, .frame, true, true, true, false, 0⟩ := by
  have offered : History (initial m) ⟨m, .offered, true, false, true, false, 0⟩ :=
    .next (.refl _) (.offer m true false)
  have accepted := History.next offered (Move.accepted m true false)
  have received := History.next accepted (Move.received m true false)
  exact History.next received (Move.framed m true false)

theorem early_refund_breaks_same_property (m : Message α) (w a) :
    ¬ Safe ⟨m, .frame, false, true, w, a, 1⟩ := by
  simp [Safe, ownsBytes]

end SandboxModel.Boundaries.OwnedMessage
