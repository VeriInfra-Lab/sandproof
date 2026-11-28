import SandboxModel.Lifecycle.Protocol.ValidationModel

/-! Complete decoded-value contracts, shared unchanged by correct and mutant Rust.
The shape preserves the protocol-3 Retention.ValidConfirmation conditions from
153b2d2b, expressed through the current shared versionOK/retentionOK/ValidRequest.
No historical local-retention policy is imported or claimed here. -/
set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.renewal
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.Renewal

def Deadline (value : RenewalConfirmation) : Prop :=
  match value.request.request.operation with
  | .Renew _ requested extendOnly =>
    0 ≤ requested.val ∧ 0 ≤ value.effective_end_at_ms.val ∧
      (if extendOnly then requested.val ≤ value.effective_end_at_ms.val
       else value.effective_end_at_ms = requested)
  | _ => False

def Shape (value : RenewalConfirmation) : Prop :=
  value.request.request.binding.key.window.val = 2 ∧
    2 ≤ value.request.accepted_version.val ∧ versionOK value.request ∧
    value.request.center.deployment = value.request.request.binding.node.deployment_id ∧
    retentionOK value.request.retention ∧
    ValidRequest value.request.request value.request.request.binding.node ∧ Deadline value

def Matches (value : RenewalConfirmation) (request : AcceptedLifecycleIntent) : Prop :=
  value.request = request ∧ Shape value

def ConfirmedMatches (ack : ConfirmedRenewal) (value : RenewalConfirmation) : Prop :=
  ack.result = value ∧ Shape value

def FullShapeContract (run : RenewalConfirmation → Result Bool) : Prop :=
  ∀ value, run value = .ok (by classical exact decide (Shape value))

def FullMatchesContract
    (run : RenewalConfirmation → AcceptedLifecycleIntent → Result Bool) : Prop :=
  ∀ value request, run value request = .ok (by classical exact decide (Matches value request))

def FullConfirmedContract
    (run : ConfirmedRenewal → RenewalConfirmation → Result Bool) : Prop :=
  ∀ ack value, run ack value = .ok (by classical exact decide (ConfirmedMatches ack value))

end SandboxModel.Stage1Protocol.Renewal
