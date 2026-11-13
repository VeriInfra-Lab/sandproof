import SandboxModel.Lifecycle.PauseStop.CommitGuards
import SandboxModel.Lifecycle.PauseStop.EnqueueFrame

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

/-- Exact allowed fields: material registration and the real queue writer.
    All other fields, including sandbox, VM and fence tables, are preserved. -/
def RegistrationFrame (before after : model.World) : Prop :=
  after = { before with
    operation_plans := after.operation_plans,
    snapshots := after.snapshots,
    resource_indexes := after.resource_indexes,
    logical_effects := after.logical_effects,
    effect_keys := after.effect_keys,
    effects := after.effects,
    inflight := after.inflight }

/-- A real successful normal Commit registers the exact original plan and
    snapshot. It calls the original enqueue only when the actual resulting
    handoff grants permission; existing rows/current attempts are not reset. -/
def RegisteredCommit (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (decision : worker.WorkerDecision) : Prop :=
    ∃ (record : model.SandboxRecord) (vm : ids.VmId) (key : model.OperationKey)
      (plan : model.OperationPlan) (stored : model.Snapshot)
      (before after : capture.CaptureHandoff)
      (plans : store.Store model.OperationKey model.OperationPlan) (material : model.World)
      (allowed : Bool),
      tableLookup sandbox world.sandboxes.entries.val = some record ∧
      tableLookup snapshot world.snapshots.entries.val = some stored ∧
      record.current_vm = some vm ∧ record.phase = .Pausing ∧
      epoch = record.epoch ∧ generation = record.generation ∧ plan.vm = some vm ∧
      plan.target_snapshot = some ⟨snapshot,manifest⟩ ∧
      transition.current_capture world sandbox record = .ok (some before) ∧
      capture.CaptureHandoff.observe_bundle before = .ok (some after) ∧
      after.commit = .Registered ∧ after.unload_source = true ∧
      after.eligibility = (match before.eligibility with
        | .Capturing => .Recoverable | eligibility => eligibility) ∧
      plans.entries.val = tablePut key {plan with capture := some after}
        world.operation_plans.entries.val ∧
      worker.put_snapshot {world with operation_plans := plans} snapshot
        {stored with stage := .Durable,immutable := true} = .ok material ∧
      capture.CaptureHandoff.may_stop_original after = .ok allowed ∧
      (if allowed then transition.enqueue material
        (.StopVm sandbox record.epoch record.generation vm) = .ok decision.next
       else decision.next = material) ∧
      tableLookup key decision.next.operation_plans.entries.val =
        some {plan with capture := some after} ∧
      tableLookup snapshot decision.next.snapshots.entries.val =
        some {stored with stage := .Durable,immutable := true} ∧
      RegistrationFrame world decision.next ∧
      decision.event = .CommitSnapshot snapshot

theorem commit_success_registration (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (decision : worker.WorkerDecision)
    (evaluated : worker.pause_stop.commit_snapshot world sandbox epoch generation snapshot manifest =
      .ok (.Ok decision)) :
    RegisteredCommit world sandbox epoch generation snapshot manifest decision := by
  obtain ⟨stored,record,vm,key,plan,_operation,before,after,
    snapshotFound,recordFound,_epoch,_generation,_manifest,_sandbox,_phase,
    currentVm,_sourceGeneration,_workspace,_stage,_currentPlan,_currentOperation,
    _keyEq,storedPlan,planVm,target,captured,_kind,_operationEpoch,_planKey,
    _planEpoch,_planGeneration,_oldCapture,observed,_work,_sourceAbsent,
    _unregistered,_eligibility,tail⟩ :=
    commit_success_checks world sandbox epoch generation snapshot manifest decision evaluated
  simp only [model_World_clone_exact,model_OperationPlan_clone_exact,bind_tc_ok] at tail
  obtain ⟨plans,plansWrite,tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨material,materialWrite,tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨allowed,permitRead,tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨expectedPlans,expectedWrite,plansEntries⟩ := put_exact
    model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operation_plans key {plan with capture := some after}
    (Or.inl (by rw [storedPlan]; simp))
  have plansSame : plans = expectedPlans := by simpa only [plansWrite,Result.ok.injEq] using expectedWrite
  subst expectedPlans
  obtain ⟨indexes,snapshots,materialShape,snapshotEntries⟩ :=
    put_existing_snapshot_exact {world with operation_plans := plans} material snapshot stored
      {stored with stage := .Durable,immutable := true}
      snapshotFound materialWrite
  have handoffShape := caller_observation_success_output before after observed
  have registered : after.commit = .Registered := by rw [handoffShape]
  have hasSource : after.unload_source = true := by rw [handoffShape]
  have eligibility : after.eligibility = (match before.eligibility with
      | .Capturing => .Recoverable | eligibility => eligibility) := by
    rw [handoffShape]
    rfl
  have outcome : decision.event = .CommitSnapshot snapshot ∧
      (if allowed then transition.enqueue material
        (.StopVm sandbox record.epoch record.generation vm) = .ok decision.next
       else decision.next = material) ∧
      ∃ effects keys logical inflight,
        decision.next = {material with
          logical_effects := logical,effect_keys := keys,effects := effects,inflight := inflight} := by
    cases allowed with
    | true =>
      simp only [if_true] at tail
      obtain ⟨next,queued,returned⟩ := bind_ok_inv _ _ _ tail
      have same : ({event := worker.WorkerEvent.CommitSnapshot snapshot,next := next} :
          worker.WorkerDecision) = decision := by
        simpa only [Result.ok.injEq,core.result.Result.Ok.injEq] using returned
      subst decision
      obtain ⟨effects,keys,logical,inflight,shape⟩ := enqueue_shape material next
        (.StopVm sandbox record.epoch record.generation vm) queued
      exact ⟨rfl,queued,effects,keys,logical,inflight,shape⟩
    | false =>
      simp only [Bool.false_eq_true,if_false] at tail
      have same : ({event := worker.WorkerEvent.CommitSnapshot snapshot,next := material} :
          worker.WorkerDecision) = decision := by
        simpa only [Result.ok.injEq,core.result.Result.Ok.injEq] using tail
      subst decision
      exact ⟨rfl,rfl,material.effects,material.effect_keys,material.logical_effects,material.inflight,rfl⟩
  obtain ⟨event,gated,effects,keys,logical,inflight,nextShape⟩ := outcome
  have finalPlan : tableLookup key decision.next.operation_plans.entries.val =
      some {plan with capture := some after} := by
    rw [nextShape,materialShape]
    change tableLookup key plans.entries.val = _
    rw [plansEntries,table_lookup_put]
    simp
  have finalSnapshot : tableLookup snapshot decision.next.snapshots.entries.val =
      some {stored with stage := .Durable,immutable := true} := by
    rw [nextShape,materialShape]
    change tableLookup snapshot snapshots.entries.val = _
    rw [snapshotEntries,table_lookup_put]
    simp
  have frame : RegistrationFrame world decision.next := by
    unfold RegistrationFrame
    rw [nextShape,materialShape]
  exact ⟨record,vm,key,plan,stored,before,after,plans,material,allowed,
    recordFound,snapshotFound,currentVm,_phase,_epoch,_generation,planVm,target,captured,observed,
    registered,hasSource,eligibility,plansEntries,materialWrite,permitRead,gated,
    finalPlan,finalSnapshot,frame,event⟩

/-- An actually revoked normal Commit can account for material, but leaves
    effects, inflight, VM state and every field outside the three material
    tables unchanged. Ordinary failure after registration need not be revoked. -/
theorem revoked_commit_material_only (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (decision : worker.WorkerDecision)
    (record : model.SandboxRecord) (before : capture.CaptureHandoff)
    (storedRecord : tableLookup sandbox world.sandboxes.entries.val = some record)
    (captureRead : transition.current_capture world sandbox record = .ok (some before))
    (revoked : before.eligibility = .Revoked)
    (evaluated : worker.pause_stop.commit_snapshot world sandbox epoch generation snapshot manifest =
      .ok (.Ok decision)) :
    LateFrame world decision.next := by
  obtain ⟨row,_vm,_key,_plan,stored,prior,after,plans,material,allowed,
    rowFound,_snapshotFound,_currentVm,_phase,_epoch,_generation,_planVm,_target,
    captured,_observed,_registered,_source,eligibility,_plansEntries,materialWrite,
    permitRead,gated,_finalPlan,_finalSnapshot,_frame,_event⟩ :=
    commit_success_registration world sandbox epoch generation snapshot manifest decision evaluated
  have sameRow : row = record := by
    have same : some row = some record := rowFound.symm.trans storedRecord
    exact Option.some.inj same
  subst row
  have sameBefore : prior = before := by
    simpa only [captureRead,Result.ok.injEq,Option.some.injEq] using captured.symm
  subst prior
  have revokedAfter : after.eligibility = .Revoked := by
    rw [eligibility,revoked]
  have noPermit : allowed = false := by
    cases allowed with
    | false => rfl
    | true =>
      have recoverable := ((stop_requires_complete_registration after).1 permitRead).2.1
      rw [revokedAfter] at recoverable
      cases recoverable
  have sameMaterial : decision.next = material := by
    simpa only [noPermit,Bool.false_eq_true,if_false] using gated
  obtain ⟨indexes,snapshots,materialShape⟩ := put_snapshot_shape
    {world with operation_plans := plans} material snapshot
    {stored with stage := .Durable,immutable := true} materialWrite
  simp only [LateFrame,sameMaterial,materialShape]

end SandboxModel.Lifecycle.PauseStop
