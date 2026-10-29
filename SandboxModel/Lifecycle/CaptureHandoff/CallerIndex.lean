import SandboxModel.Lifecycle.CaptureHandoff.CallerStores
import SandboxModel.Lifecycle.CaptureHandoff.CallerCloning
import SandboxModel.Foundations.FiniteInsert

/-! The actual worker helper prepends a missing snapshot; it does not append.
Its inner loop carries (cursor, vector), unlike the existing transition loop.
The finite-loop argument below follows FiniteCopy with that actual state order.
Only ordinary values and logical capacity are covered, not physical allocation,
material existence, resource release or a permission to collect snapshots. -/
namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations

@[simp] theorem caller_index_snapshot_eq_exact
    (left right : sandbox_controller_core.ids.SnapshotId) :
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq left right =
      .ok (decide (left = right)) := by
  rfl

theorem caller_index_snapshot_eq_lawful :
    ValueEqExact sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId := by
  intro left right
  exact caller_index_snapshot_eq_exact left right

theorem caller_index_sandbox_eq_lawful :
    TableLawfulEq sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId := by
  intro left right
  rfl

@[step] theorem caller_index_copy_refines
    (values prepared : alloc.vec.Vec sandbox_controller_core.ids.SnapshotId)
    (capacity : prepared.val.length + values.val.length ≤ Usize.max) :
    sandbox_controller_core.worker.insert_unique_loop0_loop0
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCloneClone values 0#usize prepared
      ⦃ result => result.val = prepared.val ++ values.val ⦄ := by
  unfold sandbox_controller_core.worker.insert_unique_loop0_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : Usize × alloc.vec.Vec sandbox_controller_core.ids.SnapshotId =>
      values.val.length - state.1.val)
    (fun state : Usize × alloc.vec.Vec sandbox_controller_core.ids.SnapshotId =>
      state.1.val ≤ values.val.length ∧
      state.2.val = prepared.val ++ values.val.take state.1.val)
    (fun (result : alloc.vec.Vec sandbox_controller_core.ids.SnapshotId) =>
      result.val = prepared.val ++ values.val)
  · intro state invariant
    rcases state with ⟨index, current⟩
    rcases invariant with ⟨indexBound, currentEq⟩
    dsimp only [Prod.fst, Prod.snd] at indexBound currentEq
    unfold sandbox_controller_core.worker.insert_unique_loop0_loop0.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨value, valuePost⟩
      simp only [ids_SnapshotId_clone_exact, bind_tc_ok]
      have indexStrict : index.val < values.val.length := by simpa using inBounds
      have strict : current.val.length < Usize.max := by
        rw [currentEq, List.length_append, List.length_take]
        simp only [Nat.min_eq_left indexBound]
        omega
      step as ⟨pushed, pushedPost⟩
      step as ⟨nextIndex, nextIndexPost⟩
      have takeStep : values.val.take (index.val + 1) =
          values.val.take index.val ++ [value] := by
        rw [List.take_succ_eq_append_getElem indexStrict, ← valuePost]
      constructor
      · scalar_tac
      · constructor
        · rw [nextIndexPost, pushedPost, currentEq, takeStep, List.append_assoc]
        · rw [nextIndexPost]
          omega
    · rename_i outside
      simp only [WP.spec_ok]
      have lower : values.val.length ≤ index.val := by simpa using outside
      have same : index.val = values.val.length := by omega
      simpa [same] using currentEq
  · exact ⟨by simp, by simp⟩

@[step] theorem caller_index_insert_refines
    (values : alloc.vec.Vec sandbox_controller_core.ids.SnapshotId)
    (value : sandbox_controller_core.ids.SnapshotId)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max) :
    sandbox_controller_core.worker.insert_unique
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCloneClone
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId values value
      ⦃ result => result.val = prependIfAbsent value values.val ⦄ := by
  unfold sandbox_controller_core.worker.insert_unique
    sandbox_controller_core.worker.insert_unique_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index : Usize => values.val.length - index.val)
    (fun index : Usize => index.val ≤ values.val.length ∧ value ∉ values.val.take index.val)
    (fun (result : alloc.vec.Vec sandbox_controller_core.ids.SnapshotId) =>
      result.val = prependIfAbsent value values.val)
  · intro index invariant
    rcases invariant with ⟨indexBound, absentPrefix⟩
    unfold sandbox_controller_core.worker.insert_unique_loop0.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨stored, storedPost⟩
      simp only [caller_index_snapshot_eq_exact, bind_tc_ok]
      by_cases same : stored = value
      · simp [same]
        simp only [prependIfAbsent]
        have present : value ∈ values.val := by
          rw [← same, storedPost]
          exact List.getElem_mem (l := values.val) (by simpa using inBounds)
        simp [present]
      · simp [same]
        step as ⟨nextIndex, nextIndexPost⟩
        have indexStrict : index.val < values.val.length := by simpa using inBounds
        constructor
        · rw [nextIndexPost]
          omega
        · constructor
          · rw [nextIndexPost, List.take_succ_eq_append_getElem indexStrict, ← storedPost]
            simp [absentPrefix]
            exact fun equal => same (UScalar.eq_of_val_eq equal.symm)
          · rw [nextIndexPost]
            omega
    · rename_i outside
      have lower : values.val.length ≤ index.val := by simpa using outside
      have sameIndex : index.val = values.val.length := by omega
      have absent : value ∉ values.val := by simpa [sameIndex] using absentPrefix
      have room : values.val.length < Usize.max := capacity.resolve_left absent
      have emptyRoom : (alloc.vec.Vec.new sandbox_controller_core.ids.SnapshotId).val.length <
          Usize.max := by
        simp [alloc.vec.Vec.new]
        scalar_tac
      step as ⟨initial, initialPost⟩
      have initialLength : initial.val.length = 1 := by
        rw [initialPost]
        simp [alloc.vec.Vec.new]
      have copyRoom : initial.val.length + values.val.length ≤ Usize.max := by
        rw [initialLength]
        omega
      step as ⟨result, resultPost⟩
      simpa [prependIfAbsent, absent, initialPost, alloc.vec.Vec.new] using resultPost
  · exact ⟨by simp, by simp⟩

