import SandboxModel.Foundations.BtreeSetBinding.Contracts
import SandproofBtreeSetProbe.Funs

open Aeneas Aeneas.Std
open SandproofBtreeSetProbeRust
namespace SandboxModel.Foundations.BtreeSetBinding

theorem byte_compare_eq_iff (left right : List U8) :
    SetValues.byteCompare left right = .eq ↔ left = right := by
  induction left generalizing right with
  | nil => cases right <;> simp [SetValues.byteCompare]
  | cons x xs ih =>
    cases right with
    | nil => simp [SetValues.byteCompare]
    | cons y ys =>
      by_cases first : x.val < y.val
      · have different : x ≠ y := by intro same; subst y; omega
        simp [SetValues.byteCompare, first, different]
      · by_cases second : y.val < x.val
        · have different : x ≠ y := by intro same; subst y; omega
          simp [SetValues.byteCompare, first, second, different]
        · have same : x = y := UScalar.eq_of_val_eq (by omega)
          subst y
          simpa [SetValues.byteCompare] using ih ys

theorem str_cmp_law : CmpLaw Str.Insts.CoreCmpOrd := by
  intro left right
  refine ⟨SetValues.byteCompare left.val right.val, rfl, ?_⟩
  rw [byte_compare_eq_iff]
  exact ⟨fun same => Slice.ext left right same, fun same => congrArg Slice.val same⟩

theorem reference_cmp_law {T : Type} (order : core.cmp.Ord T)
    (law : CmpLaw order) : CmpLaw (Shared0A.Insts.CoreCmpOrd order) := by
  exact law

theorem member_by_value {T : Type} [DecidableEq T] (order : core.cmp.Ord T)
    (law : CmpLaw order) (key : T) (entries : List T) :
    SetValues.memberBy order key entries = .ok (decide (key ∈ entries)) := by
  induction entries with
  | nil => simp [SetValues.memberBy]
  | cons head tail ih =>
    rcases law head key with ⟨observed, run, equal⟩
    simp only [SetValues.memberBy, run, bind_tc_ok]
    by_cases present : head = key
    · have same : observed = .eq := equal.mpr present
      simp [same, present]
    · have different : observed ≠ .eq := fun same => present (equal.mp same)
      simp [different, ih, Ne.symm present]

theorem insert_value {T : Type} [DecidableEq T] (order : core.cmp.Ord T)
    (law : CmpLaw order) (seen : Set T) (key : T) :
    alloc.collections.btree.set.BTreeSet.insert core.core.clone.CloneGlobal order seen key =
      .ok (decide (key ∉ seen.entries), insertedView seen key) := by
  unfold alloc.collections.btree.set.BTreeSet.insert
  rw [member_by_value order law]
  by_cases present : key ∈ seen.entries <;> simp [present, insertedView]

theorem pair_cmp_law {U T : Type} (first : core.cmp.Ord U) (second : core.cmp.Ord T)
    (firstLaw : CmpLaw first) (secondLaw : CmpLaw second) :
    CmpLaw (Pair.Insts.CoreCmpOrd first second) := by
  intro left right
  rcases firstLaw left.1 right.1 with ⟨observed, firstRun, sameFirst⟩
  by_cases firstEqual : left.1 = right.1
  · have sameObserved : observed = .eq := sameFirst.mpr firstEqual
    subst observed
    rcases secondLaw left.2 right.2 with ⟨secondObserved, secondRun, sameSecond⟩
    refine ⟨secondObserved, ?_, ?_⟩
    · simp [Pair.Insts.CoreCmpOrd.cmp, firstRun, secondRun]
    · simpa [Prod.ext_iff, firstEqual] using sameSecond
  · have notObserved : observed ≠ .eq := fun equal => firstEqual (sameFirst.mp equal)
    have different : left ≠ right := fun equal => firstEqual (congrArg Prod.fst equal)
    cases observed with
    | eq => exact False.elim (notObserved rfl)
    | lt =>
      refine ⟨.lt, ?_, ?_⟩
      · simp [Pair.Insts.CoreCmpOrd.cmp, firstRun]
      · simp [different]
    | gt =>
      refine ⟨.gt, ?_, ?_⟩
      · simp [Pair.Insts.CoreCmpOrd.cmp, firstRun]
      · simp [different]

theorem inserted_members {T : Type} [DecidableEq T] (seen : Set T) (key item : T) :
    item ∈ (insertedView seen key).entries ↔ item ∈ seen.entries ∨ item = key := by
  by_cases present : key ∈ seen.entries
  · simp only [insertedView, if_pos present]
    constructor
    · exact Or.inl
    · intro member
      rcases member with old | same
      · exact old
      · simpa [same] using present
  · simp [insertedView, present]

theorem inserted_valid {T : Type} [DecidableEq T] (seen : Set T) (key : T)
    (valid : seen.entries.Nodup) : (insertedView seen key).entries.Nodup := by
  by_cases present : key ∈ seen.entries
  · simpa [insertedView, present] using valid
  · simp only [insertedView, if_neg present, List.nodup_append, valid, List.nodup_singleton, true_and]
    intro item old other single same
    have keySame : other = key := by simpa using single
    exact present (by simpa [same, keySame] using old)

theorem contract_from_value {T : Type} [DecidableEq T]
    (run : Set T → T → Result (Bool × Set T))
    (actual : ∀ seen key, run seen key =
      .ok (decide (key ∉ seen.entries), insertedView seen key)) :
    InsertContract run := by
  intro seen key valid
  refine ⟨decide (key ∉ seen.entries), insertedView seen key, actual seen key, ?_⟩
  refine ⟨by simp, inserted_members seen key, inserted_valid seen key valid, ?_⟩
  by_cases present : key ∈ seen.entries
  · simp [present, insertedView]
  · simp [present]

theorem u16_cmp_law : CmpLaw core.cmp.OrdU16 := by
  intro left right
  refine ⟨compare left.val right.val, rfl, ?_⟩
  constructor
  · intro same
    exact UScalar.eq_of_val_eq (compare_eq_iff_eq.mp same)
  · intro same
    subst right
    simp

theorem inserted_has_key {T : Type} [DecidableEq T] (seen : Set T) (key : T) :
    key ∈ (insertedView seen key).entries :=
  (inserted_members seen key key).mpr (.inr rfl)

theorem inserted_idempotent {T : Type} [DecidableEq T] (seen : Set T) (key : T) :
    insertedView (insertedView seen key) key = insertedView seen key := by
  exact if_pos (inserted_has_key seen key)

end SandboxModel.Foundations.BtreeSetBinding
