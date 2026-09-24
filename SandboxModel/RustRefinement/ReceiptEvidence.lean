import SandboxModel.RustRefinement.EffectHandler

/-! Receipt evidence is checked against actual resource records; a claimed
success alone is insufficient.  All command variants are covered here. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance evidenceOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance evidenceLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance evidenceEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

theorem storeRep_lookup_map [DecidableEq K]
    (represented : StoreRep keyMap (fun concrete abstract => valueMap concrete = abstract) store model)
    (key : K) : model (keyMap key) = (storeLookup store key).map valueMap := by
  have agrees := represented.lookupAgrees key
  cases concrete : storeLookup store key <;> cases abstract : model (keyMap key) <;>
    simp_all [OptionRel]

private theorem scalar_eq_iff (left right : Std.U64) :
    u64ToNat left = u64ToNat right ↔ left = right := u64ToNat_injective.eq_iff

set_option maxHeartbeats 1000000 in
theorem generated_commandResultPresent_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (input : SandboxControllerRust.model.Command) :
    SandboxControllerRust.effect.command_result_present rust input =
      ok (SandboxModel.Controller.commandResultPresentCheck model (command input)) := by
  have getVm := fun key => generated_get_eq
    SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful rust.vms key
  have getSlot := fun key => generated_get_eq
    SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful rust.net_slots key
  have getFence := fun key => generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful rust.worker_fences key
  have getPolicy := fun key => generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful rust.applied_file_policies key
  have getSnapshot := fun key => generated_get_eq
    SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshotId_partialEq_lawful rust.snapshots key
  have vms : ∀ key, model.vms (vmId key) = (storeLookup rust.vms key).map vm :=
    storeRep_lookup_map represented.vms
  have slots : ∀ key, model.netSlots (netSlotId key) = (storeLookup rust.net_slots key).map netSlot :=
    storeRep_lookup_map represented.netSlots
  have fences : ∀ key, model.workerFences (sandboxId key) = (storeLookup rust.worker_fences key).map workerFence :=
    storeRep_lookup_map represented.workerFences
  have policies : ∀ key, model.appliedFilePolicies (sandboxId key) =
      (storeLookup rust.applied_file_policies key).map appliedFilePolicy :=
    storeRep_lookup_map represented.appliedFilePolicies
  have snapshots : ∀ key, model.snapshots (snapshotId key) = (storeLookup rust.snapshots key).map snapshot :=
    storeRep_lookup_map represented.snapshots
  cases input <;>
    simp only [SandboxControllerRust.effect.command_result_present,
      SandboxModel.Controller.commandResultPresentCheck, command,
      getVm, getSlot, getFence, getPolicy, getSnapshot, vms, slots, fences, policies, snapshots, bind_ok]
  all_goals
    simp [Aeneas.Std.instMonadResult]
  all_goals
    repeat' (split <;> simp_all [vm, netSlot, workerFence, appliedFilePolicy, snapshot,
          SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
          SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
          SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
          networkPolicy_eq_lawful, filePolicyRef_eq_lawful, snapshotManifestRef_eq_lawful,
          SandboxControllerRust.model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq,
          SandboxControllerRust.model.SnapshotStage.read_discriminant,
          SandboxControllerRust.model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq,
          SandboxControllerRust.model.VmStatus.read_discriminant,
          scalar_eq_iff, u64ToNat, vmStatus, snapshotStage])

theorem generated_effectOutcomeValid_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (input : SandboxControllerRust.model.Command)
    (outcome : SandboxControllerRust.model.EffectOutcome) :
    SandboxControllerRust.effect.effect_outcome_valid rust input outcome =
      ok (SandboxModel.Controller.effectOutcomeValidCheck model (command input) (effectOutcome outcome)) := by
  cases outcome <;>
    simp [SandboxControllerRust.effect.effect_outcome_valid,
      generated_commandResultPresent_refines rust model represented,
      SandboxModel.Controller.effectOutcomeValidCheck, effectOutcome]

end SandboxModel.RustRefinement
