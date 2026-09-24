import SandboxModel.RustRefinement.SchedulerCommitSnapshot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance schedulerPrepareChecksOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerPrepareChecksLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerPrepareChecksEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

def schedulerRestoreSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource := by
  intro left right
  if same : restoreSource left = restoreSource right then
    exact isTrue (restoreSource_injective same)
  else exact isFalse (fun equal => same (congrArg restoreSource equal))
local instance schedulerPrepareChecksSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource := schedulerRestoreSourceDecEq

theorem scheduler_restore_eq_lawful :
    LawfulPartialEq SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource := by
  intro left right
  cases left with
  | mk ls lm =>
    cases right with
    | mk rs rm =>
      cases lm with
      | mk ld =>
        cases rm with
        | mk rd =>
          simp only [
            SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq,
            SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
            SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
            SandboxControllerRust.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq]
          by_cases sameSnapshot : ls = rs <;> by_cases sameDigest : ld = rd <;>
            simp [sameSnapshot, sameDigest]

theorem generated_option_eq [DecidableEq T] (partialEq : core.cmp.PartialEq T T)
    (lawful : LawfulPartialEq partialEq) (left right : Option T) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq partialEq left right =
      ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq]
  exact lawful _ _

@[simp] theorem scheduler_plan_sealed (value : SandboxControllerRust.model.OperationPlan) :
    (operationPlan value).sealed = value.sealed := rfl
@[simp] theorem scheduler_plan_vm (value : SandboxControllerRust.model.OperationPlan) :
    (operationPlan value).vm = value.vm.map vmId := rfl
@[simp] theorem scheduler_plan_slot (value : SandboxControllerRust.model.OperationPlan) :
    (operationPlan value).slot = value.slot.map netSlotId := rfl
@[simp] theorem scheduler_plan_target (value : SandboxControllerRust.model.OperationPlan) :
    (operationPlan value).targetSnapshot = value.target_snapshot.map restoreSource := rfl
@[simp] theorem scheduler_plan_restore (value : SandboxControllerRust.model.OperationPlan) :
    (operationPlan value).restoreFrom = value.restore_from.map restoreSource := rfl
@[simp] theorem scheduler_source_mk (sid : SandboxControllerRust.ids.SnapshotId)
    (manifest : SandboxControllerRust.model.SnapshotManifestRef) :
    restoreSource ⟨sid, manifest⟩ = ⟨snapshotId sid, snapshotManifestRef manifest⟩ := rfl
@[simp] theorem scheduler_frozen : vmStatus .Frozen = .frozen := rfl

theorem scheduler_vm_option_eq (left right : Option SandboxControllerRust.ids.VmId) :
    left.map vmId = right.map vmId ↔ left = right :=
  (Option.map_injective vmId_injective).eq_iff
theorem scheduler_slot_option_eq (left right : Option SandboxControllerRust.ids.NetSlotId) :
    left.map netSlotId = right.map netSlotId ↔ left = right :=
  (Option.map_injective netSlotId_injective).eq_iff

theorem scheduler_plan_vm_eq (plan : SandboxControllerRust.model.OperationPlan) (vid : SandboxControllerRust.ids.VmId) :
    (operationPlan plan).vm = some (vmId vid) ↔ plan.vm = some vid := by
  simpa only [scheduler_plan_vm, Option.map_some] using scheduler_vm_option_eq plan.vm (some vid)
theorem scheduler_plan_slot_eq (plan : SandboxControllerRust.model.OperationPlan) (slot : SandboxControllerRust.ids.NetSlotId) :
    (operationPlan plan).slot = some (netSlotId slot) ↔ plan.slot = some slot := by
  simpa only [scheduler_plan_slot, Option.map_some] using scheduler_slot_option_eq plan.slot (some slot)
theorem scheduler_plan_target_eq (plan : SandboxControllerRust.model.OperationPlan)
    (sid : SandboxControllerRust.ids.SnapshotId) (manifest : SandboxControllerRust.model.SnapshotManifestRef) :
    (operationPlan plan).targetSnapshot = some ⟨snapshotId sid, snapshotManifestRef manifest⟩ ↔
      plan.target_snapshot = some ⟨sid, manifest⟩ := by
  simpa only [scheduler_plan_target, Option.map_some, scheduler_source_mk] using
    restoreSource_option_map_eq_iff (left := plan.target_snapshot) (right := some ⟨sid, manifest⟩)

theorem generated_schedulerUsableSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (source : SandboxControllerRust.model.RestoreSource) :
    SandboxControllerRust.scheduler.usable_snapshot_check rust sid record source =
      ok (Controller.usableSnapshotCheck model (sandboxId sid) (sandboxRecord record) (restoreSource source)) := by
  exact generated_usableSnapshot_refines rust model rep sid record source

theorem generated_restoreSourceReady_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord) :
    SandboxControllerRust.scheduler.restore_source_ready_check rust sid record =
      ok (Controller.restoreSourceReadyCheck model (sandboxId sid) (sandboxRecord record)) := by
  have allUsable := fun source => generated_schedulerUsableSnapshot_refines rust model rep sid record source
  have allEq := fun left right => scheduler_restore_eq_lawful left right
  cases selected : record.restore_from <;> cases durable : record.durable_snapshot <;>
    simp [SandboxControllerRust.scheduler.restore_source_ready_check, selected, allUsable, allEq,
      Controller.restoreSourceReadyCheck, sandboxRecord, durable, restoreSource_injective.eq_iff]
  simp only at allEq
  rw [allEq]
  simp only [bind_tc_ok]
  split <;> simp_all

end SandboxModel.RustRefinement
