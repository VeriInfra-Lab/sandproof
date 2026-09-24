import SandboxModel.RustRefinement.LifecycleTerminal
import SandboxModel.RustRefinement.RequestHandler

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance lifecycleChecksOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance lifecycleChecksLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance lifecycleChecksEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem lifecycle_record_owner (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).owner = tenantId record.owner := rfl
@[simp] theorem lifecycle_record_phase (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).phase = phase record.phase := rfl
@[simp] theorem lifecycle_record_terminal (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).terminal = record.terminal := rfl
@[simp] theorem lifecycle_record_vm (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).currentVm = record.current_vm.map vmId := rfl
@[simp] theorem lifecycle_record_snapshot (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).durableSnapshot = record.durable_snapshot.map restoreSource := rfl
@[simp] theorem lifecycle_record_operation (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).currentOperation = record.current_operation.map operation := rfl
@[simp] theorem lifecycle_phase_running : phase .Running = .running := rfl
@[simp] theorem lifecycle_phase_paused : phase .Paused = .paused := rfl
@[simp] theorem lifecycle_phase_destroyed : phase .Destroyed = .destroyed := rfl

theorem lifecycle_increment (value : U64) (room : U64Headroom value) :
    value ≠ core.num.U64.MAX ∧ ∃ next, value + 1#u64 = ok next ∧ u64ToNat next = u64ToNat value + 1 := by
  constructor
  · intro same
    have values := congrArg Std.UScalar.val same
    have maxValue : (core.num.U64.MAX).val = Std.U64.max := by
      simp [core.num.U64.MAX, Std.U64.max_eq, Std.U64.rMax]
    rw [maxValue] at values
    exact (Nat.ne_of_lt room) values
  · have spec : (value + 1#u64) ⦃ next => next.val = value.val + 1 ⦄ := by
      unfold U64Headroom at room
      step
      assumption
    exact WP.spec_imp_exists spec

theorem generated_phase_eq (left right : SandboxControllerRust.model.Phase) :
    SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase.eq left right =
      ok (decide (phase left = phase right)) := by
  cases left <;> cases right <;> simp [SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase.eq,
    SandboxControllerRust.model.Phase.read_discriminant, phase]

theorem generated_phase_ne (left right : SandboxControllerRust.model.Phase) :
    core.cmp.PartialEq.ne.trait_default SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase left right =
      ok (decide (phase left ≠ phase right)) := by
  cases left <;> cases right <;> simp [core.cmp.PartialEq.ne.trait_default,
    core.cmp.PartialEq.ne.default, SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase.eq,
    SandboxControllerRust.model.Phase.read_discriminant, phase]

theorem generated_supersedeReady_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (current : Option SandboxControllerRust.model.Operation) :
    SandboxControllerRust.transition.supersede_ready_check rust sid current =
      ok (SandboxModel.Controller.supersedeReadyCheck model (sandboxId sid) (current.map operation)) := by
  cases current with
  | none => rfl
  | some op =>
    have get := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful rust.operations ⟨sid, op.id⟩
    have lookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk sid op.id)
    change model.operations (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at lookup
    cases status : op.status <;>
      simp only [SandboxControllerRust.transition.supersede_ready_check, status,
        SandboxControllerRust.transition.operation_key, get, bind_ok,
        SandboxModel.Controller.supersedeReadyCheck, Option.map_some,
        operation_status_field, operation_id_field, operationStatus]
    all_goals cases found : storeLookup rust.operations ⟨sid, op.id⟩ with
    | none => simp [get, found, lookup]
    | some ledger =>
      by_cases same : operation ledger.operation = operation op <;>
        cases result : ledger.result <;>
        simp [get, found, lookup, generated_operation_eq, same, operationLedger, result,
          core.option.Option.is_none]

theorem generated_usableSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (source : SandboxControllerRust.model.RestoreSource) :
    SandboxControllerRust.request.usable_snapshot_check rust sid record source =
      ok (SandboxModel.Controller.usableSnapshotCheck model (sandboxId sid) (sandboxRecord record) (restoreSource source)) := by
  have get := generated_get_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
    snapshotId_partialEq_lawful rust.snapshots source.snapshot
  have lookup := storeRep_lookup_map rep.snapshots source.snapshot
  simp only [SandboxControllerRust.request.usable_snapshot_check, get,
    SandboxModel.Controller.usableSnapshotCheck, restoreSource, lookup]
  cases found : storeLookup rust.snapshots source.snapshot <;>
    simp [found, Aeneas.Std.instMonadResult]
  all_goals simp [snapshot, sandboxRecord,
    SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
    snapshotManifestRef_eq_lawful]
  all_goals repeat' (split <;> simp_all [snapshot, sandboxRecord,
    SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
    snapshotManifestRef_eq_lawful,
    SandboxControllerRust.model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq,
    SandboxControllerRust.model.SnapshotStage.read_discriminant, snapshotStage])

end SandboxModel.RustRefinement
