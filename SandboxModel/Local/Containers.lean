import SandboxModel.Access.Lookup
import SandboxModel.Access.Cloning
import SandboxModel.Foundations.FiniteInsert
import SandboxModel.Foundations.TableWriteContracts

/-! Actual current Rust container helpers connected to fixed V0 contracts.
All equality/clone dictionaries and logical append headroom are caller-owned.
These are not complete creation/queueing or physical allocation guarantees. -/
namespace SandboxModel.Local.Containers
open Aeneas Aeneas.Std SandproofNetworkRust SandboxModel.Foundations

theorem copy_body_exact (cloner : core.clone.Clone T) (values : alloc.vec.Vec T)
    (state : alloc.vec.Vec T × Usize) :
    sandbox_controller_core.transition.insert_unique_loop0_loop0.body
      cloner values state.1 state.2 = vectorCopyBody cloner values state := by
  rfl

theorem copy_loop_exact (cloner : core.clone.Clone T) (values prepared : alloc.vec.Vec T) :
    sandbox_controller_core.transition.insert_unique_loop0_loop0 cloner values prepared 0#usize =
      vectorCopyLoop cloner values prepared := by
  unfold sandbox_controller_core.transition.insert_unique_loop0_loop0 vectorCopyLoop
  rfl

theorem insert_body_exact (cloner : core.clone.Clone T) (eq : core.cmp.PartialEq T T)
    (values : alloc.vec.Vec T) (value : T) (index : Usize) :
    sandbox_controller_core.transition.insert_unique_loop0.body cloner eq values value index =
      vectorInsertBody cloner eq values value index := by
  unfold sandbox_controller_core.transition.insert_unique_loop0.body vectorInsertBody vectorPrepend
  simp only [copy_loop_exact, LawfulMonad.bind_assoc]

theorem insert_root_exact (cloner : core.clone.Clone T) (eq : core.cmp.PartialEq T T)
    (values : alloc.vec.Vec T) (value : T) :
    sandbox_controller_core.transition.insert_unique cloner eq values value =
      vectorInsert cloner eq values value := by
  unfold sandbox_controller_core.transition.insert_unique
    sandbox_controller_core.transition.insert_unique_loop0 vectorInsert
  simp only [insert_body_exact]

theorem insert_refines [DecidableEq T] (cloner : core.clone.Clone T)
    (values : alloc.vec.Vec T) (value : T)
    (clones : value ∉ values.val → CloneExact cloner)
    (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max) :
    sandbox_controller_core.transition.insert_unique cloner eq values value
      ⦃ result => result.val = prependIfAbsent value values.val ⦄ := by
  rw [insert_root_exact]
  exact vectorInsert_refines cloner values value clones eq lawful capacity

