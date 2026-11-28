import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def pausedRecord (record : model.SandboxRecord) (sandbox : ids.SandboxId)
    (source : model.RestoreSource) : model.SandboxRecord :=
  { record with
    phase := .Paused
    current_vm := none
    durable_snapshot := some source
    restore_from := none
    net_slot := none
    desired := .Paused sandbox source
    effective := .Paused sandbox source
    current_operation := record.current_operation.map
      (fun operation => {operation with status := .Succeeded}) }


theorem update_result_existing (world : model.World) (sandbox : ids.SandboxId)
    (operation : model.Operation) (result : model.OperationResult)
    (ledger : model.OperationLedgerRecord)
    (present : tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val = some ledger) :
    ∃ operations,
      publication.update_operation_result world sandbox operation result =
        .ok {world with operations} ∧
      operations.entries.val = tablePut ⟨sandbox, operation.id⟩
        {ledger with operation, result := some result} world.operations.entries.val := by
  obtain ⟨operations, putOperations, post⟩ := put_exact
    model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operations ⟨sandbox, operation.id⟩ {ledger with operation, result := some result}
    (Or.inl (by rw [present]; simp))
  refine ⟨operations, ?_, post⟩
  simp [publication.update_operation_result, transition.operation_key,
    get_exact _ operation_key_lawful, present, putOperations]

theorem publish_paused_existing (world : model.World) (sandbox : ids.SandboxId)
    (record : model.SandboxRecord) (source : model.RestoreSource)
    (operation : model.Operation) (ledger : model.OperationLedgerRecord)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (current : record.current_operation = some operation)
    (storedLedger : tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val = some ledger) :
    ∃ sandboxes operations,
      publication.publish_paused world sandbox record source =
        .ok {world with sandboxes, operations} ∧
      sandboxes.entries.val = tablePut sandbox (pausedRecord record sandbox source)
        world.sandboxes.entries.val ∧
      operations.entries.val = tablePut ⟨sandbox, operation.id⟩
        {ledger with operation := {operation with status := .Succeeded}, result := some (.Paused source)}
        world.operations.entries.val := by
  obtain ⟨sandboxes, putSandboxes, rows⟩ := put_exact
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId caller_index_sandbox_eq_lawful
    world.sandboxes sandbox (pausedRecord record sandbox source)
    (Or.inl (by rw [stored]; simp))
  let base := {world with sandboxes}
  obtain ⟨operations, updated, results⟩ := update_result_existing base sandbox
    {operation with status := .Succeeded} (.Paused source) ledger storedLedger
  refine ⟨sandboxes, operations, ?_, rows, results⟩
  simp only [publication.publish_paused, model_World_clone_exact,
    model_SandboxRecord_clone_exact, bind_tc_ok, current]
  simp only [model_Authority_clone_exact, bind_tc_ok]
  have rowEq : ({record with
      phase := model.Phase.Paused
      current_vm := none
      durable_snapshot := some source
      restore_from := none
      net_slot := none
      desired := model.Authority.Paused sandbox source
      effective := model.Authority.Paused sandbox source
      current_operation := some {operation with status := model.OperationStatus.Succeeded}}
      : model.SandboxRecord) = pausedRecord record sandbox source := by
    simp [pausedRecord, current]

  rw [rowEq, putSandboxes]
  simpa [base] using updated

end SandboxModel.Lifecycle.PausePublication
