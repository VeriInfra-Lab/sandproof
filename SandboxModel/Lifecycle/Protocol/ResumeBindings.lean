import SandboxModel.Lifecycle.Protocol.ResumeModel
import SandboxModel.Lifecycle.Protocol.ReadyFactBindings

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.observation sandproof_protocol.lifecycle.resume
namespace SandboxModel.Stage1Protocol.ResumeBinding
open Validation ReadyFacts

theorem readiness_not_current (readiness : CurrentResumeReadiness) :
    CurrentResumeReadiness.Insts.CoreCmpPartialEqCurrentResumeReadiness.eq readiness .NotCurrent =
      .ok (by classical exact decide (readiness = .NotCurrent)) := by
  classical
  cases readiness <;> simp [CurrentResumeReadiness.Insts.CoreCmpPartialEqCurrentResumeReadiness.eq,
    CurrentResumeReadiness.read_discriminant]

theorem version_ne (intent : AcceptedLifecycleIntent) :
    (do let version ← AcceptedLifecycleIntent.accepted_version_for_request intent
        core.cmp.PartialEq.ne.trait_default
          (core.option.Option.Insts.CoreCmpPartialEqOption core.cmp.PartialEqU64)
          version (some intent.accepted_version)) =
      .ok (by classical exact decide (¬ versionOK intent)) := by
  classical
  cases he : intent.request.binding.expected <;>
    simp [AcceptedLifecycleIntent.accepted_version_for_request, he, versionOK,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, Library.option_u64_eq]
  all_goals scalar_tac

theorem validation_branch (request : LifecycleRequest) (node : NodeIdentity) (success : Result Bool) :
    (match expectedResult request node with | .Ok _ => success | .Err _ => .ok false) =
      (by classical exact if ValidRequest request node then success else .ok false) := by
  classical
  cases h : expectedResult request node with
  | Ok value =>
    cases value
    have valid := (expected_success_iff request node).mp h
    simp [valid]
  | Err error =>
    have invalid : ¬ ValidRequest request node := by
      intro valid
      have accepted := (expected_success_iff request node).mpr valid
      rw [h] at accepted
      cases accepted
    simp [invalid]

