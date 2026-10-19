import SandboxModel.Foundations.Model
import SandproofFoundations.Funs

/-! Unit conversions keep caller policy separate from representable arithmetic.
Zero is a valid arithmetic input; clocks, lifetime bounds, template validity and
physical observations are checked by the actual callers, not these definitions. -/
namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

def secondsToMilliseconds (seconds : Nat) : Option Nat :=
  if seconds * 1000 ≤ maxCounter then some (seconds * 1000) else none

def mebibyte : Nat := 1024 * 1024

def ceilingMebibytes (bytes : Nat) : Nat :=
  bytes / mebibyte + if bytes % mebibyte = 0 then 0 else 1

def bytesToMebibytes (bytes : Nat) : Option Nat :=
  if ceilingMebibytes bytes ≤ 2 ^ 32 - 1 then some (ceilingMebibytes bytes) else none

def availableResource (limit used observed : Nat) : Nat :=
  min (limit - used) observed

theorem seconds_to_milliseconds_refines (seconds : U64) :
    ∃ output, numeric.seconds_to_milliseconds seconds = .ok output ∧
      output.map (fun n => n.val) = secondsToMilliseconds seconds.val := by
  have arithmetic := U64.checked_mul_bv_spec seconds 1000#u64
  cases outcome : U64.checked_mul seconds 1000#u64 with
  | none =>
    refine ⟨none, ?_, ?_⟩
    · simp [numeric.seconds_to_milliseconds,
        sandproof_protocol.numeric.seconds_to_milliseconds, outcome]
    · simp only [outcome] at arithmetic
      simp only [Option.map_none, secondsToMilliseconds]
      have overflow : ¬ seconds.val * 1000 ≤ maxCounter := by
        simp only [maxCounter] at *
        scalar_tac
      simp [overflow]
  | some milliseconds =>
    refine ⟨some milliseconds, ?_, ?_⟩
    · simp [numeric.seconds_to_milliseconds,
        sandproof_protocol.numeric.seconds_to_milliseconds, outcome]
    · simp only [outcome] at arithmetic
      have bounded : seconds.val * 1000 ≤ maxCounter := by
        simp only [maxCounter] at *
        scalar_tac
      have exactValue : milliseconds.val = seconds.val * 1000 := by scalar_tac
      simp [secondsToMilliseconds, bounded, exactValue]

theorem saturating_remaining_refines (limit used : U64) :
    (core.num.U64.saturating_sub limit used).val = limit.val - used.val := by
  simp only [core.num.U64.saturating_sub, UScalar.saturating_sub, Nat.zero_le,
    max_eq_right, UScalar.val, BitVec.toNat_ofNat]
  apply Nat.mod_eq_of_lt
  have bound := limit.hBounds
  simp only [UScalarTy.numBits] at bound
  omega

theorem available_resource_refines (limit used observed : U64) :
    ∃ output, numeric.available_resource limit used observed = .ok output ∧
      output.val = availableResource limit.val used.val observed.val := by
  let remaining := core.num.U64.saturating_sub limit used
  have remainingValue : remaining.val = limit.val - used.val :=
    saturating_remaining_refines limit used
  by_cases smaller : remaining.val < observed.val
  · refine ⟨remaining, ?_, ?_⟩
    · simp [numeric.available_resource, sandproof_protocol.numeric.available_resource,
        Aeneas.Std.lift, remaining, smaller]
    · simp only [availableResource]
      rw [remainingValue]
      exact (Nat.min_eq_left (by omega)).symm
  · refine ⟨observed, ?_, ?_⟩
    · simp [numeric.available_resource, sandproof_protocol.numeric.available_resource,
        Aeneas.Std.lift, remaining, smaller]
    · simp only [availableResource]
      exact (Nat.min_eq_right (by omega)).symm

def narrowUnits (units : U64) : Result (Option U32) := do
  let maximum ← lift (core.convert.num.FromU64U32.from core.num.U32.MAX)
  if units > maximum then .ok none
  else
    let value ← lift (UScalar.cast .U32 units)
    .ok (some value)

