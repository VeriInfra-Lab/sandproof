import SandboxModel.Lifecycle.PauseStop.StopChecks

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller
open SandboxModel.Lifecycle.PausePublication

theorem stop_late_route_none (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (vm : ids.VmId) :
    worker.late_capture.record world ⟨⟨.StopVm sandbox epoch generation vm⟩, attempt⟩ = .ok none := by
  simp only [worker.late_capture.record, model.EffectId.command,
    model.Command.sandbox, model.World.sandbox_record,
    get_exact _ caller_index_sandbox_eq_lawful, bind_tc_ok]
  cases stored : tableLookup sandbox world.sandboxes.entries.val with
  | none =>
    simp [core.option.Option.Insts.CoreOpsTry_traitTry.branch,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual]
  | some record =>
    simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch]
    cases terminal : record.terminal with
    | false => simp [terminal]
    | true =>
      simp only [core.cmp.PartialEq.ne.trait_default,
        core.cmp.PartialEq.ne.default, phase_eq, bind_tc_ok]
      split <;> simp_all

theorem stop_execute_success (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (vm : ids.VmId) (decision : worker.WorkerDecision)
    (evaluated : worker.pause_stop.execute world
      ⟨⟨.StopVm sandbox epoch generation vm⟩, attempt⟩ = .ok (.Ok decision)) :
    worker.pause_stop.stop_vm world sandbox epoch generation vm = .ok (.Ok decision) := by
  simp only [worker.pause_stop.execute, model_Command_clone_exact, bind_tc_ok] at evaluated
  obtain ⟨current, _currentRead, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨different, _compared, tail⟩ := bind_ok_inv _ _ _ tail
  cases different with
  | true => simp at tail
  | false =>
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨present, _presence, tail⟩ := bind_ok_inv _ _ _ tail
    cases present with
    | false => simp at tail
    | true =>
      simp only [if_true] at tail
      obtain ⟨status, _found, tail⟩ := bind_ok_inv _ _ _ tail
      obtain ⟨waiting, _awaiting, tail⟩ := bind_ok_inv _ _ _ tail
      cases waiting with
      | false => simp at tail
      | true => simpa only [if_true, stop_late_route_none, bind_tc_ok] using tail

theorem stop_record_success (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (vm : ids.VmId) (decision : worker.WorkerDecision)
    (evaluated : worker.pause_stop.record world
      ⟨⟨.StopVm sandbox epoch generation vm⟩, attempt⟩ = .ok (some (.Ok decision))) :
    worker.pause_stop.stop_vm world sandbox epoch generation vm = .ok (.Ok decision) := by
  obtain ⟨response, same, executed⟩ := record_supported_return world
    ⟨⟨.StopVm sandbox epoch generation vm⟩, attempt⟩ rfl _ evaluated
  have responseSame : response = .Ok decision := by simpa only [Option.some.injEq] using same.symm
  subst response
  exact stop_execute_success world sandbox epoch generation attempt vm decision executed

theorem stop_record_success_permission (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (vm : ids.VmId) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.record world
      ⟨⟨.StopVm sandbox epoch generation vm⟩, attempt⟩ = .ok (some (.Ok decision))) :
    ∃ handoff,
      transition.current_capture world sandbox record = .ok (some handoff) ∧
      handoff.commit = .Registered ∧ handoff.eligibility = .Recoverable ∧
      handoff.unload_source = true := by
  exact stop_success_permission world sandbox epoch generation vm record decision stored phase
    (stop_record_success world sandbox epoch generation attempt vm decision evaluated)

/-- The production record root changes the exact VM row to Stopped and frames
    every World field except the real VM table and resource-index table. -/
theorem stop_record_success_update (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (vm : ids.VmId) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.record world
      ⟨⟨.StopVm sandbox epoch generation vm⟩, attempt⟩ = .ok (some (.Ok decision))) :
    ∃ previous,
      tableLookup vm world.vms.entries.val = some previous ∧
      (∀ query, tableLookup query decision.next.vms.entries.val =
        if query = vm then some {previous with status := .Stopped}
        else tableLookup query world.vms.entries.val) ∧
      decision.next = { world with
        vms := decision.next.vms,
        resource_indexes := decision.next.resource_indexes } ∧
      decision.event = .StopVm sandbox vm := by
  obtain ⟨previous, _key, _plan, _handoff, _target, _snapshot,
    vmStored, _epoch, _generation, _sandbox, _currentVm, _terminal, _frozen,
    _fence, _currentPlan, _planVm, _captured, _permission, _targetRead,
    _snapshotRead, _owner, _snapshotSandbox, _sourceGeneration, _workspace,
    _manifest, _durable, _immutable, updated, event⟩ :=
    stop_success_facts world sandbox epoch generation vm record decision stored phase
      (stop_record_success world sandbox epoch generation attempt vm decision evaluated)
  exact ⟨previous, vmStored,
    put_existing_vm_queries world decision.next vm previous
      {previous with status := .Stopped} vmStored updated,
    put_vm_frame world decision.next vm {previous with status := .Stopped} updated,event⟩

end SandboxModel.Lifecycle.PauseStop
