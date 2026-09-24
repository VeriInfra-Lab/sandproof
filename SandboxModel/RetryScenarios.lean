import SandboxModel.Controller

/-! Executable checks for exact-attempt retry semantics. -/

namespace SandboxModel.RetryScenarios

open SandboxModel
open SandboxModel.Controller

def sid : SandboxId := ⟨901⟩
def vmId : VmId := ⟨902⟩
def result : ResultDigest := ⟨903⟩

def command : Command := .cleanupVm sid 1 vmId
def logical : LogicalEffectId := logicalEffectId command
def attempt0 : EffectId := effectIdAt command 0
def attempt1 : EffectId := effectIdAt command 1

def failedAttempt0 : World :=
  acknowledgeEffect
    (dispatchEffect (commitEffect emptyWorld command) attempt0)
    attempt0 (.failed .resourceFailure)

def retried : World := startNextAttempt failedAttempt0 logical .resourceFailure

def accepted {ε α : Type} : Except ε α → Bool
  | .ok _ => true
  | .error _ => false

example : accepted (handleEffect failedAttempt0
    (.retry logical .resourceFailure)) = true := by native_decide

example : retried.effects attempt1 = some .committed := by native_decide

example : retried.inflight attempt1 := by
  change attempt1 = attempt1 ∨ False
  exact Or.inl rfl

example : retried.logicalEffects logical = some {
    origin := .maintenance sid
    currentAttempt := 1
    remainingRetries := 2
    result := none } := by native_decide

/-- A conflicting late receipt from attempt zero cannot overwrite the current
    attempt or be accepted as a replay. -/
example : accepted (handleEffect retried
    (.receipt attempt0 (.succeeded result))) = false := by native_decide

def timedOut : World :=
  markEffectOutcomeUnknown
    (dispatchEffect (commitEffect emptyWorld command) attempt0) attempt0

example : (timedOut.logicalEffects logical).map
    (fun record => record.currentAttempt) = some 0 := by native_decide

example : timedOut.effects attempt1 = none := by native_decide

def terminalFailedAttempt : World :=
  acknowledgeEffect
    (dispatchEffect (commitEffect emptyWorld command) attempt0)
    attempt0 (.failed .policyFailure)

def terminalFinalized : World :=
  finalizeLogicalFailure terminalFailedAttempt logical .policyFailure

example : accepted (handleEffect terminalFailedAttempt
    (.finalizeFailure logical .policyFailure)) = true := by native_decide

example : (terminalFinalized.logicalEffects logical).bind
    (fun record => record.result) =
      some (.failed 0 .policyFailure) := by native_decide

end SandboxModel.RetryScenarios
