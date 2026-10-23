import Std

namespace SandboxModel.Boundaries.TokioRwLockContract

/- A6/B2 projection of completed operations on one original lock. Tickets
   identify logical acquisitions, not Rust addresses or queue positions.
   Raw holdings and usable guards are separate: forgetting a guard keeps its
   exclusion responsibility. Queue/poll internals are deliberately hidden;
   pending/cancel only stutter this projection, not the whole runtime state.
   Payload means guarded inline fields/handles, not aliased mutable heaps. -/
structure Guard where
  lock : Nat
  ticket : Nat
  deriving DecidableEq, Repr

structure State (α : Type) where
  lock : Nat
  payload : α
  readers : Nat → Bool
  liveReaders : Nat → Bool
  writer : Option Guard
  writerActive : Bool
  issued : Nat

def put (f : Nat → Bool) (key : Nat) (value : Bool) : Nat → Bool :=
  fun i => if i = key then value else f i

def initial (lock : Nat) (value : α) : State α :=
  ⟨lock, value, fun _ => false, fun _ => false, none, false, 0⟩

def fresh (s : State α) : Guard := ⟨s.lock, s.issued⟩
def OwnsRead (s : State α) (g : Guard) : Prop :=
  g.lock = s.lock ∧ s.liveReaders g.ticket = true
def OwnsWrite (s : State α) (g : Guard) : Prop :=
  s.writer = some g ∧ s.writerActive = true

def Safe (s : State α) : Prop :=
  (∀ i, s.readers i = true → i < s.issued) ∧
  (∀ i, s.liveReaders i = true → s.readers i = true) ∧
  (∀ g, s.writer = some g → g.lock = s.lock ∧ g.ticket < s.issued ∧
    ∀ i, s.readers i = false) ∧
  (s.writerActive = true → s.writer.isSome = true)

def readAcquired (s : State α) : State α :=
  { s with
    readers := put s.readers s.issued true
    liveReaders := put s.liveReaders s.issued true
    issued := s.issued + 1 }

def writeAcquired (s : State α) : State α :=
  { s with writer := some (fresh s), writerActive := true, issued := s.issued + 1 }

def readReleased (s : State α) (g : Guard) : State α :=
  { s with
    readers := put s.readers g.ticket false
    liveReaders := put s.liveReaders g.ticket false }

def readForgotten (s : State α) (g : Guard) : State α :=
  { s with liveReaders := put s.liveReaders g.ticket false }

def writeReleased (s : State α) : State α :=
  { s with writer := none, writerActive := false }

def writeForgotten (s : State α) : State α := { s with writerActive := false }

/- Completed read returns may be delayed by queued writers or max readers.
   writer=none is a necessary safety condition, never a sufficiency/progress
   assertion. This over-approximation does not claim FIFO or reader capacity. -/
inductive Step (writes : α → α → Prop) : State α → State α → Prop
  | readReturned (s) (noWriter : s.writer = none) : Step writes s (readAcquired s)
  | writeReturned (s) (noWriter : s.writer = none) (noReaders : ∀ i, s.readers i = false) :
      Step writes s (writeAcquired s)
  | readerDrop (s g) (owned : OwnsRead s g) : Step writes s (readReleased s g)
  | readerForget (s g) (owned : OwnsRead s g) : Step writes s (readForgotten s g)
  | writerDrop (s g) (owned : OwnsWrite s g) : Step writes s (writeReleased s)
  | writerForget (s g) (owned : OwnsWrite s g) : Step writes s (writeForgotten s)
  | writeValue (s g value) (owned : OwnsWrite s g) (body : writes s.payload value) :
      Step writes s { s with payload := value }
  | pendingOrCancelled (s) : Step writes s s

inductive History (writes : α → α → Prop) : State α → State α → Prop
  | refl (s) : History writes s s
  | next : History writes s t → Step writes t u → History writes s u

theorem initial_safe (lock : Nat) (value : α) : Safe (initial lock value) := by
  simp [Safe, initial]