theorem narrow_units_refines (units : U64) :
    ∃ output, narrowUnits units = .ok output ∧
      output.map (fun n => n.val) =
        (if units.val ≤ 2 ^ 32 - 1 then some units.val else none) := by
  have maximum : core.convert.num.FromU64U32.from core.num.U32.MAX =
      4294967295#u64 := by rfl
  by_cases bound : units.val ≤ 2 ^ 32 - 1
  · refine ⟨some (UScalar.cast .U32 units), ?_, ?_⟩
    · simp [narrowUnits, lift, maximum]
      scalar_tac
    · simp only [Option.map_some, if_pos bound, Option.some.injEq,
        UScalar.cast_val_eq, UScalarTy.numBits]
      apply Nat.mod_eq_of_lt
      omega
  · refine ⟨none, ?_, ?_⟩
    · simp [narrowUnits, lift, maximum]
      scalar_tac
    · simp only [Option.map_none, if_neg bound]

theorem bytes_to_mebibytes_refines (bytes : U64) :
    ∃ output, numeric.bytes_to_mebibytes bytes = .ok output ∧
      output.map (fun n => n.val) = bytesToMebibytes bytes.val := by
  have scale : (1024#u64 * 1024#u64 : Result U64) = Result.ok (1048576#u64) := by rfl
  obtain ⟨whole, wholeEq, wholeValue⟩ :=
    UScalar.div_spec bytes (y := 1048576#u64) (by decide)
  obtain ⟨remainder, remainderEq, remainderValue⟩ :=
    WP.spec_imp_exists (U64.rem_spec bytes (y := 1048576#u64) (by decide))
  have wholeBound : whole.val ≤ 17592186044415 := by
    have inputBound := bytes.hBounds
    simp only [UScalarTy.numBits] at inputBound
    have wholeValueSimple : whole.val = bytes.val / 1048576 := by simpa using wholeValue
    omega
  unfold numeric.bytes_to_mebibytes sandproof_protocol.numeric.bytes_to_mebibytes
  simp only [scale, wholeEq, remainderEq, bind_tc_ok]
  split
  · have exactUnit : bytes.val % 1048576 = 0 := by
      have zeroValue : ((0#64#uscalar) : U64).val = 0 := by rfl
      simpa [zeroValue] using remainderValue.symm
    obtain ⟨units, unitsEq, unitsValue⟩ :=
      WP.spec_imp_exists (U64.add_spec (x := whole) (y := 0#u64) (by scalar_tac))
    obtain ⟨output, outputEq, outputValue⟩ := narrow_units_refines units
    refine ⟨output, ?_, ?_⟩
    · simpa only [unitsEq, bind_tc_ok, narrowUnits] using outputEq
    · have unitsExact : units.val = ceilingMebibytes bytes.val := by
        simp [ceilingMebibytes, mebibyte, exactUnit]
        scalar_tac
      simpa [bytesToMebibytes, unitsExact] using outputValue
  · rename_i notZero
    have exactUnit : bytes.val % 1048576 ≠ 0 := by
      intro zero
      have remainderZero : remainder = ((0#64#uscalar) : U64) := by
        apply UScalar.eq_of_val_eq
        calc
          remainder.val = 0 := by simpa [zero] using remainderValue
          _ = ((0#64#uscalar) : U64).val := by rfl
      exact notZero remainderZero
    obtain ⟨units, unitsEq, unitsValue⟩ :=
      WP.spec_imp_exists (U64.add_spec (x := whole) (y := 1#u64) (by scalar_tac))
    obtain ⟨output, outputEq, outputValue⟩ := narrow_units_refines units
    refine ⟨output, ?_, ?_⟩
    · simpa only [unitsEq, bind_tc_ok, narrowUnits] using outputEq
    · have unitsExact : units.val = ceilingMebibytes bytes.val := by
        simp [ceilingMebibytes, mebibyte, exactUnit]
        scalar_tac
      simpa [bytesToMebibytes, unitsExact] using outputValue


theorem seconds_to_milliseconds_accepts_iff (seconds milliseconds : Nat) :
    secondsToMilliseconds seconds = some milliseconds ↔
      seconds * 1000 ≤ maxCounter ∧ milliseconds = seconds * 1000 := by
  by_cases bound : seconds * 1000 ≤ maxCounter <;>
    simp [secondsToMilliseconds, bound, eq_comm]

theorem seconds_to_milliseconds_rejects_iff (seconds : Nat) :
    secondsToMilliseconds seconds = none ↔ maxCounter < seconds * 1000 := by
  by_cases bound : seconds * 1000 ≤ maxCounter
  all_goals simp [secondsToMilliseconds, bound]
  all_goals omega

theorem available_resource_logical_bound (limit used observed : Nat) :
    availableResource limit used observed ≤ limit := by
  exact le_trans (Nat.min_le_left _ _) (Nat.sub_le _ _)

theorem available_resource_observed_bound (limit used observed : Nat) :
    availableResource limit used observed ≤ observed := by
  exact Nat.min_le_right _ _

theorem available_resource_exhausted (limit used observed : Nat) (usedUp : limit ≤ used) :
    availableResource limit used observed = 0 := by
  simp [availableResource, Nat.sub_eq_zero_of_le usedUp]

theorem available_resource_accounting_bound (limit used observed : Nat) (within : used ≤ limit) :
    availableResource limit used observed + used ≤ limit := by
  have remaining : availableResource limit used observed ≤ limit - used :=
    Nat.min_le_left _ _
  omega

theorem mebibytes_least_bound (bytes units : Nat) :
    ceilingMebibytes bytes ≤ units ↔ bytes ≤ units * mebibyte := by
  have parts := Nat.mod_add_div bytes mebibyte
  have remainder := Nat.mod_lt bytes (by decide : 0 < mebibyte)
  by_cases exactUnit : bytes % mebibyte = 0
  all_goals simp only [ceilingMebibytes, exactUnit, if_true, if_false, Nat.add_zero]
  all_goals simp only [mebibyte] at *
  all_goals omega

theorem mebibytes_cover (bytes : Nat) :
    bytes ≤ ceilingMebibytes bytes * mebibyte := by
  exact (mebibytes_least_bound bytes _).mp (le_refl _)

theorem available_resource_result_bounds (limit used observed output : U64)
    (returned : numeric.available_resource limit used observed = .ok output) :
    output.val ≤ limit.val ∧ output.val ≤ observed.val := by
  obtain ⟨expected, resultEq, viewEq⟩ := available_resource_refines limit used observed
  have same : expected = output := Result.ok_injective (resultEq.symm.trans returned)
  subst expected
  rw [viewEq]
  exact ⟨available_resource_logical_bound _ _ _, available_resource_observed_bound _ _ _⟩

theorem available_resource_result_exhausted (limit used observed output : U64)
    (returned : numeric.available_resource limit used observed = .ok output)
    (usedUp : limit.val ≤ used.val) : output.val = 0 := by
  obtain ⟨expected, resultEq, viewEq⟩ := available_resource_refines limit used observed
  have same : expected = output := Result.ok_injective (resultEq.symm.trans returned)
  subst expected
  rw [viewEq]
  exact available_resource_exhausted _ _ _ usedUp

theorem available_resource_result_accounting (limit used observed output : U64)
    (returned : numeric.available_resource limit used observed = .ok output)
    (within : used.val ≤ limit.val) : output.val + used.val ≤ limit.val := by
  obtain ⟨expected, resultEq, viewEq⟩ := available_resource_refines limit used observed
  have same : expected = output := Result.ok_injective (resultEq.symm.trans returned)
  subst expected
  rw [viewEq]
  exact available_resource_accounting_bound _ _ _ within

theorem seconds_implementation_rejects_iff (seconds : U64) :
    numeric.seconds_to_milliseconds seconds = .ok none ↔
      maxCounter < seconds.val * 1000 := by
  obtain ⟨output, resultEq, viewEq⟩ := seconds_to_milliseconds_refines seconds
  constructor
  · intro rejected
    have same : output = none := Result.ok_injective (resultEq.symm.trans rejected)
    apply (seconds_to_milliseconds_rejects_iff seconds.val).mp
    rw [← viewEq, same]
    rfl
  · intro overflow
    have modelNone := (seconds_to_milliseconds_rejects_iff seconds.val).mpr overflow
    have mapNone : output.map (fun n => n.val) = none := viewEq.trans modelNone
    have same : output = none := by simpa using mapNone
    simpa [same] using resultEq

theorem bytes_success_bound (bytes : U64) (units : U32)
    (returned : numeric.bytes_to_mebibytes bytes = .ok (some units)) :
    units.val = ceilingMebibytes bytes.val ∧ bytes.val ≤ units.val * mebibyte := by
  obtain ⟨output, resultEq, viewEq⟩ := bytes_to_mebibytes_refines bytes
  have same : output = some units := Result.ok_injective (resultEq.symm.trans returned)
  rw [same] at viewEq
  simp only [Option.map_some, bytesToMebibytes] at viewEq
  split at viewEq
  · have value : units.val = ceilingMebibytes bytes.val := by simpa using viewEq
    exact ⟨value, by rw [value]; exact mebibytes_cover _⟩
  · simp at viewEq

theorem numeric_zero_policy_witness :
    numeric.seconds_to_milliseconds 0#u64 = .ok (some 0#u64) ∧
    numeric.bytes_to_mebibytes 0#u64 = .ok (some 0#u32) := by
  constructor
  · rfl
  · obtain ⟨output, resultEq, viewEq⟩ := bytes_to_mebibytes_refines 0#u64
    have viewZero : output.map (fun n => n.val) = some 0 := by
      simpa [bytesToMebibytes, ceilingMebibytes, mebibyte] using viewEq
    cases output with
    | none => simp at viewZero
    | some units =>
      have valueZero : units.val = 0 := by simpa using viewZero
      have same : units = 0#u32 := by scalar_tac
      simpa [same] using resultEq

theorem bytes_to_mebibytes_accepts_iff (bytes units : Nat) :
    bytesToMebibytes bytes = some units ↔
      bytes ≤ (2 ^ 32 - 1) * mebibyte ∧ units = ceilingMebibytes bytes := by
  have fit := mebibytes_least_bound bytes (2 ^ 32 - 1)
  rw [← fit]
  unfold bytesToMebibytes
  split
  · rename_i bound
    constructor
    · intro returned
      exact ⟨bound, (Option.some.inj returned).symm⟩
    · intro accepted
      exact congrArg some accepted.2.symm
  · rename_i bound
    constructor
    · intro returned
      cases returned
    · intro accepted
      exact (bound accepted.1).elim

theorem bytes_to_mebibytes_rejects_iff (bytes : Nat) :
    bytesToMebibytes bytes = none ↔ (2 ^ 32 - 1) * mebibyte < bytes := by
  have fit := mebibytes_least_bound bytes (2 ^ 32 - 1)
  unfold bytesToMebibytes
  split
  · rename_i bound
    constructor
    · intro returned
      cases returned
    · intro overflow
      have within := fit.mp bound
      omega
  · rename_i bound
    have overflow : (2 ^ 32 - 1) * mebibyte < bytes := by
      have outside : ¬ bytes ≤ (2 ^ 32 - 1) * mebibyte := fun within =>
        bound (fit.mpr within)
      omega
    constructor
    · intro _
      exact overflow
    · intro _
      rfl

theorem bytes_implementation_rejects_iff (bytes : U64) :
    numeric.bytes_to_mebibytes bytes = .ok none ↔
      (2 ^ 32 - 1) * mebibyte < bytes.val := by
  obtain ⟨output, resultEq, viewEq⟩ := bytes_to_mebibytes_refines bytes
  constructor
  · intro rejected
    have same : output = none := Result.ok_injective (resultEq.symm.trans rejected)
    apply (bytes_to_mebibytes_rejects_iff bytes.val).mp
    rw [← viewEq, same]
    rfl
  · intro overflow
    have modelNone := (bytes_to_mebibytes_rejects_iff bytes.val).mpr overflow
    have mapNone : output.map (fun n => n.val) = none := viewEq.trans modelNone
    have same : output = none := by simpa using mapNone
    simpa [same] using resultEq

end SandboxModel.Foundations
