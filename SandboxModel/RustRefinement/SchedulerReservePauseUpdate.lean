import SandboxModel.RustRefinement.SchedulerDispatchPrepare

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance schedulerReservePauseUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerReservePauseUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerReservePauseUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem scheduler_plan_clone (plan : SandboxControllerRust.model.OperationPlan) :
    SandboxControllerRust.model.OperationPlan.Insts.CoreCloneClone.clone plan = ok plan := by
  have vmClone : core.option.Option.Insts.CoreCloneClone.clone
      SandboxControllerRust.ids.VmId.Insts.CoreCloneClone plan.vm = ok plan.vm := by
    cases plan.vm <;> simp [core.option.Option.Insts.CoreCloneClone.clone,
      core.option.OptionShared0T.cloned, SandboxControllerRust.ids.VmId.Insts.CoreCloneClone.clone]
  have slotClone : core.option.Option.Insts.CoreCloneClone.clone
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone plan.slot = ok plan.slot := by
    cases plan.slot <;> simp [core.option.Option.Insts.CoreCloneClone.clone,
      core.option.OptionShared0T.cloned, SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone.clone]
  have vmsClone := vec_clone_eq SandboxControllerRust.ids.VmId.Insts.CoreCloneClone
    plan.stop_vms (by intros; rfl)
  have slotsClone := vec_clone_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone
    plan.close_slots (by intros; rfl)
  simp only [SandboxControllerRust.model.OperationPlan.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.OperationKey.Insts.CoreCloneClone.clone,
    vmClone, slotClone, vmsClone, slotsClone, restore_source_option_clone_eq, lift, bind_tc_ok]

theorem generated_reservePauseSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (snapshotRoom : U64Headroom rust.next_snapshot_id)
    (manifestRoom : U64Headroom rust.next_manifest_digest)
    (planRoom : StoreHeadroom rust.operation_plans)
    (key : SandboxControllerRust.model.OperationKey) (plan : SandboxControllerRust.model.OperationPlan)
    (vid : SandboxControllerRust.ids.VmId) :
    ∃ next, SandboxControllerRust.transition.reserve_pause_snapshot rust key plan vid = ok (some next) ∧
      SchedulerWorldRep next (reservePauseSnapshot model (operationKey key) (operationPlan plan) (vmId vid)) := by
  obtain ⟨snapshotNotMax, nextSnapshot, snapshotEval, snapshotSame⟩ := lifecycle_increment rust.next_snapshot_id snapshotRoom
  obtain ⟨manifestNotMax, nextManifest, manifestEval, manifestSame⟩ := lifecycle_increment rust.next_manifest_digest manifestRoom
  let reserved : SandboxControllerRust.model.OperationPlan :=
    { plan with vm := some vid, target_snapshot := some ⟨rust.next_snapshot_id, ⟨rust.next_manifest_digest⟩⟩, sealed := true }
  obtain ⟨plans, plansEval, plansRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful operationKey operationKey_injective
      (fun a b => operationPlan a = b) rust.operation_plans model.operationPlans rep.operationPlans
      key reserved (operationPlan reserved) rfl planRoom)
  refine ⟨{ rust with operation_plans := plans, next_snapshot_id := nextSnapshot, next_manifest_digest := nextManifest }, ?_, ?_⟩
  · simp [SandboxControllerRust.transition.reserve_pause_snapshot, snapshotNotMax, manifestNotMax,
      snapshotEval, manifestEval, plansEval, world_clone_eq, reserved]
  · have targetSame : operationPlan reserved =
        { (operationPlan plan) with vm := some (vmId vid), targetSnapshot := some ⟨⟨model.nextSnapshotId⟩, ⟨⟨model.nextManifestDigest⟩⟩⟩, sealed := true } := by
      simp [reserved, operationPlan, restoreSource, snapshotId, snapshotManifestRef, manifestDigest,
        rep.nextSnapshotId, rep.nextManifestDigest]
    exact { rep with
      operationPlans := by simpa only [targetSame, reservePauseSnapshot, setOperationPlan, valueRelations] using plansRep
      nextSnapshotId := snapshotSame.trans (congrArg (· + 1) rep.nextSnapshotId)
      nextManifestDigest := manifestSame.trans (congrArg (· + 1) rep.nextManifestDigest) }

end SandboxModel.RustRefinement
