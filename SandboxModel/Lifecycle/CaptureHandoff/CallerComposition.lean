import SandboxModel.Lifecycle.CaptureHandoff.CallerRecordChecks
import SandboxModel.Lifecycle.CaptureHandoff.CallerCommitChecks

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- End-to-end for the selected real late-record Commit branch: successful
intake itself supplies terminal/Revoking, original lookup and revoked capture
facts. Only the command case and ordinary finite index room are inputs. This
is not the full worker receipt validation, physical material or V9 theorem. -/
theorem record_commit_success_accounts_without_revival
    (world : sandbox_controller_core.model.World) (id : sandbox_controller_core.model.EffectId)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (decision : sandbox_controller_core.worker.WorkerDecision)
    (command : id.logical.command = .CommitSnapshot sandbox epoch generation snapshot manifest)
    (indexCapacity : snapshot ∈ (caller_index_view world sandbox).snapshots.val ∨
      (caller_index_view world sandbox).snapshots.val.length < Usize.max)
    (indexRoom : TableWriteCapacity world.resource_indexes.entries.val sandbox)
    (evaluated : sandbox_controller_core.worker.late_capture.record world id = .ok (some (.Ok decision))) :
    ∃ row logical key plan current capture,
      tableLookup sandbox world.sandboxes.entries.val = some row ∧ row.terminal = true ∧ row.phase = .Revoking ∧
      sandbox_controller_core.model.World.logical_effect world id.logical = .ok (some logical) ∧
      logical.origin = .Operation key ∧ tableLookup key world.operation_plans.entries.val = some plan ∧
      plan.key = key ∧ key.sandbox = sandbox ∧ row.current_operation = some current ∧ key.operation ≠ current.id ∧
      plan.generation = row.generation ∧ epoch = plan.epoch ∧ plan.capture = some capture ∧
      decision.event = .CommitSnapshot snapshot ∧
      decision.next.operation_plans.entries.val = tablePut key { plan with capture := some (lateMaterial capture) }
        world.operation_plans.entries.val ∧
      (∃ old, tableLookup snapshot world.snapshots.entries.val = some old ∧
        decision.next.snapshots.entries.val = tablePut snapshot { old with stage := .Durable, immutable := true }
          world.snapshots.entries.val) ∧
      LateFrame world decision.next ∧ (lateMaterial capture).eligibility = .Revoked ∧
      SameResponsibilities capture (lateMaterial capture) ∧
      sandbox_controller_core.capture.CaptureHandoff.may_stop_original (lateMaterial capture) = .ok false ∧
      SnapshotIndexKept world decision.next sandbox snapshot := by
  obtain ⟨row, rowFound, terminal, phase, inner⟩ :=
    record_commit_success_outer world id sandbox epoch generation snapshot manifest decision command evaluated
  obtain ⟨logical, key, plan, current, capture, logicalRead, origin, planFound, planKey,
    session, currentOperation, differentOperation, planGeneration, planEpoch,
    hasCapture, cancelled, _working, committed⟩ :=
    record_inner_commit_success_identity world id row sandbox epoch generation snapshot manifest decision command inner
  have originalPlan : tableLookup plan.key world.operation_plans.entries.val = some plan := by
    rw [planKey]
    exact planFound
  obtain ⟨event, plans, snapshots, frame, revoked, responsibilities, noStop, index⟩ :=
    late_commit_success_accounts_without_revival world row plan sandbox epoch generation snapshot manifest
      capture decision originalPlan hasCapture cancelled indexCapacity indexRoom committed
  refine ⟨row, logical, key, plan, current, capture, rowFound, terminal, phase,
    logicalRead, origin, planFound, planKey, session, currentOperation, differentOperation,
    planGeneration, planEpoch, hasCapture, event, ?_, snapshots, frame, revoked, responsibilities, noStop, index⟩
  simpa [planKey] using plans

end SandboxModel.Lifecycle.CaptureHandoffCaller
