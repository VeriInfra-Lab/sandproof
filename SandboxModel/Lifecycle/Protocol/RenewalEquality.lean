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

theorem equality_refines (left right : RenewalConfirmation) :
    RenewalConfirmation.Insts.CoreCmpPartialEqRenewalConfirmation.eq left right =
      .ok (by classical exact decide (left = right)) := by
  classical
  cases left
  cases right
  simp [RenewalConfirmation.Insts.CoreCmpPartialEqRenewalConfirmation.eq,
    Replay.accepted_eq, RenewalConfirmation.mk.injEq, Replay.ok_and, Bool.and_comm]


end SandboxModel.Stage1Protocol.Renewal
#audit_controller_proof SandboxModel.Stage1Protocol.Renewal.equality_refines
