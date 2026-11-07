import SandboxModel.Lifecycle.PausePublication.Prefix
import SandboxModel.Lifecycle.PausePublication.Effects
import SandboxModel.Lifecycle.PausePublication.Conditions

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

abbrev Input := ids.SandboxId × model.RestoreSource

def publishes (world : model.World) (input : Input) : Prop :=
  ∃ decision, publication.handle_publish_paused world input.1 input.2 = .ok (.Ok decision)

/-- This is a logical receipt-table fact, not a physical VMM exit assertion. -/
noncomputable def currentStopAcknowledged (world : model.World) (input : Input) : Prop := by
  classical
  exact ∃ record vm receipt,
    tableLookup input.1 world.sandboxes.entries.val = some record ∧
    record.current_vm = some vm ∧
    tableLookup ⟨⟨.StopVm input.1 record.epoch record.generation vm⟩,
        currentAttempt world (.StopVm input.1 record.epoch record.generation vm)⟩
      world.effects.entries.val = some (.Acknowledged (.Succeeded receipt))

/-- Only actual successful execution is assumed; the Stop fact is derived. -/
theorem publish_success_requires_stop : RequiresStop publishes currentStopAcknowledged := by
  classical
  intro world input success
  obtain ⟨decision, evaluated⟩ := success
  obtain ⟨record, _snapshot, vm, _up, stored, _phase, _terminal, _snapshotStored,
    _owner, _session, _generation, _workspace, _manifest, _stage, _immutable,
    _fence, currentVm, _fenceRun, _freezeRun, _sealRun, _commitRun, stopRun⟩ :=
    success_prerequisite_prefix world input.1 input.2 decision evaluated
  obtain ⟨receipt, acknowledged⟩ :=
    (effect_success_iff world (.StopVm input.1 record.epoch record.generation vm)
      (.stop input.1 record.epoch record.generation vm)).1 stopRun
  exact ⟨record, vm, receipt, stored, currentVm, acknowledged⟩

end SandboxModel.Lifecycle.PausePublication
