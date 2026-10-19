import SandproofStorage.Funs

open Aeneas Aeneas.Std
namespace SandproofStorageRust.Results

theorem bind_returns {A B : Type} (action : Result A) (next : A → Result B)
    {output : B} (run : (do let value ← action; next value) = .ok output) :
    ∃ value, action = .ok value ∧ next value = .ok output := by
  cases action with
  | ret value => exact ⟨value, rfl, by simpa using run⟩
  | vis effect continuation => simp at run
  | div => simp at run

def Fenced {T : Type} (ledger : store.RedisLedger T) : Prop :=
  ledger.write_fenced = true ∧ ledger.connection = none

theorem fence_unknown_exact {T R : Type} (ledger : store.RedisLedger T) :
    store.RedisLedger.fence_unknown R ledger =
      .ok (.Err .OutcomeUnknown, { ledger with connection := none, write_fenced := true }) := by
  rfl

theorem fence_unknown_result {T R : Type} (ledger next : store.RedisLedger T)
    (run : store.RedisLedger.fence_unknown R ledger = .ok (.Err .OutcomeUnknown, next)) :
    Fenced next := by
  simp only [fence_unknown_exact, Result.ok.injEq, Prod.mk.injEq, true_and] at run
  subst next
  exact ⟨rfl, rfl⟩

