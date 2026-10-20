import SandboxModel.Foundations.BtreeSetBinding.Theory

open Aeneas Aeneas.Std
open SandproofBtreeSetProbeRust
namespace SandboxModel.Foundations.BtreeSetBinding

theorem insert_str_value (seen : Set Str) (key : Str) :
    insert_str seen key = .ok (decide (key ∉ seen.entries), insertedView seen key) := by
  simpa [insert_str] using
    insert_value (Shared0A.Insts.CoreCmpOrd Str.Insts.CoreCmpOrd)
      (reference_cmp_law _ str_cmp_law) seen key

theorem insert_str_contract : InsertContract insert_str :=
  contract_from_value insert_str insert_str_value

def laneRun (seen : Set (Str × U16)) (key : Str × U16) :=
  insert_lane seen key.1 key.2

theorem insert_lane_value (seen : Set (Str × U16)) (key : Str × U16) :
    laneRun seen key = .ok (decide (key ∉ seen.entries), insertedView seen key) := by
  simpa [laneRun, insert_lane] using
    insert_value (Pair.Insts.CoreCmpOrd
      (Shared0A.Insts.CoreCmpOrd Str.Insts.CoreCmpOrd) core.cmp.OrdU16)
      (pair_cmp_law _ _ (reference_cmp_law _ str_cmp_law) u16_cmp_law) seen key

theorem insert_lane_contract : InsertContract laneRun := by
  apply contract_from_value laneRun
  intro seen key
  simpa using insert_lane_value seen key

theorem repeat_str_value (seen : Set Str) (key : Str) :
    repeat_str seen key =
      .ok ((decide (key ∉ seen.entries), false), insertedView seen key) := by
  simp only [repeat_str, insert_value _ (reference_cmp_law _ str_cmp_law), bind_tc_ok]
  simp [inserted_has_key, inserted_idempotent]

theorem new_then_repeat_str_value (key : Str) :
    new_then_repeat_str key = .ok (true, false) := by
  simp only [new_then_repeat_str, alloc.collections.btree.set.BTreeSetTGlobal.new,
    insert_value _ (reference_cmp_law _ str_cmp_law), bind_tc_ok]
  simp [insertedView]

end SandboxModel.Foundations.BtreeSetBinding
