import SandboxModel.Lifecycle.PauseReceipt.Observations

namespace SandboxModel.Lifecycle.PauseReceipt
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

theorem receipt_route (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome =
      (do
        let checked ← effect.receipt_awaiting_result world ⟨⟨command⟩,attempt⟩ outcome
        match checked with
        | .Ok awaiting =>
          let result ← effect.receipt_finish world ⟨⟨command⟩,attempt⟩ outcome awaiting
          .ok (some result)
        | .Err reject => .ok (some (.Err reject))) := by
  cases supported <;>
    simp [effect.receipt_pause,model.EffectId.command,model_EffectId_clone_exact]
  all_goals
    apply congrArg (fun continuation => Bind.bind _ continuation)
    funext checked
    cases checked <;> rfl

theorem finish_first_exact (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome) :
    effect.receipt_finish world ⟨⟨command⟩,attempt⟩ outcome true =
      (if outcomeMatches world command outcome then
        do let next ← effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ outcome
           .ok (.Ok ⟨.AcknowledgeEffect ⟨⟨command⟩,attempt⟩ outcome,next⟩)
       else .ok (.Err .InvalidOutcomeEvidence)) := by
  simp [effect.receipt_finish,model.EffectId.command,
    outcome_valid_exact world command supported outcome,model_EffectId_clone_exact]

theorem finish_replay_exact (world : model.World) (id : model.EffectId)
    (outcome : model.EffectOutcome) :
    effect.receipt_finish world id outcome false =
      .ok (.Ok ⟨.AcknowledgeEffect id outcome,world⟩) := by
  simp [effect.receipt_finish,model_World_clone_exact]

noncomputable section
local instance receiptAdmissionEffectId : DecidableEq model.EffectId := Classical.decEq _

theorem first_check (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩) :
    effect.receipt_awaiting_result world ⟨⟨command⟩,attempt⟩ outcome = .ok (.Ok true) := by
  simp only [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,bind_tc_ok]
  rcases pending with dispatched | unknown
  · simp [dispatched]
  · simp [unknown]

theorem first_route (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome =
      (do let result ← effect.receipt_finish world ⟨⟨command⟩,attempt⟩ outcome true
          .ok (some result)) := by
  rw [receipt_route world command supported attempt outcome,
    first_check world command supported attempt outcome pending]
  simp only [bind_tc_ok]

/-- The body is the real entry: result validation and write arguments are
conclusions, not preconditions supplied by a caller contract. -/
theorem first_acceptance_facts (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (decision : effect.EffectDecision)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩)
    (accepted : effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Ok decision))) :
    outcomeMatches world command outcome = true ∧
    decision.event = .AcknowledgeEffect ⟨⟨command⟩,attempt⟩ outcome ∧
    effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ outcome = .ok decision.next := by
  rw [first_route world command supported attempt outcome pending] at accepted
  obtain ⟨answer,finished,returned⟩ := bind_ok_inv _ _ _ accepted
  have answerSame : answer = .Ok decision := by simpa using returned
  rw [answerSame,finish_first_exact world command supported attempt outcome] at finished
  by_cases valid : outcomeMatches world command outcome = true
  · simp only [valid,if_true] at finished
    obtain ⟨next,written,wrapped⟩ := bind_ok_inv _ _ _ finished
    have decisionSame : decision = ⟨.AcknowledgeEffect ⟨⟨command⟩,attempt⟩ outcome,next⟩ := by
      simpa using wrapped.symm
    subst decision
    exact ⟨valid,rfl,written⟩
  · simp [valid] at finished

theorem same_property_accepts :
    RequiresOutcomeEvidence FirstPauseReceipt actualAccepts expectedOutcome := by
  intro world id outcome decision first accepted
  rcases id with ⟨⟨command⟩,attempt⟩
  rcases first with ⟨supported,pending⟩
  exact (first_acceptance_facts world command supported attempt outcome decision pending accepted).1

theorem first_success_observes_result (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (digest : ids.ResultDigest)
    (decision : effect.EffectDecision)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩)
    (accepted : effect.receipt_pause world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok (some (.Ok decision))) :
    ResultObserved world command := by
  exact (observed_true_iff world command supported).mp
    (first_acceptance_facts world command supported attempt (.Succeeded digest) decision pending accepted).1

theorem first_failure_observes_absence (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (failure : model.FailureKind)
    (decision : effect.EffectDecision)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩)
    (accepted : effect.receipt_pause world ⟨⟨command⟩,attempt⟩ (.Failed failure) = .ok (some (.Ok decision))) :
    ¬ ResultObserved world command := by
  have checked := (first_acceptance_facts world command supported attempt (.Failed failure) decision pending accepted).1
  intro observedResult
  have present := (observed_true_iff world command supported).mpr observedResult
  simp [outcomeMatches,present] at checked

