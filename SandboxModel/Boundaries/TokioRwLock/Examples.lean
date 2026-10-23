import SandboxModel.Boundaries.TokioRwLock.Mapping

namespace SandboxModel.Boundaries.TokioRwLockContract.Examples

/- Nonempty legal examples and the difference between retained observations
   and an unlocked copy; no production authentication statement is assumed. -/
def start : State Nat := initial 5 7
def firstGuard : Guard := fresh start
def oneReader : State Nat := readAcquired start
def secondGuard : Guard := fresh oneReader
def twoReaders : State Nat := readAcquired oneReader

theorem one_reader_safe : Safe oneReader := read_acquisition_safe (initial_safe 5 7) rfl

theorem two_readers_are_legal : History (fun _ _ : Nat => True) start twoReaders :=
  .next (.next (.refl _) (.readReturned start rfl)) (.readReturned oneReader rfl)

theorem two_live_readers_coexist : OwnsRead twoReaders firstGuard ∧ OwnsRead twoReaders secondGuard := by
  simp [OwnsRead, twoReaders, oneReader, firstGuard, secondGuard, readAcquired, fresh, start, initial, put]

theorem drop_one_reader_keeps_the_other_and_blocks_writer :
    OwnsRead (readReleased twoReaders firstGuard) secondGuard ∧
    ¬ (∀ i, (readReleased twoReaders firstGuard).readers i = false) := by
  have safe := history_safe two_readers_are_legal (initial_safe 5 7)
  have held : OwnsRead (readReleased twoReaders firstGuard) secondGuard := by
    simp [OwnsRead, readReleased, twoReaders, oneReader, firstGuard, secondGuard, readAcquired, fresh, start, initial, put]
  exact ⟨held, reader_blocks_write_return (read_release_safe safe firstGuard) held⟩

def released : State Nat := readReleased oneReader firstGuard
def writerGuard : Guard := fresh released
def writing : State Nat := writeAcquired released
def changed : State Nat := { writing with payload := 8 }

theorem copy_then_unlock_allows_a_real_model_write :
    History (fun _ _ : Nat => True) oneReader changed ∧ changed.payload ≠ oneReader.payload := by
  constructor
  · apply History.next
    · apply History.next
      · exact History.next (.refl _) (.readerDrop oneReader firstGuard (read_return_owns_original start))
      · exact .writeReturned released rfl (by simp [released, readReleased, oneReader, readAcquired, firstGuard, fresh, start, initial, put])
    · exact .writeValue writing writerGuard 8 (write_return_owns_original released) trivial
  · decide

theorem completed_writer_is_not_a_retained_snapshot :
    ¬ ProtectedRead oneReader.payload changed firstGuard := by
  intro claim
  exact copy_then_unlock_allows_a_real_model_write.2 claim.2

end SandboxModel.Boundaries.TokioRwLockContract.Examples
