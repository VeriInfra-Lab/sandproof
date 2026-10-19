import SandboxModel.Boundaries.OwnedMessage.Ownership

/-! A ledger for ONE shared resource allocation. Strong references can become
zero before the inner destructor returns the permit, so a last-drop-in-progress
phase remains charged. This is not an atomic snapshot of strong_count and the
pool. Arc<Shared> only retains that object: a separate invariant is needed that
its mutable inner record still contains a refundable permit. No eventual Drop
completion on unwind/abort/crash or scheduling fairness is asserted.
-/
namespace SandboxModel.Boundaries.OwnedMessage

structure LeaseKey where
  object : Nat
  pool : Nat
  permit : Nat
  units : Nat
  deriving DecidableEq, Repr

structure SharedLease where
  key : LeaseKey
  references : Nat
  dropping : Bool
  returns : Nat
  deriving DecidableEq, Repr

def LeaseSafe (s : SharedLease) : Prop :=
  s.returns ≤ 1 ∧
  (0 < s.references → s.dropping = false ∧ s.returns = 0) ∧
  (s.dropping = true → s.references = 0 ∧ s.returns = 0) ∧
  (s.returns = 1 → s.references = 0 ∧ s.dropping = false) ∧
  (s.references = 0 → s.dropping = true ∨ s.returns = 1)

def charge (s : SharedLease) : Nat := if s.returns = 0 then s.key.units else 0

inductive RefStep : SharedLease → SharedLease → Prop
  | clone (k n) : RefStep ⟨k, n + 1, false, 0⟩ ⟨k, n + 2, false, 0⟩
  | dropOther (k n) : RefStep ⟨k, n + 2, false, 0⟩ ⟨k, n + 1, false, 0⟩
  -- Completed normal drop may be observed as a whole, or split to expose
  -- interleavings between the last reference decrement and resource return.
  | dropLast (k) : RefStep ⟨k, 1, false, 0⟩ ⟨k, 0, false, 1⟩
  | beginLastDrop (k) : RefStep ⟨k, 1, false, 0⟩ ⟨k, 0, true, 0⟩
  | returnLastPermit (k) : RefStep ⟨k, 0, true, 0⟩ ⟨k, 0, false, 1⟩

inductive RefHistory : SharedLease → SharedLease → Prop
  | refl (s) : RefHistory s s
  | next : RefHistory s t → RefStep t u → RefHistory s u

theorem initial_lease_safe (k : LeaseKey) : LeaseSafe ⟨k, 1, false, 0⟩ := by
  simp [LeaseSafe]

theorem ref_step_safe {s t} (step : RefStep s t) : LeaseSafe t := by
  cases step <;> simp [LeaseSafe]

theorem ref_history_safe {s t} (history : RefHistory s t) (safe : LeaseSafe s) : LeaseSafe t := by
  cases history with
  | refl => exact safe
  | next _ step => exact ref_step_safe step

theorem original_lease_key {s t} (step : RefStep s t) : t.key = s.key := by
  cases step <;> rfl

theorem live_reference_prevents_return {s} (safe : LeaseSafe s) (live : 0 < s.references) :
    s.returns = 0 := (safe.2.1 live).2

theorem live_reference_keeps_charge {s} (safe : LeaseSafe s) (live : 0 < s.references) :
    charge s = s.key.units := by
  simp [charge, live_reference_prevents_return safe live]

theorem clone_does_not_allocate_again (k : LeaseKey) (n : Nat) :
    charge ⟨k, n + 2, false, 0⟩ = charge ⟨k, n + 1, false, 0⟩ := rfl

theorem nonlast_drop_does_not_refund (k : LeaseKey) (n : Nat) :
    charge ⟨k, n + 1, false, 0⟩ = k.units ∧
      RefStep ⟨k, n + 2, false, 0⟩ ⟨k, n + 1, false, 0⟩ := ⟨rfl, .dropOther ..⟩

theorem last_normal_drop_refunds_once (k : LeaseKey) :
    RefStep ⟨k, 1, false, 0⟩ ⟨k, 0, false, 1⟩ ∧ charge ⟨k, 0, false, 1⟩ = 0 := by
  exact ⟨.dropLast k, rfl⟩

theorem last_drop_starts_without_credit (k : LeaseKey) :
    RefStep ⟨k, 1, false, 0⟩ ⟨k, 0, true, 0⟩ ∧ charge ⟨k, 0, true, 0⟩ = k.units :=
  ⟨.beginLastDrop k, rfl⟩

theorem zero_references_can_still_be_charged (k : LeaseKey) :
    LeaseSafe ⟨k, 0, true, 0⟩ ∧ charge ⟨k, 0, true, 0⟩ = k.units := by
  exact ⟨by simp [LeaseSafe], rfl⟩

theorem pending_last_drop_keeps_charge {s} (safe : LeaseSafe s) (pending : s.dropping = true) :
    charge s = s.key.units := by
  simp [charge, (safe.2.2.1 pending).2]

theorem no_refund_then_clone {s t} (done : s.returns = 1) : ¬ RefStep s t := by
  intro step
  cases step <;> simp_all

theorem early_shared_refund_breaks_same_property (k : LeaseKey) (n : Nat) :
    ¬ LeaseSafe ⟨k, n + 1, false, 1⟩ := by
  simp [LeaseSafe]

end SandboxModel.Boundaries.OwnedMessage
