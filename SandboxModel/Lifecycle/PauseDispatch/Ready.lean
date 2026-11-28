import SandboxModel.Lifecycle.PauseDispatch.ReceiptPair
import SandboxModel.Lifecycle.PausePublication.Acceptance

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

noncomputable section
open SandboxModel.Lifecycle.PausePublication
local instance readyDecidable1 : DecidableEq model.LogicalEffectId := Classical.decEq _
local instance readyDecidable2 : DecidableEq model.EffectId := Classical.decEq _

/-- Exactly the existing publication inputs, leaving the two receipt checks to actual executions below. -/
structure ReadyRest (world : model.World) (sandbox : ids.SandboxId) (source : model.RestoreSource) where
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

def ready_rest_frame (world next : model.World) (sandbox : ids.SandboxId) (source : model.RestoreSource)
    (rest : ReadyRest world sandbox source)
    (frame : next = {world with logical_effects := next.logical_effects, effects := next.effects, effect_keys := next.effect_keys})
    (fence : effectValue next (.InstallFence sandbox rest.record.epoch false) = true)
    (freeze : effectValue next (.FreezeVm sandbox rest.record.epoch rest.record.generation rest.vm) = true)
    (sealedCheck : effectValue next (.SealSnapshot sandbox rest.record.epoch rest.record.generation rest.vm rest.record.workspace source.snapshot source.manifest) = true) :
    ReadyRest next sandbox source := by
  have controller_upSame : next.controller_up = world.controller_up := by
    have projected := congrArg model.World.controller_up frame
    exact projected
  have sandboxesSame : next.sandboxes = world.sandboxes := by
    have projected := congrArg model.World.sandboxes frame
    exact projected
  have snapshotsSame : next.snapshots = world.snapshots := by
    have projected := congrArg model.World.snapshots frame
    exact projected
  have worker_fencesSame : next.worker_fences = world.worker_fences := by
    have projected := congrArg model.World.worker_fences frame
    exact projected
  have operationsSame : next.operations = world.operations := by
    have projected := congrArg model.World.operations frame
    exact projected
  have operation_plansSame : next.operation_plans = world.operation_plans := by
    have projected := congrArg model.World.operation_plans frame
    exact projected
  have resource_indexesSame : next.resource_indexes = world.resource_indexes := by
    have projected := congrArg model.World.resource_indexes frame
    exact projected
  have vmsSame : next.vms = world.vms := by
    have projected := congrArg model.World.vms frame
    exact projected
  have net_slotsSame : next.net_slots = world.net_slots := by
    have projected := congrArg model.World.net_slots frame
    exact projected
  have closedVms : vmClosed next = vmClosed world := by
    funext id
    simp only [vmClosed,vmsSame]
  have closedSlots : slotClosed next = slotClosed world := by
    funext id
    simp only [slotClosed,net_slotsSame]
  exact {
    record := rest.record
    snapshot := rest.snapshot
    vm := rest.vm
    operation := rest.operation
    ledger := rest.ledger
    plan := rest.plan
    handoff := rest.handoff
    up := by simpa only [controller_upSame] using rest.up
    stored := by simpa only [sandboxesSame] using rest.stored
    phase := rest.phase
    terminal := rest.terminal
    snapshotStored := by simpa only [snapshotsSame] using rest.snapshotStored
    owner := rest.owner
    session := rest.session
    generation := rest.generation
    workspace := rest.workspace
    manifest := rest.manifest
    stage := rest.stage
    immutable := rest.immutable
    fence := by simpa only [worker_fencesSame] using rest.fence
    currentVm := rest.currentVm
    fenceSucceeded := fence
    freezeSucceeded := freeze
    sealSucceeded := sealedCheck
    vmsClosed := by simpa only [caller_index_view,resource_indexesSame,closedVms] using rest.vmsClosed
    slotsClosed := by simpa only [caller_index_view,resource_indexesSame,closedSlots] using rest.slotsClosed
    currentOp := rest.currentOp
    ledgerStored := by simpa only [operationsSame] using rest.ledgerStored
    ledgerSandbox := rest.ledgerSandbox
    ledgerOperation := rest.ledgerOperation
    noResult := rest.noResult
    planStored := by simpa only [operation_plansSame] using rest.planStored
    target := rest.target
    planVm := rest.planVm
    operationKind := rest.operationKind
    operationEpoch := rest.operationEpoch
    planKey := rest.planKey
    planEpoch := rest.planEpoch
    planGeneration := rest.planGeneration
    planCapture := rest.planCapture
    registered := rest.registered
    recoverable := rest.recoverable
    sourcePresent := rest.sourcePresent
  }

