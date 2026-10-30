import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

/-! Actual VM/index writes used by the Stop worker. No physical exit, owner,
generation or global uniqueness property is assumed or established here.
The same first-match Store update and prepend-if-absent index operation remain
visible. The proof owner checks this new file before registering its audits. -/
namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

theorem register_vm_shape (world next : model.World)
    (sandbox : ids.SandboxId) (vm : ids.VmId)
    (evaluated : worker.register_vm world sandbox vm = .ok next) :
    ∃ indexes, next = { world with resource_indexes := indexes } := by
  unfold worker.register_vm at evaluated
  obtain ⟨index, _read, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨values, _insert, tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨indexes, _write, tail⟩ := bind_ok_inv _ _ _ tail
  exact ⟨indexes, (by simpa using tail.symm)⟩

theorem put_vm_shape (world next : model.World) (vm : ids.VmId) (value : model.Vm)
    (evaluated : worker.put_vm world vm value = .ok next) :
    ∃ indexes vms, next = { world with resource_indexes := indexes, vms := vms } := by
  unfold worker.put_vm at evaluated
  obtain ⟨registered, registeredEq, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨indexes, registeredShape⟩ :=
    register_vm_shape world registered value.sandbox vm registeredEq
  obtain ⟨vms, _write, tail⟩ := bind_ok_inv _ _ _ tail
  subst registered
  exact ⟨indexes, vms, (by simpa using tail.symm)⟩

theorem vm_id_lawful : TableLawfulEq ids.VmId.Insts.CoreCmpPartialEqVmId := by
  intro left right
  rfl

/-- Existing VM replacement needs no spare VM-table row. The index writer is
still real; its successful result is kept rather than assumed unchanged. -/
theorem put_existing_vm_exact (world next : model.World) (vm : ids.VmId)
    (old value : model.Vm)
    (present : tableLookup vm world.vms.entries.val = some old)
    (evaluated : worker.put_vm world vm value = .ok next) :
    ∃ indexes vms, next = { world with resource_indexes := indexes, vms := vms } ∧
      vms.entries.val = tablePut vm value world.vms.entries.val := by
  unfold worker.put_vm at evaluated
  obtain ⟨registered, registeredEq, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨indexes, registeredShape⟩ :=
    register_vm_shape world registered value.sandbox vm registeredEq
  obtain ⟨vms, written, entries⟩ := put_exact
    ids.VmId.Insts.CoreCmpPartialEqVmId vm_id_lawful world.vms vm value
    (Or.inl (by rw [present]; simp))
  subst registered
  rw [written] at tail
  have same : { world with resource_indexes := indexes, vms := vms } = next := by
    simpa using tail
  subst next
  exact ⟨indexes, vms, rfl, entries⟩

/-- Query behavior follows the first matching row, without a Nodup premise. -/
theorem put_existing_vm_queries (world next : model.World) (vm : ids.VmId)
    (old value : model.Vm)
    (present : tableLookup vm world.vms.entries.val = some old)
    (evaluated : worker.put_vm world vm value = .ok next) :
    ∀ query, tableLookup query next.vms.entries.val =
      if query = vm then some value else tableLookup query world.vms.entries.val := by
  obtain ⟨indexes, vms, shape, entries⟩ :=
    put_existing_vm_exact world next vm old value present evaluated
  subst next
  intro query
  dsimp only
  rw [entries, table_lookup_put]

/-- Fixed-generation representation bridge only: these extracted ID types
are U64 and their actual clone/equality implementations compute identically.
This reuses CallerIndex's real loop proof, not a new generic insertion model. -/
theorem vm_index_insert_agrees (values : alloc.vec.Vec ids.VmId) (vm : ids.VmId) :
    worker.insert_unique ids.VmId.Insts.CoreCloneClone
      ids.VmId.Insts.CoreCmpPartialEqVmId values vm =
    worker.insert_unique ids.SnapshotId.Insts.CoreCloneClone
      ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId values vm := by
  rfl

theorem vm_index_insert_refines (values : alloc.vec.Vec ids.VmId) (vm : ids.VmId)
    (capacity : vm ∈ values.val ∨ values.val.length < Usize.max) :
    worker.insert_unique ids.VmId.Insts.CoreCloneClone
      ids.VmId.Insts.CoreCmpPartialEqVmId values vm
      ⦃ result => result.val = prependIfAbsent vm values.val ⦄ := by
  rw [vm_index_insert_agrees]
  exact caller_index_insert_refines values vm capacity

