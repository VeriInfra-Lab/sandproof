import Authority.Funs

namespace CredentialProofs
open Aeneas Aeneas.Std Authority

abbrev Digest := Array U8 32#usize
abbrev Reject := credential_core.CredentialReject

def registeredExpected (stored : Option (U64 × Digest)) (generation : U64) :
    core.result.Result Digest Reject :=
  match stored with
  | none => .Err .Missing
  | some (registeredGeneration, digest) =>
      if registeredGeneration = generation then .Ok digest else .Err .Mismatch

def inverseExpected (sandbox generation : U64) (found : Option (U64 × U64)) :
    core.result.Result Unit Reject :=
  if found = some (sandbox, generation) then .Ok () else .Err .Mismatch

theorem registered_refines (stored : Option (U64 × Digest)) (generation : U64) :
    credential_core.registered_digest stored generation = .ok (registeredExpected stored generation) := by
  cases stored with
  | none =>
      simp [credential_core.registered_digest, registeredExpected,
        core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | some pair =>
      rcases pair with ⟨registeredGeneration, digest⟩
      by_cases same : registeredGeneration = generation <;>
        simp [credential_core.registered_digest, registeredExpected,
          core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          same]
      all_goals simp_all [← UScalar.eq_equiv]

theorem registered_success_iff (stored : Option (U64 × Digest)) (generation : U64) (digest : Digest) :
    credential_core.registered_digest stored generation = .ok (.Ok digest) ↔
      stored = some (generation, digest) := by
  rw [registered_refines]
  cases stored with
  | none => simp [registeredExpected]
  | some pair =>
      rcases pair with ⟨registeredGeneration, registeredDigest⟩
      by_cases same : registeredGeneration = generation <;> simp [registeredExpected, same]

theorem missing_rejected (generation : U64) :
    credential_core.registered_digest none generation = .ok (.Err .Missing) := by
  rw [registered_refines]
  rfl

theorem generation_mismatch_rejected (registeredGeneration generation : U64) (digest : Digest)
    (different : registeredGeneration ≠ generation) :
    credential_core.registered_digest (some (registeredGeneration, digest)) generation =
      .ok (.Err .Mismatch) := by
  rw [registered_refines]
  simp [registeredExpected, different]

theorem inverse_refines (sandbox generation : U64) (found : Option (U64 × U64)) :
    credential_core.inverse_binding sandbox generation found =
      .ok (inverseExpected sandbox generation found) := by
  cases found with
  | none => simp [credential_core.inverse_binding, inverseExpected]
  | some pair =>
      rcases pair with ⟨boundSandbox, boundGeneration⟩
      by_cases sid : boundSandbox = sandbox <;> by_cases gen : boundGeneration = generation <;>
        simp [credential_core.inverse_binding, inverseExpected, sid, gen]

theorem inverse_success_iff (sandbox generation : U64) (found : Option (U64 × U64)) :
    credential_core.inverse_binding sandbox generation found = .ok (.Ok ()) ↔
      found = some (sandbox, generation) := by
  rw [inverse_refines]
  by_cases exactBinding : found = some (sandbox, generation) <;>
    simp [inverseExpected, exactBinding]

theorem inverse_missing_rejected (sandbox generation : U64) :
    credential_core.inverse_binding sandbox generation none = .ok (.Err .Mismatch) := by
  rw [inverse_refines]
  simp [inverseExpected]

theorem inverse_mismatch_rejected (sandbox generation boundSandbox boundGeneration : U64)
    (different : boundSandbox ≠ sandbox ∨ boundGeneration ≠ generation) :
    credential_core.inverse_binding sandbox generation (some (boundSandbox, boundGeneration)) =
      .ok (.Err .Mismatch) := by
  rw [inverse_refines]
  have mismatch : (boundSandbox, boundGeneration) ≠ (sandbox, generation) := by
    intro same
    cases same
    simp at different
  simp [inverseExpected, mismatch]

end CredentialProofs
