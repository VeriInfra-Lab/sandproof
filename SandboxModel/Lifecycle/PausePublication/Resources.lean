import SandboxModel.Lifecycle.CaptureHandoff.CallerIndex
import SandboxModel.Foundations.FiniteScan

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def vmClosed (world : model.World) (id : ids.VmId) : Bool :=
  match tableLookup id world.vms.entries.val with
  | none => false
  | some vm => match vm.status with
    | .Stopped | .Reclaimed => true
    | _ => false

def slotClosed (world : model.World) (id : ids.NetSlotId) : Bool :=
  match tableLookup id world.net_slots.entries.val with
  | none => false
  | some slot => !slot.active

theorem vm_id_lawful : TableLawfulEq ids.VmId.Insts.CoreCmpPartialEqVmId := by
  intro left right
  rfl

theorem slot_id_lawful : TableLawfulEq ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId := by
  intro left right
  rfl

theorem vms_body (world : model.World) (index : model.ResourceIndex) (cursor : Usize) :
    publication.indexed_vms_closed_loop.body world index cursor =
      vectorAllBody index.vms (vmClosed world) cursor := by
  unfold publication.indexed_vms_closed_loop.body vectorAllBody
  by_cases bound : cursor.val < index.vms.val.length
  · have inside : cursor < alloc.vec.Vec.len index.vms := by simpa using bound
    simp only [if_pos inside, dif_pos bound]
    obtain ⟨value, evaluated, post⟩ := WP.spec_imp_exists
      (alloc.vec.Vec.index_usize_spec index.vms cursor bound)
    rw [alloc.vec.Vec.index_slice_index, evaluated]
    simp only [bind_tc_ok, get_exact _ vm_id_lawful]
    subst value
    simp only [vmClosed]
    split
    · simp_all
    · rename_i vm found
      cases status : vm.status <;>
        simp_all [core.cmp.PartialEq.ne.trait_default,
          core.cmp.PartialEq.ne.default,
          model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq, model.VmStatus.read_discriminant]
  · have outside : ¬ cursor < alloc.vec.Vec.len index.vms := by simpa using bound
    simp only [if_neg outside, dif_neg bound]

theorem slots_body (world : model.World) (index : model.ResourceIndex) (cursor : Usize) :
    publication.indexed_slots_closed_loop.body world index cursor =
      vectorAllBody index.slots (slotClosed world) cursor := by
  unfold publication.indexed_slots_closed_loop.body vectorAllBody
  by_cases bound : cursor.val < index.slots.val.length
  · have inside : cursor < alloc.vec.Vec.len index.slots := by simpa using bound
    simp only [if_pos inside, dif_pos bound]
    obtain ⟨value, evaluated, post⟩ := WP.spec_imp_exists
      (alloc.vec.Vec.index_usize_spec index.slots cursor bound)
    rw [alloc.vec.Vec.index_slice_index, evaluated]
    simp only [bind_tc_ok, get_exact _ slot_id_lawful]
    subst value
    simp only [slotClosed]
    split
    · simp_all
    · rename_i slot found
      cases active : slot.active <;> simp_all
  · have outside : ¬ cursor < alloc.vec.Vec.len index.slots := by simpa using bound
    simp only [if_neg outside, dif_neg bound]

theorem vms_loop_exact (world : model.World) (index : model.ResourceIndex) (cursor : Usize)
    (bound : cursor.val ≤ index.vms.val.length) :
    publication.indexed_vms_closed_loop world index cursor =
      .ok ((index.vms.val.drop cursor.val).all (vmClosed world)) := by
  obtain ⟨out, evaluated, post⟩ := WP.spec_imp_exists
    (vectorAllLoop_refines index.vms (vmClosed world) cursor bound)
  subst out
  simpa only [publication.indexed_vms_closed_loop, vms_body] using evaluated

theorem slots_loop_exact (world : model.World) (index : model.ResourceIndex) (cursor : Usize)
    (bound : cursor.val ≤ index.slots.val.length) :
    publication.indexed_slots_closed_loop world index cursor =
      .ok ((index.slots.val.drop cursor.val).all (slotClosed world)) := by
  obtain ⟨out, evaluated, post⟩ := WP.spec_imp_exists
    (vectorAllLoop_refines index.slots (slotClosed world) cursor bound)
  subst out
  simpa only [publication.indexed_slots_closed_loop, slots_body] using evaluated

@[simp] theorem indexed_vms_closed_exact (world : model.World) (sandbox : ids.SandboxId) :
    publication.indexed_vms_closed world sandbox =
      .ok ((caller_index_view world sandbox).vms.val.all (vmClosed world)) := by
  simp only [publication.indexed_vms_closed, caller_index_resource_index_exact, bind_tc_ok]
  simpa using vms_loop_exact world (caller_index_view world sandbox) 0#usize (by simp)

@[simp] theorem indexed_slots_closed_exact (world : model.World) (sandbox : ids.SandboxId) :
    publication.indexed_slots_closed world sandbox =
      .ok ((caller_index_view world sandbox).slots.val.all (slotClosed world)) := by
  simp only [publication.indexed_slots_closed, caller_index_resource_index_exact, bind_tc_ok]
  simpa using slots_loop_exact world (caller_index_view world sandbox) 0#usize (by simp)

end SandboxModel.Lifecycle.PausePublication
