import SandboxModel.Lifecycle.Protocol.ValidationModel

/-! Independent confirmation and history-deletion policy on represented values.
No generated function is imported. This policy supplies no durable or physical evidence. -/

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.identity
open sandproof_protocol.lifecycle.request sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.renewal
namespace SandboxModel.Stage1Protocol.Retention

def Successor (intent : AcceptedLifecycleIntent) : Prop :=
  match intent.request.binding.expected with
  | .Absent => intent.accepted_version.val = 1
  | .Version previous => previous.val + 1 = intent.accepted_version.val

def DeadlineOK (operation : LifecycleOperation) (effective : I64) : Prop :=
  match operation with
  | .Renew _ requested extendOnly =>
    0 ≤ requested.val ∧ 0 ≤ effective.val ∧
      (if extendOnly then requested.val ≤ effective.val else effective = requested)
  | _ => False

def ValidConfirmation (confirmation : RenewalConfirmation) : Prop :=
  let intent := confirmation.request
  intent.request.binding.key.window.val = 2 ∧
    2 ≤ intent.accepted_version.val ∧ Successor intent ∧
    intent.center.deployment = intent.request.binding.node.deployment_id ∧
    0 < intent.retention.slot.val ∧ intent.retention.slot.val ≤ 4096 ∧
    Validation.ValidRequest intent.request intent.request.binding.node ∧
    DeadlineOK intent.request.operation confirmation.effective_end_at_ms

def IsRenew (operation : LifecycleOperation) : Prop :=
  match operation with
  | .Renew _ _ _ => True
  | _ => False

/-- The complete old checkpoint is required; non-current alone does not imply older. -/
def MayDiscard (intent : AcceptedLifecycleIntent) (applied previous : Option I64)
    (pending : Option sandbox_controller_core.ids.OperationId) (confirmation : RenewalConfirmation)
    (current : U64) (pinned : Bool) : Prop :=
  pinned = false ∧ intent.request.binding.key.window.val = 2 ∧
    IsRenew intent.request.operation ∧ confirmation.request = intent ∧
    ValidConfirmation confirmation ∧ applied = some confirmation.effective_end_at_ms ∧
    previous.isSome = true ∧ pending = none ∧ intent.accepted_version ≠ current

def FullRetentionContract
    (run : AcceptedLifecycleIntent → Option I64 → Option I64 →
      Option sandbox_controller_core.ids.OperationId → RenewalConfirmation → U64 → Bool → Result Bool) : Prop :=
  ∀ intent applied previous pending confirmation current pinned,
    run intent applied previous pending confirmation current pinned =
      .ok (by classical exact decide (MayDiscard intent applied previous pending confirmation current pinned))

end SandboxModel.Stage1Protocol.Retention
