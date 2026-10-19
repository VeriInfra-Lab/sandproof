import SandboxModel.Boundaries.OwnedMessage.Ownership

/-! A complete ledger of strong references to ONE Arc<OwnedSemaphorePermit>.
This is not a claim that repeated strong_count observations form a snapshot.
The original allocation is counted once regardless of reference clones.
Arc<Shared> is only a retention reference: application code can explicitly
remove/forget its inner record permit. It is NOT an instance of this ledger
without a separate invariant that the inner permit remains owned.
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
  returns : Nat
  deriving DecidableEq, Repr

def LeaseSafe (s : SharedLease) : Prop :=
  s.returns ≤ 1 ∧ (s.references = 0 ↔ s.returns = 1)

def charge (s : SharedLease) : Nat := if s.returns = 0 then s.key.units else 0

inductive RefStep : SharedLease → SharedLease → Prop
  | clone (k n) : RefStep ⟨k, n + 1, 0⟩ ⟨k, n + 2, 0⟩
  | dropOther (k n) : RefStep ⟨k, n + 2, 0⟩ ⟨k, n + 1, 0⟩
  | dropLast (k) : RefStep ⟨k, 1, 0⟩ ⟨k, 0, 1⟩

inductive RefHistory : SharedLease → SharedLease → Prop
  | refl (s) : RefHistory s s
  | next : RefHistory s t → RefStep t u → RefHistory s u

theorem initial_lease_safe (k : LeaseKey) : LeaseSafe ⟨k, 1, 0⟩ := by
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
    s.returns = 0 := by
  by_cases returned : s.returns = 0
  · exact returned
  · have one : s.returns = 1 := by have bound := safe.1; omega
    have zero := safe.2.mpr one
    omega

theorem live_reference_keeps_charge {s} (safe : LeaseSafe s) (live : 0 < s.references) :
    charge s = s.key.units := by
  simp [charge, live_reference_prevents_return safe live]

theorem clone_does_not_allocate_again (k : LeaseKey) (n : Nat) :
    charge ⟨k, n + 2, 0⟩ = charge ⟨k, n + 1, 0⟩ := rfl

theorem nonlast_drop_does_not_refund (k : LeaseKey) (n : Nat) :
    charge ⟨k, n + 1, 0⟩ = k.units ∧ RefStep ⟨k, n + 2, 0⟩ ⟨k, n + 1, 0⟩ :=
  ⟨rfl, .dropOther ..⟩

theorem last_normal_drop_refunds_once (k : LeaseKey) :
    RefStep ⟨k, 1, 0⟩ ⟨k, 0, 1⟩ ∧ charge ⟨k, 0, 1⟩ = 0 := by
  exact ⟨.dropLast k, rfl⟩

theorem no_refund_then_clone {s t} (done : s.returns = 1) : ¬ RefStep s t := by
  intro step
  cases step <;> simp_all

theorem early_shared_refund_breaks_same_property (k : LeaseKey) (n : Nat) :
    ¬ LeaseSafe ⟨k, n + 1, 1⟩ := by
  simp [LeaseSafe]

end SandboxModel.Boundaries.OwnedMessage
