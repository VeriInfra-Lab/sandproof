import SandboxModel.Foundations.Representation

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

theorem counter_constructor_refines (value : U64) :
    ∃ out, counter_from_u64 value = .ok out ∧
      counterResultView out = makeCounter value.val := by
  simp only [counter_from_u64,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from]
  split <;> simp_all [counterResultView, makeCounter]

theorem counter_constructor_valid (value result : U64)
    (h : counter_from_u64 value = .ok (.Ok result)) :
    CounterRep result value.val := by
  simp only [counter_from_u64,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from] at h
  split at h
  · simp at h
  · simp only [Result.ok.injEq, core.result.Result.Ok.injEq] at h
    subst result
    simp only [CounterRep, validCounter, maxCounter, true_and]
    scalar_tac

theorem checked_counter_refines (value : U64) :
    ∃ out, counter_next value = .ok out ∧
      counterResultView out = nextCounter value.val := by
  have hs := U64.checked_add_bv_spec value 1#u64
  cases he : U64.checked_add value 1#u64 with
  | none =>
    simp only [he] at hs
    simp only [counter_next, sandproof_protocol.internal.PositiveCounter.checked_next,
      sandproof_protocol.internal.PositiveCounter.get, Aeneas.Std.lift]
    simp_all [counterResultView, nextCounter, maxCounter]
    scalar_tac
  | some next =>
    simp only [he] at hs
    simp only [counter_next, sandproof_protocol.internal.PositiveCounter.checked_next,
      sandproof_protocol.internal.PositiveCounter.get, Aeneas.Std.lift,
      sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from]
    simp_all [counterResultView, nextCounter, maxCounter]
    scalar_tac

theorem counter_exhausted :
    counter_next 18446744073709551615#u64 =
      .ok (.Err sandproof_protocol.internal.InternalError.InvalidCounter) := by
  have he : U64.checked_add 18446744073709551615#u64 1#u64 = none := by decide
  simp [counter_next, sandproof_protocol.internal.PositiveCounter.checked_next,
    sandproof_protocol.internal.PositiveCounter.get, Aeneas.Std.lift, he]

theorem counter_value_refines (value : U64) : counter_value value = .ok value := by rfl

theorem counter_value_roundtrip (value : U64) (h : 0 < value.val) :
    counter_from_u64 value = .ok (.Ok value) ∧ counter_value value = .ok value := by
  have hn : value ≠ 0#u64 := by scalar_tac
  simp [counter_from_u64,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from,
    hn, counter_value, sandproof_protocol.internal.PositiveCounter.get]

theorem counter_init_realizable :
    CounterRep 1#u64 1 ∧ counter_from_u64 1#u64 = .ok (.Ok 1#u64) := by
  constructor
  · simp [CounterRep, validCounter, maxCounter]
  · exact (counter_value_roundtrip 1#u64 (by decide)).1

end SandboxModel.Foundations
