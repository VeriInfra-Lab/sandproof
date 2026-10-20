import SandboxModel.Foundations.BtreeSetBinding.Refinement

open Aeneas Aeneas.Std
open SandproofBtreeSetProbeRust
namespace SandboxModel.Foundations.BtreeSetBinding

def sampleA : Str := Slice.from [97#u8] (by scalar_tac)

def sampleB : Str := Slice.from [98#u8] (by scalar_tac)

theorem sample_keys_distinct : sampleA ≠ sampleB := by
  intro same
  have bytes := congrArg (fun text : Str => text.val.map (fun byte => byte.val)) same
  norm_num [sampleA, sampleB] at bytes

theorem nonempty_fresh_accepts :
    insert_str ⟨[sampleA]⟩ sampleB = .ok (true, ⟨[sampleA, sampleB]⟩) := by
  rw [insert_str_value]
  simp [insertedView, Ne.symm sample_keys_distinct]

theorem original_duplicate_keeps_view :
    insert_str ⟨[sampleA, sampleB]⟩ sampleA =
      .ok (false, ⟨[sampleA, sampleB]⟩) := by
  rw [insert_str_value]
  simp [insertedView]

theorem lane_slot_distinguishes_keys :
    insert_lane ⟨[(sampleA, 1#u16)]⟩ sampleA 2#u16 =
      .ok (true, ⟨[(sampleA, 1#u16), (sampleA, 2#u16)]⟩) := by
  have different : (sampleA, 2#u16) ≠ (sampleA, 1#u16) := by
    intro same
    have slots := congrArg (fun lane : Str × U16 => lane.2.val) same
    norm_num at slots
  change laneRun ⟨[(sampleA, 1#u16)]⟩ (sampleA, 2#u16) =
    .ok (true, ⟨[(sampleA, 1#u16), (sampleA, 2#u16)]⟩)
  rw [insert_lane_value]
  simp [insertedView, different]

end SandboxModel.Foundations.BtreeSetBinding
