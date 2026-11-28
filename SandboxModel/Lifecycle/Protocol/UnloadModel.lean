import SandboxModel.Lifecycle.Protocol.ReadyFactModel

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.unload
namespace SandboxModel.Stage1Protocol.UnloadBinding
open Validation ReadyFacts

-- Pure decoded values. No validation/authentication, material or exit truth is inferred.
def UnloadedShape (facts : UnloadedFacts) (source : IncarnationId) (capture : CaptureId) : Prop :=
  0 < facts.material_bytes.val ∧ facts.source_incarnation = source ∧ facts.capture = capture ∧
  facts.started_at_ms ≤ facts.startup_deadline_ms ∧ facts.started_at_ms < facts.end_at_ms ∧
  facts.end_at_ms ≤ facts.max_end_at_ms

def Matches (observation : UnloadObservation) (original : AcceptedLifecycleIntent) : Prop :=
  match original.request.operation with
  | .Unload source capture =>
    observation.request = original ∧ 0 < original.accepted_version.val ∧
    versionOK original ∧ retentionOK original.retention ∧
      match observation.outcome with
      | .Pending => True
      | .Unloaded facts => UnloadedShape facts source capture
      | .OriginalContinued facts => Timing facts
  | _ => False

def UnloadedProjection (outcome : UnloadOutcome) : Option UnloadedFacts :=
  match outcome with | .Unloaded facts => some facts | _ => none

def ContinuedProjection (outcome : UnloadOutcome) : Option sandproof_protocol.lifecycle.observation.CurrentReadyFacts :=
  match outcome with | .OriginalContinued facts => some facts | _ => none

def FullContract (run : UnloadObservation → AcceptedLifecycleIntent → Result Bool) : Prop :=
  ∀ observation original, run observation original = .ok (by classical exact decide (Matches observation original))

def FullUnloadedProjection (run : UnloadOutcome → Result (Option UnloadedFacts)) : Prop :=
  ∀ outcome, run outcome = .ok (UnloadedProjection outcome)

def FullContinuedProjection
    (run : UnloadOutcome → Result (Option sandproof_protocol.lifecycle.observation.CurrentReadyFacts)) : Prop :=
  ∀ outcome, run outcome = .ok (ContinuedProjection outcome)

end SandboxModel.Stage1Protocol.UnloadBinding
