import SandboxModel.Foundations.Counter

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

abbrev RustLane := sandproof_center_core.model.RetentionLane

-- This pure query does not authorize slot reuse, establish terminal evidence,
-- or prove that any caller preserves the durable retirement frontier.
theorem lane_current_refines (lane : RustLane) :
    ∃ out, lane_current lane = .ok out ∧
      out.map (fun n => n.val) = nextCounter lane.retired_through.val := by
  have hs := U64.checked_add_bv_spec lane.retired_through 1#u64
  cases he : U64.checked_add lane.retired_through 1#u64 with
  | none =>
    simp only [he] at hs
    simp only [lane_current, sandproof_center_core.model.RetentionLane.current,
      he, Aeneas.Std.lift]
    simp_all [nextCounter, maxCounter]
    scalar_tac
  | some next =>
    simp only [he] at hs
    have hn : next ≠ 0#u64 := by scalar_tac
    simp only [lane_current, sandproof_center_core.model.RetentionLane.current,
      he, Aeneas.Std.lift]
    simp_all [nextCounter, maxCounter,
      sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from]
    scalar_tac

theorem lane_success_is_strict_successor (lane : RustLane) (next : U64)
    (h : lane_current lane = .ok (some next)) :
    CounterRep next (lane.retired_through.val + 1) ∧
      lane.retired_through.val < next.val := by
  obtain ⟨out, he, hr⟩ := lane_current_refines lane
  have ho : out = some next := by simpa [h] using he.symm
  rw [ho] at hr
  simp only [Option.map_some, nextCounter] at hr
  split at hr
  · simp only [Option.some.injEq] at hr
    simp only [CounterRep, validCounter, maxCounter]
    scalar_tac
  · simp at hr

theorem lane_current_exhausted_iff (lane : RustLane) :
    lane_current lane = .ok none ↔ lane.retired_through.val = maxCounter := by
  obtain ⟨out, he, hr⟩ := lane_current_refines lane
  constructor
  · intro h
    have ho : out = none := by simpa [h] using he.symm
    rw [ho] at hr
    simp only [Option.map_none, nextCounter] at hr
    split at hr
    · simp at hr
    · simp only [maxCounter] at *
      scalar_tac
  · intro h
    simp only [nextCounter, h, lt_self_iff_false, if_false] at hr
    cases out <;> simp_all

theorem lane_current_depends_only_on_frontier (left right : RustLane)
    (h : left.retired_through = right.retired_through) :
    lane_current left = lane_current right := by
  simp only [lane_current, sandproof_center_core.model.RetentionLane.current]
  rw [h]

-- A representable query witness, not a proof of CenterState initialization.
def initialLane : RustLane :=
  ⟨⟨"deployment", "node", 1#u64⟩, 1#u16, 0#u64,
    sandproof_center_core.model.LanePhase.Open, none⟩

theorem lane_initial_current_realizable : lane_current initialLane = .ok (some 1#u64) := by
  have he : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  simp [lane_current, sandproof_center_core.model.RetentionLane.current,
    initialLane, Aeneas.Std.lift, he,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from]

end SandboxModel.Foundations
