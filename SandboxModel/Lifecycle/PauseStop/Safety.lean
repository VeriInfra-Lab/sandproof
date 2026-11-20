import SandboxModel.Lifecycle.PauseStop.StopRoute
import SandboxModel.Lifecycle.PauseStop.Conditions

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

/-- Acceptance by the real production record root for an ordinary Pause row. -/
def stops (world : model.World) (input : StopInput) : Prop :=
  ∃ record decision,
    tableLookup input.sandbox world.sandboxes.entries.val = some record ∧
    record.phase = .Pausing ∧
    worker.pause_stop.record world
      ⟨⟨.StopVm input.sandbox input.epoch input.generation input.vm⟩, input.attempt⟩ =
      .ok (some (.Ok decision))

/-- A control-state permission, not a statement that a physical VM has exited. -/
def hasPermission (world : model.World) (input : StopInput) : Prop :=
  ∃ record handoff,
    tableLookup input.sandbox world.sandboxes.entries.val = some record ∧
    record.phase = .Pausing ∧
    transition.current_capture world input.sandbox record = .ok (some handoff) ∧
    handoff.commit = .Registered ∧ handoff.eligibility = .Recoverable ∧
    handoff.unload_source = true

theorem record_stop_requires_permission : RequiresPermission stops hasPermission := by
  intro world input accepted
  obtain ⟨record, decision, stored, phase, evaluated⟩ := accepted
  obtain ⟨handoff, captured, registered, recoverable, source⟩ :=
    stop_record_success_permission world input.sandbox input.epoch input.generation
      input.attempt input.vm record decision stored phase evaluated
  exact ⟨record, handoff, stored, phase, captured, registered, recoverable, source⟩

end SandboxModel.Lifecycle.PauseStop
