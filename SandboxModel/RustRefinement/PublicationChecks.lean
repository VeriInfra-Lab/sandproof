import SandboxModel.RustRefinement.FiniteScan
import SandboxModel.RustRefinement.EffectSuccess

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance publicationChecksOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance publicationChecksLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance publicationChecksEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

@[simp] theorem resourceIndex_clone_eq (index : SandboxControllerRust.model.ResourceIndex) :
    SandboxControllerRust.model.ResourceIndex.Insts.CoreCloneClone.clone index = ok index := by
  cases index
  simp [SandboxControllerRust.model.ResourceIndex.Insts.CoreCloneClone.clone,
    vec_clone_eq, SandboxControllerRust.ids.VmId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.SnapshotId.Insts.CoreCloneClone.clone]

theorem generated_resourceIndex_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (sid : SandboxControllerRust.ids.SandboxId) :
    ∃ index, SandboxControllerRust.model.World.resource_index rust sid = ok index ∧
      resourceIndex index = model.resourceIndexes (sandboxId sid) := by
  have get := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.resource_indexes sid
  have agrees := represented.resourceIndexes.lookupAgrees sid
  cases found : storeLookup rust.resource_indexes sid with
  | none =>
      refine ⟨⟨alloc.vec.Vec.new _, alloc.vec.Vec.new _, alloc.vec.Vec.new _⟩, ?_, ?_⟩
      · simp [SandboxControllerRust.model.World.resource_index, get, found,
          core.option.OptionShared0T.cloned, core.option.Option.unwrap_or_default,
          SandboxControllerRust.model.ResourceIndex.Insts.CoreDefaultDefault.default,
          alloc.vec.Vec.Insts.CoreDefaultDefault.default]
      · rw [found] at agrees
        simpa [resourceIndex, ResourceIndex.empty] using agrees.symm
  | some index =>
      refine ⟨index, ?_, ?_⟩
      · simp [SandboxControllerRust.model.World.resource_index, get, found,
          core.option.OptionShared0T.cloned, core.option.Option.unwrap_or_default]
      · simpa [found, valueRelations] using agrees

private def concreteSlotInactive (rust : SandboxControllerRust.model.World)
    (id : SandboxControllerRust.ids.NetSlotId) : Bool :=
  match storeLookup rust.net_slots id with
  | none => false
  | some slot => !slot.active

