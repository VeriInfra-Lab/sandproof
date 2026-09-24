import SandboxModel.Controller

/-!
Backward coverage for the executable reducer.

Soundness is carried by every `Controller.Decision`.  The theorems here prove
the converse for the controller-owned transition families: whenever the
relational specification admits one of these transitions, the corresponding
concrete input is accepted and returns the same event and successor world.
-/

namespace SandboxModel.Controller

theorem handleCreate_complete
    (transition : Step world
      (.acceptCreate actor owner sid workspace filePolicy policy operationId)
      next) :
    ∃ decision,
      handle world (.create actor owner sid workspace filePolicy policy
        operationId) = .ok decision ∧
      decision.event =
        .acceptCreate actor owner sid workspace filePolicy policy operationId ∧
      decision.next = next := by
  cases transition
  case acceptCreate =>
    simp only [handle]
    unfold handleCreate
    split <;> simp_all
    split <;> simp_all
    split <;> simp_all
    split <;> simp_all
    split <;> simp_all

theorem handlePause_complete
    (transition : Step world (.acceptPause actor sid operationId) next) :
    ∃ decision,
      handle world (.pause actor sid operationId) = .ok decision ∧
      decision.event = .acceptPause actor sid operationId ∧
      decision.next = next := by
  cases transition
  case acceptPause =>
    simp only [handle]
    unfold handlePause
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all
    all_goals subst_vars
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all

theorem handleResume_complete
    (transition : Step world (.acceptResume actor sid operationId) next) :
    ∃ decision,
      handle world (.resume actor sid operationId) = .ok decision ∧
      decision.event = .acceptResume actor sid operationId ∧
      decision.next = next := by
  cases transition
  case acceptResume =>
    simp only [handle]
    unfold handleResume
    all_goals (try split) <;> simp_all [usableSnapshotCheck_spec]
    all_goals (try split) <;> simp_all [usableSnapshotCheck_spec]
    all_goals subst_vars
    all_goals (try split) <;> simp_all [usableSnapshotCheck_spec]
    all_goals (try split) <;> simp_all [usableSnapshotCheck_spec]
    all_goals (try split) <;> simp_all [usableSnapshotCheck_spec]

theorem handleKill_complete
    (transition : Step world (.acceptKill actor sid operationId) next) :
    ∃ decision,
      handle world (.kill actor sid operationId) = .ok decision ∧
      decision.event = .acceptKill actor sid operationId ∧
      decision.next = next := by
  cases transition
  case acceptKill =>
    simp only [handle]
    unfold handleKill
    all_goals (try split) <;> simp_all [supersedeReadyCheck_spec]
    all_goals (try split) <;> simp_all [supersedeReadyCheck_spec]
    all_goals (try split) <;> simp_all [supersedeReadyCheck_spec]
    all_goals (try split) <;> simp_all [supersedeReadyCheck_spec]
    all_goals (try split) <;> simp_all [supersedeReadyCheck_spec]
    all_goals (try split) <;> simp_all [supersedeReadyCheck_spec]
    all_goals (try split) <;> simp_all [supersedeReadyCheck_spec]

theorem handleReplay_complete
    (transition : Step world
      (.retryOperation actor sid operationId kind input) next) :
    ∃ decision,
      handle world (.replay actor sid operationId kind input) = .ok decision ∧
      decision.event = .retryOperation actor sid operationId kind input ∧
      decision.next = next := by
  cases transition
  case retryOperation =>
    simp only [handle]
    unfold handleReplay
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all

theorem handleQuery_complete
    (transition : Step world
      (.queryOperation actor sid operationId status result) next) :
    ∃ decision,
      handle world (.query actor sid operationId) = .ok decision ∧
      decision.event = .queryOperation actor sid operationId status result ∧
      decision.next = next := by
  cases transition
  case queryOperation =>
    simp only [handle]
    unfold handleQuery
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all

theorem compensationEvidence_has_supplied
    (world : World) (key : OperationKey) (epoch : Epoch)
    (failure : FailureKind)
    (evidence : CompensationEvidence world key epoch failure) :
    ∃ supplied,
      SuppliedCompensationEvidence world key epoch failure supplied := by
  rcases evidence with cancelled | deadline | witnessed
  · exact ⟨none, Or.inl cancelled⟩
  · exact ⟨none, Or.inr deadline⟩
  · rcases witnessed with ⟨id, finalized⟩
    exact ⟨some id, finalized⟩

