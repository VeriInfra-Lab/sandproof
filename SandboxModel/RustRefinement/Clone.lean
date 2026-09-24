import SandboxModel.RustRefinement.World

/-!
Executable clone facts for the Aeneas translation of the Rust controller.

Every controller handler first clones the persistent Rust `World`.  These
lemmas establish that the generated clone is observationally the identity,
so later commuting-diagram proofs can focus on the handler's actual update.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

theorem vec_clone_eq {α : Type} (clone : core.clone.Clone α)
    (values : alloc.vec.Vec α)
    (clonesExactly : ∀ value ∈ values.val, clone.clone value = .ok value) :
    alloc.vec.CloneVec.clone clone values = .ok values := by
  have sliceSpec := Slice.clone_spec (s := values.slice) clonesExactly
  obtain ⟨cloned, clonedOk, clonedEq⟩ :=
    Aeneas.Std.WP.spec_imp_exists sliceSpec
  simp only [alloc.vec.CloneVec.clone, clonedOk]
  cases values
  cases clonedEq
  simp

theorem builtin_vec_clone_eq {α : Type} (values : alloc.vec.Vec α) :
    alloc.vec.CloneVec.clone (BuiltinClone α) values = .ok values := by
  apply vec_clone_eq
  simp [BuiltinClone]

theorem store_clone_eq {key value : Type}
    (keyClone : core.clone.Clone key) (valueClone : core.clone.Clone value)
    (store : SandboxControllerRust.store.Store key value) :
    SandboxControllerRust.store.Store.Insts.CoreCloneClone.clone
        keyClone valueClone store = .ok store := by
  simp [SandboxControllerRust.store.Store.Insts.CoreCloneClone.clone,
    builtin_vec_clone_eq]

@[simp] theorem endpoint_clone_eq
    (endpoint : SandboxControllerRust.model.Endpoint) :
    SandboxControllerRust.model.Endpoint.Insts.CoreCloneClone.clone endpoint =
      .ok endpoint := by
  cases endpoint
  simp [SandboxControllerRust.model.Endpoint.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.Protocol.Insts.CoreCloneClone.clone,
    Aeneas.Std.lift]

@[simp] theorem network_policy_clone_eq
    (policy : SandboxControllerRust.model.NetworkPolicy) :
    SandboxControllerRust.model.NetworkPolicy.Insts.CoreCloneClone.clone policy =
      .ok policy := by
  cases policy <;>
    simp [SandboxControllerRust.model.NetworkPolicy.Insts.CoreCloneClone.clone,
      vec_clone_eq]

@[simp] theorem restore_source_option_clone_eq
    (source : Option SandboxControllerRust.model.RestoreSource) :
    core.option.Option.Insts.CoreCloneClone.clone
        SandboxControllerRust.model.RestoreSource.Insts.CoreCloneClone source =
      .ok source := by
  cases source <;>
    simp [core.option.Option.Insts.CoreCloneClone.clone,
      core.option.OptionShared0T.cloned,
      SandboxControllerRust.model.RestoreSource.Insts.CoreCloneClone.clone]

@[simp] theorem command_clone_eq
    (command : SandboxControllerRust.model.Command) :
    SandboxControllerRust.model.Command.Insts.CoreCloneClone.clone command =
      .ok command := by
  cases command <;>
    simp [SandboxControllerRust.model.Command.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.SandboxId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.VmId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.FsId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.SnapshotId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.model.FilePolicyRef.Insts.CoreCloneClone.clone,
      SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCloneClone.clone,
      SandboxControllerRust.model.RestoreSource.Insts.CoreCloneClone.clone,
      Aeneas.Std.lift]
  case PrepareVm source =>
    cases source <;> simp

@[simp] theorem logical_effect_id_clone_eq
    (logical : SandboxControllerRust.model.LogicalEffectId) :
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCloneClone.clone
        logical = .ok logical := by
  cases logical
  simp [SandboxControllerRust.model.LogicalEffectId.Insts.CoreCloneClone.clone]

@[simp] theorem effect_id_clone_eq
    (effect : SandboxControllerRust.model.EffectId) :
    SandboxControllerRust.model.EffectId.Insts.CoreCloneClone.clone effect =
      .ok effect := by
  cases effect
  simp [SandboxControllerRust.model.EffectId.Insts.CoreCloneClone.clone,
    Aeneas.Std.lift]

