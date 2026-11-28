import SandboxModel.Lifecycle.PausePublication.Checks
import SandboxModel.Lifecycle.PausePublication.Post
import SandboxModel.Lifecycle.PausePublication.Effects

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

/-- Input-domain conditions, not an assumption that the Rust entry succeeds.
Acknowledgement fields refer to exact complete keys and their current attempt;
indexed closure says nothing about resources absent from the index. -/
structure Ready (world : model.World) (sandbox : ids.SandboxId) (source : model.RestoreSource) where
  record : model.SandboxRecord
  snapshot : model.Snapshot
  vm : ids.VmId
  operation : model.Operation
  ledger : model.OperationLedgerRecord
  plan : model.OperationPlan
  handoff : capture.CaptureHandoff
  up : world.controller_up = true
  stored : tableLookup sandbox world.sandboxes.entries.val = some record
  phase : record.phase = .Pausing
  terminal : record.terminal = false
  snapshotStored : tableLookup source.snapshot world.snapshots.entries.val = some snapshot
  owner : snapshot.owner = record.owner
  session : snapshot.sandbox = sandbox
  generation : snapshot.source_generation = record.generation
  workspace : snapshot.workspace = record.workspace
  manifest : snapshot.manifest = source.manifest
  stage : snapshot.stage = .Durable
  immutable : snapshot.immutable = true
  fence : tableLookup sandbox world.worker_fences.entries.val = some ⟨record.epoch, false⟩
  currentVm : record.current_vm = some vm
  fenceSucceeded : effectValue world (.InstallFence sandbox record.epoch false) = true
  freezeSucceeded : effectValue world (.FreezeVm sandbox record.epoch record.generation vm) = true
  sealSucceeded : effectValue world
    (.SealSnapshot sandbox record.epoch record.generation vm record.workspace source.snapshot source.manifest) = true
  commitSucceeded : effectValue world
    (.CommitSnapshot sandbox record.epoch record.generation source.snapshot source.manifest) = true
  stopSucceeded : effectValue world (.StopVm sandbox record.epoch record.generation vm) = true
  vmsClosed : (caller_index_view world sandbox).vms.val.all (vmClosed world) = true
  slotsClosed : (caller_index_view world sandbox).slots.val.all (slotClosed world) = true
  currentOp : record.current_operation = some operation
  ledgerStored : tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val = some ledger
  ledgerSandbox : ledger.sandbox = sandbox
  ledgerOperation : ledger.operation = operation
  noResult : ledger.result = none
  planStored : tableLookup ⟨sandbox, operation.id⟩ world.operation_plans.entries.val = some plan
  target : plan.target_snapshot = some source
  planVm : plan.vm = some vm
  operationKind : operation.kind = .Pause
  operationEpoch : operation.epoch = record.epoch
  planKey : plan.key = ⟨sandbox, operation.id⟩
  planEpoch : plan.epoch = record.epoch
  planGeneration : plan.generation = record.generation
  planCapture : plan.capture = some handoff
  registered : handoff.commit = .Registered
  recoverable : handoff.eligibility = .Recoverable
  sourcePresent : handoff.unload_source = true

