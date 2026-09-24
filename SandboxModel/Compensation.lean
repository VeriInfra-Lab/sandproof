import SandboxModel.Proofs

/-!
Failure compensation as part of the authoritative lifecycle relation.

Earlier versions of this project defined a standalone `abortCreate` function.
That function was not a `Step`, was not reachable through `Reachable`, and did
not preserve `CurrentOperationValid`. Compensation now uses the ordinary
terminal protocol:

1. `beginCompensation` retains the create/resume/pause operation identity, records a
   structured failure, closes authority, and commits a terminal fence.
2. Normal stop/cleanup steps close resources.
3. `publishFailure` is admitted only after every VM is stopped/reclaimed and
   every network slot is inactive; it finalizes the operation as failed.

Consequently the general safety theorems cover compensation without a second
state machine or an atomic-delete escape hatch.
-/

namespace SandboxModel.Compensation

theorem beginCompensation_closes_authority
    (failure : FailureKind)
    (transition : Step world (.beginCompensation sid failure) next) :
    ∃ record operation,
      next.sandboxes sid = some record ∧
      record.phase = .revoking ∧
      record.terminal = true ∧
      record.desired = .closed ∧
      record.effective = .closed ∧
      record.currentOperation = some operation ∧
      operation.status = .compensating failure := by
  cases transition
  simp [SandboxModel.beginCompensation, setSandbox, put]

theorem publishFailure_requires_closed_resources
    (failure : FailureKind)
    (transition : Step world (.publishFailure sid failure) next) :
    allVmsStoppedOrReclaimed world sid ∧ noActiveSlot world sid := by
  cases transition
  exact ⟨by assumption, by assumption⟩

theorem publishFailure_finalizes_operation
    (failure : FailureKind)
    (transition : Step world (.publishFailure sid failure) next) :
    ∃ record operation ledger,
      next.sandboxes sid = some record ∧
      record.phase = .destroyed ∧
      record.terminal = true ∧
      record.currentOperation = some operation ∧
      operation.status = .failed failure ∧
      next.operations (operationKey sid operation.id) = some ledger ∧
      ledger.result = some (.failed failure) := by
  cases transition
  simp [SandboxModel.publishFailure, updateCurrentOperation, setSandbox, put, *]

/-- Compensation is covered by the same bundled one-step security contract as
    every other protocol step. -/
theorem compensation_step_secure
    (transition : Step world event next) :
    ProtocolStepSecurity world event next :=
  step_protocol_secure transition

end SandboxModel.Compensation