theorem caller_index_insert_exact
    (values : alloc.vec.Vec sandbox_controller_core.ids.SnapshotId)
    (value : sandbox_controller_core.ids.SnapshotId)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max) :
    ∃ result, sandbox_controller_core.worker.insert_unique
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCloneClone
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId values value =
        .ok result ∧ result.val = (if value ∈ values.val then values.val else value :: values.val) := by
  obtain ⟨result, evaluated, post⟩ :=
    WP.spec_imp_exists (caller_index_insert_refines values value capacity)
  refine ⟨result, evaluated, ?_⟩
  by_cases present : value ∈ values.val
  · simpa [prependIfAbsent, present] using post
  · simpa [prependIfAbsent, present] using post

theorem caller_index_insert_membership
    (values : alloc.vec.Vec sandbox_controller_core.ids.SnapshotId)
    (value : sandbox_controller_core.ids.SnapshotId)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max) :
    ∃ result, sandbox_controller_core.worker.insert_unique
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCloneClone
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId values value =
        .ok result ∧
      ∀ query, query ∈ result.val ↔ query = value ∨ query ∈ values.val := by
  obtain ⟨result, evaluated, post⟩ :=
    WP.spec_imp_exists (caller_index_insert_refines values value capacity)
  refine ⟨result, evaluated, ?_⟩
  intro query
  rw [post]
  exact prependIfAbsent_membership value query values.val

def caller_index_empty : sandbox_controller_core.model.ResourceIndex :=
  ⟨alloc.vec.Vec.new _, alloc.vec.Vec.new _, alloc.vec.Vec.new _⟩

def caller_index_view (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) : sandbox_controller_core.model.ResourceIndex :=
  (tableLookup sandbox world.resource_indexes.entries.val).getD caller_index_empty

@[simp] theorem caller_index_resource_index_exact
    (world : sandbox_controller_core.model.World) (sandbox : sandbox_controller_core.ids.SandboxId) :
    sandbox_controller_core.model.World.resource_index world sandbox =
      .ok (caller_index_view world sandbox) := by
  unfold sandbox_controller_core.model.World.resource_index
  rw [get_exact _ caller_index_sandbox_eq_lawful]
  cases observed : tableLookup sandbox world.resource_indexes.entries.val <;>
    simp [caller_index_view, observed, caller_index_empty,
      core.option.OptionShared0T.cloned, core.option.Option.Insts.CoreCloneClone.clone,
      core.option.Option.unwrap_or_default,
      sandbox_controller_core.model.ResourceIndex.Insts.CoreDefaultDefault.default]

/-- Real index registration preserves old members and all other index rows.
The separate snapshots table is untouched; this is not material/GC evidence. -/
theorem caller_index_register_membership
    (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (capacity : snapshot ∈ (caller_index_view world sandbox).snapshots.val ∨
      (caller_index_view world sandbox).snapshots.val.length < Usize.max)
    (tableRoom : TableWriteCapacity world.resource_indexes.entries.val sandbox) :
    ∃ next index, sandbox_controller_core.worker.register_snapshot world sandbox snapshot = .ok next ∧
      tableLookup sandbox next.resource_indexes.entries.val = some index ∧
      index.vms = (caller_index_view world sandbox).vms ∧
      index.slots = (caller_index_view world sandbox).slots ∧
      (∀ query, query ∈ index.snapshots.val ↔
        query = snapshot ∨ query ∈ (caller_index_view world sandbox).snapshots.val) ∧
      (∀ other, other ≠ sandbox → tableLookup other next.resource_indexes.entries.val =
        tableLookup other world.resource_indexes.entries.val) ∧
      next.snapshots = world.snapshots := by
  obtain ⟨snapshots, inserted, post⟩ := WP.spec_imp_exists
    (caller_index_insert_refines (caller_index_view world sandbox).snapshots snapshot capacity)
  let index := { caller_index_view world sandbox with snapshots := snapshots }
  obtain ⟨indexes, written, entries⟩ := put_exact _ caller_index_sandbox_eq_lawful
    world.resource_indexes sandbox index tableRoom
  refine ⟨{ world with resource_indexes := indexes }, index, ?_, ?_, rfl, rfl, ?_, ?_, rfl⟩
  · simp only [sandbox_controller_core.worker.register_snapshot,
      caller_index_resource_index_exact, bind_tc_ok, inserted]
    simp only [index, written, bind_tc_ok]
  · rw [entries, table_lookup_put]
    simp
  · intro query
    change query ∈ snapshots.val ↔ _
    rw [post]
    exact prependIfAbsent_membership snapshot query _
  · intro other different
    rw [entries, table_lookup_put]
    simp [different]

end SandboxModel.Lifecycle.CaptureHandoffCaller
