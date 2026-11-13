import SandboxModel.Lifecycle.PauseReceipt.Admission
import SandboxModel.Lifecycle.PauseDispatch.Acceptance

/-! The finite witness below reuses the existing World, worker execution and Ack
proofs.  In particular, receipt acceptance is derived from the actual post-Stop
VM and durable snapshot observations, rather than assumed from the Ack result.
-/

open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

namespace SandboxModel.Lifecycle.PauseReceipt

namespace Acceptance
open SandboxModel.Lifecycle.PauseDispatch

noncomputable section
local instance receiptAcceptanceId : DecidableEq model.EffectId := Classical.decEq _

theorem stop_observed : observed beforeReceipts SandboxModel.Lifecycle.PausePublication.Fixture.stop = true := by
  classical
  simp [observed,stopObserved,beforeReceipts,SandboxModel.Lifecycle.PausePublication.Fixture.stop,SandboxModel.Lifecycle.PausePublication.Fixture.world,SandboxModel.Lifecycle.PausePublication.Fixture.entry,
    SandboxModel.Lifecycle.PausePublication.Fixture.one,SandboxModel.Lifecycle.PausePublication.Fixture.vm,tableLookup]

theorem commit_observed : observed beforeReceipts SandboxModel.Lifecycle.PausePublication.Fixture.commit = true := by
  classical
  simp [observed,commitObserved,beforeReceipts,SandboxModel.Lifecycle.PausePublication.Fixture.commit,SandboxModel.Lifecycle.PausePublication.Fixture.world,SandboxModel.Lifecycle.PausePublication.Fixture.entry,
    SandboxModel.Lifecycle.PausePublication.Fixture.one,SandboxModel.Lifecycle.PausePublication.Fixture.snapshot,SandboxModel.Lifecycle.PausePublication.Fixture.source,tableLookup]

theorem stop_first_unresolved : FirstUnresolved beforeReceipts (SandboxModel.Lifecycle.PausePublication.Fixture.id SandboxModel.Lifecycle.PausePublication.Fixture.stop) := by
  classical
  apply Or.inl
  simp [beforeReceipts,SandboxModel.Lifecycle.PausePublication.Fixture.id,SandboxModel.Lifecycle.PausePublication.Fixture.five,SandboxModel.Lifecycle.PausePublication.Fixture.stop,SandboxModel.Lifecycle.PausePublication.Fixture.commit,
    SandboxModel.Lifecycle.PausePublication.Fixture.fence,SandboxModel.Lifecycle.PausePublication.Fixture.freeze,SandboxModel.Lifecycle.PausePublication.Fixture.sealing,tableLookup]

theorem commit_lookup :
    tableLookup (SandboxModel.Lifecycle.PausePublication.Fixture.id SandboxModel.Lifecycle.PausePublication.Fixture.commit) beforeReceipts.effects.entries.val =
      some .OutcomeUnknown := by
  classical
  simp [beforeReceipts,SandboxModel.Lifecycle.PausePublication.Fixture.id,SandboxModel.Lifecycle.PausePublication.Fixture.five,SandboxModel.Lifecycle.PausePublication.Fixture.stop,SandboxModel.Lifecycle.PausePublication.Fixture.commit,
    SandboxModel.Lifecycle.PausePublication.Fixture.fence,SandboxModel.Lifecycle.PausePublication.Fixture.freeze,SandboxModel.Lifecycle.PausePublication.Fixture.sealing,tableLookup]

