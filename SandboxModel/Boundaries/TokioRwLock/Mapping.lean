import SandboxModel.Boundaries.TokioRwLock.Core

namespace SandboxModel.Boundaries.TokioRwLockContract

/- Only a normally returning pure Option::as_ref-style shared projection.
   Guard equality denotes the same lease, not two independently owned Rust
   values: try_map consumes its argument. A6/B2 must bind that linear move.
   Values are observations, not copied Rust resources. No arbitrary stateful
   closure, custom Drop, interior-mutability, panic or unwinding contract. -/
inductive MapReply (β : Type)
  | mapped (guard : Guard) (view : β)
  | unchanged (guard : Guard)

def mappedGuard : MapReply β → Guard
  | .mapped guard _ | .unchanged guard => guard

def tryMap (s : State α) (g : Guard) (project : α → Option β) : State α × MapReply β :=
  (s, match project s.payload with
      | some view => .mapped g view
      | none => .unchanged g)

def ProtectedRead (original : α) (s : State α) (g : Guard) : Prop :=
  OwnsRead s g ∧ s.payload = original

theorem mapped_guard_is_original (s : State α) (g : Guard) (project : α → Option β) :
    mappedGuard (tryMap s g project).2 = g := by
  cases h : project s.payload <;> simp [tryMap, h, mappedGuard]

theorem map_keeps_original_state (s : State α) (g : Guard) (project : α → Option β) :
    (tryMap s g project).1 = s := rfl

theorem map_failure_returns_original_guard {s : State α} {g : Guard} {project : α → Option β}
    (absent : project s.payload = none) : (tryMap s g project).2 = .unchanged g := by
  simp [tryMap, absent]

theorem map_success_returns_original_view {s : State α} {g : Guard} {project : α → Option β}
    (present : project s.payload = some view) : (tryMap s g project).2 = .mapped g view := by
  simp [tryMap, present]

theorem mapped_reply_still_owns_read {s : State α} (held : OwnsRead s g) (project : α → Option β) :
    OwnsRead (tryMap s g project).1 (mappedGuard (tryMap s g project).2) := by
  rw [map_keeps_original_state, mapped_guard_is_original]
  exact held

theorem mapped_reply_blocks_writer {s : State α} (safe : Safe s) (held : OwnsRead s g)
    (project : α → Option β) : ¬ (∀ i, (tryMap s g project).1.readers i = false) := by
  change ¬ (∀ i, s.readers i = false)
  exact reader_blocks_write_return safe held

theorem dropping_mapped_reply_invalidates_original (s : State α) (g : Guard) (project : α → Option β) :
    ¬ OwnsRead (readReleased (tryMap s g project).1 (mappedGuard (tryMap s g project).2)) g := by
  rw [map_keeps_original_state, mapped_guard_is_original]
  exact released_reader_cannot_access s g

theorem retained_mapped_snapshot {s t : State α} (history : ReadHistory writes g s t)
    (safe : Safe s) : ProtectedRead s.payload t g :=
  ⟨read_history_owns_last history, retained_reader_keeps_payload history safe⟩

theorem mapped_reply_keeps_same_protected_read {s : State α} (held : OwnsRead s g)
    (project : α → Option β) :
    ProtectedRead s.payload (tryMap s g project).1 (mappedGuard (tryMap s g project).2) := by
  exact ⟨mapped_reply_still_owns_read held project, rfl⟩

/- An intentionally wrong ownership shortcut: copied fields are returned but
   the original lease is released before their user finishes. Same contract
   as above rejects it. This is a model counterexample, not Rust extraction. -/
def earlyReleaseReply (s : State α) (g : Guard) (project : α → Option β) : State α × MapReply β :=
  (readReleased s g, (tryMap s g project).2)

theorem early_release_rejected_by_same_protected_read (s : State α) (g : Guard)
    (project : α → Option β) :
    ¬ ProtectedRead s.payload (earlyReleaseReply s g project).1
        (mappedGuard (earlyReleaseReply s g project).2) := by
  simp only [earlyReleaseReply, mapped_guard_is_original, ProtectedRead]
  intro wrong
  exact released_reader_cannot_access s g wrong.1

end SandboxModel.Boundaries.TokioRwLockContract
