import SandboxModel.Lifecycle.PauseDispatch.Safety
import SandboxModel.Lifecycle.PauseStop.Composition

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

theorem dispatch_preserves_original_generation (before after : model.World)
    (sandbox epoch generation vm attempt : U64) (decision : effect.EffectDecision)
    (registered : SandboxModel.Lifecycle.PauseStop.OriginalStopGeneration before after sandbox)
    (dispatched : effect.dispatch_stop after (stopId sandbox epoch generation vm attempt) = .ok (some (.Ok decision))) :
    SandboxModel.Lifecycle.PauseStop.OriginalStopGeneration before decision.next sandbox := by
  obtain ⟨record,originalVm,found,current,phase,preserves⟩ := registered
  have frame := (selected_stop_bound after sandbox epoch generation vm attempt decision dispatched).2.1
  have sandboxFrame : decision.next.sandboxes = after.sandboxes := by
    have projected := congrArg model.World.sandboxes frame
    exact projected
  have vmFrame : decision.next.vms = after.vms := by
    have projected := congrArg model.World.vms frame
    exact projected
  refine ⟨record,originalVm,found,current,phase,?_⟩
  intro later laterAttempt stopped sameSandboxes sameVms accepted
  exact preserves later laterAttempt stopped (sameSandboxes.trans sandboxFrame) (sameVms.trans vmFrame) accepted

theorem commit_dispatch_original_generation (world : model.World)
    (sandbox epoch generation commitAttempt stopAttempt : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (record : model.SandboxRecord) (vm : ids.VmId)
    (committed : worker.WorkerDecision) (dispatched : effect.EffectDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (commit : worker.pause_stop.record world ⟨⟨.CommitSnapshot sandbox epoch generation snapshot manifest⟩,commitAttempt⟩ =
      .ok (some (.Ok committed)))
    (dispatch : effect.dispatch_stop committed.next (stopId sandbox epoch generation vm stopAttempt) =
      .ok (some (.Ok dispatched))) :
    SandboxModel.Lifecycle.PauseStop.OriginalStopGeneration world dispatched.next sandbox := by
  apply dispatch_preserves_original_generation world committed.next sandbox epoch generation vm stopAttempt dispatched
  · exact SandboxModel.Lifecycle.PauseStop.commit_record_original_generation world sandbox epoch generation commitAttempt snapshot manifest record committed stored phase commit
  · exact dispatch

end SandboxModel.Lifecycle.PauseDispatch
