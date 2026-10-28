import SandboxModel.Lifecycle.Protocol.ResumeModel

/-! Complete Create observation value contract against a9c. Actual guard refinement
remains pending the exact signed conversion representation; witnesses alone are not that proof.
Reuse the existing ReadyFacts and Continued contracts; do not infer current
intent, durable refusal, applied renewal or physical readiness from these values. -/
set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.observation
namespace SandboxModel.Stage1Protocol.CreateBinding
open Validation ReadyFacts

def RenewedFacts (facts : CurrentReadyFacts) (original current : AcceptedLifecycleIntent) : Prop :=
  match original.request.operation, current.request.operation with
  | .Create create target, .Renew incarnation deadline extend =>
    0 ≤ deadline.val ∧ incarnation = target ∧
    facts.startup_deadline_ms = create.startup_deadline_ms ∧
    U64.checked_add facts.started_at_ms create.max_lifetime_ms = some facts.max_end_at_ms ∧
    Timing facts ∧
    (if extend then deadline.val ≤ (facts.end_at_ms.val : Int)
     else (facts.end_at_ms.val : Int) = deadline.val)
  | _, _ => False

def Header (observation : CreateObservation) (original : AcceptedLifecycleIntent) : Prop :=
  observation.request = original ∧
    original.center.deployment = original.request.binding.node.deployment_id ∧
    (match original.request.operation with | .Create _ _ => True | _ => False) ∧
    observation.current_request.center = original.center ∧
    observation.current_request.retention = original.retention ∧
    observation.current_request.request.binding.node = original.request.binding.node ∧
    observation.current_request.request.binding.session = original.request.binding.session ∧
    observation.current_request.request.binding.key.owner = original.request.binding.key.owner ∧
    observation.current_request.request.binding.key.deployment = original.request.binding.key.deployment ∧
    ValidRequest observation.current_request.request original.request.binding.node ∧
    original.accepted_version ≤ observation.current_request.accepted_version ∧
    original.accepted_version ≠ 0#u64

def Refused (observation : CreateObservation) (original : AcceptedLifecycleIntent) : Prop :=
  observation.historical_create_succeeded = false ∧ original.accepted_version = 1#u64 ∧
    versionOK original ∧ original.request.binding.expected = .Absent ∧
    ValidRequest original.request original.request.binding.node ∧
    (observation.current_request = original ∨
      ((match observation.current_request.request.operation with | .Destroy => True | _ => False) ∧
       ResumeBinding.Newer original observation.current_request ∧ versionOK observation.current_request))

def SameReadiness (history : Bool) (readiness : CurrentCreateReadiness)
    (original : AcceptedLifecycleIntent) : Prop :=
  match readiness with
  | .NotCurrent | .Refused _ => False
  | .Observed facts => history = true ∧ CreateMatches facts original
  | .NotRunning | .Expired | .Pending => True

def ContinuedReadiness (history : Bool) (readiness : CurrentCreateReadiness) : Prop :=
  match readiness with
  | .Observed facts => history = true ∧ Timing facts
  | .NotRunning | .Expired | .Pending => True
  | _ => False

def RenewReadiness (observation : CreateObservation) (original : AcceptedLifecycleIntent) : Prop :=
  match observation.readiness with
  | .Observed facts => observation.historical_create_succeeded = true ∧
      RenewedFacts facts original observation.current_request
  | .NotRunning | .Expired | .Pending => True
  | .NotCurrent | .Refused _ => False

def Body (observation : CreateObservation) (original : AcceptedLifecycleIntent) : Prop := by
  classical
  exact match observation.readiness with
  | .Refused _ => Refused observation original
  | readiness =>
    if observation.current_request = original then
      SameReadiness observation.historical_create_succeeded readiness original
    else match observation.current_request.request.operation with
    | .Unload _ _ =>
      if readiness ≠ .NotCurrent then
        ResumeBinding.Continued original observation.current_request ∧
          ContinuedReadiness observation.historical_create_succeeded readiness
      else original.accepted_version < observation.current_request.accepted_version ∧ readiness = .NotCurrent
    | .Renew _ _ _ =>
      original.accepted_version < observation.current_request.accepted_version ∧
        versionOK observation.current_request ∧ RenewReadiness observation original
    | _ => original.accepted_version < observation.current_request.accepted_version ∧ readiness = .NotCurrent

def Matches (observation : CreateObservation) (original : AcceptedLifecycleIntent) : Prop :=
  Header observation original ∧ Body observation original

def FullContract (run : CreateObservation → AcceptedLifecycleIntent → Result Bool) : Prop :=
  ∀ observation original,
    run observation original = .ok (by classical exact decide (Matches observation original))

end SandboxModel.Stage1Protocol.CreateBinding