theorem normalized_unknown_is_fenced {T : Type} (ledger next : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (run : store.RedisLedger.write_normalized ledger prior expected input =
      .ok (.Err .OutcomeUnknown, next)) : Fenced next := by
  unfold store.RedisLedger.write_normalized at run
  obtain ⟨key, _keyResult, run⟩ := bind_returns _ _ run
  obtain ⟨deployment, _deploymentResult, run⟩ := bind_returns _ _ run
  simp only [sandproof_protocol.internal.PositiveCounter.get, bind_tc_ok] at run
  obtain ⟨⟨reply, connection⟩, _evalResult, run⟩ := bind_returns _ _ run
  cases reply with
  | Err error => exact fence_unknown_result _ _ run
  | Ok code =>
    simp only [Aeneas.Std.uncurry] at run
    split at run
    · obtain ⟨⟨health, checked⟩, _healthResult, run⟩ := bind_returns _ _ run
      simp only [Aeneas.Std.uncurry] at run
      cases health with
      | Err error =>
        simp only [core.result.Result.is_err, bind_tc_ok, ↓reduceIte] at run
        exact fence_unknown_result _ _ run
      | Ok _value =>
        simp [core.result.Result.is_err,
          sandproof_protocol.DeploymentId.Insts.CoreCloneClone.clone,
          alloc.string.String.Insts.CoreCloneClone.clone] at run
    · simp at run
    · simp at run
    · simp at run
    · simp at run
    · exact fence_unknown_result _ _ run

theorem decoded_unknown_is_fenced {T : Type} (ledger next : store.RedisLedger T)
    (expected : Option (store.Observed T)) (input : store.DecodedWrite T)
    (run : finish ledger expected input = .ok (.Err .OutcomeUnknown, next)) : Fenced next := by
  unfold finish store.RedisLedger.write_decoded at run
  cases expected with
  | none =>
    simp only [alloc.string.String.new, bind_tc_ok] at run
    obtain ⟨empty, _emptyResult, run⟩ := bind_returns _ _ run
    exact normalized_unknown_is_fenced _ _ _ _ _ run
  | some previous =>
    simp only [sandproof_protocol.internal.PositiveCounter.get, bind_tc_ok] at run
    obtain ⟨prior, _priorResult, run⟩ := bind_returns _ _ run
    simp only [alloc.vec.Vec.as_slice, bind_tc_ok] at run
    exact normalized_unknown_is_fenced _ _ _ _ _ run

theorem commit_fenced_preserves_all_inputs {T C D : Type}
    (codecApi : store.ValueCodec C T) (clockApi : store.OperationClock D)
    (ledger : store.RedisLedger T) (expected : store.Observed T) (value : T)
    (format : U32) (codec : C) (clock : D) (fenced : ledger.write_fenced = true) :
    commit codecApi clockApi ledger expected value format codec clock =
      .ok (.Err .WriteFenced, ledger, codec, clock) := by
  simp [commit, store.RedisLedger.commit_using, fenced]

def SuccessEvidence {T : Type} (ledger next : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (receipt : store.Committed T) : Prop :=
  ∃ key deployment connection checked,
    store.StoreConfig.key ledger.config = .ok key ∧
    sandproof_protocol.DeploymentId.as_str ledger.config.deployment = .ok deployment ∧
    store.storage_io.eval ledger.connection
      { script := scripts.CAS, key, deployment, format := input.format,
        prior, revision := input.revision, payload := input.encoded.deref,
        limit := ledger.config.max_block_bytes, expected } input.deadline =
      .ok (.Ok 0#i64, connection) ∧
    store.RedisLedger.check_health { ledger with connection := connection } input.deadline =
      .ok (.Ok (), checked) ∧
    receipt = store.Observed.mk checked.config.deployment input.revision input.state input.encoded ∧
    next = { checked with write_fenced := false }

theorem normalized_success_evidence {T : Type} (ledger next : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (receipt : store.Committed T)
    (run : store.RedisLedger.write_normalized ledger prior expected input =
      .ok (.Ok receipt, next)) : SuccessEvidence ledger next prior expected input receipt := by
  unfold store.RedisLedger.write_normalized at run
  obtain ⟨key, keyResult, run⟩ := bind_returns _ _ run
  obtain ⟨deployment, deploymentResult, run⟩ := bind_returns _ _ run
  simp only [sandproof_protocol.internal.PositiveCounter.get, bind_tc_ok] at run
  obtain ⟨⟨reply, connection⟩, evalResult, run⟩ := bind_returns _ _ run
  cases reply with
  | Err error => simp [store.RedisLedger.fence_unknown, uncurry] at run
  | Ok code =>
    simp only [uncurry] at run
    split at run
    · obtain ⟨⟨health, checked⟩, healthResult, run⟩ := bind_returns _ _ run
      simp only [uncurry] at run
      cases health with
      | Err error =>
        simp [core.result.Result.is_err, store.RedisLedger.fence_unknown] at run
      | Ok value =>
        cases value
        simp [core.result.Result.is_err,
          sandproof_protocol.DeploymentId.Insts.CoreCloneClone.clone,
          alloc.string.String.Insts.CoreCloneClone.clone] at run
        rcases run with ⟨rfl, rfl⟩
        exact ⟨key, deployment, connection, checked, keyResult, deploymentResult,
          evalResult, healthResult, rfl, rfl⟩
    · simp at run
    · simp at run
    · simp at run
    · simp at run
    · simp [store.RedisLedger.fence_unknown] at run

theorem success_preserves_complete_payload {T : Type} (ledger next : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (receipt : store.Committed T)
    (run : store.RedisLedger.write_normalized ledger prior expected input =
      .ok (.Ok receipt, next)) :
    receipt.revision = input.revision ∧ receipt.encoded = input.encoded ∧
      receipt.state = input.state ∧ next.write_fenced = false := by
  obtain ⟨_, _, _, checked, _, _, _, _, same, result⟩ :=
    normalized_success_evidence ledger next prior expected input receipt run
  subst receipt
  subst next
  exact ⟨rfl, rfl, rfl, rfl⟩

theorem eval_error_returns_unknown {T : Type} (ledger : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (key : String) (deployment : Str) (error : sandproof_center_store.StoreError)
    (connection : Option redis.connection.Connection)
    (keyResult : store.StoreConfig.key ledger.config = .ok key)
    (deploymentResult : sandproof_protocol.DeploymentId.as_str ledger.config.deployment = .ok deployment)
    (evalResult : store.storage_io.eval ledger.connection
      { script := scripts.CAS, key, deployment, format := input.format,
        prior, revision := input.revision, payload := input.encoded.deref,
        limit := ledger.config.max_block_bytes, expected } input.deadline = .ok (.Err error, connection)) :
    store.RedisLedger.write_normalized ledger prior expected input =
      .ok (.Err .OutcomeUnknown, { ledger with connection := none, write_fenced := true }) := by
  simp [store.RedisLedger.write_normalized, keyResult, deploymentResult,
    sandproof_protocol.internal.PositiveCounter.get, evalResult, uncurry,
    store.RedisLedger.fence_unknown]

def CodeRejection (code : I64) (error : sandproof_center_store.StoreError) : Prop :=
  (code = 1#i64 ∧ error = .Missing) ∨
  (code = 2#i64 ∧ error = .AlreadyInitialized) ∨
  (code = 3#i64 ∧ error = .InvalidRecord) ∨
  (code = 4#i64 ∧ error = .Conflict)

/- This is return-code interpretation only. Physical no-write/rollback requires
the separate own CAS-script obligation; it is not inferred from an Err tag. -/
theorem known_code_preserves_fence {T : Type} (ledger : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (key : String) (deployment : Str) (code : I64) (error : sandproof_center_store.StoreError)
    (connection : Option redis.connection.Connection)
    (keyResult : store.StoreConfig.key ledger.config = .ok key)
    (deploymentResult : sandproof_protocol.DeploymentId.as_str ledger.config.deployment = .ok deployment)
    (evalResult : store.storage_io.eval ledger.connection
      { script := scripts.CAS, key, deployment, format := input.format,
        prior, revision := input.revision, payload := input.encoded.deref,
        limit := ledger.config.max_block_bytes, expected } input.deadline = .ok (.Ok code, connection))
    (known : CodeRejection code error) :
    store.RedisLedger.write_normalized ledger prior expected input =
      .ok (.Err error, { ledger with connection := connection }) := by
  rcases known with ⟨rfl, rfl⟩ | ⟨rfl, rfl⟩ | ⟨rfl, rfl⟩ | ⟨rfl, rfl⟩
  all_goals
    simp [store.RedisLedger.write_normalized, keyResult, deploymentResult,
      sandproof_protocol.internal.PositiveCounter.get, evalResult, uncurry]
    rfl

#print axioms decoded_unknown_is_fenced
#print axioms normalized_success_evidence
#print axioms commit_fenced_preserves_all_inputs
#print axioms eval_error_returns_unknown
#print axioms known_code_preserves_fence

end SandproofStorageRust.Results