theorem normal_inputs_accept (world : model.World) (sandbox : ids.SandboxId)
    (source : model.RestoreSource) (ready : Ready world sandbox source) :
    ∃ decision, publication.handle_publish_paused world sandbox source = .ok (.Ok decision) := by
  rcases ready with ⟨record, snapshot, vm, operation, ledger, plan, handoff, up, stored,
    phase, terminal, snapshotStored, owner, session, generation, workspace, manifest,
    stage, immutable, fence, currentVm, fenceSucceeded, freezeSucceeded, sealSucceeded,
    commitSucceeded, stopSucceeded, vmsClosed, slotsClosed, currentOp, ledgerStored,
    ledgerSandbox, ledgerOperation, noResult, planStored, target, planVm, operationKind,
    operationEpoch, planKey, planEpoch, planGeneration, planCapture, registered, recoverable,
    sourcePresent⟩
  obtain ⟨sandboxes, operations, published, _rows, _results⟩ :=
    publish_paused_existing world sandbox record source operation ledger stored currentOp ledgerStored
  have phaseCheck : core.cmp.PartialEq.ne.trait_default model.Phase.Insts.CoreCmpPartialEqPhase
      record.phase .Pausing = .ok false := by
    simp [phase, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
  have ownerCheck : core.cmp.PartialEq.ne.trait_default ids.TenantId.Insts.CoreCmpPartialEqTenantId
      snapshot.owner record.owner = .ok false := by
    simp [owner, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq]
  have sessionCheck : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      snapshot.sandbox sandbox = .ok false := by
    simp [session, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
  have generationCheck : (snapshot.source_generation != record.generation) = false := by simp [generation]
  have workspaceCheck : core.cmp.PartialEq.ne.trait_default ids.FsId.Insts.CoreCmpPartialEqFsId
      snapshot.workspace record.workspace = .ok false := by
    simp [workspace, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      ids.FsId.Insts.CoreCmpPartialEqFsId.eq]
  have manifestCheck : core.cmp.PartialEq.ne.trait_default model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef
      snapshot.manifest source.manifest = .ok false := by
    simp [manifest, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
  have stageCheck : core.cmp.PartialEq.ne.trait_default model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage
      snapshot.stage .Durable = .ok false := by
    simp [stage, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
  have fenceCheck : core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption
        (core.cmp.PartialEqShared model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence))
      (tableLookup sandbox world.worker_fences.entries.val) (some ⟨record.epoch, false⟩) = .ok false := by
    simp [fence, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
  have fenceRun := effect_exact world (.InstallFence sandbox record.epoch false)
    (.fence sandbox record.epoch false)
  have freezeRun := effect_exact world (.FreezeVm sandbox record.epoch record.generation vm)
    (.freeze sandbox record.epoch record.generation vm)
  have sealRun := effect_exact world
    (.SealSnapshot sandbox record.epoch record.generation vm record.workspace source.snapshot source.manifest)
    (.seal sandbox record.epoch record.generation vm record.workspace source.snapshot source.manifest)
  have commitRun := effect_exact world
    (.CommitSnapshot sandbox record.epoch record.generation source.snapshot source.manifest)
    (.commit sandbox record.epoch record.generation source.snapshot source.manifest)
  have stopRun := effect_exact world (.StopVm sandbox record.epoch record.generation vm)
    (.stop sandbox record.epoch record.generation vm)
  simp only [fenceSucceeded] at fenceRun
  simp only [freezeSucceeded] at freezeRun
  simp only [sealSucceeded] at sealRun
  simp only [commitSucceeded] at commitRun
  simp only [stopSucceeded] at stopRun
  have vmsRun := indexed_vms_closed_exact world sandbox
  have slotsRun := indexed_slots_closed_exact world sandbox
  simp only [vmsClosed] at vmsRun
  simp only [slotsClosed] at slotsRun
  have ledgerSessionCheck : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      ledger.sandbox sandbox = .ok false := by
    simp [ledgerSandbox, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
  have ledgerOperationCheck : core.cmp.PartialEq.ne.trait_default model.Operation.Insts.CoreCmpPartialEqOperation
      ledger.operation operation = .ok false := by
    simp [ledgerOperation, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
  have noResultCheck : core.option.Option.is_some ledger.result = false := by
    simp [noResult, core.option.Option.is_some]
  have targetCheck : core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource)
      plan.target_snapshot (some source) = .ok false := by
    simp [target, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
  have planVmCheck : core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption ids.VmId.Insts.CoreCmpPartialEqVmId)
      plan.vm (some vm) = .ok false := by
    simp [planVm, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
  have captured : currentCaptureView world sandbox record = some handoff := by
    simp [currentCaptureView, currentOp, operationKind, operationEpoch,
      planStored, planKey, planEpoch, planGeneration, planCapture]
  have mayStop : capture.CaptureHandoff.may_stop_original handoff = .ok true :=
    (stop_requires_complete_registration handoff).2 ⟨registered, recoverable, sourcePresent⟩
  refine ⟨⟨.PublishPaused sandbox source.snapshot, {world with sandboxes, operations}⟩, ?_⟩
  simp only [publication.handle_publish_paused, up, if_true,
    get_exact _ caller_index_sandbox_eq_lawful, get_exact _ snapshot_id_lawful,
    get_exact _ operation_key_lawful, bind_tc_ok, stored, phaseCheck, Bool.false_eq_true,
    if_false, terminal, snapshotStored, ownerCheck, sessionCheck, generationCheck,
    workspaceCheck, manifestCheck, stageCheck, immutable, fenceCheck, currentVm,
    fenceRun, freezeRun, sealRun, commitRun, stopRun, vmsRun, slotsRun, currentOp,
    transition.operation_key, ledgerStored, ledgerSessionCheck, ledgerOperationCheck,
    noResultCheck, planStored, targetCheck, planVmCheck, current_capture_exact, captured,
    SandproofFoundationsRust.core.option.Option.is_some_and,
    P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once, mayStop, published]

end SandboxModel.Lifecycle.PausePublication