def ready_from_receipts (world : model.World) (sandbox : ids.SandboxId) (source : model.RestoreSource)
    (rest : ReadyRest world sandbox source)
    (commit : effect.effect_succeeded_check world (.CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest) = .ok true)
    (stop : effect.effect_succeeded_check world (.StopVm sandbox rest.record.epoch rest.record.generation rest.vm) = .ok true) :
    Ready world sandbox source := by
  have commitValue : effectValue world (.CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest) = true := by
    rw [effect_exact _ _ (.commit _ _ _ _ _)] at commit
    simpa only [Result.ok.injEq] using commit
  have stopValue : effectValue world (.StopVm sandbox rest.record.epoch rest.record.generation rest.vm) = true := by
    rw [effect_exact _ _ (.stop _ _ _ _)] at stop
    simpa only [Result.ok.injEq] using stop
  exact {
    record := rest.record
    snapshot := rest.snapshot
    vm := rest.vm
    operation := rest.operation
    ledger := rest.ledger
    plan := rest.plan
    handoff := rest.handoff
    up := rest.up
    stored := rest.stored
    phase := rest.phase
    terminal := rest.terminal
    snapshotStored := rest.snapshotStored
    owner := rest.owner
    session := rest.session
    generation := rest.generation
    workspace := rest.workspace
    manifest := rest.manifest
    stage := rest.stage
    immutable := rest.immutable
    fence := rest.fence
    currentVm := rest.currentVm
    fenceSucceeded := rest.fenceSucceeded
    freezeSucceeded := rest.freezeSucceeded
    sealSucceeded := rest.sealSucceeded
    vmsClosed := rest.vmsClosed
    slotsClosed := rest.slotsClosed
    currentOp := rest.currentOp
    ledgerStored := rest.ledgerStored
    ledgerSandbox := rest.ledgerSandbox
    ledgerOperation := rest.ledgerOperation
    noResult := rest.noResult
    planStored := rest.planStored
    target := rest.target
    planVm := rest.planVm
    operationKind := rest.operationKind
    operationEpoch := rest.operationEpoch
    planKey := rest.planKey
    planEpoch := rest.planEpoch
    planGeneration := rest.planGeneration
    planCapture := rest.planCapture
    registered := rest.registered
    recoverable := rest.recoverable
    sourcePresent := rest.sourcePresent
    commitSucceeded := commitValue
    stopSucceeded := stopValue
  }

end

noncomputable section
open SandboxModel.Lifecycle.PausePublication
local instance readyDecidable3 : DecidableEq model.LogicalEffectId := Classical.decEq _
local instance readyDecidable4 : DecidableEq model.EffectId := Classical.decEq _

def ready_rest_after_receipt (world next : model.World) (sandbox : ids.SandboxId) (source : model.RestoreSource)
    (rest : ReadyRest world sandbox source) (command : model.Command)
    (supported : PublicationCommand command)
    (isReceipt : command = .CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest ∨
      command = .StopVm sandbox rest.record.epoch rest.record.generation rest.vm)
    (attempt : U64) (digest : ids.ResultDigest) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨command⟩ = .ok (some stored))
    (current : stored.current_attempt = attempt)
    (capacity : TableWriteCapacity world.effects.entries.val ⟨⟨command⟩,attempt⟩)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok next) :
    ReadyRest next sandbox source := by
  have frame := acknowledge_frame world next ⟨⟨command⟩,attempt⟩ (.Succeeded digest) evaluated
  have fenceCheck := acknowledge_current_other_check world next command (.InstallFence sandbox rest.record.epoch false) supported
    (.fence _ _ _) (by rcases isReceipt with rfl | rfl <;> intro same <;> cases same)
    attempt digest stored read current capacity evaluated
  have fenceValue : effectValue next (.InstallFence sandbox rest.record.epoch false) = true := by
    rw [effect_exact next _ (.fence _ _ _),effect_exact world _ (.fence _ _ _)] at fenceCheck
    have unchanged : effectValue next (.InstallFence sandbox rest.record.epoch false) = effectValue world (.InstallFence sandbox rest.record.epoch false) := by
      simpa only [Result.ok.injEq] using fenceCheck
    exact unchanged.trans rest.fenceSucceeded
  have freezeCheck := acknowledge_current_other_check world next command (.FreezeVm sandbox rest.record.epoch rest.record.generation rest.vm) supported
    (.freeze _ _ _ _) (by rcases isReceipt with rfl | rfl <;> intro same <;> cases same)
    attempt digest stored read current capacity evaluated
  have freezeValue : effectValue next (.FreezeVm sandbox rest.record.epoch rest.record.generation rest.vm) = true := by
    rw [effect_exact next _ (.freeze _ _ _ _),effect_exact world _ (.freeze _ _ _ _)] at freezeCheck
    have unchanged : effectValue next (.FreezeVm sandbox rest.record.epoch rest.record.generation rest.vm) = effectValue world (.FreezeVm sandbox rest.record.epoch rest.record.generation rest.vm) := by
      simpa only [Result.ok.injEq] using freezeCheck
    exact unchanged.trans rest.freezeSucceeded
  have sealingCheck := acknowledge_current_other_check world next command (.SealSnapshot sandbox rest.record.epoch rest.record.generation rest.vm rest.record.workspace source.snapshot source.manifest) supported
    (.seal _ _ _ _ _ _ _) (by rcases isReceipt with rfl | rfl <;> intro same <;> cases same)
    attempt digest stored read current capacity evaluated
  have sealingValue : effectValue next (.SealSnapshot sandbox rest.record.epoch rest.record.generation rest.vm rest.record.workspace source.snapshot source.manifest) = true := by
    rw [effect_exact next _ (.seal _ _ _ _ _ _ _),effect_exact world _ (.seal _ _ _ _ _ _ _)] at sealingCheck
    have unchanged : effectValue next (.SealSnapshot sandbox rest.record.epoch rest.record.generation rest.vm rest.record.workspace source.snapshot source.manifest) = effectValue world (.SealSnapshot sandbox rest.record.epoch rest.record.generation rest.vm rest.record.workspace source.snapshot source.manifest) := by
      simpa only [Result.ok.injEq] using sealingCheck
    exact unchanged.trans rest.sealSucceeded
  exact ready_rest_frame world next sandbox source rest frame fenceValue freezeValue sealingValue

end

end SandboxModel.Lifecycle.PauseDispatch