theorem handleCompensation_complete
    (world : World) (sid : SandboxId) (failure : FailureKind) (next : World)
    (transition : Step world (.beginCompensation sid failure) next) :
    ∃ supplied decision,
      handle world (.compensate sid failure supplied) = .ok decision ∧
      decision.event = .beginCompensation sid failure ∧
      decision.next = next := by
  cases transition
  case beginCompensation failure up found compensablePhase live
      currentOperation recoverableKind unfinished persistedOperation
      operationMatches evidence =>
    rcases compensationEvidence_has_supplied _ _ _ _ evidence with
      ⟨supplied, justified⟩
    refine ⟨supplied, ?_⟩
    simp only [handle]
    unfold handleCompensation
    all_goals (try split) <;>
      try simp_all [compensationEvidenceCheck_spec]
    all_goals subst_vars
    all_goals (try split) <;>
      try simp_all [compensationEvidenceCheck_spec]
    all_goals subst_vars
    all_goals (try split) <;>
      try simp_all [compensationEvidenceCheck_spec]
    all_goals subst_vars
    all_goals (try split) <;>
      try simp_all [compensationEvidenceCheck_spec]
    all_goals subst_vars
    all_goals (try split) <;>
      try simp_all [compensationEvidenceCheck_spec]
    all_goals subst_vars
    all_goals (try split) <;>
      try simp_all [compensationEvidenceCheck_spec]
    all_goals simp_all [compensationEvidenceCheck_spec]
    all_goals subst_vars
    all_goals (try split) <;>
      simp_all [compensationEvidenceCheck_spec]
    all_goals (try split) <;>
      simp_all [compensationEvidenceCheck_spec]

theorem handleEffectDispatch_complete
    (transition : Step world (.dispatchEffect effect) next) :
    ∃ decision,
      handleEffect world (.dispatch effect) = .ok decision ∧
      decision.event = .dispatchEffect effect ∧ decision.next = next := by
  cases transition
  case dispatchEffect =>
    simp only [handleEffect]
    unfold handleEffectDispatch
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all

theorem handleEffectReceipt_complete
    (transition : Step world (.acknowledgeEffect effect outcome) next) :
    ∃ decision,
      handleEffect world (.receipt effect outcome) = .ok decision ∧
      decision.event = .acknowledgeEffect effect outcome ∧
      decision.next = next := by
  cases transition
  case acknowledgeEffect =>
    simp only [handleEffect]
    unfold handleEffectReceipt
    all_goals (try split) <;> simp_all [effectOutcomeValidCheck_spec]
    all_goals (try split) <;> simp_all [effectOutcomeValidCheck_spec]
  case replayEffectReceipt =>
    simp only [handleEffect]
    unfold handleEffectReceipt
    all_goals (try split) <;> simp_all
    all_goals (try split) <;> simp_all

theorem handleEffectTimeout_complete
    (transition : Step world (.effectTimeout effect) next) :
    ∃ decision,
      handleEffect world (.timeout effect) = .ok decision ∧
      decision.event = .effectTimeout effect ∧ decision.next = next := by
  cases transition
  case effectTimeout =>
    simp only [handleEffect]
    unfold handleEffectTimeout
    all_goals (try split) <;> simp_all
  case repeatEffectTimeout =>
    simp only [handleEffect]
    unfold handleEffectTimeout
    all_goals (try split) <;> simp_all

theorem handleEffectRetry_complete
    (transition : Step world (.startNextAttempt previous nextAttempt) next) :
    ∃ logical failure decision,
      handleEffect world (.retry logical failure) = .ok decision ∧
      decision.event = .startNextAttempt previous nextAttempt ∧
      decision.next = next := by
  cases transition
  case startNextAttempt failure unfinished retryable budget up found
      previousIdentity failed nextIdentity fresh =>
    cases previous with
    | mk previousLogical previousAttempt =>
        refine ⟨previousLogical, failure, ?_⟩
        injection previousIdentity with sameLogical sameAttempt
        subst previousLogical
        subst previousAttempt
        simp only [handleEffect]
        unfold handleEffectRetry
        all_goals (try split) <;> simp_all
        all_goals (try split) <;> simp_all
        all_goals subst_vars
        all_goals (try split) <;> simp_all
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals simp_all

theorem handleEffectFinalize_complete
    (world : World) (current : EffectId) (failure : FailureKind) (next : World)
    (transition : Step world (.finalizeEffectFailure current failure) next) :
    ∃ logical decision,
      handleEffect world (.finalizeFailure logical failure) = .ok decision ∧
      decision.event = .finalizeEffectFailure current failure ∧
      decision.next = next := by
  cases transition
  case finalizeEffectFailure failure up found unfinished currentIdentity failed
      terminal =>
    cases current with
    | mk currentLogical currentAttempt =>
        refine ⟨currentLogical, ?_⟩
        injection currentIdentity with sameLogical sameAttempt
        subst currentLogical
        subst currentAttempt
        simp only [handleEffect]
        unfold handleEffectFinalizeFailure
        all_goals (try split) <;> simp_all
        all_goals (try split) <;> simp_all
        all_goals subst_vars
        all_goals (try split) <;> simp_all
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals subst_vars
        all_goals (try split) <;> try simp_all
        all_goals simp_all

end SandboxModel.Controller
