import SandboxModel.Lifecycle.CaptureHandoff.CallerCurrent
import SandboxModel.Lifecycle.CaptureHandoff.CallerCloning
import SandboxModel.Lifecycle.CaptureHandoff.CallerValues

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- A field-level result specification, not a replacement Rust algorithm. -/
def failedCapture (capture : sandbox_controller_core.capture.CaptureHandoff) :
    sandbox_controller_core.capture.CaptureHandoff :=
  match capture.commit with
  | .Registered => capture
  | .NotRegistered | .Unknown => { capture with eligibility := .Revoked }

@[simp] theorem ordinary_failure_exact (capture : sandbox_controller_core.capture.CaptureHandoff) :
    sandbox_controller_core.capture.CaptureHandoff.ordinary_failure capture =
      .ok (failedCapture capture) := by
  cases commit : capture.commit <;>
    simp [failedCapture, sandbox_controller_core.capture.CaptureHandoff.ordinary_failure,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      sandbox_controller_core.capture.CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      sandbox_controller_core.capture.CaptureCommit.read_discriminant, commit]

theorem failed_capture_responsibilities (capture : sandbox_controller_core.capture.CaptureHandoff) :
    SameResponsibilities capture (failedCapture capture) ∧
      (revoked capture → revoked (failedCapture capture)) ∧
      (capture.commit = .Unknown → (failedCapture capture).commit = .Unknown) := by
  cases commit : capture.commit <;> simp [failedCapture, SameResponsibilities, revoked, commit]

/-- Equality fixes every World field except the three explicitly written tables. -/
def CompensationFrame (before after : sandbox_controller_core.model.World) : Prop :=
  after = { before with sandboxes := after.sandboxes, operations := after.operations, operation_plans := after.operation_plans }

def compensatedRecord (record : sandbox_controller_core.model.SandboxRecord)
    (operation : sandbox_controller_core.model.Operation)
    (failure : sandbox_controller_core.model.FailureKind) : sandbox_controller_core.model.SandboxRecord :=
  { record with desired := .Closed, current_operation := some { operation with status := .Compensating failure } }

def compensatedLedger (ledger : sandbox_controller_core.model.OperationLedgerRecord)
    (operation : sandbox_controller_core.model.Operation)
    (failure : sandbox_controller_core.model.FailureKind) : sandbox_controller_core.model.OperationLedgerRecord :=
  { ledger with operation := { operation with status := .Compensating failure } }

def compensatedPlan (plan : sandbox_controller_core.model.OperationPlan)
    (capture : sandbox_controller_core.capture.CaptureHandoff) : sandbox_controller_core.model.OperationPlan :=
  { plan with capture := some (failedCapture capture) }

theorem sandbox_id_lawful :
    TableLawfulEq sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId := by
  intro left right
  rfl