theorem world_clone_eq (world : SandboxControllerRust.model.World) :
    SandboxControllerRust.model.World.Insts.CoreCloneClone.clone world =
      .ok world := by
  simp [SandboxControllerRust.model.World.Insts.CoreCloneClone.clone,
    store_clone_eq, vec_clone_eq, Aeneas.Std.lift]

theorem world_clone_success_eq
    {world cloned : SandboxControllerRust.model.World}
    (success :
      SandboxControllerRust.model.World.Insts.CoreCloneClone.clone world =
        .ok cloned) :
    cloned = world := by
  rw [world_clone_eq world] at success
  exact (Aeneas.Std.Result.ok_injective success).symm

/-- The concrete clone step used at the start of Rust handlers cannot break
    the complete persistent-world representation relation. -/
theorem world_clone_preserves_rep
    [DecidableEq SandboxControllerRust.model.OperationKey]
    [DecidableEq SandboxControllerRust.model.LogicalEffectId]
    [DecidableEq SandboxControllerRust.model.EffectId]
    (relations : WorldValueRelations)
    {rust cloned : SandboxControllerRust.model.World}
    {model : SandboxModel.World}
    (represented : WorldRep relations rust model)
    (success :
      SandboxControllerRust.model.World.Insts.CoreCloneClone.clone rust =
        .ok cloned) :
    WorldRep relations cloned model := by
  rw [world_clone_success_eq success]
  exact represented

@[simp] theorem authority_clone_eq (a : SandboxControllerRust.model.Authority) :
    SandboxControllerRust.model.Authority.Insts.CoreCloneClone.clone a = ok a := by
  cases a <;> simp [SandboxControllerRust.model.Authority.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.SandboxId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.VmId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.FsId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.model.WorkspaceApiAccess.Insts.CoreCloneClone.clone,
      SandboxControllerRust.model.RestoreSource.Insts.CoreCloneClone.clone, lift]

@[simp] theorem netSlot_clone_eq (slot : SandboxControllerRust.model.NetSlot) :
    SandboxControllerRust.model.NetSlot.Insts.CoreCloneClone.clone slot = ok slot := by
  cases slot
  simp [SandboxControllerRust.model.NetSlot.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.TenantId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.VmId.Insts.CoreCloneClone.clone, lift]

@[simp] theorem sandboxRecord_clone_eq (record : SandboxControllerRust.model.SandboxRecord) :
    SandboxControllerRust.model.SandboxRecord.Insts.CoreCloneClone.clone record = ok record := by
  simp [SandboxControllerRust.model.SandboxRecord.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.TenantId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.Phase.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.VmId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.FsId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.WorkspaceApiAccess.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.FilePolicyRef.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.RestoreSource.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.Operation.Insts.CoreCloneClone.clone,
    lift]
  cases currentVm : record.current_vm <;> cases durableSnapshot : record.durable_snapshot <;>
    cases restoreFrom : record.restore_from <;> cases netSlot : record.net_slot <;>
    cases currentOperation : record.current_operation <;> simp <;> cases record <;> simp_all

@[simp] theorem operationLedger_clone_eq (record : SandboxControllerRust.model.OperationLedgerRecord) :
    SandboxControllerRust.model.OperationLedgerRecord.Insts.CoreCloneClone.clone record = ok record := by
  have inputClone : ∀ i, SandboxControllerRust.model.OperationInput.Insts.CoreCloneClone.clone i = ok i := by
    intro i
    cases i <;> simp [SandboxControllerRust.model.OperationInput.Insts.CoreCloneClone.clone,
      SandboxControllerRust.ids.FsId.Insts.CoreCloneClone.clone,
      SandboxControllerRust.model.FilePolicyRef.Insts.CoreCloneClone.clone,
      SandboxControllerRust.model.RestoreSource.Insts.CoreCloneClone.clone, lift]
  simp [SandboxControllerRust.model.OperationLedgerRecord.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.Operation.Insts.CoreCloneClone.clone,
    inputClone, SandboxControllerRust.model.OperationResult.Insts.CoreCloneClone.clone]
  cases resultCase : record.result <;> simp <;> cases record <;> simp_all


end SandboxModel.RustRefinement