theorem read_acquisition_safe {s : State α} (safe : Safe s) (free : s.writer = none) :
    Safe (readAcquired s) := by
  refine ⟨?_, ?_, ?_, ?_⟩
  · intro i held
    by_cases h : i = s.issued
    · simp [readAcquired, h]
    · have prior : s.readers i = true := by simpa [readAcquired, put, h] using held
      have := safe.1 i prior
      simp only [readAcquired]
      omega
  · intro i live
    by_cases h : i = s.issued
    · simp [readAcquired, put, h]
    · have prior : s.liveReaders i = true := by simpa [readAcquired, put, h] using live
      simpa [readAcquired, put, h] using safe.2.1 i prior
  · simp [readAcquired, free]
  · exact safe.2.2.2

theorem write_acquisition_safe {s : State α} (safe : Safe s)
    (empty : ∀ i, s.readers i = false) : Safe (writeAcquired s) := by
  refine ⟨?_, safe.2.1, ?_, ?_⟩
  · intro i held
    have := safe.1 i held
    simp only [writeAcquired]
    omega
  · intro g same
    have : fresh s = g := Option.some.inj same
    subst g
    simp [writeAcquired, fresh, empty]
  · simp [writeAcquired]

theorem read_release_safe {s : State α} (safe : Safe s) (g : Guard) :
    Safe (readReleased s g) := by
  refine ⟨?_, ?_, ?_, safe.2.2.2⟩
  · intro i held
    by_cases h : i = g.ticket
    · simp [readReleased, put, h] at held
    · exact safe.1 i (by simpa [readReleased, put, h] using held)
  · intro i live
    by_cases h : i = g.ticket
    · simp [readReleased, put, h] at live
    · simpa [readReleased, put, h] using
        safe.2.1 i (by simpa [readReleased, put, h] using live)
  · intro w same
    rcases safe.2.2.1 w same with ⟨identity, issued, empty⟩
    refine ⟨identity, issued, ?_⟩
    intro i
    simp [readReleased, put, empty]

theorem read_forget_safe {s : State α} (safe : Safe s) (g : Guard) :
    Safe (readForgotten s g) := by
  refine ⟨safe.1, ?_, safe.2.2.1, safe.2.2.2⟩
  intro i live
  by_cases h : i = g.ticket
  · simp [readForgotten, put, h] at live
  · exact safe.2.1 i (by simpa [readForgotten, put, h] using live)

theorem write_release_safe {s : State α} (safe : Safe s) : Safe (writeReleased s) := by
  exact ⟨safe.1, safe.2.1, by simp [writeReleased], by simp [writeReleased]⟩

theorem write_forget_safe {s : State α} (safe : Safe s) : Safe (writeForgotten s) := by
  exact ⟨safe.1, safe.2.1, safe.2.2.1, by simp [writeForgotten]⟩

theorem step_safe {s t : State α} (step : Step writes s t) (safe : Safe s) : Safe t := by
  cases step with
  | readReturned free => exact read_acquisition_safe safe free
  | writeReturned _ empty => exact write_acquisition_safe safe empty
  | readerDrop g _ => exact read_release_safe safe g
  | readerForget g _ => exact read_forget_safe safe g
  | writerDrop _ _ => exact write_release_safe safe
  | writerForget _ _ => exact write_forget_safe safe
  | writeValue _ _ _ _ => exact safe
  | pendingOrCancelled => exact safe

theorem history_safe {s t : State α} (history : History writes s t) (safe : Safe s) : Safe t := by
  induction history with
  | refl => exact safe
  | next _ step ih => exact step_safe step ih

theorem read_return_owns_original (s : State α) : OwnsRead (readAcquired s) (fresh s) := by
  simp [OwnsRead, readAcquired, fresh, put]

theorem write_return_owns_original (s : State α) : OwnsWrite (writeAcquired s) (fresh s) := by
  simp [OwnsWrite, writeAcquired]

