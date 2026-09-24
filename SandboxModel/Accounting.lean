import SandboxModel.Types

/-!
An accounting layer: a live store plus a published index/cache over it.

This formalizes the class of failure reported as runtime#3566: a sweep that
removes a sandbox from the store without removing it from the index leaves a
**stale reference**. `Consistent` says the index and store agree; the paired
operations preserve it, the store-only sweep breaks it, and a heal restores it.
-/

namespace SandboxModel.Accounting

/-- Live sandboxes and the published index over them. -/
structure State where
  store : SandboxId → Prop
  index : SandboxId → Prop

/-- The index references exactly the live sandboxes. -/
def Consistent (s : State) : Prop := ∀ sid, s.index sid ↔ s.store sid

/-- Insert a sandbox into both store and index together. -/
def insert (s : State) (sid : SandboxId) : State :=
  { store := fun x => s.store x ∨ x = sid,
    index := fun x => s.index x ∨ x = sid }

/-- Correct removal: drop from both. -/
def removeBoth (s : State) (sid : SandboxId) : State :=
  { store := fun x => s.store x ∧ x ≠ sid,
    index := fun x => s.index x ∧ x ≠ sid }

/-- The buggy sweep: drop from the store only, leaving the index entry. -/
def sweepStoreOnly (s : State) (sid : SandboxId) : State :=
  { store := fun x => s.store x ∧ x ≠ sid, index := s.index }

/-- Heal the index by rebuilding it from the store. -/
def healIndex (s : State) : State :=
  { s with index := s.store }

theorem consistent_insert {s : State} (h : Consistent s) (sid : SandboxId) :
    Consistent (insert s sid) := by
  intro x
  dsimp [insert]
  rw [h x]

theorem consistent_removeBoth {s : State} (h : Consistent s) (sid : SandboxId) :
    Consistent (removeBoth s sid) := by
  intro x
  dsimp [removeBoth]
  rw [h x]

theorem consistent_healIndex (s : State) : Consistent (healIndex s) := by
  intro x
  exact Iff.rfl

/-- The store-only sweep breaks consistency: the removed sandbox stays in the
    index, i.e. it becomes a stale reference. -/
theorem sweepStoreOnly_is_stale {s : State} (h : Consistent s)
    {sid : SandboxId} (live : s.store sid) :
    ¬ Consistent (sweepStoreOnly s sid) := by
  intro hc
  have inIndex : (sweepStoreOnly s sid).index sid := by
    dsimp [sweepStoreOnly]
    exact (h sid).mpr live
  have inStore : (sweepStoreOnly s sid).store sid := (hc sid).mp inIndex
  dsimp [sweepStoreOnly] at inStore
  exact inStore.2 rfl

/-- After the store-only sweep, `sid` is absent from the store but still named
    by the index. -/
theorem sweepStoreOnly_leaves_stale {s : State} (h : Consistent s)
    {sid : SandboxId} (live : s.store sid) :
    ¬ (sweepStoreOnly s sid).store sid ∧ (sweepStoreOnly s sid).index sid := by
  constructor
  · intro lost; exact lost.2 rfl
  · dsimp [sweepStoreOnly]; exact (h sid).mpr live

end SandboxModel.Accounting