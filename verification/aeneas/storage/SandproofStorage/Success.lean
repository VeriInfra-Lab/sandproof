import SandproofStorage.Caller
import SandproofStorage.Key
import SandboxModel.Foundations.Cloning

open Aeneas Aeneas.Std
namespace SandproofStorageRust.Results

theorem normalized_success_deployment {T : Type} (ledger after : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (receipt : store.Committed T)
    (run : store.RedisLedger.write_normalized ledger prior expected input = .ok (.Ok receipt, after)) :
    receipt.deployment = ledger.config.deployment := by
  obtain ⟨_, _, connection, checked, _, _, _, healthResult, same, _⟩ :=
    normalized_success_evidence ledger after prior expected input receipt run
  have config := health_preserves_config { ledger with connection := connection }
    checked input.deadline (.Ok ()) healthResult
  have deployment := congrArg store.StoreConfig.deployment config
  rw [same]
  exact deployment

def Metadata {T : Type} (ledger after : store.RedisLedger T)
    (encoded : alloc.vec.Vec U8) (revision : U64) (state : T)
    (receipt : store.Committed T) : Prop :=
  receipt.deployment = ledger.config.deployment ∧ receipt.revision = revision ∧
    receipt.encoded = encoded ∧ receipt.state = state ∧ after.write_fenced = false

theorem normalized_success_metadata {T : Type} (ledger after : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (receipt : store.Committed T)
    (run : store.RedisLedger.write_normalized ledger prior expected input = .ok (.Ok receipt, after)) :
    Metadata ledger after input.encoded input.revision input.state receipt := by
  have fields := success_preserves_complete_payload ledger after prior expected input receipt run
  exact ⟨normalized_success_deployment ledger after prior expected input receipt run,
    fields.1, fields.2.1, fields.2.2.1, fields.2.2.2⟩

theorem decoded_success_metadata {T : Type} (ledger after : store.RedisLedger T)
    (expected : Option (store.Observed T)) (input : store.DecodedWrite T)
    (receipt : store.Committed T)
    (run : store.RedisLedger.write_decoded ledger expected input = .ok (.Ok receipt, after)) :
    Metadata ledger after input.encoded input.revision input.state receipt := by
  unfold store.RedisLedger.write_decoded at run
  cases expected with
  | none =>
    simp only [alloc.string.String.new, bind_tc_ok] at run
    obtain ⟨empty, _emptyResult, run⟩ := bind_returns _ _ run
    exact normalized_success_metadata _ _ _ _ _ _ run
  | some previous =>
    simp only [sandproof_protocol.internal.PositiveCounter.get, bind_tc_ok] at run
    obtain ⟨prior, _priorResult, run⟩ := bind_returns _ _ run
    simp only [alloc.vec.Vec.as_slice, bind_tc_ok] at run
    exact normalized_success_metadata _ _ _ _ _ _ run

theorem write_using_success_evidence {T C : Type} (api : store.ValueCodec C T)
    (ledger after : store.RedisLedger T) (expected : Option (store.Observed T))
    (encoded : alloc.vec.Vec U8) (revision : U64) (deadline : std.time.Instant)
    (format : U32) (codec finalCodec : C) (receipt : store.Committed T)
    (run : store.RedisLedger.write_using api ledger expected encoded revision deadline format codec =
      .ok (.Ok receipt, after, finalCodec)) :
    ∃ state, api.decode codec encoded.deref ledger.config.max_block_bytes = .ok (.Ok state, finalCodec) ∧
      Metadata ledger after encoded revision state receipt := by
  unfold store.RedisLedger.write_using at run
  obtain ⟨⟨decoded, updatedCodec⟩, decodeResult, run⟩ := bind_returns _ _ run
  simp only [uncurry] at run
  cases decoded with
  | Err error =>
    simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
  | Ok value =>
    simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
    obtain ⟨⟨result, nextLedger⟩, resultProof, run⟩ := bind_returns _ _ run
    simp only [uncurry, Result.ok.injEq, Prod.mk.injEq] at run
    rcases run with ⟨rfl, rfl, rfl⟩
    exact ⟨value, decodeResult, decoded_success_metadata _ _ _ _ _ resultProof⟩

theorem revision_success_is_fresh (before after : U64)
    (run : sandproof_protocol.internal.PositiveCounter.checked_next before = .ok (.Ok after)) :
    after.val = before.val + 1 ∧ before.val < after.val := by
  have arithmetic := U64.checked_add_bv_spec before 1#u64
  simp only [sandproof_protocol.internal.PositiveCounter.checked_next,
    sandproof_protocol.internal.PositiveCounter.get, bind_tc_ok, lift] at run
  cases addition : U64.checked_add before 1#u64 with
  | none => simp [addition] at run
  | some next =>
    simp only [addition,
      sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from] at run
    split at run
    · simp at run
    · simp only [Result.ok.injEq, core.result.Result.Ok.injEq] at run
      subst after
      simp only [addition] at arithmetic
      scalar_tac

theorem deployment_ne_false (left right : String)
    (run : core.cmp.PartialEq.ne.trait_default
      sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId left right = .ok false) :
    left = right := by
  simpa [core.cmp.PartialEq.ne.trait_default,
    core.cmp.PartialEq.ne.default,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq] using run

theorem payload_clone_exact (bytes : alloc.vec.Vec U8) :
    alloc.vec.CloneVec.clone core.clone.CloneU8 bytes = .ok bytes := by
  exact SandboxModel.Foundations.vec_clone_exact _ bytes (fun _ => rfl)

theorem decoded_original_binding {T : Type} (ledger : store.RedisLedger T)
    (expected : store.Observed T) (input : store.DecodedWrite T) :
    store.RedisLedger.write_decoded ledger (some expected) input =
      store.RedisLedger.write_normalized ledger (toString expected.revision.val)
        expected.encoded.deref input := by
  simp [store.RedisLedger.write_decoded, sandproof_protocol.internal.PositiveCounter.get,
    alloc.string.ToString.Blanket.to_string,
    SandproofFoundationsRust.alloc.string.ToString.Blanket.to_string,
    core.fmt.num.imp.DisplayU64.fmt,
    SandproofFoundationsRust.core.fmt.num.imp.DisplayU64.fmt, alloc.vec.Vec.as_slice, uncurry]

theorem replace_success_evidence {T C : Type} (api : store.ValueCodec C T)
    (ledger after : store.RedisLedger T) (expected : store.Observed T)
    (encoded : alloc.vec.Vec U8) (deadline : std.time.Instant) (format : U32)
    (codec finalCodec : C) (receipt : store.Committed T)
    (run : store.RedisLedger.replace_using api ledger expected encoded deadline format codec =
      .ok (.Ok receipt, after, finalCodec)) :
    expected.deployment = ledger.config.deployment ∧
    ∃ revision state,
      sandproof_protocol.internal.PositiveCounter.checked_next expected.revision = .ok (.Ok revision) ∧
      api.decode codec encoded.deref ledger.config.max_block_bytes = .ok (.Ok state, finalCodec) ∧
      Metadata ledger after encoded revision state receipt := by
  unfold store.RedisLedger.replace_using at run
  obtain ⟨foreign, identityResult, run⟩ := bind_returns _ _ run
  by_cases rejected : foreign = true
  · rw [if_pos rejected] at run
    simp at run
  · rw [if_neg rejected] at run
    have noForeign : foreign = false := by
      cases foreign with
      | false => rfl
      | true => exact False.elim (rejected rfl)
    have identity := deployment_ne_false expected.deployment ledger.config.deployment
      (by rw [noForeign] at identityResult; exact identityResult)
    obtain ⟨revision, revisionResult, run⟩ := bind_returns _ _ run
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
      | Ok value =>
        simp only [bind_tc_ok] at run
        obtain ⟨state, decoded, metadata⟩ := write_using_success_evidence api checked after
          (some expected) encoded nextRevision deadline format codec finalCodec receipt run
        have config := health_preserves_config ledger checked deadline (.Ok value) healthResult
        refine ⟨identity, nextRevision, state, revisionResult, ?_, ?_⟩
        · simpa only [config] using decoded
        · simpa only [Metadata, config] using metadata

theorem reaffirm_success_uses_original_payload_and_fresh_revision {T C D : Type}
    (api : store.ValueCodec C T) (clockApi : store.OperationClock D)
    (ledger after : store.RedisLedger T) (expected : store.Observed T)
    (format : U32) (codec finalCodec : C) (clock finalClock : D) (receipt : store.Committed T)
    (run : reaffirm api clockApi ledger expected format codec clock =
      .ok (.Ok receipt, after, finalCodec, finalClock)) :
    receipt.deployment = expected.deployment ∧ receipt.encoded = expected.encoded ∧
      receipt.revision.val = expected.revision.val + 1 ∧
      expected.revision.val < receipt.revision.val ∧ after.write_fenced = false := by
  unfold reaffirm store.RedisLedger.reaffirm_using at run
  rw [payload_clone_exact] at run
  simp only [bind_tc_ok] at run
  obtain ⟨⟨deadline, updatedClock⟩, _clockResult, run⟩ := bind_returns _ _ run
  simp only [uncurry] at run
  obtain ⟨⟨result, nextLedger, nextCodec⟩, resultProof, run⟩ := bind_returns _ _ run
  simp only [uncurry, Result.ok.injEq, Prod.mk.injEq] at run
  rcases run with ⟨rfl, rfl, rfl, rfl⟩
  obtain ⟨identity, revision, state, revisionResult, _decoded, fields⟩ :=
    replace_success_evidence api ledger _ expected expected.encoded deadline format codec _ receipt resultProof
  have fresh := revision_success_is_fresh expected.revision revision revisionResult
  exact ⟨fields.1.trans identity.symm, fields.2.2.1,
    by rw [fields.2.1]; exact fresh.1,
    by rw [fields.2.1]; exact fresh.2, fields.2.2.2.2⟩

def CommitEvidence {T C D : Type} (api : store.ValueCodec C T)
    (clockApi : store.OperationClock D)
    (ledger after : store.RedisLedger T) (expected : store.Observed T) (value : T)
    (codec finalCodec : C) (clock finalClock : D) (receipt : store.Committed T) : Prop :=
  ∃ deadline encoded encodedCodec,
    clockApi.deadline clock ledger.config.operation_timeout = .ok (deadline, finalClock) ∧
    api.encode codec value ledger.config.max_block_bytes = .ok (.Ok encoded, encodedCodec) ∧
    api.decode encodedCodec encoded.deref ledger.config.max_block_bytes = .ok (.Ok receipt.state, finalCodec) ∧
    receipt.deployment = expected.deployment ∧ receipt.encoded = encoded ∧
    receipt.revision.val = expected.revision.val + 1 ∧ expected.revision.val < receipt.revision.val ∧
    after.write_fenced = false

theorem commit_success_evidence {T C D : Type} (api : store.ValueCodec C T)
    (clockApi : store.OperationClock D)
    (ledger after : store.RedisLedger T) (expected : store.Observed T) (value : T)
    (format : U32) (codec finalCodec : C) (clock finalClock : D) (receipt : store.Committed T)
    (run : commit api clockApi ledger expected value format codec clock =
      .ok (.Ok receipt, after, finalCodec, finalClock)) :
    CommitEvidence api clockApi ledger after expected value codec finalCodec clock finalClock receipt := by
  unfold commit store.RedisLedger.commit_using at run
  by_cases blocked : ledger.write_fenced = true
  · rw [if_pos blocked] at run
    simp at run
  · rw [if_neg blocked] at run
    obtain ⟨⟨deadline, updatedClock⟩, clockResult, run⟩ := bind_returns _ _ run
    simp only [uncurry] at run
    obtain ⟨⟨encoded, updatedCodec⟩, encodeResult, run⟩ := bind_returns _ _ run
    simp only [uncurry] at run
    cases encoded with
    | Err error =>
      simp [core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
    | Ok bytes =>
      simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
      obtain ⟨⟨result, nextLedger, nextCodec⟩, resultProof, run⟩ := bind_returns _ _ run
      simp only [uncurry, Result.ok.injEq, Prod.mk.injEq] at run
      rcases run with ⟨rfl, rfl, rfl, rfl⟩
      obtain ⟨identity, revision, state, revisionResult, decoded, fields⟩ :=
        replace_success_evidence api ledger _ expected bytes deadline format updatedCodec _ receipt resultProof
      have fresh := revision_success_is_fresh expected.revision revision revisionResult
      refine ⟨deadline, bytes, updatedCodec, clockResult, encodeResult, ?_,
        fields.1.trans identity.symm, fields.2.2.1, ?_, ?_, fields.2.2.2.2⟩
      · simpa only [fields.2.2.2.1] using decoded
      · rw [fields.2.1]
        exact fresh.1
      · rw [fields.2.1]
        exact fresh.2

/- V2 must provide this for the actual payload type, schema and own codec.
The callback states are retained; two unrelated encodings are not equated. -/
def CodecRoundtrip {T C : Type} (api : store.ValueCodec C T) : Prop :=
  ∀ codec value limit encoded updatedCodec decoded finalCodec,
    api.encode codec value limit = .ok (.Ok encoded, updatedCodec) →
    api.decode updatedCodec encoded.deref limit = .ok (.Ok decoded, finalCodec) → decoded = value

theorem commit_success_state {T C D : Type} (api : store.ValueCodec C T)
    (clockApi : store.OperationClock D) (roundtrip : CodecRoundtrip api)
    (ledger after : store.RedisLedger T) (expected : store.Observed T) (value : T)
    (format : U32) (codec finalCodec : C) (clock finalClock : D) (receipt : store.Committed T)
    (run : commit api clockApi ledger expected value format codec clock =
      .ok (.Ok receipt, after, finalCodec, finalClock)) : receipt.state = value := by
  obtain ⟨_, _, _, _, encodeResult, decodeResult, _⟩ :=
    commit_success_evidence api clockApi ledger after expected value format codec finalCodec clock finalClock receipt run
  exact roundtrip _ _ _ _ _ _ _ encodeResult decodeResult

#print axioms write_using_success_evidence
#print axioms revision_success_is_fresh
#print axioms payload_clone_exact
#print axioms reaffirm_success_uses_original_payload_and_fresh_revision
#print axioms commit_success_evidence
#print axioms commit_success_state

end SandproofStorageRust.Results