theorem read_write_exclusive {s : State α} (safe : Safe s)
    (read : OwnsRead s r) (write : OwnsWrite s w) : False := by
  have held := safe.2.1 r.ticket read.2
  have empty := (safe.2.2.1 w write.1).2.2 r.ticket
  simp_all

theorem two_live_writers_are_same {s : State α}
    (left : OwnsWrite s g) (right : OwnsWrite s h) : g = h :=
  Option.some.inj (left.1.symm.trans right.1)

theorem reader_blocks_write_return {s : State α} (safe : Safe s) (read : OwnsRead s g) :
    ¬ (∀ i, s.readers i = false) := by
  intro empty
  have held := safe.2.1 g.ticket read.2
  simp_all

theorem released_reader_cannot_access (s : State α) (g : Guard) :
    ¬ OwnsRead (readReleased s g) g := by simp [OwnsRead, readReleased, put]

theorem forgotten_reader_cannot_access (s : State α) (g : Guard) :
    ¬ OwnsRead (readForgotten s g) g := by simp [OwnsRead, readForgotten, put]

theorem forgotten_reader_still_blocks_write {s : State α} (safe : Safe s) (held : OwnsRead s g) :
    ¬ (∀ i, (readForgotten s g).readers i = false) := by
  change ¬ (∀ i, s.readers i = false)
  exact reader_blocks_write_return safe held

theorem forgotten_writer_still_blocks_read (s : State α) (g : Guard) (held : OwnsWrite s g) :
    (writeForgotten s).writer ≠ none := by simp [writeForgotten, held.1]

theorem release_keeps_original_payload (s : State α) (g : Guard) :
    (readReleased s g).payload = s.payload ∧ (writeReleased s).payload = s.payload := ⟨rfl, rfl⟩

theorem step_keeps_original_lock {s t : State α} (step : Step writes s t) : t.lock = s.lock := by
  cases step <;> rfl

theorem step_preserves_payload_invariant (invariant : α → Prop) {s t : State α}
    (step : Step writes s t)
    (body : ∀ old next, writes old next → invariant old → invariant next)
    (prior : invariant s.payload) : invariant t.payload := by
  cases step with
  | writeValue _ _ _ valid => exact body _ _ valid prior
  | _ => exact prior

theorem history_preserves_payload_invariant (invariant : α → Prop) {s t : State α}
    (history : History writes s t)
    (body : ∀ old next, writes old next → invariant old → invariant next)
    (prior : invariant s.payload) : invariant t.payload := by
  induction history with
  | refl => exact prior
  | next _ step ih => exact step_preserves_payload_invariant invariant step body ih

theorem step_keeps_payload_while_reader {s t : State α} (step : Step writes s t)
    (safe : Safe s) (read : OwnsRead s g) : t.payload = s.payload := by
  cases step with
  | writeValue _ _ owned _ => exact False.elim (read_write_exclusive safe read owned)
  | _ => rfl

/- The caller retains the original live guard at every boundary. This is a
   lifetime obligation, not an assumption that the protected value is stable. -/
inductive ReadHistory (writes : α → α → Prop) (g : Guard) : State α → State α → Prop
  | refl (s) (held : OwnsRead s g) : ReadHistory writes g s s
  | next : ReadHistory writes g s t → Step writes t u → OwnsRead u g → ReadHistory writes g s u

theorem read_history_owns_last {s t : State α} (history : ReadHistory writes g s t) : OwnsRead t g := by
  cases history with
  | refl held => exact held
  | next _ _ held => exact held

theorem read_history_is_history {s t : State α} (history : ReadHistory writes g s t) : History writes s t := by
  induction history with
  | refl => exact .refl _
  | next _ step _ ih => exact .next ih step

theorem retained_reader_keeps_payload {s t : State α}
    (history : ReadHistory writes g s t) (safe : Safe s) : t.payload = s.payload := by
  induction history with
  | refl => rfl
  | next prior step _ ih =>
      exact (step_keeps_payload_while_reader step
        (history_safe (read_history_is_history prior) safe) (read_history_owns_last prior)).trans ih

end SandboxModel.Boundaries.TokioRwLockContract
