import SandboxModel.Lifecycle.Protocol.RenewalModel
import SandboxModel.Lifecycle.Protocol.Accepted
import SandboxModel.Lifecycle.Protocol.RenewalResultBindings

/-! Exact decoded-value refinement of the real Rust renewal guards. -/
set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.renewal sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.Renewal

theorem request_is_error (request : LifecycleRequest) (node : sandproof_protocol.internal.NodeIdentity) :
    (do let result ← LifecycleRequest.validate request node
        SandproofStage1RequestRust.core.result.Result.is_err result) =
      .ok (by classical exact decide (¬ ValidRequest request node)) := by
  classical
  rw [validate_body_refines]
  cases h : expectedResult request node with
  | Ok value =>
    cases value
    have valid := (expected_success_iff request node).mp h
    simp [SandboxModel.Stage1Protocol.RenewalResultTag.actual_is_err,
      SandboxModel.Stage1Protocol.RenewalResultTag.IsError, valid]
  | Err error =>
    have invalid : ¬ ValidRequest request node := by
      intro valid
      have success := (expected_success_iff request node).mpr valid
      rw [h] at success
      cases success
    simp [SandboxModel.Stage1Protocol.RenewalResultTag.actual_is_err,
      SandboxModel.Stage1Protocol.RenewalResultTag.IsError, invalid]

theorem request_error_bind {T : Type} (request : LifecycleRequest)
    (node : sandproof_protocol.internal.NodeIdentity) (next : Bool → Result T) :
    (do let result ← LifecycleRequest.validate request node
        let bad ← SandproofStage1RequestRust.core.result.Result.is_err result
        next bad) = next (by classical exact decide (¬ ValidRequest request node)) := by
  classical
  rw [← bind_assoc_eq, request_is_error]
  simp


end SandboxModel.Stage1Protocol.Renewal
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.request_is_error
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.request_error_bind
