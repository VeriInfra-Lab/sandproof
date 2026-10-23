import SandboxModel.Lifecycle.Protocol.Confirmation
import SandboxModel.Lifecycle.Protocol.RetentionValues

/-! Correspondence for the actual protocol confirmation and controller deletion rule.
The SQL adapter must supply same-transaction fields and the true source pin.
Version ordering, durable commit, and crash/recovery reachability remain caller obligations. -/

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.identity
open sandproof_protocol.lifecycle.request sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.renewal
namespace SandboxModel.Stage1Protocol.Retention

theorem intent_eq (a b : AcceptedLifecycleIntent) :
    AcceptedLifecycleIntent.Insts.CoreCmpPartialEqAcceptedLifecycleIntent.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  rcases a with ⟨ra, va, ⟨da, ca⟩, ⟨sa, wa⟩⟩
  rcases b with ⟨rb, vb, ⟨db, cb⟩, ⟨sb, wb⟩⟩
  simp [AcceptedLifecycleIntent.Insts.CoreCmpPartialEqAcceptedLifecycleIntent.eq,
    Replay.request_eq, CenterIdentity.Insts.CoreCmpPartialEqCenterIdentity.eq,
    CenterId.Insts.CoreCmpPartialEqCenterId.eq,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    RetentionBinding.Insts.CoreCmpPartialEqRetentionBinding.eq,
    RetentionWindowId.Insts.CoreCmpPartialEqRetentionWindowId.eq,
    PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    Replay.ok_and, AcceptedLifecycleIntent.mk.injEq, CenterIdentity.mk.injEq,
    RetentionBinding.mk.injEq, Bool.and_assoc, Bool.and_left_comm]

theorem matches_refines (confirmation : RenewalConfirmation) (intent : AcceptedLifecycleIntent) :
    RenewalConfirmation.matches_request confirmation intent =
      .ok (by classical exact decide (confirmation.request = intent ∧ ValidConfirmation confirmation)) := by
  classical
  simp [RenewalConfirmation.matches_request, intent_eq, confirmation_refines, Replay.ok_and]

theorem discard_refines : FullRetentionContract renewal_rule := by
  classical
  intro intent applied previous pending confirmation current pinned
  have heq : ∀ a b : I64, core.cmp.PartialEqI64.eq a b = .ok (decide (a = b)) := by
    intros; simp
  cases hp : pinned <;> cases ho : intent.request.operation <;>
    simp [renewal_rule, sandproof_controller_service.lifecycle_renewal.can_discard_confirmed_sdk,
      ho, RequestWindowId.get, PositiveCounter.get, matches_refines,
      option_eq core.cmp.PartialEqI64 heq, MayDiscard, IsRenew,
      core.option.Option.is_some, core.option.Option.is_none, Replay.ok_and, Bool.and_assoc]
  have hw : (intent.request.binding.key.window = 2#u64) ↔
      intent.request.binding.key.window.val = 2 := by scalar_tac
  have hn : (intent.accepted_version != current) = !decide (intent.accepted_version.val = current.val) := by
    apply Bool.eq_iff_iff.mpr
    simp [bne, BEq.beq]
    exact not_congr BitVec.toNat_eq
  simp [← ho, hw, hn]

theorem discard_iff (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) :
    renewal_rule intent applied previous pending confirmation current pinned = .ok true ↔
      MayDiscard intent applied previous pending confirmation current pinned := by
  classical
  rw [discard_refines]; simp

theorem preserve_current (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (pinned : Bool) :
    renewal_rule intent applied previous pending confirmation intent.accepted_version pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard]

theorem preserve_pinned (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) :
    renewal_rule intent applied previous pending confirmation current true = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard]

theorem preserve_unapplied (intent : AcceptedLifecycleIntent) (previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) :
    renewal_rule intent none previous pending confirmation current pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard]

theorem preserve_pending (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (operation : sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) :
    renewal_rule intent applied previous (some operation) confirmation current pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard]

theorem preserve_other_intent (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) (different : confirmation.request ≠ intent) :
    renewal_rule intent applied previous pending confirmation current pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard, different]

theorem deleted_is_older (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool)
    (ordered : intent.accepted_version.val ≤ current.val)
    (deleted : renewal_rule intent applied previous pending confirmation current pinned = .ok true) :
    intent.accepted_version.val < current.val := by
  have h := (discard_iff _ _ _ _ _ _ _).mp deleted
  have ne := h.2.2.2.2.2.2.2.2
  scalar_tac

theorem preserve_public (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) (notSdk : intent.request.binding.key.window.val ≠ 2) :
    renewal_rule intent applied previous pending confirmation current pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard, notSdk]

theorem preserve_nonrenew (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) (other : ¬ IsRenew intent.request.operation) :
    renewal_rule intent applied previous pending confirmation current pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard, other]

theorem preserve_missing_previous (intent : AcceptedLifecycleIntent) (applied : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) :
    renewal_rule intent applied none pending confirmation current pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard]

theorem preserve_changed_end (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) (different : applied ≠ some confirmation.effective_end_at_ms) :
    renewal_rule intent applied previous pending confirmation current pinned = .ok false := by
  classical
  rw [discard_refines]; simp [MayDiscard, different]

theorem eligible_checkpoint_discarded :
    renewal_rule checkpoint.request (some 200#i64) (some 100#i64) none checkpoint 3#u64 false = .ok true := by
  apply (discard_iff _ _ _ _ _ _ _).mpr
  simp [MayDiscard, checkpoint, Request.renew, Request.binding, Request.node,
    IsRenew, ValidConfirmation, Successor, DeadlineOK,
    Validation.ValidRequest, Validation.headersOK, Validation.expectationOK, Validation.operationOK]

theorem different_center_preserved :
    renewal_rule otherCenter (some 200#i64) (some 100#i64) none checkpoint 3#u64 false = .ok false := by
  apply preserve_other_intent
  simp [otherCenter, checkpoint, AcceptedLifecycleIntent.mk.injEq, CenterIdentity.mk.injEq]
end SandboxModel.Stage1Protocol.Retention

#audit_controller_proof SandboxModel.Stage1Protocol.Retention.matches_refines
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.discard_refines
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.discard_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_current
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_pinned
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_unapplied
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_pending
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_other_intent
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.deleted_is_older
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_public
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_nonrenew
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_missing_previous
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.preserve_changed_end
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.eligible_checkpoint_discarded
#audit_controller_proof SandboxModel.Stage1Protocol.Retention.different_center_preserved

#audit_controller_proof SandboxModel.Stage1Protocol.Retention.intent_eq
