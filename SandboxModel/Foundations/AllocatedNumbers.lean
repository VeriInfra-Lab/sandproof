import SandboxModel.Foundations.Identifiers
open Aeneas Aeneas.Std SandproofFoundationsRust
namespace SandboxModel.Foundations
open IdentifierLibrary

def allocatedNumberModel (value stem : List U8) (highwater : Nat) : Option U64 :=
  let suffix := value.drop stem.length
  if stem.length ≤ value.length ∧
      (byteArray (value.take stem.length)).IsValidUTF8 ∧
      (byteArray suffix).IsValidUTF8 then
    if value.take stem.length = stem then
      match parseUnsigned suffix with
      | none => none
      | some number =>
        if 0 < number.val ∧ number.val ≤ highwater ∧
            stringBytes (toString number.val) = suffix then some number else none
    else none
  else none

theorem allocated_number_refines (value stem : Str) (highwater : U64) :
    allocated_number value stem highwater =
      .ok (allocatedNumberModel value.val stem.val highwater.val) := by
  simp only [allocated_number, sandproof_protocol.identifiers.parse_allocated_number,
    core.str.Str.len, core.str.Str.split_at_checked,
    core.option.Option.Insts.CoreOpsTry_traitTry.branch,
    core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual,
    core.cmp.impls.PartialEqShared.ne, core.cmp.PartialEq.ne.default,
    Str.Insts.CoreCmpPartialEqStr.eq, core.str.Str.parse,
    U64.Insts.CoreStrTraitsFromStrParseIntError.from_str, core.result.Result.ok,
    core.bool.Bool.then_some, alloc.string.ToString.Blanket.to_string,
    SandproofFoundationsRust.core.fmt.num.imp.DisplayU64.fmt,
    alloc.string.String.Insts.CoreCmpPartialEqShared0Str.eq]
  by_cases size : stem.val.length ≤ value.val.length
  · by_cases utf8 : (byteArray (value.val.take stem.val.length)).IsValidUTF8 ∧
        (byteArray (value.val.drop stem.val.length)).IsValidUTF8
    · by_cases same : value.val.take stem.val.length = stem.val
      · have validStem : (byteArray stem.val).IsValidUTF8 := by simpa only [same] using utf8.1
        cases parsed : parseUnsigned (value.val.drop stem.val.length) with
        | none => simp [allocatedNumberModel, size, utf8, same, validStem, parsed]
        | some number =>
          by_cases positive : 0 < number.val
          · by_cases bounded : number.val ≤ highwater.val
            · simp [allocatedNumberModel, size, utf8, same, validStem, parsed, positive, bounded]
            · simp [allocatedNumberModel, size, utf8, same, validStem, parsed, positive, bounded]
          · simp [allocatedNumberModel, size, utf8, same, validStem, parsed, positive]
      · simp [allocatedNumberModel, size, utf8, same]
    · simp [allocatedNumberModel, size, utf8]
  · simp [allocatedNumberModel, size]

theorem allocated_model_success_iff (value stem : List U8) (highwater : Nat) (number : U64) :
    allocatedNumberModel value stem highwater = some number ↔
      stem.length ≤ value.length ∧
      (byteArray (value.take stem.length)).IsValidUTF8 ∧
      (byteArray (value.drop stem.length)).IsValidUTF8 ∧
      value.take stem.length = stem ∧
      parseUnsigned (value.drop stem.length) = some number ∧
      0 < number.val ∧ number.val ≤ highwater ∧
      stringBytes (toString number.val) = value.drop stem.length := by
  by_cases cut : stem.length ≤ value.length ∧
      (byteArray (value.take stem.length)).IsValidUTF8 ∧
      (byteArray (value.drop stem.length)).IsValidUTF8
  · by_cases same : value.take stem.length = stem
    · have validStem : (byteArray stem).IsValidUTF8 := by simpa only [same] using cut.2.1
      cases parsed : parseUnsigned (value.drop stem.length) with
      | none => simp [allocatedNumberModel, cut.1, cut.2.2, same, validStem, parsed]
      | some found =>
        by_cases equal : found = number
        · subst found
          simp [allocatedNumberModel, cut.1, cut.2.2, same, validStem, parsed]
        · simp [allocatedNumberModel, cut.1, cut.2.2, same, validStem, parsed, equal]
    · simp [allocatedNumberModel, cut, same]
  · simp [allocatedNumberModel, cut]
    tauto

theorem center_allocated_number_refines (value stem : Str) (highwater : U64) :
    center_allocated_number value stem highwater =
      .ok (allocatedNumberModel value.val stem.val highwater.val) := by
  exact allocated_number_refines value stem highwater

theorem allocated_number_success_bounds (value stem : Str) (highwater number : U64)
    (accepted : allocated_number value stem highwater = .ok (some number)) :
    validCounter number.val ∧ number.val ≤ highwater.val := by
  rw [allocated_number_refines] at accepted
  simp only [Result.ok.injEq] at accepted
  have successful := (allocated_model_success_iff value.val stem.val highwater.val number).mp
    accepted
  refine ⟨⟨successful.2.2.2.2.2.1, ?_⟩, successful.2.2.2.2.2.2.1⟩
  simp only [maxCounter]
  scalar_tac

theorem allocated_number_canonical (value stem : Str) (highwater number : U64)
    (accepted : allocated_number value stem highwater = .ok (some number)) :
    value.val = stem.val ++ stringBytes (toString number.val) := by
  rw [allocated_number_refines] at accepted
  simp only [Result.ok.injEq] at accepted
  have successful := (allocated_model_success_iff value.val stem.val highwater.val number).mp
    accepted
  rw [← successful.2.2.2.1, successful.2.2.2.2.2.2.2]
  exact (List.take_append_drop stem.val.length value.val).symm

theorem allocated_number_no_alias (first second stem : Str) (highwater number : U64)
    (one : allocated_number first stem highwater = .ok (some number))
    (two : allocated_number second stem highwater = .ok (some number)) :
    first = second := by
  have same := (allocated_number_canonical first stem highwater number one).trans
    (allocated_number_canonical second stem highwater number two).symm
  exact Slice.ext first second same

theorem allocated_number_zero_frontier (value stem : Str) :
    allocated_number value stem (0#u64) = .ok none := by
  rw [allocated_number_refines]
  congr 1
  cases result : allocatedNumberModel value.val stem.val 0 with
  | none => exact result
  | some number =>
    have accepted := (allocated_model_success_iff value.val stem.val 0 number).mp result
    have positive := accepted.2.2.2.2.2.1
    have bounded := accepted.2.2.2.2.2.2.1
    omega

theorem allocated_number_examples :
    allocatedNumberModel (stringBytes "sandbox-1") (stringBytes "sandbox-") 1 = some (1#u64) ∧
    allocatedNumberModel (stringBytes "sandbox-01") (stringBytes "sandbox-") 1 = none ∧
    allocatedNumberModel (stringBytes "sandbox-+1") (stringBytes "sandbox-") 1 = none ∧
    allocatedNumberModel (stringBytes "sandbox-0") (stringBytes "sandbox-") 1 = none ∧
    allocatedNumberModel (stringBytes "sandbox-2") (stringBytes "sandbox-") 1 = none ∧
    allocatedNumberModel (stringBytes "operation-1") (stringBytes "sandbox-") 1 = none ∧
    allocatedNumberModel (stringBytes "sandbox-18446744073709551616")
      (stringBytes "sandbox-") maxCounter = none := by
  decide

end SandboxModel.Foundations