/-- The target index row is an actual first-match Store write. A missing VM
is prepended; an existing VM leaves its vector unchanged. Slots, snapshots,
other sandbox rows and every other World field retain their original values. -/
theorem register_vm_index_exact (world : model.World) (sandbox : ids.SandboxId)
    (vm : ids.VmId)
    (capacity : vm ∈ (caller_index_view world sandbox).vms.val ∨
      (caller_index_view world sandbox).vms.val.length < Usize.max)
    (tableRoom : TableWriteCapacity world.resource_indexes.entries.val sandbox) :
    ∃ next index, worker.register_vm world sandbox vm = .ok next ∧
      next = { world with resource_indexes := next.resource_indexes } ∧
      next.resource_indexes.entries.val =
        tablePut sandbox index world.resource_indexes.entries.val ∧
      tableLookup sandbox next.resource_indexes.entries.val = some index ∧
      index.vms.val = prependIfAbsent vm (caller_index_view world sandbox).vms.val ∧
      index.slots = (caller_index_view world sandbox).slots ∧
      index.snapshots = (caller_index_view world sandbox).snapshots ∧
      (∀ other, other ≠ sandbox → tableLookup other next.resource_indexes.entries.val =
        tableLookup other world.resource_indexes.entries.val) := by
  obtain ⟨vms, inserted, post⟩ := WP.spec_imp_exists
    (vm_index_insert_refines (caller_index_view world sandbox).vms vm capacity)
  let index := { caller_index_view world sandbox with vms := vms }
  obtain ⟨indexes, written, entries⟩ := put_exact _ caller_index_sandbox_eq_lawful
    world.resource_indexes sandbox index tableRoom
  refine ⟨{ world with resource_indexes := indexes }, index,
    ?_, rfl, entries, ?_, post, rfl, rfl, ?_⟩
  · simp only [worker.register_vm, caller_index_resource_index_exact, bind_tc_ok, inserted]
    simp only [index, written, bind_tc_ok]
  · rw [entries, table_lookup_put]
    simp
  · intro other different
    rw [entries, table_lookup_put]
    simp [different]

def VmIndexKept (before after : model.World) (sandbox : ids.SandboxId)
    (vm : ids.VmId) : Prop :=
  ∃ index, after.resource_indexes.entries.val =
      tablePut sandbox index before.resource_indexes.entries.val ∧
    tableLookup sandbox after.resource_indexes.entries.val = some index ∧
    index.vms.val = prependIfAbsent vm (caller_index_view before sandbox).vms.val ∧
    index.slots = (caller_index_view before sandbox).slots ∧
    index.snapshots = (caller_index_view before sandbox).snapshots ∧
    (∀ other, other ≠ sandbox → tableLookup other after.resource_indexes.entries.val =
      tableLookup other before.resource_indexes.entries.val)

theorem put_vm_index_kept (world next : model.World) (vm : ids.VmId) (value : model.Vm)
    (capacity : vm ∈ (caller_index_view world value.sandbox).vms.val ∨
      (caller_index_view world value.sandbox).vms.val.length < Usize.max)
    (tableRoom : TableWriteCapacity world.resource_indexes.entries.val value.sandbox)
    (evaluated : worker.put_vm world vm value = .ok next) :
    VmIndexKept world next value.sandbox vm := by
  unfold worker.put_vm at evaluated
  obtain ⟨registered, registeredEq, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨indexed, index, indexEq, _shape, entries, present, vmsPost, slots, snapshots, others⟩ :=
    register_vm_index_exact world value.sandbox vm capacity tableRoom
  have same : indexed = registered := by
    simpa [registeredEq] using indexEq.symm
  subst indexed
  obtain ⟨vms, _write, returned⟩ := bind_ok_inv _ _ _ tail
  have output : { registered with vms := vms } = next := by simpa using returned
  subst next
  exact ⟨index, entries, present, vmsPost, slots, snapshots, others⟩

/-- The final two-table frame excludes changes to sandboxes, authority,
operations, effects, slots, snapshots, fences and every other World field. -/
theorem put_vm_frame (world next : model.World) (vm : ids.VmId) (value : model.Vm)
    (evaluated : worker.put_vm world vm value = .ok next) :
    next = { world with vms := next.vms, resource_indexes := next.resource_indexes } := by
  obtain ⟨indexes, vms, shape⟩ := put_vm_shape world next vm value evaluated
  subst next
  rfl

end SandboxModel.Lifecycle.PauseStop
