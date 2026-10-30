import SandboxModel.Lifecycle.PausePublication.Checks
import SandboxModel.Lifecycle.PausePublication.Post

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def PublicationFrame (before after : model.World) : Prop :=
  after = {before with sandboxes := after.sandboxes, operations := after.operations}

/-- Exact writes and complete World frame for the actual successful entry.
Both writes replace rows whose existence is derived from that same execution. -/
theorem publish_success_frame (world : model.World) (sandbox : ids.SandboxId)
    (source : model.RestoreSource) (decision : publication.PublicationDecision)
    (evaluated : publication.handle_publish_paused world sandbox source = .ok (.Ok decision)) :
    ∃ record operation ledger,
      tableLookup sandbox world.sandboxes.entries.val = some record ∧
      record.current_operation = some operation ∧
      tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val = some ledger ∧
      decision.event = .PublishPaused sandbox source.snapshot ∧
      PublicationFrame world decision.next ∧
      decision.next.sandboxes.entries.val = tablePut sandbox (pausedRecord record sandbox source)
        world.sandboxes.entries.val ∧
      decision.next.operations.entries.val = tablePut ⟨sandbox, operation.id⟩
        {ledger with operation := {operation with status := .Succeeded}, result := some (.Paused source)}
        world.operations.entries.val := by
  obtain ⟨record, _snapshot, _vm, _up, stored, _phase, _terminal, _snapshotStored,
    _owner, _session, _generation, _workspace, _manifest, _stage, _immutable,
    _fence, _currentVm, _fenceRun, _freezeRun, _sealRun, _commitRun, _stopRun,
    operation, ledger, _plan, _handoff, currentOp, ledgerStored, _ledgerSandbox,
    _ledgerOperation, _noResult, _planStored, _target, _planVm, _captured,
    _kind, _operationEpoch, _planKey, _planEpoch, _planGeneration, _planCapture,
    _registered, _recoverable, _sourcePresent, _vmsRun, _slotsRun, published, event⟩ :=
    publish_success_checks world sandbox source decision evaluated
  obtain ⟨sandboxes, operations, updated, rows, results⟩ :=
    publish_paused_existing world sandbox record source operation ledger stored currentOp ledgerStored
  rw [published] at updated
  have shape : decision.next = {world with sandboxes, operations} := by
    simpa only [Result.ok.injEq] using updated
  refine ⟨record, operation, ledger, stored, currentOp, ledgerStored, event, ?_, ?_, ?_⟩
  · simp [PublicationFrame, shape]
  · simpa only [shape] using rows
  · simpa only [shape] using results

end SandboxModel.Lifecycle.PausePublication
