import SandboxModel.Lifecycle.PausePublication.Checks

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

theorem success_prerequisite_prefix (world : model.World) (sandbox : ids.SandboxId)
    (source : model.RestoreSource) (decision : publication.PublicationDecision)
    (evaluated : publication.handle_publish_paused world sandbox source = .ok (.Ok decision)) :
    ∃ record snapshot vm,
      world.controller_up = true ∧
      tableLookup sandbox world.sandboxes.entries.val = some record ∧
      record.phase = .Pausing ∧ record.terminal = false ∧
      tableLookup source.snapshot world.snapshots.entries.val = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sandbox ∧
      snapshot.source_generation = record.generation ∧ snapshot.workspace = record.workspace ∧
      snapshot.manifest = source.manifest ∧ snapshot.stage = .Durable ∧ snapshot.immutable = true ∧
      tableLookup sandbox world.worker_fences.entries.val = some ⟨record.epoch, false⟩ ∧
      record.current_vm = some vm ∧
      effect.effect_succeeded_check world (.InstallFence sandbox record.epoch false) = .ok true ∧
      effect.effect_succeeded_check world (.FreezeVm sandbox record.epoch record.generation vm) = .ok true ∧
      effect.effect_succeeded_check world
        (.SealSnapshot sandbox record.epoch record.generation vm record.workspace source.snapshot source.manifest) = .ok true ∧
      effect.effect_succeeded_check world
        (.CommitSnapshot sandbox record.epoch record.generation source.snapshot source.manifest) = .ok true ∧
      effect.effect_succeeded_check world (.StopVm sandbox record.epoch record.generation vm) = .ok true := by
  obtain ⟨record, snapshot, vm, up, stored, phase, terminal, snapshotStored,
    owner, session, generation, workspace, manifest, stage, immutable, fence,
    currentVm, fenceRun, freezeRun, sealRun, commitRun, stopRun, _remaining⟩ :=
    publish_success_checks world sandbox source decision evaluated
  exact ⟨record, snapshot, vm, up, stored, phase, terminal, snapshotStored,
    owner, session, generation, workspace, manifest, stage, immutable, fence,
    currentVm, fenceRun, freezeRun, sealRun, commitRun, stopRun⟩

end SandboxModel.Lifecycle.PausePublication