/-- The first receipt cannot remove the other unresolved receipt or its material. -/
theorem after_stop_ack_commit_ready (middle : model.World)
    (ack : effect.acknowledge_effect beforeReceipts (SandboxModel.Lifecycle.PausePublication.Fixture.id SandboxModel.Lifecycle.PausePublication.Fixture.stop)
      (.Succeeded 12#u64) = .ok middle) :
    FirstUnresolved middle (SandboxModel.Lifecycle.PausePublication.Fixture.id SandboxModel.Lifecycle.PausePublication.Fixture.commit) ∧
      observed middle SandboxModel.Lifecycle.PausePublication.Fixture.commit = true := by
  classical
  have frame := acknowledge_frame beforeReceipts middle (SandboxModel.Lifecycle.PausePublication.Fixture.id SandboxModel.Lifecycle.PausePublication.Fixture.stop)
    (.Succeeded 12#u64) ack
  have entries := acknowledge_effect_entries beforeReceipts middle SandboxModel.Lifecycle.PausePublication.Fixture.stop
    (.stop _ _ _ _) 0#u64 (.Succeeded 12#u64)
    (Or.inl (by
      simp [beforeReceipts,SandboxModel.Lifecycle.PausePublication.Fixture.id,SandboxModel.Lifecycle.PausePublication.Fixture.five,SandboxModel.Lifecycle.PausePublication.Fixture.stop,SandboxModel.Lifecycle.PausePublication.Fixture.commit,
        SandboxModel.Lifecycle.PausePublication.Fixture.fence,SandboxModel.Lifecycle.PausePublication.Fixture.freeze,SandboxModel.Lifecycle.PausePublication.Fixture.sealing,tableLookup])) ack
  constructor
  · apply Or.inr
    change tableLookup (SandboxModel.Lifecycle.PausePublication.Fixture.id SandboxModel.Lifecycle.PausePublication.Fixture.commit) middle.effects.entries.val = _
    rw [entries,table_lookup_put]
    simpa [SandboxModel.Lifecycle.PausePublication.Fixture.id,SandboxModel.Lifecycle.PausePublication.Fixture.commit,SandboxModel.Lifecycle.PausePublication.Fixture.stop] using commit_lookup
  · have snapshots : middle.snapshots = beforeReceipts.snapshots :=
      by
        have preserved := congrArg model.World.snapshots frame
        exact preserved
    simpa only [observed,SandboxModel.Lifecycle.PausePublication.Fixture.commit,commitObserved,snapshots] using commit_observed

/-- A nonempty normal chain using both actual receipt entry points.  This is a
finite interface-domain witness, not a V9 reachability or deployment claim. -/
theorem fresh_commit_dispatch_stop_receipts_publish :
    ∃ committed dispatched stopped middle next published,
      worker.pause_stop.record SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.before SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.commitId =
        .ok (some (.Ok committed)) ∧
      effect.dispatch_stop committed.next SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.stopId =
        .ok (some (.Ok dispatched)) ∧
      worker.pause_stop.record dispatched.next SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.stopId =
        .ok (some (.Ok stopped)) ∧
      effect.receipt_pause stopped.next SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.stopId (.Succeeded 12#u64) =
        .ok (some (.Ok ⟨.AcknowledgeEffect SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.stopId (.Succeeded 12#u64),middle⟩)) ∧
      effect.receipt_pause middle SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.commitId (.Succeeded 12#u64) =
        .ok (some (.Ok ⟨.AcknowledgeEffect SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.commitId (.Succeeded 12#u64),next⟩)) ∧
      publication.handle_publish_paused next 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.source = .ok (.Ok published) := by
  obtain ⟨middle,next,published,stopAck,commitAck,publish⟩ := normal_two_receipts_then_publish
  have stopReceipt := wrap_first_ack beforeReceipts SandboxModel.Lifecycle.PausePublication.Fixture.stop
    (.stop _ _ _ _) 0#u64 (.Succeeded 12#u64) middle stop_first_unresolved
    (by simpa [outcomeMatches] using stop_observed) stopAck
  obtain ⟨commitUnresolved,commitMatches⟩ := after_stop_ack_commit_ready middle stopAck
  have commitReceipt := wrap_first_ack middle SandboxModel.Lifecycle.PausePublication.Fixture.commit
    (.commit _ _ _ _ _) 0#u64 (.Succeeded 12#u64) next commitUnresolved
    (by simpa [outcomeMatches] using commitMatches) commitAck
  exact ⟨⟨.CommitSnapshot 5#u64,SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.queued⟩,
    ⟨.DispatchEffect SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world⟩,
    ⟨.StopVm 1#u64 2#u64,SandboxModel.Lifecycle.PauseDispatch.Acceptance.afterStop⟩,middle,next,published,
    SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.actual_commit_record_accepts,
    SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit.actual_registered_stop_dispatches,SandboxModel.Lifecycle.PauseDispatch.Acceptance.valid_record_accepts,
    stopReceipt,commitReceipt,publish⟩

end
end Acceptance
end SandboxModel.Lifecycle.PauseReceipt
