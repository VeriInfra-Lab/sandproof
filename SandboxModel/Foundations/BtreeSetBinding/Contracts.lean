import SandproofBtreeSetProbe.FunsExternal

open Aeneas Aeneas.Std
open SandproofBtreeSetProbeRust
namespace SandboxModel.Foundations.BtreeSetBinding

abbrev Set (T : Type) := alloc.collections.btree.set.BTreeSet T Global

def CmpLaw {T : Type} (order : core.cmp.Ord T) : Prop :=
  ∀ left right, ∃ observed, order.cmp left right = .ok observed ∧
    (observed = .eq ↔ left = right)

def insertedView {T : Type} [DecidableEq T] (seen : Set T) (key : T) : Set T :=
  if key ∈ seen.entries then seen else ⟨seen.entries ++ [key]⟩

def InsertPost {T : Type} (before : Set T) (key : T) (flag : Bool) (after : Set T) : Prop :=
  (flag = true ↔ key ∉ before.entries) ∧
  (∀ item, item ∈ after.entries ↔ item ∈ before.entries ∨ item = key) ∧
  after.entries.Nodup ∧
  (flag = false → after.entries = before.entries)

def InsertContract {T : Type} (run : Set T → T → Result (Bool × Set T)) : Prop :=
  ∀ seen key, seen.entries.Nodup → ∃ flag after,
    run seen key = .ok (flag, after) ∧ InsertPost seen key flag after

end SandboxModel.Foundations.BtreeSetBinding
