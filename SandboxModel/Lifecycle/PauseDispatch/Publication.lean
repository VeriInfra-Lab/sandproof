import SandboxModel.Lifecycle.PauseDispatch.Ready

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

noncomputable section
open SandboxModel.Lifecycle.PausePublication
local instance : DecidableEq model.LogicalEffectId := Classical.decEq _
local instance : DecidableEq model.EffectId := Classical.decEq _

/-- Both actual acknowledgements and actual publication are constructed; no future success is assumed. -/
theorem stop_commit_receipts_publish (world : model.World) (sandbox : ids.SandboxId) (source : model.RestoreSource)
    (rest : ReadyRest world sandbox source)
    (stopAttempt commitAttempt : U64) (stopDigest commitDigest : ids.ResultDigest)
    (stopRecord commitRecord : model.LogicalEffectRecord)
    (stopRead : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects
      ⟨.StopVm sandbox rest.record.epoch rest.record.generation rest.vm⟩ = .ok (some stopRecord))
    (commitRead : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects
      ⟨.CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest⟩ = .ok (some commitRecord))
    (stopCurrent : stopRecord.current_attempt = stopAttempt)
    (commitCurrent : commitRecord.current_attempt = commitAttempt)
    (stopPresent : tableLookup ⟨⟨.StopVm sandbox rest.record.epoch rest.record.generation rest.vm⟩,stopAttempt⟩ world.effects.entries.val ≠ none)
    (commitPresent : tableLookup ⟨⟨.CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest⟩,commitAttempt⟩ world.effects.entries.val ≠ none)
    (stopIndexed : (⟨⟨.StopVm sandbox rest.record.epoch rest.record.generation rest.vm⟩,stopAttempt⟩ : model.EffectId) ∈ world.effect_keys.val)
    (commitIndexed : (⟨⟨.CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest⟩,commitAttempt⟩ : model.EffectId) ∈ world.effect_keys.val) :
    ∃ middle next decision,
      effect.acknowledge_effect world ⟨⟨.StopVm sandbox rest.record.epoch rest.record.generation rest.vm⟩,stopAttempt⟩ (.Succeeded stopDigest) = .ok middle ∧
      effect.acknowledge_effect middle ⟨⟨.CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest⟩,commitAttempt⟩ (.Succeeded commitDigest) = .ok next ∧
      publication.handle_publish_paused next sandbox source = .ok (.Ok decision) := by
  classical
  let stopCommand := model.Command.StopVm sandbox rest.record.epoch rest.record.generation rest.vm
  let commitCommand := model.Command.CommitSnapshot sandbox rest.record.epoch rest.record.generation source.snapshot source.manifest
  obtain ⟨middle,next,stopAck,commitAck,secondRead,secondRoom,stopSuccess,commitSuccess,_sandboxFrame,_vmFrame⟩ :=
    two_current_receipts_accept world stopCommand commitCommand (.stop _ _ _ _) (.commit _ _ _ _ _)
      (by simp [stopCommand,commitCommand]) stopAttempt commitAttempt stopDigest commitDigest stopRecord commitRecord
      stopRead commitRead stopCurrent commitCurrent stopPresent commitPresent stopIndexed commitIndexed
  let middleRest := ready_rest_after_receipt world middle sandbox source rest stopCommand (.stop _ _ _ _)
    (Or.inr rfl) stopAttempt stopDigest stopRecord stopRead stopCurrent (Or.inl stopPresent) stopAck
  let nextRest := ready_rest_after_receipt middle next sandbox source middleRest commitCommand (.commit _ _ _ _ _)
    (Or.inl rfl) commitAttempt commitDigest commitRecord secondRead commitCurrent secondRoom commitAck
  have ready := ready_from_receipts next sandbox source nextRest commitSuccess stopSuccess
  obtain ⟨decision,published⟩ := normal_inputs_accept next sandbox source ready
  exact ⟨middle,next,decision,stopAck,commitAck,published⟩

end

end SandboxModel.Lifecycle.PauseDispatch
