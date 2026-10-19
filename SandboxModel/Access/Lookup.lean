import SandboxModel.Access.Representation
import SandboxModel.Foundations.Containers
namespace SandboxModel.Access
open Aeneas Aeneas.Std SandproofNetworkRust

def foundationStore (store : sandbox_controller_core.store.Store K V) :
    SandproofFoundationsRust.sandbox_controller_core.store.Store K V := ⟨store.entries⟩

theorem borrow_copy_loop_body_agree (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (index : Usize) :
    sandbox_controller_core.store.Store.get_loop.body eq store key index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop.body
        eq (foundationStore store) key index := by
  rfl

theorem borrow_copy_loop_agree (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (index : Usize) :
    sandbox_controller_core.store.Store.get_loop eq store key index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop
        eq (foundationStore store) key index := by
  unfold sandbox_controller_core.store.Store.get_loop
    SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop
  simp only [borrow_copy_loop_body_agree]

theorem store_get_refines [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : SandboxModel.Foundations.TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) :
    sandbox_controller_core.store.Store.get eq store key =
      .ok (SandboxModel.Foundations.tableLookup key store.entries.val) := by
  obtain ⟨out, he, hr⟩ := WP.spec_imp_exists
    (SandboxModel.Foundations.table_get_copy_loop_refines eq lawful
      (foundationStore store) key 0#usize (by simp [foundationStore]))
  subst out
  simpa [sandbox_controller_core.store.Store.get, borrow_copy_loop_agree,
    foundationStore] using he

theorem sandbox_equality_lawful : SandboxModel.Foundations.TableLawfulEq
    sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId := by
  intro left right
  rfl

theorem table_lookup_map [DecidableEq K] [DecidableEq M]
    (keyMap : K → M) (valueMap : V → W) (injective : Function.Injective keyMap)
    (entries : List (K × V)) (key : K) :
    SandboxModel.Foundations.tableLookup (keyMap key)
        (entries.map (fun (stored, value) => (keyMap stored, valueMap value))) =
      (SandboxModel.Foundations.tableLookup key entries).map valueMap := by
  induction entries with
  | nil => rfl
  | cons pair tail ih =>
      rcases pair with ⟨stored, value⟩
      simp only [List.map_cons, SandboxModel.Foundations.tableLookup]
      by_cases same : key = stored <;> simp [injective.eq_iff, same, ih]

theorem sandbox_lookup_refines (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) :
    SandboxModel.Foundations.tableLookup sandbox.val (recordsView world) =
      (SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val).map recordView := by
  exact table_lookup_map UScalar.val recordView
    (fun _ _ same => UScalar.eq_of_val_eq same) _ _

end SandboxModel.Access
