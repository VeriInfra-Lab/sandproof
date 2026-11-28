import SandboxModel.Lifecycle.Protocol.UnloadModel
import SandboxModel.Lifecycle.Protocol.Accepted
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.unload
namespace SandboxModel.Stage1Protocol.UnloadBinding
open Validation ReadyFacts

theorem version_eq_bind (intent : AcceptedLifecycleIntent) (next : Bool → Result Bool) :
    (do let version ← AcceptedLifecycleIntent.accepted_version_for_request intent
        let valid ← core.option.Option.Insts.CoreCmpPartialEqOption.eq core.cmp.PartialEqU64
          version (some intent.accepted_version)
        next valid) = next (by classical exact decide (versionOK intent)) := by
  rw [← bind_assoc_eq, version_comparison]
  simp

theorem matches_refines (observation : UnloadObservation) (original : AcceptedLifecycleIntent) :
    unload_matches observation original = .ok (by classical exact decide (Matches observation original)) := by
  classical
  rcases original with ⟨⟨binding, operation⟩, version, center, retention⟩
  cases operation <;> cases outcome : observation.outcome <;>
    simp [unload_matches, UnloadObservation.matches_request, Matches, outcome, UnloadedShape, Timing,
      Replay.accepted_eq, retention_valid,
      IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq,
      CaptureId.Insts.CoreCmpPartialEqCaptureId.eq,
      PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, Replay.ok_and]
  all_goals rw [version_eq_bind]
  all_goals simp [Replay.ok_and]

theorem unloaded_refines (outcome : UnloadOutcome) :
    unloaded_facts outcome = .ok (UnloadedProjection outcome) := by
  cases outcome <;> simp [unloaded_facts, UnloadOutcome.unloaded, UnloadedProjection]

theorem continued_refines (outcome : UnloadOutcome) :
    continued_facts outcome = .ok (ContinuedProjection outcome) := by
  cases outcome <;> simp [continued_facts, UnloadOutcome.continued, ContinuedProjection]

theorem matches_complete : FullContract unload_matches := matches_refines

theorem unloaded_complete : FullUnloadedProjection unloaded_facts := unloaded_refines

theorem continued_complete : FullContinuedProjection continued_facts := continued_refines

theorem matches_accepts_iff (observation : UnloadObservation) (original : AcceptedLifecycleIntent) :
    unload_matches observation original = .ok true ↔ Matches observation original := by
  classical
  simp [matches_refines]

theorem matches_rejects_iff (observation : UnloadObservation) (original : AcceptedLifecycleIntent) :
    unload_matches observation original = .ok false ↔ ¬ Matches observation original := by
  classical
  simp [matches_refines]

theorem preserves_full_original (observation : UnloadObservation) (original : AcceptedLifecycleIntent)
    (accepted : unload_matches observation original = .ok true) : observation.request = original := by
  have good := (matches_accepts_iff observation original).mp accepted
  cases ho : original.request.operation <;> simp [Matches, ho] at good
  exact good.1

theorem unloaded_digest_not_checked (request original : AcceptedLifecycleIntent) (facts : UnloadedFacts)
    (digest : sandproof_protocol.lifecycle.digest.ContentDigest) :
    unload_matches ⟨request, .Unloaded { facts with manifest_digest := digest }⟩ original =
      unload_matches ⟨request, .Unloaded facts⟩ original := by
  rw [matches_refines, matches_refines]
  cases original.request.operation <;> simp [Matches, UnloadedShape]

end SandboxModel.Stage1Protocol.UnloadBinding

#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.matches_refines
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.unloaded_refines
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.continued_refines
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.matches_complete
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.unloaded_complete
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.continued_complete
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.matches_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.matches_rejects_iff
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.preserves_full_original
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.unloaded_digest_not_checked

#audit_controller_proof SandboxModel.Stage1Protocol.UnloadBinding.version_eq_bind
