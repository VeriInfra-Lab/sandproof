import SandboxModel.Boundaries.Condvar.Protocol

namespace SandboxModel.Boundaries.CondvarContract.Waiting

/- Finite legal histories establish non-vacuity and reject stronger guarantees.
   These are conditional model witnesses, not Rust implementation proofs. -/

def exampleUnlocked : StdMutexContract.State Bool := StdMutexContract.initial 7 false

def exampleLease : StdMutexContract.Guard := StdMutexContract.freshGuard exampleUnlocked false

def exampleHolding : State Bool := initial 3 11 (StdMutexContract.acquiredState exampleUnlocked false) exampleLease

def exampleWaiting : State Bool := beginWait exampleHolding exampleLease false

def exampleReady : State Bool := signal exampleWaiting

def exampleReturned : State Bool := finishWait exampleReady (waiterFor exampleHolding exampleLease) false

theorem example_start_safe : Safe exampleHolding := by
  simp [Safe, exampleHolding, initial, exampleLease, exampleUnlocked, StdMutexContract.Safe,
    StdMutexContract.Owns, StdMutexContract.freshGuard, StdMutexContract.acquiredState, StdMutexContract.initial]

theorem spurious_return_is_a_legal_history :
    History (fun _ _ : Bool => True) exampleHolding exampleReturned := by
  apply History.next
  · apply History.next
    · exact History.next (.refl _) (.begin exampleHolding exampleLease false rfl (by constructor <;> rfl))
    · exact .spurious exampleWaiting (waiterFor exampleHolding exampleLease) false rfl
  · exact .returned exampleReady (waiterFor exampleHolding exampleLease) false false rfl (by constructor <;> rfl)

theorem spurious_return_does_not_prove_business_predicate :
    Safe exampleReturned ∧ exampleReturned.mutex.payload = false := by
  exact ⟨history_safe spurious_return_is_a_legal_history example_start_safe, rfl⟩

def exampleTimedWaiting : State Bool := beginWait exampleHolding exampleLease true

def examplePeerHolding : State Bool :=
  { exampleTimedWaiting with mutex := StdMutexContract.acquiredState exampleTimedWaiting.mutex false }

def exampleTimedOutButLocked : State Bool :=
  { examplePeerHolding with phase := .ready (waiterFor exampleHolding exampleLease) true true }

theorem elapsed_wait_need_not_have_reacquired_mutex :
    History (fun _ _ : Bool => True) exampleHolding exampleTimedOutButLocked ∧
    ¬ MayReturn exampleTimedOutButLocked.mutex (waiterFor exampleHolding exampleLease) := by
  constructor
  · apply History.next
    · apply History.next
      · exact History.next (.refl _) (.begin exampleHolding exampleLease true rfl (by constructor <;> rfl))
      · exact .peer exampleTimedWaiting _ (by trivial)
          (.lockReturned exampleTimedWaiting.mutex false rfl)
    · exact .elapsed examplePeerHolding (waiterFor exampleHolding exampleLease) rfl
  · simp [MayReturn, exampleTimedOutButLocked, examplePeerHolding, StdMutexContract.acquiredState]

end SandboxModel.Boundaries.CondvarContract.Waiting