/-- Exact writes of the actual Pause branch. Existing records guarantee all
three writes replace a row; no spare capacity or allocation success is assumed. -/
theorem begin_pause_compensation_exact
    (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId)
    (record : sandbox_controller_core.model.SandboxRecord)
    (operation : sandbox_controller_core.model.Operation)
    (failure : sandbox_controller_core.model.FailureKind)
    (ledger : sandbox_controller_core.model.OperationLedgerRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (kind : operation.kind = .Pause)
    (storedRecord : tableLookup sandbox world.sandboxes.entries.val = some record)
    (storedLedger : tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val = some ledger)
    (storedPlan : tableLookup ⟨sandbox, operation.id⟩ world.operation_plans.entries.val = some plan)
    (hasCapture : plan.capture = some capture) :
    ∃ next, sandbox_controller_core.transition.begin_compensation
      world sandbox record operation failure = .ok next ∧
      next.sandboxes.entries.val = tablePut sandbox (compensatedRecord record operation failure)
        world.sandboxes.entries.val ∧
      next.operations.entries.val = tablePut ⟨sandbox, operation.id⟩
        (compensatedLedger ledger operation failure) world.operations.entries.val ∧
      next.operation_plans.entries.val = tablePut ⟨sandbox, operation.id⟩
        (compensatedPlan plan capture) world.operation_plans.entries.val ∧
      CompensationFrame world next := by
  obtain ⟨sandboxes, putSandboxes, sandboxesView⟩ := put_exact
    sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandbox_id_lawful
    world.sandboxes sandbox (compensatedRecord record operation failure)
    (Or.inl (by rw [storedRecord]; simp))
  obtain ⟨operations, putOperations, operationsView⟩ := put_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operations ⟨sandbox, operation.id⟩ (compensatedLedger ledger operation failure)
    (Or.inl (by rw [storedLedger]; simp))
  obtain ⟨plans, putPlans, plansView⟩ := put_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operation_plans ⟨sandbox, operation.id⟩ (compensatedPlan plan capture)
    (Or.inl (by rw [storedPlan]; simp))
  have getLedger := get_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operations ⟨sandbox, operation.id⟩
  have getPlan := get_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operation_plans ⟨sandbox, operation.id⟩
  refine ⟨{ world with sandboxes := sandboxes, operations := operations, operation_plans := plans },
    ?_, sandboxesView, operationsView, plansView, rfl⟩
  simp only [compensatedRecord, kind] at putSandboxes
  simp only [compensatedLedger, kind] at putOperations
  simp only [compensatedPlan] at putPlans
  simp [sandbox_controller_core.transition.begin_compensation, kind,
    sandbox_controller_core.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
    sandbox_controller_core.model.OperationKind.read_discriminant,
    sandbox_controller_core.transition.operation_key,
    getLedger, storedLedger, getPlan, storedPlan, hasCapture,
    putSandboxes, putOperations, putPlans]

/-- Connection from the actual public handler for exactly the 2×2 ordinary
cases: Cancelled/DeadlineExceeded and Pending/OutcomeUnknown Pause. Evidence is
None; both ledger and current-plan checks remain actual code. The generated
handler's reconstructed operation and record are connected back to the same
original values, without adding a terminal-field update or a commit premise. -/
theorem compensation_ordinary_caller_connects
    (world : sandbox_controller_core.model.World)
    (failure : sandbox_controller_core.model.FailureKind)
    (ordinary : failure = .Cancelled ∨ failure = .DeadlineExceeded)
    (sandbox : sandbox_controller_core.ids.SandboxId)
    (record : sandbox_controller_core.model.SandboxRecord)
    (operation : sandbox_controller_core.model.Operation)
    (ledger : sandbox_controller_core.model.OperationLedgerRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (up : world.controller_up = true) (phase : record.phase = .Pausing)
    (nonterminal : record.terminal = false)
    (current : record.current_operation = some operation)
    (kind : operation.kind = .Pause) (status : operation.status = .Pending ∨ operation.status = .OutcomeUnknown)
    (storedRecord : tableLookup sandbox world.sandboxes.entries.val = some record)
    (storedLedger : tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val = some ledger)
    (ledgerSandbox : ledger.sandbox = sandbox) (ledgerOperation : ledger.operation = operation)
    (noResult : ledger.result = none)
    (storedPlan : tableLookup ⟨sandbox, operation.id⟩ world.operation_plans.entries.val = some plan)
    (planKey : plan.key = ⟨sandbox, operation.id⟩)
    (epoch : operation.epoch = record.epoch) (planEpoch : plan.epoch = record.epoch)
    (generation : plan.generation = record.generation) (hasCapture : plan.capture = some capture) :
    sandbox_controller_core.request.handle_compensation world ⟨sandbox, failure, none⟩ =
      (do
        let next ← sandbox_controller_core.transition.begin_compensation world sandbox record operation failure
        .ok (.Ok ({ event := .BeginCompensation sandbox failure, next := next } :
          sandbox_controller_core.request.Decision))) := by
  rcases status with statusEq | statusEq
  all_goals
    have getRecord := get_exact
      sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandbox_id_lawful
      world.sandboxes sandbox
    have getLedger := get_exact
      sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
      world.operations ⟨sandbox, operation.id⟩
    have currentCapture : sandbox_controller_core.transition.current_capture world sandbox record =
        .ok (some capture) := by
      rw [current_capture_exact]
      simp [currentCaptureView, current, kind, epoch, storedPlan, planKey, planEpoch, generation, hasCapture]
    have operationNormalized : { operation with kind := .Pause, status := operation.status } = operation := by
      cases operation
      simp_all
    simp only [statusEq] at operationNormalized
    have recordNormalized :
        { record with phase := .Pausing, current_operation := some operation } = record := by
      cases record
      simp_all
    -- Match the simplified constructor's known field; this is value
    -- normalization, not an additional write in the Rust transition.
    simp only [nonterminal] at recordNormalized
    have operationEquals : sandbox_controller_core.model.Operation.Insts.CoreCmpPartialEqOperation.eq
        operation operation = .ok true := by
      simp [sandbox_controller_core.model.Operation.Insts.CoreCmpPartialEqOperation.eq,
        sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
        kind, statusEq, sandbox_controller_core.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
        sandbox_controller_core.model.OperationKind.read_discriminant,
        sandbox_controller_core.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq,
        sandbox_controller_core.model.OperationStatus.read_discriminant]
    have justified : sandbox_controller_core.request.supplied_compensation_evidence
        world ⟨sandbox, operation.id⟩ operation.epoch failure none = .ok true := by
      rcases ordinary with rfl | rfl <;> rfl
    simp [sandbox_controller_core.request.handle_compensation, up, getRecord, storedRecord,
      phase, nonterminal, current, kind, statusEq, sandbox_controller_core.transition.operation_key,
      getLedger, storedLedger, ledgerSandbox, ledgerOperation, noResult,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      operationNormalized, recordNormalized, operationEquals, currentCapture,
      sandbox_controller_core.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
      sandbox_controller_core.model.OperationKind.read_discriminant,
      justified]

end SandboxModel.Lifecycle.CaptureHandoffCaller
