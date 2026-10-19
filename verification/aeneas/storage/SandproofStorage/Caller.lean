import SandproofStorage.Health

open Aeneas Aeneas.Std
namespace SandproofStorageRust.Results

/- This is an OWN codec obligation for V2, not an A6/B4 library promise.
It restricts errors, not codec effects, schema correctness or roundtrip. -/
def LocalCodecError (error : sandproof_center_store.StoreError) : Prop :=
  error = .InvalidRecord ∨ error = .InvalidState ∨ error = .Capacity

structure CodecErrorContract {C T : Type} (api : store.ValueCodec C T) : Prop where
  encodeErrors : ∀ codec value limit error next,
    api.encode codec value limit = .ok (.Err error, next) → LocalCodecError error
  decodeErrors : ∀ codec bytes limit error next,
    api.decode codec bytes limit = .ok (.Err error, next) → LocalCodecError error

theorem codec_error_not_unknown (error : sandproof_center_store.StoreError)
    (localError : LocalCodecError error) : error ≠ .OutcomeUnknown := by
  rcases localError with same | same | same <;> subst error <;> intro impossible <;> cases impossible

theorem write_using_unknown_is_fenced {T C : Type}
    (api : store.ValueCodec C T) (contract : CodecErrorContract api)
    (ledger after : store.RedisLedger T) (expected : Option (store.Observed T))
    (encoded : alloc.vec.Vec U8) (revision : U64) (deadline : std.time.Instant)
    (format : U32) (codec finalCodec : C)
    (run : store.RedisLedger.write_using api ledger expected encoded revision deadline format codec =
      .ok (.Err .OutcomeUnknown, after, finalCodec)) : Fenced after := by
  unfold store.RedisLedger.write_using at run
  obtain ⟨⟨decoded, updatedCodec⟩, decodeResult, run⟩ := bind_returns _ _ run
  simp only [uncurry] at run
  cases decoded with
  | Err error =>
    have different := codec_error_not_unknown error (contract.decodeErrors _ _ _ _ _ decodeResult)
    simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
      different] at run
  | Ok value =>
    simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
    obtain ⟨⟨result, nextLedger⟩, resultProof, run⟩ := bind_returns _ _ run
    simp only [uncurry, Result.ok.injEq, Prod.mk.injEq] at run
    rcases run with ⟨rfl, rfl, rfl⟩
    exact decoded_unknown_is_fenced _ _ _ _ resultProof

theorem replace_unknown_is_fenced {T C : Type}
    (api : store.ValueCodec C T) (contract : CodecErrorContract api)
    (ledger after : store.RedisLedger T) (expected : store.Observed T)
    (encoded : alloc.vec.Vec U8) (deadline : std.time.Instant)
    (format : U32) (codec finalCodec : C)
    (run : store.RedisLedger.replace_using api ledger expected encoded deadline format codec =
      .ok (.Err .OutcomeUnknown, after, finalCodec)) : Fenced after := by
  unfold store.RedisLedger.replace_using at run
  obtain ⟨foreign, _identityResult, run⟩ := bind_returns _ _ run
  by_cases rejected : foreign = true
  · rw [if_pos rejected] at run
    simp at run
  · rw [if_neg rejected] at run
    obtain ⟨revision, _revisionResult, run⟩ := bind_returns _ _ run
    cases revision with
    | Err error =>
      simp [core.result.Result.map_err,
        store.RedisLedger.replace_using.closure.Insts.CoreOpsFunctionFnOnceTupleInternalErrorStoreError.call_once,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
    | Ok nextRevision =>
      simp only [core.result.Result.map_err, bind_tc_ok,
        core.result.Result.Insts.CoreOpsTry.branch] at run
      obtain ⟨⟨health, checked⟩, healthResult, run⟩ := bind_returns _ _ run
      simp only [uncurry] at run
      cases health with
      | Err error =>
        simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
        rcases run with ⟨rfl, rfl, rfl⟩
        exact False.elim (health_cannot_report_unknown _ _ _ healthResult)
      | Ok value =>
        simp only [bind_tc_ok] at run
        exact write_using_unknown_is_fenced _ contract _ _ _ _ _ _ _ _ _ run

theorem commit_unknown_is_fenced {T C D : Type}
    (api : store.ValueCodec C T) (clockApi : store.OperationClock D)
    (contract : CodecErrorContract api)
    (ledger after : store.RedisLedger T) (expected : store.Observed T) (value : T)
    (format : U32) (codec finalCodec : C) (clock finalClock : D)
    (run : commit api clockApi ledger expected value format codec clock =
      .ok (.Err .OutcomeUnknown, after, finalCodec, finalClock)) : Fenced after := by
  unfold commit store.RedisLedger.commit_using at run
  by_cases blocked : ledger.write_fenced = true
  · rw [if_pos blocked] at run
    simp at run
  · rw [if_neg blocked] at run
    obtain ⟨⟨deadline, updatedClock⟩, _clockResult, run⟩ := bind_returns _ _ run
    simp only [uncurry] at run
    obtain ⟨⟨encoded, updatedCodec⟩, encodeResult, run⟩ := bind_returns _ _ run
    simp only [uncurry] at run
    cases encoded with
    | Err error =>
      have different := codec_error_not_unknown error (contract.encodeErrors _ _ _ _ _ encodeResult)
      simp [core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
        different] at run
    | Ok bytes =>
      simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
      obtain ⟨⟨result, nextLedger, nextCodec⟩, resultProof, run⟩ := bind_returns _ _ run
      simp only [uncurry, Result.ok.injEq, Prod.mk.injEq] at run
      rcases run with ⟨rfl, rfl, rfl, rfl⟩
      exact replace_unknown_is_fenced _ contract _ _ _ _ _ _ _ _ resultProof

theorem reaffirm_unknown_is_fenced {T C D : Type}
    (api : store.ValueCodec C T) (clockApi : store.OperationClock D)
    (contract : CodecErrorContract api)
    (ledger after : store.RedisLedger T) (expected : store.Observed T)
    (format : U32) (codec finalCodec : C) (clock finalClock : D)
    (run : reaffirm api clockApi ledger expected format codec clock =
      .ok (.Err .OutcomeUnknown, after, finalCodec, finalClock)) : Fenced after := by
  unfold reaffirm store.RedisLedger.reaffirm_using at run
  obtain ⟨encoded, _cloneResult, run⟩ := bind_returns _ _ run
  obtain ⟨⟨deadline, updatedClock⟩, _clockResult, run⟩ := bind_returns _ _ run
  simp only [uncurry] at run
  obtain ⟨⟨result, nextLedger, nextCodec⟩, resultProof, run⟩ := bind_returns _ _ run
  simp only [uncurry, Result.ok.injEq, Prod.mk.injEq] at run
  rcases run with ⟨rfl, rfl, rfl, rfl⟩
  exact replace_unknown_is_fenced _ contract _ _ _ _ _ _ _ _ resultProof

#print axioms commit_unknown_is_fenced
#print axioms reaffirm_unknown_is_fenced

end SandproofStorageRust.Results
