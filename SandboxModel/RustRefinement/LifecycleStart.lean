import SandboxModel.RustRefinement.LifecycleEffects

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance lifecycleStartOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance lifecycleStartLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance lifecycleStartEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_beginCreate_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (owner : SandboxControllerRust.ids.TenantId)
    (workspace : SandboxControllerRust.ids.FsId) (filePolicy : SandboxControllerRust.model.FilePolicyRef)
    (policy : SandboxControllerRust.model.NetworkPolicy) (oid : SandboxControllerRust.ids.OperationId) :
    ∃ next, SandboxControllerRust.transition.begin_create rust sid owner workspace filePolicy policy oid = ok next ∧
      EffectWorldRep next (beginCreate model (sandboxId sid) (tenantId owner) (fsId workspace)
        (filePolicyRef filePolicy) (networkPolicy policy) (operationId oid)) := by
  let op : SandboxControllerRust.model.Operation := ⟨oid, 1#u64, .Create, .Pending⟩
  let record : SandboxControllerRust.model.SandboxRecord := {
    owner := owner, phase := .Starting, terminal := false, epoch := 1#u64, generation := 1#u64,
    current_vm := none, workspace := workspace, workspace_access := .ManagedReadWrite,
    file_policy := filePolicy, durable_snapshot := none, restore_from := none, net_slot := none,
    policy := policy, desired := .Closed, effective := .Closed, current_operation := some op }
  let fs : SandboxControllerRust.model.Filesystem := ⟨owner, sid, 1#u64⟩
  obtain ⟨filesystems, fsEval, fsRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId
      fsId_partialEq_lawful fsId fsId_injective (fun a b => filesystem a = b)
      rust.filesystems model.filesystems rep.filesystems workspace fs (filesystem fs) rfl room.filesystems)
  obtain ⟨sandboxes, sandboxEval, sandboxRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => sandboxRecord a = b)
      rust.sandboxes model.sandboxes rep.sandboxes sid record (sandboxRecord record) rfl room.sandboxes)
  let base := { rust with filesystems := filesystems, sandboxes := sandboxes }
  let baseModel := setSandbox (setFilesystem model (fsId workspace) (filesystem fs)) (sandboxId sid) (sandboxRecord record)
  have baseRep : EffectWorldRep base baseModel := { rep with filesystems := fsRep, sandboxes := sandboxRep }
  have baseCapacity : EffectCapacity base 1 := ⟨capacity.effects, capacity.logical, capacity.keys, capacity.inflight⟩
  obtain ⟨stored, storedEval, storedRep, storedCapacity⟩ := generated_storeNewOperation_refines
    base baseModel baseRep room.operations room.operationPlans baseCapacity sid op (.Create workspace policy filePolicy)
  obtain ⟨next, nextEval, nextRep, _⟩ := generated_enqueue_refines stored _ storedRep storedCapacity (.InstallFence sid 1#u64 false)
  refine ⟨next, ?_, ?_⟩
  · dsimp [record, op] at sandboxEval
    dsimp [fs] at fsEval
    dsimp [base, op] at storedEval
    simp [SandboxControllerRust.transition.begin_create, world_clone_eq, fsEval, sandboxEval, storedEval, nextEval]
  · simpa [beginCreate, baseModel, record, op, fs, sandboxRecord, filesystem,
      operation, operationKind, operationStatus, operationInput, command,
      phase, authority, workspaceApiAccess, u64ToNat] using nextRep

theorem generated_beginPause_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 2)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (oid : SandboxControllerRust.ids.OperationId) (vid : SandboxControllerRust.ids.VmId)
    (epoch : U64) (epochNext : u64ToNat epoch = u64ToNat record.epoch + 1) :
    ∃ next, SandboxControllerRust.transition.begin_pause rust sid record oid vid epoch = ok next ∧
      EffectWorldRep next (beginPause model (sandboxId sid) (sandboxRecord record) (operationId oid) (vmId vid)) := by
  let op : SandboxControllerRust.model.Operation := ⟨oid, epoch, .Pause, .Pending⟩
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Pausing
    epoch := epoch
    desired := SandboxControllerRust.model.Authority.Closed
    current_operation := some op }
  obtain ⟨sandboxes, sandboxEval, sandboxRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => sandboxRecord a = b)
      rust.sandboxes model.sandboxes rep.sandboxes sid updated (sandboxRecord updated) rfl room.sandboxes)
  let base := { rust with sandboxes := sandboxes }
  let baseModel := setSandbox model (sandboxId sid) (sandboxRecord updated)
  have baseRep : EffectWorldRep base baseModel := { rep with sandboxes := sandboxRep }
  have baseCapacity : EffectCapacity base 2 := ⟨capacity.effects, capacity.logical, capacity.keys, capacity.inflight⟩
  obtain ⟨stored, storedEval, storedRep, storedCapacity⟩ := generated_storeNewOperation_refines
    base baseModel baseRep room.operations room.operationPlans baseCapacity sid op (.Pause record.generation)
  obtain ⟨fenced, fenceEval, fenceRep, fenceCapacity⟩ := generated_enqueue_refines stored _ storedRep storedCapacity (.InstallFence sid epoch false)
  obtain ⟨next, nextEval, nextRep, _⟩ := generated_enqueue_refines fenced _ fenceRep fenceCapacity (.FreezeVm sid epoch record.generation vid)
  refine ⟨next, ?_, ?_⟩
  · dsimp [updated, op] at sandboxEval
    dsimp [base, op] at storedEval
    simp [SandboxControllerRust.transition.begin_pause, world_clone_eq, sandboxEval, storedEval, fenceEval, nextEval]
  · simpa [beginPause, baseModel, updated, op, sandboxRecord,
      operation, operationKind, operationStatus, operationInput, command, phase, authority, epochNext] using nextRep

