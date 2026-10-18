import SandboxModel.ProofAuditCommand
import SandproofStorageMutant.Funs

open Aeneas Aeneas.Std
namespace SandproofStorageRust.FaultWitness

def Fenced {T : Type} (ledger : store.RedisLedger T) : Prop :=
  ledger.write_fenced = true ∧ ledger.connection = none

theorem early_release_counterexample {T R : Type} (ledger : store.RedisLedger T) :
    ∃ next, store.RedisLedger.fence_unknown R ledger = .ok (.Err .OutcomeUnknown, next) ∧
      ¬ Fenced next := by
  refine ⟨{ ledger with connection := none, write_fenced := false }, rfl, ?_⟩
  simp [Fenced]

theorem delivery_error_is_allowed (input : store.storage_io.Cas) (deadline : std.time.Instant) :
    store.storage_io.eval (some ⟨1, [.deliveryUnknown], []⟩) input deadline =
      .ok (.Err .Unavailable, none) := by
  rfl

/- The premises here are byte-for-byte the successful model's actual key,
deployment and EVAL call premises. No error is reclassified as no effect. -/
theorem early_release_after_eval_error {T : Type} (ledger : store.RedisLedger T)
    (prior : String) (expected : Slice U8) (input : store.DecodedWrite T)
    (key : String) (deployment : Str) (error : sandproof_center_store.StoreError)
    (connection : Option redis.connection.Connection)
    (keyResult : store.StoreConfig.key ledger.config = .ok key)
    (deploymentResult : sandproof_protocol.DeploymentId.as_str ledger.config.deployment = .ok deployment)
    (evalResult : store.storage_io.eval ledger.connection
      { script := scripts.CAS, key, deployment, format := input.format,
        prior, revision := input.revision, payload := input.encoded.deref,
        limit := ledger.config.max_block_bytes, expected } input.deadline = .ok (.Err error, connection)) :
    ∃ next, store.RedisLedger.write_normalized ledger prior expected input =
      .ok (.Err .OutcomeUnknown, next) ∧ ¬ Fenced next := by
  refine ⟨{ ledger with connection := none, write_fenced := false }, ?_, ?_⟩
  · simp [store.RedisLedger.write_normalized, keyResult, deploymentResult,
      sandproof_protocol.internal.PositiveCounter.get, evalResult, uncurry,
      store.RedisLedger.fence_unknown]
  · simp [Fenced]

#audit_controller_proof SandproofStorageRust.FaultWitness.early_release_counterexample
#audit_controller_proof SandproofStorageRust.FaultWitness.delivery_error_is_allowed
#audit_controller_proof SandproofStorageRust.FaultWitness.early_release_after_eval_error

end SandproofStorageRust.FaultWitness