private theorem slotBody_eq (rust : SandboxControllerRust.model.World)
    (index : SandboxControllerRust.model.ResourceIndex) (cursor : Std.Usize) :
    SandboxControllerRust.publication.indexed_slots_closed_loop.body rust index cursor =
      vectorAllBody index.slots (concreteSlotInactive rust) cursor := by
  unfold SandboxControllerRust.publication.indexed_slots_closed_loop.body vectorAllBody
  by_cases bound : cursor.val < index.slots.val.length
  · have inBounds : cursor < alloc.vec.Vec.len index.slots := by simpa using bound
    obtain ⟨value, valueEval, valueSame⟩ := Aeneas.Std.WP.spec_imp_exists
      (alloc.vec.Vec.index_usize_spec index.slots cursor bound)
    subst value
    have get := generated_get_eq
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
      netSlotId_partialEq_lawful rust.net_slots (index.slots.val[cursor.val]'bound)
    simp only [inBounds, bound, if_pos, dif_pos]
    simp [valueEval]
    rw [get]
    cases found : storeLookup rust.net_slots (index.slots.val[cursor.val]'bound) with
    | none => simp [concreteSlotInactive, found]
    | some slot => cases active : slot.active <;> simp [concreteSlotInactive, found, active]
  · have outside : ¬ cursor < alloc.vec.Vec.len index.slots := by simpa using bound
    simp [outside, bound]

theorem generated_indexedSlotsClosed_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (sid : SandboxControllerRust.ids.SandboxId) :
    SandboxControllerRust.publication.indexed_slots_closed rust sid =
      ok (indexedSlotsClosedCheck model (sandboxId sid)) := by
  obtain ⟨index, indexEval, indexSame⟩ := generated_resourceIndex_refines rust model represented sid
  have bodySame : SandboxControllerRust.publication.indexed_slots_closed_loop.body rust index =
      vectorAllBody index.slots (concreteSlotInactive rust) := funext (slotBody_eq rust index)
  obtain ⟨found, foundEval, foundSame⟩ := Aeneas.Std.WP.spec_imp_exists
    (vectorAllLoop_refines index.slots (concreteSlotInactive rust) 0#usize (by simp))
  have predicates (id) : concreteSlotInactive rust id = slotInactiveCheck model (netSlotId id) := by
    have lookup := storeRep_lookup_map represented.netSlots id
    unfold slotInactiveCheck concreteSlotInactive
    rw [lookup]
    cases storeLookup rust.net_slots id <;> simp [netSlot]
  simp only [SandboxControllerRust.publication.indexed_slots_closed, indexEval]
  simp [SandboxControllerRust.publication.indexed_slots_closed_loop, bodySame, foundEval]
  subst found
  unfold indexedSlotsClosedCheck
  rw [← indexSame]
  simp only [resourceIndex, List.all_map]
  congr 1
  funext id
  exact predicates id

private def concreteVmClosed (rust : SandboxControllerRust.model.World)
    (id : SandboxControllerRust.ids.VmId) : Bool :=
  match storeLookup rust.vms id with
  | none => false
  | some value => match value.status with
    | .Stopped | .Reclaimed => true
    | _ => false

private theorem vmBody_eq (rust : SandboxControllerRust.model.World)
    (index : SandboxControllerRust.model.ResourceIndex) (cursor : Std.Usize) :
    SandboxControllerRust.publication.indexed_vms_closed_loop.body rust index cursor =
      vectorAllBody index.vms (concreteVmClosed rust) cursor := by
  unfold SandboxControllerRust.publication.indexed_vms_closed_loop.body vectorAllBody
  by_cases bound : cursor.val < index.vms.val.length
  · have inBounds : cursor < alloc.vec.Vec.len index.vms := by simpa using bound
    obtain ⟨value, valueEval, valueSame⟩ := Aeneas.Std.WP.spec_imp_exists
      (alloc.vec.Vec.index_usize_spec index.vms cursor bound)
    subst value
    have get := generated_get_eq
      SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
      vmId_partialEq_lawful rust.vms (index.vms.val[cursor.val]'bound)
    simp only [inBounds, bound, if_pos, dif_pos]
    simp [valueEval]
    rw [get]
    cases found : storeLookup rust.vms (index.vms.val[cursor.val]'bound) with
    | none => simp [concreteVmClosed, found]
    | some value =>
        cases status : value.status <;>
          simp [concreteVmClosed, found, status, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
            SandboxControllerRust.model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq,
            SandboxControllerRust.model.VmStatus.read_discriminant]
  · have outside : ¬ cursor < alloc.vec.Vec.len index.vms := by simpa using bound
    simp [outside, bound]

theorem generated_indexedVmsClosed_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (sid : SandboxControllerRust.ids.SandboxId) :
    SandboxControllerRust.publication.indexed_vms_closed rust sid =
      ok (indexedVmsClosedCheck model (sandboxId sid)) := by
  obtain ⟨index, indexEval, indexSame⟩ := generated_resourceIndex_refines rust model represented sid
  have bodySame : SandboxControllerRust.publication.indexed_vms_closed_loop.body rust index =
      vectorAllBody index.vms (concreteVmClosed rust) := funext (vmBody_eq rust index)
  obtain ⟨found, foundEval, foundSame⟩ := Aeneas.Std.WP.spec_imp_exists
    (vectorAllLoop_refines index.vms (concreteVmClosed rust) 0#usize (by simp))
  have predicates (id) : concreteVmClosed rust id = vmStoppedOrReclaimedCheck model (vmId id) := by
    have lookup := storeRep_lookup_map represented.vms id
    unfold vmStoppedOrReclaimedCheck concreteVmClosed
    rw [lookup]
    cases storeLookup rust.vms id with
    | none => simp
    | some value => cases status : value.status <;> simp [status, vm, vmStatus]
  simp only [SandboxControllerRust.publication.indexed_vms_closed, indexEval]
  simp [SandboxControllerRust.publication.indexed_vms_closed_loop, bodySame, foundEval]
  subst found
  unfold indexedVmsClosedCheck
  rw [← indexSame]
  simp only [resourceIndex, List.all_map]
  congr 1
  funext id
  exact predicates id

theorem generated_stopReceiptCandidate_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (candidate : SandboxControllerRust.model.EffectId) :
    SandboxControllerRust.publication.stop_receipt_candidate rust sid generation vid candidate =
      ok (SandboxModel.Controller.stopReceiptCandidateCheck model (sandboxId sid)
        (u64ToNat generation) (vmId vid) (effectId candidate)) := by
  have scalarEq (left right : U64) : u64ToNat left = u64ToNat right ↔ left = right :=
    u64ToNat_injective.eq_iff
  cases candidate with
  | mk logical attempt =>
    cases logical with
    | mk input =>
      cases input <;>
        simp [SandboxControllerRust.publication.stop_receipt_candidate,
          SandboxControllerRust.model.EffectId.command,
          SandboxModel.Controller.stopReceiptCandidateCheck, EffectId.command,
          effectId, logicalEffectId, command,
          SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
          SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
          generated_effectSucceeded_refines rust model represented, scalarEq]
      repeat' (split <;> simp_all [command])

private theorem stopAnyBody_eq
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (cursor : Std.Usize) :
    SandboxControllerRust.publication.vm_has_stop_receipt_loop.body rust sid generation vid cursor =
      vectorAnyBody rust.effect_keys
        (fun id => SandboxModel.Controller.stopReceiptCandidateCheck model (sandboxId sid)
          (u64ToNat generation) (vmId vid) (effectId id)) cursor := by
  unfold SandboxControllerRust.publication.vm_has_stop_receipt_loop.body vectorAnyBody
  by_cases bound : cursor.val < rust.effect_keys.val.length
  · have inBounds : cursor < alloc.vec.Vec.len rust.effect_keys := by simpa using bound
    obtain ⟨value, valueEval, valueSame⟩ := Aeneas.Std.WP.spec_imp_exists
      (alloc.vec.Vec.index_usize_spec rust.effect_keys cursor bound)
    subst value
    simp [inBounds, bound, valueEval, generated_stopReceiptCandidate_refines rust model represented]
  · have outside : ¬ cursor < alloc.vec.Vec.len rust.effect_keys := by simpa using bound
    simp [outside, bound]

theorem generated_vmStopSucceeded_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (vid : SandboxControllerRust.ids.VmId) :
    SandboxControllerRust.publication.vm_has_stop_receipt rust sid generation vid =
      ok (SandboxModel.Controller.vmStopSucceededFiniteCheck model (sandboxId sid)
        (u64ToNat generation) (vmId vid)) := by
  let predicate := fun id => SandboxModel.Controller.stopReceiptCandidateCheck model (sandboxId sid)
    (u64ToNat generation) (vmId vid) (effectId id)
  have bodySame : SandboxControllerRust.publication.vm_has_stop_receipt_loop.body rust sid generation vid =
      vectorAnyBody rust.effect_keys predicate :=
    funext (stopAnyBody_eq rust model represented sid generation vid)
  obtain ⟨found, foundEval, foundSame⟩ := Aeneas.Std.WP.spec_imp_exists
    (vectorAnyLoop_refines rust.effect_keys predicate 0#usize (by simp))
  subst found
  simp [SandboxControllerRust.publication.vm_has_stop_receipt,
    SandboxControllerRust.publication.vm_has_stop_receipt_loop, bodySame, foundEval]
  unfold SandboxModel.Controller.vmStopSucceededFiniteCheck
  rw [← represented.effectKeys]
  simp [predicate, List.any_map, valueRelations]
  rfl

private def stopPredicate (model : World) (sid : SandboxId)
    (id : SandboxControllerRust.ids.VmId) : Bool :=
  match model.vms (vmId id) with
  | none => false
  | some value => SandboxModel.Controller.vmStopSucceededFiniteCheck model sid value.generation (vmId id)

private theorem stopsBody_eq
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId)
    (index : SandboxControllerRust.model.ResourceIndex) (cursor : Std.Usize) :
    SandboxControllerRust.publication.indexed_stops_succeeded_loop.body rust sid index cursor =
      vectorAllBody index.vms (stopPredicate model (sandboxId sid)) cursor := by
  unfold SandboxControllerRust.publication.indexed_stops_succeeded_loop.body vectorAllBody
  by_cases bound : cursor.val < index.vms.val.length
  · have inBounds : cursor < alloc.vec.Vec.len index.vms := by simpa using bound
    obtain ⟨value, valueEval, valueSame⟩ := Aeneas.Std.WP.spec_imp_exists
      (alloc.vec.Vec.index_usize_spec index.vms cursor bound)
    subst value
    have get := generated_get_eq
      SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
      vmId_partialEq_lawful rust.vms (index.vms.val[cursor.val]'bound)
    have lookup := storeRep_lookup_map represented.vms (index.vms.val[cursor.val]'bound)
    simp only [inBounds, bound, if_pos, dif_pos]
    simp [valueEval]
    rw [get]
    unfold stopPredicate
    rw [lookup]
    cases found : storeLookup rust.vms (index.vms.val[cursor.val]'bound) with
    | none => simp
    | some value => simp [generated_vmStopSucceeded_refines rust model represented, vm]
  · have outside : ¬ cursor < alloc.vec.Vec.len index.vms := by simpa using bound
    simp [outside, bound]

theorem generated_indexedStopsSucceeded_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (sid : SandboxControllerRust.ids.SandboxId) :
    SandboxControllerRust.publication.indexed_stops_succeeded rust sid =
      ok (SandboxModel.Controller.indexedAnyStopsSucceededCheck model (sandboxId sid)) := by
  obtain ⟨index, indexEval, indexSame⟩ := generated_resourceIndex_refines rust model represented sid
  have bodySame : SandboxControllerRust.publication.indexed_stops_succeeded_loop.body rust sid index =
      vectorAllBody index.vms (stopPredicate model (sandboxId sid)) :=
    funext (stopsBody_eq rust model represented sid index)
  obtain ⟨found, foundEval, foundSame⟩ := Aeneas.Std.WP.spec_imp_exists
    (vectorAllLoop_refines index.vms (stopPredicate model (sandboxId sid)) 0#usize (by simp))
  subst found
  simp [SandboxControllerRust.publication.indexed_stops_succeeded, indexEval,
    SandboxControllerRust.publication.indexed_stops_succeeded_loop, bodySame, foundEval]
  unfold SandboxModel.Controller.indexedAnyStopsSucceededCheck
  rw [← indexSame]
  simp [resourceIndex, List.all_map, stopPredicate, Function.comp_def]
  rfl

end SandboxModel.RustRefinement
