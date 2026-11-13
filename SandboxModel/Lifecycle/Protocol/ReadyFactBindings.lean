import SandboxModel.Lifecycle.Protocol.ReadyFactModel
import SandboxModel.Lifecycle.Protocol.Accepted

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.observation sandproof_protocol.lifecycle.unload
namespace SandboxModel.Stage1Protocol.ReadyFacts

theorem timing_refines (facts : CurrentReadyFacts) :
    sandproof_protocol.lifecycle.resume.current_facts facts =
      .ok (by classical exact decide (Timing facts)) := by
  classical
  simp [sandproof_protocol.lifecycle.resume.current_facts, Timing, Replay.ok_and]

theorem create_refines (facts : CurrentReadyFacts) (intent : AcceptedLifecycleIntent) :
    ready_matches_create facts intent = .ok (by classical exact decide (CreateMatches facts intent)) := by
  classical
  rcases intent with ⟨⟨binding, operation⟩, version, center, retention⟩
  cases operation <;>
    simp [ready_matches_create, CurrentReadyFacts.matches_create, CreateMatches,
      Library.option_u64_eq, lift, Replay.ok_and]

theorem source_refines (facts : CurrentReadyFacts) (original : AcceptedLifecycleIntent)
    (source : UnloadedFacts) :
    sandproof_protocol.lifecycle.resume.CurrentReadyFacts.matches_resume_source facts original source =
      .ok (by classical exact decide (ResumeSource facts original source)) := by
  classical
  rcases original with ⟨⟨binding, operation⟩, version, center, retention⟩
  cases operation <;>
    simp [sandproof_protocol.lifecycle.resume.CurrentReadyFacts.matches_resume_source,
      ResumeSource, IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq,
      CaptureId.Insts.CoreCmpPartialEqCaptureId.eq,
      sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
      sandproof_protocol.lifecycle.digest.ContentDigest.Insts.CoreCmpPartialEqContentDigest.eq,
      Replay.array_eq, Replay.ok_and]

theorem resumed_refines (facts : CurrentReadyFacts) (original : AcceptedLifecycleIntent)
    (source : UnloadedFacts) :
    ready_matches_resumed facts original source =
      .ok (by classical exact decide (ResumedMatches facts original source)) := by
  classical
  simp [ready_matches_resumed, sandproof_protocol.lifecycle.resume.CurrentReadyFacts.matches_resumed,
    source_refines, timing_refines, ResumedMatches, Replay.ok_and]

theorem cast_nonnegative_exact (deadline : I64) (nonnegative : 0 ≤ deadline.val) :
    ((IScalar.hcast .U64 deadline).val : Int) = deadline.val := by
  have spec := IScalar.hcast_inBounds_spec .U64 deadline (by scalar_tac)
  simpa [lift, WP.spec_ok] using spec

theorem cast_comparisons (deadline : I64) (endValue : U64) (nonnegative : 0 ≤ deadline.val) :
    (IScalar.hcast .U64 deadline ≤ endValue ↔ deadline.val ≤ (endValue.val : Int)) ∧
    (endValue = IScalar.hcast .U64 deadline ↔ (endValue.val : Int) = deadline.val) := by
  have exact_cast := cast_nonnegative_exact deadline nonnegative
  constructor
  · change (IScalar.hcast .U64 deadline).val ≤ endValue.val ↔ _
    omega
  · constructor
    · intro same
      simpa [same] using exact_cast
    · intro same
      apply UScalar.eq_of_val_eq
      omega

theorem renewed_refines (facts : CurrentReadyFacts) (original current : AcceptedLifecycleIntent)
    (source : UnloadedFacts) :
    ready_matches_resumed_renewal facts original current source =
      .ok (by classical exact decide (RenewedMatches facts original current source)) := by
  classical
  rcases original with ⟨⟨binding, operation⟩, version, center, retention⟩
  rcases current with ⟨⟨currentBinding, currentOperation⟩, currentVersion, currentCenter, currentRetention⟩
  cases operation <;> cases currentOperation <;>
    simp [ready_matches_resumed_renewal,
      sandproof_protocol.lifecycle.resume.CurrentReadyFacts.matches_resumed_renewal,
      RenewedMatches, source_refines, timing_refines,
      IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq,
      PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, lift]
  rename_i sourceInc capture target digest incarnation deadline extend
  by_cases nonnegative : 0 ≤ deadline.val
  · have bounded : deadline.val < 18446744073709551616 := by scalar_tac
    have remainder := Int.emod_eq_of_lt nonnegative bounded
    have same := (cast_comparisons deadline facts.end_at_ms nonnegative).2
    cases extend <;> simp [nonnegative, not_lt.mpr nonnegative, remainder, same, Replay.ok_and]
  · have negative : deadline.val < 0 := lt_of_not_ge nonnegative
    simp [nonnegative, negative]

theorem create_complete : FullCreateContract ready_matches_create := create_refines

theorem resumed_complete : FullResumedContract ready_matches_resumed := resumed_refines

theorem renewed_complete : FullRenewedContract ready_matches_resumed_renewal := renewed_refines

theorem create_accepts_iff (facts : CurrentReadyFacts) (intent : AcceptedLifecycleIntent) :
    ready_matches_create facts intent = .ok true ↔ CreateMatches facts intent := by
  classical
  simp [create_refines]

theorem resumed_accepts_iff (facts : CurrentReadyFacts) (original : AcceptedLifecycleIntent)
    (source : UnloadedFacts) :
    ready_matches_resumed facts original source = .ok true ↔ ResumedMatches facts original source := by
  classical
  simp [resumed_refines]

theorem renewed_accepts_iff (facts : CurrentReadyFacts) (original current : AcceptedLifecycleIntent)
    (source : UnloadedFacts) :
    ready_matches_resumed_renewal facts original current source = .ok true ↔
      RenewedMatches facts original current source := by
  classical
  simp [renewed_refines]

theorem resumed_material_bytes_irrelevant (facts : CurrentReadyFacts) (original : AcceptedLifecycleIntent)
    (source : UnloadedFacts) (bytes : U64) :
    ready_matches_resumed facts original { source with material_bytes := bytes } =
      ready_matches_resumed facts original source := by
  rw [resumed_refines, resumed_refines]
  cases original.request.operation <;> simp [ResumedMatches, ResumeSource]

theorem renewed_previous_end_irrelevant (facts : CurrentReadyFacts) (original current : AcceptedLifecycleIntent)
    (source : UnloadedFacts) (previous : U64) :
    ready_matches_resumed_renewal facts original current { source with end_at_ms := previous } =
      ready_matches_resumed_renewal facts original current source := by
  rw [renewed_refines, renewed_refines]
  cases original.request.operation <;> cases current.request.operation <;>
    simp [RenewedMatches, ResumeSource]

end SandboxModel.Stage1Protocol.ReadyFacts

#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.timing_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.create_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.source_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.resumed_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.cast_nonnegative_exact
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.cast_comparisons
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.renewed_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.create_complete
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.resumed_complete
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.renewed_complete
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.create_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.resumed_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.renewed_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.resumed_material_bytes_irrelevant
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFacts.renewed_previous_end_irrelevant
