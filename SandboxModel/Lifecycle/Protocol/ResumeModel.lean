import SandboxModel.Lifecycle.Protocol.ReadyFactModel

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.observation sandproof_protocol.lifecycle.resume
namespace SandboxModel.Stage1Protocol.ResumeBinding
open Validation ReadyFacts

-- A value contract. Readiness truth and authority still need their providers.
def Newer (original current : AcceptedLifecycleIntent) : Prop :=
  original.accepted_version < current.accepted_version ∧
  current.request.binding.key ≠ original.request.binding.key

def Continued (original current : AcceptedLifecycleIntent) : Prop :=
  match original.request.operation, current.request.operation with
  | .Create _ target, .Unload source _ | .Resume _ _ target _, .Unload source _ =>
    target = source ∧ Newer original current ∧ versionOK current ∧ versionOK original
  | _, _ => False

def OriginalShape (original : AcceptedLifecycleIntent) : Prop :=
  ValidRequest original.request original.request.binding.node ∧
  original.center.deployment = original.request.binding.node.deployment_id ∧
  original.accepted_version ≠ 0#u64 ∧ versionOK original ∧ retentionOK original.retention

def CurrentShape (original current : AcceptedLifecycleIntent) : Prop :=
  current.center = original.center ∧ current.retention = original.retention ∧
  current.request.binding.node = original.request.binding.node ∧
  current.request.binding.session = original.request.binding.session ∧
  current.request.binding.key.owner = original.request.binding.key.owner ∧
  current.request.binding.key.deployment = original.request.binding.key.deployment ∧
  ValidRequest current.request original.request.binding.node ∧ versionOK current

def SameReadiness (readiness : CurrentResumeReadiness) : Prop :=
  match readiness with
  | .NotCurrent => False
  | .Observed facts => Timing facts
  | .NotRunning | .Expired | .Pending | .StateLost => True

def UnloadReadiness (readiness : CurrentResumeReadiness) : Prop :=
  match readiness with
  | .Observed facts => Timing facts
  | .NotRunning | .Expired | .Pending | .StateLost => True
  | .NotCurrent => False

def RenewReadiness (readiness : CurrentResumeReadiness) (deadline : I64) (extend : Bool) : Prop :=
  match readiness with
  | .NotCurrent => False
  | .Observed facts => 0 ≤ deadline.val ∧ Timing facts ∧
    (if extend then deadline.val ≤ (facts.end_at_ms.val : Int) else (facts.end_at_ms.val : Int) = deadline.val)
  | .NotRunning | .Expired | .Pending | .StateLost => True

def ReadinessMatches (original current : AcceptedLifecycleIntent)
    (target : IncarnationId) (readiness : CurrentResumeReadiness) : Prop := by
  classical
  exact if current = original then SameReadiness readiness
    else match current.request.operation with
    | .Unload _ _ => if readiness ≠ .NotCurrent
        then Continued original current ∧ UnloadReadiness readiness
        else Newer original current ∧ readiness = .NotCurrent
    | .Renew incarnation deadline extend =>
        Newer original current ∧ incarnation = target ∧ RenewReadiness readiness deadline extend
    | _ => Newer original current ∧ readiness = .NotCurrent

def Matches (observation : ResumeObservation) (original : AcceptedLifecycleIntent) : Prop :=
  observation.request = original ∧
    match original.request.operation with
    | .Resume _ _ target _ => OriginalShape original ∧ CurrentShape original observation.current_request ∧
        ReadinessMatches original observation.current_request target observation.readiness
    | _ => False

def FullContract (run : ResumeObservation → AcceptedLifecycleIntent → Result Bool) : Prop :=
  ∀ observation original, run observation original = .ok (by classical exact decide (Matches observation original))

end SandboxModel.Stage1Protocol.ResumeBinding