theorem continued_refines (original current : AcceptedLifecycleIntent) :
    sandproof_protocol.lifecycle.observation.continued_request original current =
      .ok (by classical exact decide (Continued original current)) := by
  classical
  rcases original with ⟨⟨binding, operation⟩, version, center, retention⟩
  rcases current with ⟨⟨currentBinding, currentOperation⟩, currentVersion, currentCenter, currentRetention⟩
  cases operation <;> cases currentOperation <;>
    simp [sandproof_protocol.lifecycle.observation.continued_request, Continued, Newer,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq,
      PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, Replay.key_eq,
      version_comparison, Replay.ok_and, and_assoc]
  all_goals
    have one : (1#u64 = currentVersion) ↔ currentVersion.val = 1 := by scalar_tac
    have oneRight : (currentVersion = 1#u64) ↔ currentVersion.val = 1 := by scalar_tac
    cases hc : currentBinding.expected <;>
      simp [AcceptedLifecycleIntent.accepted_version_for_request, hc, versionOK, Library.option_u64_eq, ok_unless, Replay.ok_and, one, oneRight]


theorem version_ne_bind (intent : AcceptedLifecycleIntent) (next : Bool → Result Bool) :
    (do let version ← AcceptedLifecycleIntent.accepted_version_for_request intent
        let bad ← core.cmp.PartialEq.ne.trait_default
          (core.option.Option.Insts.CoreCmpPartialEqOption core.cmp.PartialEqU64)
          version (some intent.accepted_version)
        next bad) = next (by classical exact decide (¬ versionOK intent)) := by
  rw [← bind_assoc_eq, version_ne]
  simp

theorem matches_refines (observation : ResumeObservation) (original : AcceptedLifecycleIntent) :
    resume_matches_request observation original =
      .ok (by classical exact decide (Matches observation original)) := by
  classical
  rcases observation with ⟨request, current, readiness⟩
  cases ho : original.request.operation <;>
    simp only [resume_matches_request, ResumeObservation.matches_request, ho, version_ne_bind]
  all_goals
    simp [Matches, ho, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      Replay.accepted_eq, Replay.center_eq, Replay.retention_eq, Replay.node_eq,
      sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
      sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
      SessionId.Insts.CoreCmpPartialEqSessionId.eq,
      sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      validate_body_refines, retention_valid, OriginalShape, CurrentShape]

  cases hro : expectedResult original.request original.request.binding.node <;>
    cases hrc : expectedResult current.request original.request.binding.node <;>
    simp [hro, hrc, ← expected_success_iff]
  by_cases same : current = original
  · subst current
    cases readiness <;>
      simp [ReadinessMatches, SameReadiness, ReadyFacts.timing_refines, ok_unless, Replay.ok_and]
  · cases hc : current.request.operation <;> cases readiness <;>
      simp [hc, same, ReadinessMatches, UnloadReadiness, RenewReadiness,
        readiness_not_current, continued_refines, ReadyFacts.timing_refines, Newer,
        Replay.key_eq, IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq,
        PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, lift, ok_unless, Replay.ok_and]
    all_goals try rw [← hc]
    rename_i incarnation deadline extend facts
    by_cases nonnegative : 0 ≤ deadline.val
    · have bounded : deadline.val < 18446744073709551616 := by scalar_tac
      have remainder := Int.emod_eq_of_lt nonnegative bounded
      have cast_same := (ReadyFacts.cast_comparisons deadline facts.end_at_ms nonnegative).2
      cases extend <;>
        simp [nonnegative, not_lt.mpr nonnegative, remainder, cast_same, ok_unless, Replay.ok_and]
    · have negative : deadline.val < 0 := lt_of_not_ge nonnegative
      simp [nonnegative, negative]


theorem matches_complete : FullContract resume_matches_request := matches_refines

theorem matches_accepts_iff (observation : ResumeObservation) (original : AcceptedLifecycleIntent) :
    resume_matches_request observation original = .ok true ↔ Matches observation original := by
  classical
  simp [matches_refines]

theorem matches_rejects_iff (observation : ResumeObservation) (original : AcceptedLifecycleIntent) :
    resume_matches_request observation original = .ok false ↔ ¬ Matches observation original := by
  classical
  simp [matches_refines]

theorem preserves_full_original (observation : ResumeObservation) (original : AcceptedLifecycleIntent)
    (accepted : resume_matches_request observation original = .ok true) : observation.request = original :=
  ((matches_accepts_iff observation original).mp accepted).1

theorem preserves_envelope_shapes (observation : ResumeObservation) (original : AcceptedLifecycleIntent)
    (accepted : resume_matches_request observation original = .ok true) :
    OriginalShape original ∧ CurrentShape original observation.current_request := by
  have good := (matches_accepts_iff observation original).mp accepted
  cases ho : original.request.operation <;> simp [Matches, ho] at good
  exact ⟨good.2.1, good.2.2.1⟩

theorem changed_original_rejected (observation : ResumeObservation) (original : AcceptedLifecycleIntent)
    (changed : observation.request ≠ original) : resume_matches_request observation original = .ok false := by
  classical
  simp [matches_refines, Matches, changed]

theorem continued_state_lost_accepts_iff (original current : AcceptedLifecycleIntent)
    (prior source target : IncarnationId) (saved capture : CaptureId)
    (digest : sandproof_protocol.lifecycle.digest.ContentDigest)
    (originalResume : original.request.operation = .Resume prior saved target digest)
    (currentUnload : current.request.operation = .Unload source capture) :
    resume_matches_request ⟨original, current, .StateLost⟩ original = .ok true ↔
      OriginalShape original ∧ CurrentShape original current ∧
      target = source ∧ Newer original current ∧ versionOK current ∧ versionOK original := by
  have changed : current ≠ original := by
    intro same
    subst current
    rw [originalResume] at currentUnload
    cases currentUnload
  simp [matches_accepts_iff, Matches, originalResume, ReadinessMatches, changed,
    currentUnload, UnloadReadiness, Continued]

end SandboxModel.Stage1Protocol.ResumeBinding

#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.readiness_not_current
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.version_ne
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.validation_branch
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.continued_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.version_ne_bind
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.matches_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.matches_complete
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.matches_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.matches_rejects_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.preserves_full_original
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.preserves_envelope_shapes
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.changed_original_rejected

#audit_controller_proof SandboxModel.Stage1Protocol.ResumeBinding.continued_state_lost_accepts_iff
