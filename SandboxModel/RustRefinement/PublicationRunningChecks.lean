import SandboxModel.RustRefinement.PublicationPaused

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance : DecidableEq AppliedFilePolicy := by
  intro left right
  cases left
  cases right
  simp only [AppliedFilePolicy.mk.injEq]
  infer_instance
local instance : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem publication_vm_some {α : Sort u} {value : Option Vm}
    {record : Vm} (found : value = some record)
    (missing : value = none → α) (present : (r : Vm) → value = some r → α) :
    Controller.handleScheduleSealSnapshot.match_1 (fun _ => α) value missing present = present record found := by
  cases found
  rfl
theorem publication_vm_none {α : Sort u} {value : Option Vm}
    (found : value = none)
    (missing : value = none → α) (present : (r : Vm) → value = some r → α) :
    Controller.handleScheduleSealSnapshot.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl
theorem publication_slotId_some {α : Sort u} {value : Option NetSlotId}
    {id : NetSlotId} (found : value = some id)
    (missing : value = none → α) (present : (r : NetSlotId) → value = some r → α) :
    Controller.handlePublishRunning.match_1 (fun _ => α) value missing present = present id found := by
  cases found
  rfl
theorem publication_slotId_none {α : Sort u} {value : Option NetSlotId}
    (found : value = none)
    (missing : value = none → α) (present : (r : NetSlotId) → value = some r → α) :
    Controller.handlePublishRunning.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl
theorem publication_slot_some {α : Sort u} {value : Option NetSlot}
    {record : NetSlot} (found : value = some record)
    (missing : value = none → α) (present : (r : NetSlot) → value = some r → α) :
    Controller.handleScheduleCleanupSlot.match_1 (fun _ => α) value missing present = present record found := by
  cases found
  rfl
theorem publication_slot_none {α : Sort u} {value : Option NetSlot}
    (found : value = none)
    (missing : value = none → α) (present : (r : NetSlot) → value = some r → α) :
    Controller.handleScheduleCleanupSlot.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

theorem publication_vmStatus_eq (left right : SandboxControllerRust.model.VmStatus) :
    SandboxControllerRust.model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq left right =
      ok (decide (vmStatus left = vmStatus right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq,
      SandboxControllerRust.model.VmStatus.read_discriminant, vmStatus]

theorem publication_applied_ne (left right : Option SandboxControllerRust.model.AppliedFilePolicy) :
    core.cmp.PartialEq.ne.trait_default
      (SandboxControllerRust.core.option.Option.Insts.CoreCmpPartialEqOption
        (core.cmp.PartialEqShared SandboxControllerRust.model.AppliedFilePolicy.Insts.CoreCmpPartialEqAppliedFilePolicy)) left right =
      ok (decide (left.map appliedFilePolicy ≠ right.map appliedFilePolicy)) := by
  cases left <;> cases right <;>
    simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      core.option.Option.Insts.CoreCmpPartialEqOption.eq, core.cmp.PartialEqShared,
      core.cmp.impls.PartialEqShared.eq,
      SandboxControllerRust.model.AppliedFilePolicy.Insts.CoreCmpPartialEqAppliedFilePolicy.eq,
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
      filePolicyRef_eq_lawful,
      appliedFilePolicy, AppliedFilePolicy.mk.injEq]
  all_goals (repeat' split) <;> simp_all [u64ToNat_injective.eq_iff, sandboxId_eq_iff, fsId_eq_iff, filePolicyRef_eq_iff, AppliedFilePolicy.mk.injEq]

theorem publication_applied_store_ne (rust : SandboxControllerRust.model.World) (model : World)
    (rep : PublicationWorldRep rust model) (sid : SandboxControllerRust.ids.SandboxId)
    (generation : U64) (workspace : SandboxControllerRust.ids.FsId)
    (policy : SandboxControllerRust.model.FilePolicyRef) :
    core.cmp.PartialEq.ne.trait_default
      (SandboxControllerRust.core.option.Option.Insts.CoreCmpPartialEqOption
        (core.cmp.PartialEqShared SandboxControllerRust.model.AppliedFilePolicy.Insts.CoreCmpPartialEqAppliedFilePolicy))
      (storeLookup rust.applied_file_policies sid) (some ⟨sid, generation, workspace, policy⟩) =
    ok (!(Controller.appliedPolicyCheck model (sandboxId sid) (u64ToNat generation) (fsId workspace) (filePolicyRef policy))) := by
  rw [publication_applied_ne]
  simp only [Option.map_some, ← storeRep_lookup_map rep.appliedFilePolicies sid]
  cases found : model.appliedFilePolicies (sandboxId sid) with
  | none => simp [Controller.appliedPolicyCheck, found]
  | some value =>
    cases value
    simp [Controller.appliedPolicyCheck, found, appliedFilePolicy, AppliedFilePolicy.mk.injEq]

@[simp] theorem publication_starting : phase .Starting = .starting := rfl
@[simp] theorem publication_executing : vmStatus .Executing = .executing := rfl
@[simp] theorem publication_record_slot (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).netSlot = record.net_slot.map netSlotId := rfl
@[simp] theorem publication_record_policy (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).policy = networkPolicy record.policy := rfl
@[simp] theorem publication_record_filePolicy (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).filePolicy = filePolicyRef record.file_policy := rfl
@[simp] theorem publication_record_restore (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).restoreFrom = record.restore_from.map restoreSource := rfl

end SandboxModel.RustRefinement