theorem generated_beginResume_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (oid : SandboxControllerRust.ids.OperationId) (source : SandboxControllerRust.model.RestoreSource)
    (epoch generation : U64) (epochNext : u64ToNat epoch = u64ToNat record.epoch + 1)
    (generationNext : u64ToNat generation = u64ToNat record.generation + 1) :
    ∃ next, SandboxControllerRust.transition.begin_resume rust sid record oid source epoch generation = ok next ∧
      EffectWorldRep next (beginResume model (sandboxId sid) (sandboxRecord record) (operationId oid) (restoreSource source)) := by
  let op : SandboxControllerRust.model.Operation := ⟨oid, epoch, .Resume, .Pending⟩
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Starting
    epoch := epoch
    generation := generation
    current_vm := none
    restore_from := some source
    net_slot := none
    desired := SandboxControllerRust.model.Authority.Closed
    effective := SandboxControllerRust.model.Authority.Closed
    current_operation := some op }
  obtain ⟨sandboxes, sandboxEval, sandboxRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => sandboxRecord a = b)
      rust.sandboxes model.sandboxes rep.sandboxes sid updated (sandboxRecord updated) rfl room.sandboxes)
  let base := { rust with sandboxes := sandboxes }
  let baseModel := setSandbox model (sandboxId sid) (sandboxRecord updated)
  have baseRep : EffectWorldRep base baseModel := { rep with sandboxes := sandboxRep }
  have baseCapacity : EffectCapacity base 1 := ⟨capacity.effects, capacity.logical, capacity.keys, capacity.inflight⟩
  obtain ⟨stored, storedEval, storedRep, storedCapacity⟩ := generated_storeNewOperation_refines
    base baseModel baseRep room.operations room.operationPlans baseCapacity sid op (.Resume source)
  obtain ⟨next, nextEval, nextRep, _⟩ := generated_enqueue_refines stored _ storedRep storedCapacity (.InstallFence sid epoch false)
  refine ⟨next, ?_, ?_⟩
  · dsimp [updated, op] at sandboxEval
    dsimp [base, op] at storedEval
    simp [SandboxControllerRust.transition.begin_resume, world_clone_eq, sandboxEval, storedEval, nextEval]
  · simpa [beginResume, baseModel, updated, op, sandboxRecord,
      operation, operationKind, operationStatus, operationInput, command, phase, authority, epochNext, generationNext] using nextRep

end SandboxModel.RustRefinement
