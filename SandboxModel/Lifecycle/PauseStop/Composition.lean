import SandboxModel.Lifecycle.PauseStop.CommitPost
import SandboxModel.Lifecycle.PauseStop.StopRoute

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

/-- Ordinary Pausing cannot take the terminal Revoking late-capture route,
    even for a malformed Pausing row with terminal=true. -/
theorem pausing_late_route_none (world : model.World) (id : model.EffectId)
    (sandbox : ids.SandboxId) (record : model.SandboxRecord)
    (commandSandbox : model.Command.sandbox id.logical.command = .ok sandbox)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing) :
    worker.late_capture.record world id = .ok none := by
  have rowRead : store.Store.get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      world.sandboxes sandbox = .ok (some record) := by
    simp only [get_exact _ caller_index_sandbox_eq_lawful,stored]
  have phaseRead : core.cmp.PartialEq.ne.trait_default
      model.Phase.Insts.CoreCmpPartialEqPhase record.phase .Revoking = .ok true := by
    simp [phase,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      SandboxModel.Lifecycle.PausePublication.phase_eq]
  simp only [worker.late_capture.record,model.EffectId.command,commandSandbox,
    model.World.sandbox_record,rowRead,bind_tc_ok,
    SandproofFoundationsRust.core.option.Option.Insts.CoreOpsTry_traitTry.branch,phaseRead]
  cases record.terminal <;> simp

theorem commit_execute_success (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.execute world
      ⟨⟨.CommitSnapshot sandbox epoch generation snapshot manifest⟩,attempt⟩ = .ok (.Ok decision)) :
    worker.pause_stop.commit_snapshot world sandbox epoch generation snapshot manifest = .ok (.Ok decision) := by
  simp only [worker.pause_stop.execute,model_Command_clone_exact,bind_tc_ok] at evaluated
  obtain ⟨current,_currentRead,tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨different,_compared,tail⟩ := bind_ok_inv _ _ _ tail
  cases different with
  | true => simp at tail
  | false =>
    simp only [Bool.false_eq_true,if_false] at tail
    obtain ⟨present,_presence,tail⟩ := bind_ok_inv _ _ _ tail
    cases present with
    | false => simp at tail
    | true =>
      simp only [if_true] at tail
      obtain ⟨status,_found,tail⟩ := bind_ok_inv _ _ _ tail
      obtain ⟨waiting,_awaiting,tail⟩ := bind_ok_inv _ _ _ tail
      cases waiting with
      | false => simp at tail
      | true =>
        have late := pausing_late_route_none world
          ⟨⟨.CommitSnapshot sandbox epoch generation snapshot manifest⟩,attempt⟩
          sandbox record rfl stored phase
        simpa only [if_true,late,bind_tc_ok] using tail

theorem commit_record_success (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.record world
      ⟨⟨.CommitSnapshot sandbox epoch generation snapshot manifest⟩,attempt⟩ = .ok (some (.Ok decision))) :
    worker.pause_stop.commit_snapshot world sandbox epoch generation snapshot manifest = .ok (.Ok decision) := by
  obtain ⟨response,same,executed⟩ := record_supported_return world
    ⟨⟨.CommitSnapshot sandbox epoch generation snapshot manifest⟩,attempt⟩ rfl _ evaluated
  have responseSame : response = .Ok decision := by simpa only [Option.some.injEq] using same.symm
  subst response
  exact commit_execute_success world sandbox epoch generation attempt snapshot manifest
    record decision stored phase executed

theorem commit_record_registration (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.record world
      ⟨⟨.CommitSnapshot sandbox epoch generation snapshot manifest⟩,attempt⟩ = .ok (some (.Ok decision))) :
    RegisteredCommit world sandbox epoch generation snapshot manifest decision :=
  commit_success_registration world sandbox epoch generation snapshot manifest decision
    (commit_record_success world sandbox epoch generation attempt snapshot manifest record decision stored phase evaluated)

/-- The intermediate dispatcher must preserve these two tables. Its task
    state changes and V9 reachability are not proved here. A later real Stop
    still performs all of its own current-attempt, permission and target checks. -/
def OriginalStopGeneration (before after : model.World) (sandbox : ids.SandboxId) : Prop :=
  ∃ record vm,
    tableLookup sandbox before.sandboxes.entries.val = some record ∧
    record.current_vm = some vm ∧ record.phase = .Pausing ∧
    ∀ (later : model.World) (attempt : U64) (decision : worker.WorkerDecision),
      later.sandboxes = after.sandboxes → later.vms = after.vms →
      worker.pause_stop.record later ⟨⟨.StopVm sandbox record.epoch record.generation vm⟩,attempt⟩ =
        .ok (some (.Ok decision)) →
      ∃ previous,
        tableLookup vm before.vms.entries.val = some previous ∧
        record.generation = previous.generation ∧ previous.sandbox = sandbox ∧
        decision.event = .StopVm sandbox vm

theorem registered_commit_original_generation (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (decision : worker.WorkerDecision)
    (registered : RegisteredCommit world sandbox epoch generation snapshot manifest decision) :
    OriginalStopGeneration world decision.next sandbox := by
  obtain ⟨record,vm,_key,_plan,_stored,_before,_after,_plans,_material,_allowed,
    recordFound,_snapshotFound,currentVm,phase,_epoch,_generation,_planVm,_target,
    _captured,_observed,_registered,_source,_eligibility,_plansEntries,_materialWrite,
    _permitRead,_gated,_finalPlan,_finalSnapshot,frame,_event⟩ := registered
  have sandboxesSame : decision.next.sandboxes = world.sandboxes := by
    have projected := congrArg model.World.sandboxes frame
    exact projected
  have vmsSame : decision.next.vms = world.vms := by
    have projected := congrArg model.World.vms frame
    exact projected
  refine ⟨record,vm,recordFound,currentVm,phase,?_⟩
  intro later attempt stopped sandboxFrame vmFrame accepted
  have laterRecord : tableLookup sandbox later.sandboxes.entries.val = some record := by
    rw [sandboxFrame,sandboxesSame]
    exact recordFound
  obtain ⟨previous,_key,_plan,_handoff,_target,_snapshot,
    previousFound,_epoch,generationChecked,sandboxChecked,_currentVm,_terminal,_frozen,_fence,
    _planRead,_planVm,_captured,_permission,_targetRead,_snapshotRead,_owner,
    _snapshotSandbox,_sourceGeneration,_workspace,_manifest,_stage,_immutable,_updated,event⟩ :=
    stop_success_facts later sandbox record.epoch record.generation vm record stopped
      laterRecord phase (stop_record_success later sandbox record.epoch record.generation attempt vm stopped accepted)
  refine ⟨previous,?_,generationChecked,sandboxChecked,event⟩
  simpa only [vmFrame,vmsSame] using previousFound

theorem commit_record_original_generation (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation attempt : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.record world
      ⟨⟨.CommitSnapshot sandbox epoch generation snapshot manifest⟩,attempt⟩ = .ok (some (.Ok decision))) :
    OriginalStopGeneration world decision.next sandbox :=
  registered_commit_original_generation world sandbox epoch generation snapshot manifest decision
    (commit_record_registration world sandbox epoch generation attempt snapshot manifest record decision stored phase evaluated)

end SandboxModel.Lifecycle.PauseStop
