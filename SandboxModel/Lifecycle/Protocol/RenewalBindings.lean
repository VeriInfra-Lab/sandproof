import SandboxModel.Lifecycle.Protocol.RenewalEquality
import SandboxModel.Lifecycle.Protocol.RenewalLibrary
/-! Exact decoded-value refinement of the real Rust renewal guards.
The historical Retention.confirmation_refines shape is rebound to protocol 4
and the actual extracted is_err. Full request/result matching is separate. -/
set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.renewal sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.Renewal

theorem shape_refines (value : RenewalConfirmation) :
    renewal_valid value = .ok (by classical exact decide (Shape value)) := by
  classical
  rcases value with ⟨⟨⟨binding, operation⟩, version, center, retention⟩, effective⟩
  have one_version : (1#u64 = version) ↔ version.val = 1 := by scalar_tac
  have version_one : (version = 1#u64) ↔ version.val = 1 := by scalar_tac
  have version_bit : (!decide (version.val ≤ 1)) = decide (1 < version.val) := by
    by_cases low : version.val ≤ 1
    · have high : ¬ 1 < version.val := by omega
      simp [low, high]
    · have high : 1 < version.val := by omega
      simp [low, high]
  cases he : binding.expected <;> cases operation <;>
    simp [renewal_valid, RenewalConfirmation.is_sdk_renewal, Shape, Deadline,
      RequestWindowId.get, sandproof_protocol.internal.PositiveCounter.get,
      AcceptedLifecycleIntent.accepted_version_for_request, he, versionOK,
      Library.option_u64_eq, core.cmp.PartialEq.ne.trait_default,
      core.cmp.PartialEq.ne.default,
      sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
      alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      retention_valid, request_error_bind, ge_iff_le, one_version, version_one, Nat.succ_le_iff]
  all_goals simp [← ok_ite, version_bit]

theorem matches_refines (value : RenewalConfirmation) (request : AcceptedLifecycleIntent) :
    renewal_matches value request =
      .ok (by classical exact decide (Matches value request)) := by
  classical
  have shape := shape_refines value
  simp only [renewal_valid] at shape
  simp [renewal_matches, RenewalConfirmation.matches_request, Replay.accepted_eq,
    shape, Matches, Replay.ok_and]

theorem confirmed_refines (ack : ConfirmedRenewal) (value : RenewalConfirmation) :
    renewal_confirmed_matches ack value =
      .ok (by classical exact decide (ConfirmedMatches ack value)) := by
  classical
  have shape := shape_refines value
  simp only [renewal_valid] at shape
  simp [renewal_confirmed_matches, ConfirmedRenewal.matches, equality_refines,
    shape, ConfirmedMatches, Replay.ok_and]


theorem shape_complete : FullShapeContract renewal_valid := shape_refines

theorem matches_complete : FullMatchesContract renewal_matches := matches_refines

theorem confirmed_complete : FullConfirmedContract renewal_confirmed_matches := confirmed_refines

theorem shape_accepts_iff (value : RenewalConfirmation) :
    renewal_valid value = .ok true ↔ Shape value := by
  classical
  simp [shape_refines]

theorem matches_accepts_iff (value : RenewalConfirmation) (request : AcceptedLifecycleIntent) :
    renewal_matches value request = .ok true ↔ Matches value request := by
  classical
  simp [matches_refines]

theorem confirmed_accepts_iff (ack : ConfirmedRenewal) (value : RenewalConfirmation) :
    renewal_confirmed_matches ack value = .ok true ↔ ConfirmedMatches ack value := by
  classical
  simp [confirmed_refines]

end SandboxModel.Stage1Protocol.Renewal
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.shape_refines
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.matches_refines
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.confirmed_refines
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.shape_complete
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.matches_complete
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.confirmed_complete
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.shape_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.matches_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.confirmed_accepts_iff