theorem insert_present [DecidableEq T] (cloner : core.clone.Clone T)
    (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (values : alloc.vec.Vec T) (value : T) (present : value ∈ values.val) :
    sandbox_controller_core.transition.insert_unique cloner eq values value
      ⦃ result => result.val = values.val ⦄ := by
  rw [insert_root_exact]
  exact vectorInsert_present_refines cloner eq lawful values value present

theorem insert_absent [DecidableEq T] (cloner : core.clone.Clone T)
    (clones : CloneExact cloner) (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (values : alloc.vec.Vec T) (value : T) (absent : value ∉ values.val)
    (capacity : values.val.length < Usize.max) :
    sandbox_controller_core.transition.insert_unique cloner eq values value
      ⦃ result => result.val = value :: values.val ⦄ := by
  rw [insert_root_exact]
  exact vectorInsert_absent_refines cloner clones eq lawful values value absent capacity

theorem insert_unique_preserved [DecidableEq T] (cloner : core.clone.Clone T)
    (values : alloc.vec.Vec T) (value : T)
    (clones : value ∉ values.val → CloneExact cloner)
    (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max)
    (unique : values.val.Nodup) :
    sandbox_controller_core.transition.insert_unique cloner eq values value
      ⦃ result => result.val.Nodup ⦄ := by
  apply WP.spec_mono (insert_refines cloner values value clones eq lawful capacity)
  intro result same
  rw [same]
  exact prependIfAbsent_unique value values.val unique

theorem insert_view [DecidableEq T] [DecidableEq M]
    (view : T → M) (injective : Function.Injective view)
    (cloner : core.clone.Clone T) (values : alloc.vec.Vec T) (value : T)
    (clones : value ∉ values.val → CloneExact cloner)
    (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max) :
    sandbox_controller_core.transition.insert_unique cloner eq values value
      ⦃ result => result.val.map view = prependIfAbsent (view value) (values.val.map view) ⦄ := by
  apply WP.spec_mono (insert_refines cloner values value clones eq lawful capacity)
  intro result same
  rw [same, prependIfAbsent_view view injective]

theorem insert_nonempty [DecidableEq T] (cloner : core.clone.Clone T)
    (clones : CloneExact cloner) (eq : core.cmp.PartialEq T T)
    (lawful : ValueEqExact eq) (value : T) :
    ∃ result, sandbox_controller_core.transition.insert_unique cloner eq (alloc.vec.Vec.new T) value =
      .ok result ∧ result.val = [value] := by
  rw [insert_root_exact]
  exact vectorInsert_init_realizable cloner clones eq lawful value

theorem insert_body_compare_failure (cloner : core.clone.Clone T)
    (eq : core.cmp.PartialEq T T) (values : alloc.vec.Vec T) (value : T)
    (index : Usize) (error : Error) (bound : index.val < values.val.length)
    (failed : eq.eq values.val[index.val] value = .fail error) :
    sandbox_controller_core.transition.insert_unique_loop0.body cloner eq values value index =
      .fail error := by
  rw [insert_body_exact]
  exact vectorInsertBody_compare_failure cloner eq values value index error bound failed

theorem copy_body_clone_failure (cloner : core.clone.Clone T)
    (values current : alloc.vec.Vec T) (index : Usize) (error : Error)
    (bound : index.val < values.val.length)
    (failed : cloner.clone values.val[index.val] = .fail error) :
    sandbox_controller_core.transition.insert_unique_loop0_loop0.body cloner values current index =
      .fail error := by
  change sandbox_controller_core.transition.insert_unique_loop0_loop0.body cloner values
    (current, index).1 (current, index).2 = _
  rw [copy_body_exact]
  exact vectorCopyBody_clone_failure cloner values current index error bound failed

theorem put_body_exact (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V) (index : Usize) :
    sandbox_controller_core.store.Store.put_loop.body eq store key value index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.put_loop.body
        eq (SandboxModel.Access.foundationStore store) key value index := by
  rfl

theorem put_loop_exact (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V) (index : Usize) :
    sandbox_controller_core.store.Store.put_loop eq store key value index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.put_loop
        eq (SandboxModel.Access.foundationStore store) key value index := by
  unfold sandbox_controller_core.store.Store.put_loop
    SandproofFoundationsRust.sandbox_controller_core.store.Store.put_loop
  simp only [put_body_exact]

theorem put_refines [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (value : V) (capacity : TableWriteCapacity store.entries.val key) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => next.entries.val = tablePut key value store.entries.val ⦄ := by
  unfold sandbox_controller_core.store.Store.put
  rw [put_loop_exact]
  apply WP.spec_bind (table_put_loop_refines eq lawful (SandboxModel.Access.foundationStore store)
    key value 0#usize (by simp [SandboxModel.Access.foundationStore]) (by
      simpa [TableWriteCapacity, SandboxModel.Access.foundationStore] using capacity))
  intro entries same
  simpa [SandboxModel.Access.foundationStore] using same

theorem put_existing [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (old value : V) (present : tableLookup key store.entries.val = some old) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => next.entries.val = tablePut key value store.entries.val ∧
        next.entries.val.length = store.entries.val.length ⦄ := by
  apply WP.spec_mono (put_refines eq lawful store key value (Or.inl (by rw [present]; simp)))
  intro next same
  exact ⟨same, by rw [same]; exact table_put_length_present key value _ old present⟩

theorem put_fresh [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (value : V) (absent : tableLookup key store.entries.val = none)
    (capacity : store.entries.val.length < Usize.max) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => next.entries.val = tablePut key value store.entries.val ∧
        next.entries.val.length = store.entries.val.length + 1 ⦄ := by
  apply WP.spec_mono (put_refines eq lawful store key value (Or.inr capacity))
  intro next same
  exact ⟨same, by rw [same]; exact table_put_length_absent key value _ absent⟩

theorem put_query_frame [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (value : V) (capacity : TableWriteCapacity store.entries.val key) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => ∀ query, tableLookup query next.entries.val =
        if query = key then some value else tableLookup query store.entries.val ⦄ := by
  apply WP.spec_mono (put_refines eq lawful store key value capacity)
  intro next same query
  rw [same, table_lookup_put]

theorem put_unique_preserved [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (value : V) (capacity : TableWriteCapacity store.entries.val key)
    (unique : (store.entries.val.map Prod.fst).Nodup) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => (next.entries.val.map Prod.fst).Nodup ⦄ := by
  apply WP.spec_mono (put_refines eq lawful store key value capacity)
  intro next same
  rw [same]
  exact table_put_preserves_unique key value _ unique

end SandboxModel.Local.Containers
