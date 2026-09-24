import SandboxModel.RustRefinement.WorkerCleanupVm

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerActivationCheckOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerActivationCheckLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerActivationCheckEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq


private def workerVmNotExecuting (rust : SandboxControllerRust.model.World)
    (id : SandboxControllerRust.ids.VmId) : Bool :=
  match storeLookup rust.vms id with
  | none => false
  | some value => match value.status with
    | .Executing => false
    | _ => true

private theorem workerVmNotExecutingBody_eq (rust : SandboxControllerRust.model.World)
    (index : SandboxControllerRust.model.ResourceIndex) (cursor : Std.Usize) :
    SandboxControllerRust.worker.indexed_no_executing_vm_loop.body rust index cursor =
      vectorAllBody index.vms (workerVmNotExecuting rust) cursor := by
  unfold SandboxControllerRust.worker.indexed_no_executing_vm_loop.body vectorAllBody
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
    | none => simp [workerVmNotExecuting, found]
    | some value =>
        cases status : value.status <;>
          simp [workerVmNotExecuting, found, status, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
            SandboxControllerRust.model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq,
            SandboxControllerRust.model.VmStatus.read_discriminant]
  · have outside : ¬ cursor < alloc.vec.Vec.len index.vms := by simpa using bound
    simp [outside, bound]

theorem generated_workerNoExecutingVm_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (sid : SandboxControllerRust.ids.SandboxId) :
    SandboxControllerRust.worker.indexed_no_executing_vm rust sid =
      ok (Controller.indexedNoExecutingVmCheck model (sandboxId sid)) := by
  obtain ⟨index, indexEval, indexSame⟩ := generated_resourceIndex_refines rust model represented sid
  have bodySame : SandboxControllerRust.worker.indexed_no_executing_vm_loop.body rust index =
      vectorAllBody index.vms (workerVmNotExecuting rust) := funext (workerVmNotExecutingBody_eq rust index)
  obtain ⟨found, foundEval, foundSame⟩ := Aeneas.Std.WP.spec_imp_exists
    (vectorAllLoop_refines index.vms (workerVmNotExecuting rust) 0#usize (by simp))
  have predicates (id) : workerVmNotExecuting rust id = Controller.vmNotExecutingCheck model (vmId id) := by
    have lookup := storeRep_lookup_map represented.vms id
    unfold Controller.vmNotExecutingCheck workerVmNotExecuting
    rw [lookup]
    cases storeLookup rust.vms id with
    | none => simp
    | some value => cases status : value.status <;> simp [status, vm, vmStatus]
  simp only [SandboxControllerRust.worker.indexed_no_executing_vm, indexEval]
  simp [SandboxControllerRust.worker.indexed_no_executing_vm_loop, bodySame, foundEval]
  subst found
  unfold Controller.indexedNoExecutingVmCheck
  rw [← indexSame]
  simp only [resourceIndex, List.all_map]
  congr 1
  funext id
  exact predicates id

end SandboxModel.RustRefinement
