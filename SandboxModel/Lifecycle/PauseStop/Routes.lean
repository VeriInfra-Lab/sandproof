import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def supported : model.Command → Bool
  | .CommitSnapshot _ _ _ _ _ => true
  | .StopVm _ _ _ _ => true
  | _ => false

theorem record_dispatch (world : model.World) (id : model.EffectId) :
    worker.pause_stop.record world id =
      if supported id.logical.command then
        (do let response ← worker.pause_stop.execute world id
            .ok (some response))
      else .ok none := by
  rcases id with ⟨⟨command⟩, attempt⟩
  cases command <;> simp [worker.pause_stop.record, model.EffectId.command, supported]

theorem record_outside (world : model.World) (id : model.EffectId)
    (outside : supported id.logical.command = false) :
    worker.pause_stop.record world id = .ok none := by
  simp [record_dispatch, outside]

theorem record_supported_return (world : model.World) (id : model.EffectId)
    (inside : supported id.logical.command = true)
    (value : Option (core.result.Result worker.WorkerDecision worker.WorkerReject))
    (evaluated : worker.pause_stop.record world id = .ok value) :
    ∃ response, value = some response ∧ worker.pause_stop.execute world id = .ok response := by
  simp only [record_dispatch, inside, if_true] at evaluated
  obtain ⟨response, executed, tail⟩ := bind_ok_inv _ _ _ evaluated
  have same : some response = value := by simpa only [Result.ok.injEq] using tail
  exact ⟨response, same.symm, executed⟩

theorem record_supported_of_execute (world : model.World) (id : model.EffectId)
    (inside : supported id.logical.command = true)
    (response : core.result.Result worker.WorkerDecision worker.WorkerReject)
    (executed : worker.pause_stop.execute world id = .ok response) :
    worker.pause_stop.record world id = .ok (some response) := by
  simp [record_dispatch, inside, executed]

end SandboxModel.Lifecycle.PauseStop