theorem first_receipt_frame (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (decision : effect.EffectDecision)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩)
    (accepted : effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Ok decision))) :
    decision.next =
      { world with
        logical_effects := decision.next.logical_effects
        effects := decision.next.effects
        effect_keys := decision.next.effect_keys } := by
  exact SandboxModel.Lifecycle.PauseDispatch.acknowledge_frame world decision.next
    ⟨⟨command⟩,attempt⟩ outcome
    (first_acceptance_facts world command supported attempt outcome decision pending accepted).2.2

theorem identical_replay (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (recorded : tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val = some (.Acknowledged outcome)) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome =
      .ok (some (.Ok ⟨.AcknowledgeEffect ⟨⟨command⟩,attempt⟩ outcome,world⟩)) := by
  rw [receipt_route world command supported attempt outcome]
  simp [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,
    recorded,outcome_ne_exact,finish_replay_exact]

theorem conflicting_replay (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (prior outcome : model.EffectOutcome)
    (different : prior ≠ outcome)
    (recorded : tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val = some (.Acknowledged prior)) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Err .ConflictingReceipt)) := by
  rw [receipt_route world command supported attempt outcome]
  simp [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,
    recorded,outcome_ne_exact,different]

theorem missing_effect_rejects (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (missing : tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val = none) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Err .EffectNotFound)) := by
  rw [receipt_route world command supported attempt outcome]
  simp [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,missing]

theorem undispatched_committed_rejects (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (recorded : tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val = some .Committed) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Err .NotAwaitingResult)) := by
  rw [receipt_route world command supported attempt outcome]
  simp [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,recorded]

theorem retired_stop_rejects (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (coverage : model.StopCoverage)
    (recorded : tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val = some (.RetiredByStop coverage)) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Err .EffectRetired)) := by
  rw [receipt_route world command supported attempt outcome]
  simp [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,recorded]

theorem retired_capture_end_rejects (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (receipt : capture.continuation.CaptureWorkEndReceipt)
    (recorded : tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val = some (.RetiredByCaptureEnd receipt)) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Err .EffectRetired)) := by
  rw [receipt_route world command supported attempt outcome]
  simp [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,recorded]

theorem retired_capture_drain_rejects (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (coverage : capture.drain.CaptureDrainCoverage)
    (recorded : tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val = some (.RetiredByCaptureDrain coverage)) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Err .EffectRetired)) := by
  rw [receipt_route world command supported attempt outcome]
  simp [effect.receipt_awaiting_result,effect_get_exact world command supported attempt,recorded]

theorem first_stale_current_check_unchanged (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome)
    (decision : effect.EffectDecision) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      world.logical_effects ⟨command⟩ = .ok (some stored))
    (stale : stored.current_attempt ≠ attempt)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩)
    (accepted : effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome = .ok (some (.Ok decision))) :
    effect.effect_succeeded_check decision.next command = effect.effect_succeeded_check world command := by
  have capacity : TableWriteCapacity world.effects.entries.val ⟨⟨command⟩,attempt⟩ := by
    apply Or.inl
    rcases pending with dispatched | unknown
    · simp [dispatched]
    · simp [unknown]
  exact SandboxModel.Lifecycle.PauseDispatch.acknowledge_stale_current_check_unchanged
    world decision.next command (publication_command command supported) attempt outcome stored
    read stale capacity
    (first_acceptance_facts world command supported attempt outcome decision pending accepted).2.2

theorem first_failure_preserves_logical (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (failure : model.FailureKind)
    (decision : effect.EffectDecision)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩)
    (accepted : effect.receipt_pause world ⟨⟨command⟩,attempt⟩ (.Failed failure) = .ok (some (.Ok decision))) :
    decision.next.logical_effects = world.logical_effects := by
  exact SandboxModel.Lifecycle.PauseDispatch.acknowledge_failed_preserves_logical
    world decision.next ⟨⟨command⟩,attempt⟩ failure
    (first_acceptance_facts world command supported attempt (.Failed failure) decision pending accepted).2.2

theorem wrap_first_ack (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) (outcome : model.EffectOutcome) (next : model.World)
    (pending : FirstUnresolved world ⟨⟨command⟩,attempt⟩)
    (valid : outcomeMatches world command outcome = true)
    (ack : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ outcome = .ok next) :
    effect.receipt_pause world ⟨⟨command⟩,attempt⟩ outcome =
      .ok (some (.Ok ⟨.AcknowledgeEffect ⟨⟨command⟩,attempt⟩ outcome,next⟩)) := by
  rw [first_route world command supported attempt outcome pending,
    finish_first_exact world command supported attempt outcome]
  simp [valid,ack]

end
end SandboxModel.Lifecycle.PauseReceipt
