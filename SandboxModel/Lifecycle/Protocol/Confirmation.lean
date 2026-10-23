import SandboxModel.Lifecycle.Protocol.Validation
import SandboxModel.Lifecycle.Protocol.RetentionModel

/-! The actual SDK renewal shape guard and exact primitive comparisons. -/

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.identity
open sandproof_protocol.lifecycle.request sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.renewal
namespace SandboxModel.Stage1Protocol.Retention

theorem option_eq {T : Type} [DecidableEq T] (dictionary : core.cmp.PartialEq T T)
    (h : ∀ a b, dictionary.eq a b = .ok (decide (a = b)))
    (left right : Option T) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq dictionary left right =
      .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq,
      SandproofNetworkRust.core.option.Option.Insts.CoreCmpPartialEqOption.eq, h]

theorem checked_successor (previous accepted : U64) :
    U64.checked_add previous 1#u64 = some accepted ↔ previous.val + 1 = accepted.val := by
  have hs := U64.checked_add_bv_spec previous 1#u64
  cases he : U64.checked_add previous 1#u64 with
  | none => simp_all; scalar_tac
  | some value => simp_all; scalar_tac

theorem validate_is_err (request : LifecycleRequest) :
    (do let result ← LifecycleRequest.validate request request.binding.node
        core.result.Result.is_err result) =
      .ok (by classical exact decide (¬ Validation.ValidRequest request request.binding.node)) := by
  classical
  change (do let result ← request_validate request request.binding.node
             core.result.Result.is_err result) = _
  rw [Validation.validate_refines, ← Validation.expected_success_iff]
  cases Validation.expectedResult request request.binding.node with
  | Ok value => cases value; simp [core.result.Result.is_err]
  | Err error => simp [core.result.Result.is_err]

theorem version_matches (intent : AcceptedLifecycleIntent) :
    (do let next ← AcceptedLifecycleIntent.accepted_version_for_request intent
        core.option.Option.Insts.CoreCmpPartialEqOption.eq core.cmp.PartialEqU64 next
          (some intent.accepted_version)) =
      .ok (by classical exact decide (Successor intent)) := by
  classical
  have heq : ∀ a b : U64, core.cmp.PartialEqU64.eq a b = .ok (decide (a = b)) := by
    intros; simp
  cases he : intent.request.binding.expected <;>
    simp [AcceptedLifecycleIntent.accepted_version_for_request, he, option_eq core.cmp.PartialEqU64 heq,
      checked_successor, Successor]
  scalar_tac

theorem expected_is_err (request : LifecycleRequest) :
    core.result.Result.is_err (Validation.expectedResult request request.binding.node) =
      .ok (by classical exact decide (¬ Validation.ValidRequest request request.binding.node)) := by
  classical
  rw [← Validation.expected_success_iff]
  cases Validation.expectedResult request request.binding.node with
  | Ok value => cases value; simp [core.result.Result.is_err]
  | Err error => simp [core.result.Result.is_err]

theorem confirmation_refines (confirmation : RenewalConfirmation) :
    RenewalConfirmation.is_sdk_renewal confirmation =
      .ok (by classical exact decide (ValidConfirmation confirmation)) := by
  classical
  have heq : ∀ a b : U64, core.cmp.PartialEqU64.eq a b = .ok (decide (a = b)) := by
    intros; simp
  have hv := Validation.validate_refines confirmation.request.request
    confirmation.request.request.binding.node
  change LifecycleRequest.validate _ _ = _ at hv
  cases he : confirmation.request.request.binding.expected <;>
    cases ho : confirmation.request.request.operation <;>
    simp [RenewalConfirmation.is_sdk_renewal,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      RequestWindowId.get, PositiveCounter.get,
      AcceptedLifecycleIntent.accepted_version_for_request, he,
      option_eq core.cmp.PartialEqU64 heq, checked_successor,
      sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
      alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      RetentionBinding.valid_for, hv, expected_is_err, Replay.ok_and,
      ValidConfirmation, Successor, DeadlineOK, ho]
  all_goals
    rename_i incarnation deadline mode
    have h1 : (1#u64 = confirmation.request.accepted_version) ↔
        confirmation.request.accepted_version.val = 1 := by scalar_tac
    have h2 : (¬ confirmation.request.accepted_version.val ≤ 1) ↔
        2 ≤ confirmation.request.accepted_version.val := by omega
    cases mode <;> simp [Replay.ok_and, Validation.ok_unless, Bool.and_assoc, h1, h2]

end SandboxModel.Stage1Protocol.Retention

#audit_controller_proof SandboxModel.Stage1Protocol.Retention.option_eq
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.checked_successor
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.validate_is_err
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.version_matches
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.expected_is_err
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.confirmation_refines
