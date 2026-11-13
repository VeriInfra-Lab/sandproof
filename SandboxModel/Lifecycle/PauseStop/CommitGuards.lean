import SandboxModel.Lifecycle.CaptureHandoff.CallerCommitChecks
import SandboxModel.Lifecycle.CaptureHandoff.CallerCurrent
import SandboxModel.Lifecycle.PausePublication.Checks

/-! Guards derived from the actual normal Commit implementation. Success is
only an input premise. The final equality retains the actual generated suffix
for composition with the real writes and enqueue; it supplies no snapshot-owner
or nonterminal check, and no physical persistence or dispatch conclusion. -/
namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller
open SandboxModel.Lifecycle.PausePublication

theorem commit_reject_true {A E : Type} (condition : Bool)
    (continuation : Result (core.result.Result A E)) (error : E) (out : A)
    (evaluated : (if condition then .ok (.Err error) else continuation) = .ok (.Ok out)) :
    condition = false := by
  cases condition with
  | false => rfl
  | true => simp at evaluated

/-- current_plan checks an operation and its first matching plan, not the
plan's kind, key/epoch/generation fields or capture value. -/
theorem current_plan_success_rows (world : model.World) (sandbox : ids.SandboxId)
    (record : model.SandboxRecord) (key : model.OperationKey) (plan : model.OperationPlan)
    (evaluated : worker.current_plan world sandbox record = .ok (.Ok (key, plan))) :
    ∃ operation, record.current_operation = some operation ∧
      key = ⟨sandbox, operation.id⟩ ∧
      tableLookup key world.operation_plans.entries.val = some plan := by
  cases current : record.current_operation with
  | none => simp [worker.current_plan, current] at evaluated
  | some operation =>
    have queried := get_exact model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operation_key_lawful world.operation_plans ⟨sandbox, operation.id⟩
    simp only [worker.current_plan, current, transition.operation_key,
      bind_tc_ok, queried] at evaluated
    cases found : tableLookup ⟨sandbox, operation.id⟩ world.operation_plans.entries.val with
    | none => simp [found] at evaluated
    | some actualPlan =>
      simp only [found] at evaluated
      have pair : ((⟨sandbox, operation.id⟩ : model.OperationKey), actualPlan) = (key, plan) := by
        simpa using evaluated
      have keyEq : key = ⟨sandbox, operation.id⟩ := by
        simpa using congrArg Prod.fst pair.symm
      have planEq : actualPlan = plan := by simpa using congrArg Prod.snd pair
      exact ⟨operation, rfl, keyEq, (by simpa [keyEq, planEq] using found)⟩

/-- These stronger plan identities belong to current_capture, not to the
current_plan helper. Both calls query the same first matching plan row. -/
theorem current_capture_bound_plan (world : model.World) (sandbox : ids.SandboxId)
    (record : model.SandboxRecord) (operation : model.Operation)
    (key : model.OperationKey) (plan : model.OperationPlan)
    (before : capture.CaptureHandoff)
    (current : record.current_operation = some operation)
    (keyEq : key = ⟨sandbox, operation.id⟩)
    (found : tableLookup key world.operation_plans.entries.val = some plan)
    (observed : transition.current_capture world sandbox record = .ok (some before)) :
    operation.kind = .Pause ∧ operation.epoch = record.epoch ∧
      plan.key = key ∧ plan.epoch = record.epoch ∧ plan.generation = record.generation ∧
      plan.capture = some before := by
  classical
  have view : currentCaptureView world sandbox record = some before := by
    simpa only [current_capture_exact, Result.ok.injEq] using observed
  simp [currentCaptureView, current, ← keyEq, found] at view
  exact view

/-- All actual ordinary Commit guards are conclusions. Unknown registration
is not assumed; NotRegistered is also allowed when observe_bundle permits it.
There is deliberately no snapshot.owner or record.terminal conclusion. -/
theorem commit_success_checks (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) (decision : worker.WorkerDecision)
    (evaluated : worker.pause_stop.commit_snapshot world sandbox epoch generation snapshot manifest =
      .ok (.Ok decision)) :
    ∃ stored record vm key plan operation before after,
      tableLookup snapshot world.snapshots.entries.val = some stored ∧
      tableLookup sandbox world.sandboxes.entries.val = some record ∧
      epoch = record.epoch ∧ generation = record.generation ∧
      manifest = stored.manifest ∧ stored.sandbox = sandbox ∧ record.phase = .Pausing ∧
      record.current_vm = some vm ∧ stored.source_generation = record.generation ∧
      stored.workspace = record.workspace ∧ stored.stage = .LocalSealed ∧
      worker.current_plan world sandbox record = .ok (.Ok (key, plan)) ∧
      record.current_operation = some operation ∧ key = ⟨sandbox, operation.id⟩ ∧
      tableLookup key world.operation_plans.entries.val = some plan ∧
      plan.vm = some vm ∧ plan.target_snapshot = some ⟨snapshot, manifest⟩ ∧
      transition.current_capture world sandbox record = .ok (some before) ∧
      operation.kind = .Pause ∧ operation.epoch = record.epoch ∧
      plan.key = key ∧ plan.epoch = record.epoch ∧ plan.generation = record.generation ∧
      plan.capture = some before ∧
      capture.CaptureHandoff.observe_bundle before = .ok (some after) ∧
      before.capture_work = true ∧ before.unload_source = false ∧
      before.commit ≠ .Registered ∧ before.eligibility ≠ .Recoverable ∧
      ((do
        let next ← model.World.Insts.CoreCloneClone.clone world
        let registered ← model.OperationPlan.Insts.CoreCloneClone.clone plan
        let plans ← store.Store.put model.OperationKey.Insts.CoreCmpPartialEqOperationKey
          next.operation_plans key { registered with capture := some after }
        let next1 ← worker.put_snapshot { next with operation_plans := plans } snapshot
          { stored with stage := .Durable, immutable := true }
        let allowed ← capture.CaptureHandoff.may_stop_original after
        if allowed then
          let next2 ← transition.enqueue next1
            (.StopVm sandbox record.epoch record.generation vm)
          .ok (.Ok ({ event := .CommitSnapshot snapshot, next := next2 } : worker.WorkerDecision))
        else
          .ok (.Ok ({ event := .CommitSnapshot snapshot, next := next1 } : worker.WorkerDecision))) :
        Result (core.result.Result worker.WorkerDecision worker.WorkerReject)) = .ok (.Ok decision) := by
  have snapshotRead := get_exact ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
    snapshot_id_lawful world.snapshots snapshot
  have sandboxRead := get_exact ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    caller_index_sandbox_eq_lawful world.sandboxes sandbox
  simp only [worker.pause_stop.commit_snapshot, snapshotRead, sandboxRead, bind_tc_ok] at evaluated
  cases snapshotFound : tableLookup snapshot world.snapshots.entries.val with
  | none => simp [snapshotFound] at evaluated
  | some stored =>
    simp only [snapshotFound] at evaluated
    cases recordFound : tableLookup sandbox world.sandboxes.entries.val with
    | none => simp [recordFound] at evaluated
    | some record =>
      simp only [recordFound] at evaluated
      have epochGuard := commit_reject_true _ _ _ _ evaluated
      have epochOk : epoch = record.epoch := by
        by_contra different
        simp [different] at epochGuard
      simp only [epochGuard, Bool.false_eq_true, if_false] at evaluated
      have generationGuard := commit_reject_true _ _ _ _ evaluated
      have generationOk : generation = record.generation := by
        by_contra different
        simp [different] at generationGuard
      simp only [generationGuard, Bool.false_eq_true, if_false] at evaluated
      obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ evaluated
      have allowed := commit_reject_true _ _ _ _ tail
      subst different
      have manifestOk : manifest = stored.manifest := by
        simpa [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default] using compared
      simp only [Bool.false_eq_true, if_false] at tail
      obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ tail
      have allowed := commit_reject_true _ _ _ _ tail
      subst different
      have sandboxOk : stored.sandbox = sandbox := by
        simpa [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
          ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, ← UScalar.eq_equiv] using compared
      simp only [Bool.false_eq_true, if_false] at tail
      obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ tail
      have allowed := commit_reject_true _ _ _ _ tail
      subst different
      have phaseOk : record.phase = .Pausing := by
        simpa [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default] using compared
      simp only [Bool.false_eq_true, if_false] at tail
      cases currentVm : record.current_vm with
      | none => simp [currentVm] at tail
      | some vm =>
        simp only [currentVm] at tail
        have sourceGuard := commit_reject_true _ _ _ _ tail
        have sourceOk : stored.source_generation = record.generation := by
          by_contra different
          simp [different] at sourceGuard
        simp only [sourceGuard, Bool.false_eq_true, if_false] at tail
        obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ tail
        have allowed := commit_reject_true _ _ _ _ tail
        subst different
        have workspaceOk : stored.workspace = record.workspace := by
          simpa [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
            ids.FsId.Insts.CoreCmpPartialEqFsId.eq, ← UScalar.eq_equiv] using compared
        simp only [Bool.false_eq_true, if_false] at tail
        obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ tail
        have allowed := commit_reject_true _ _ _ _ tail
        subst different
        have stageOk : stored.stage = .LocalSealed := by
          simpa [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default] using compared
        simp only [Bool.false_eq_true, if_false] at tail
        obtain ⟨planResult, planRead, tail⟩ := bind_ok_inv _ _ _ tail
        cases planResult with
        | Err error =>
          simp [core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at tail
        | Ok pair =>
          rcases pair with ⟨key, plan⟩
          simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at tail
          obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ tail
          have allowed := commit_reject_true _ _ _ _ tail
          subst different
          have planVm : plan.vm = some vm := by
            simpa [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
              currentVm] using compared
          simp only [Bool.false_eq_true, if_false] at tail
          obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ tail
          have allowed := commit_reject_true _ _ _ _ tail
          subst different
          have target : plan.target_snapshot = some ⟨snapshot, manifest⟩ := by
            simpa [option_restore_ne_exact] using compared
          simp only [Bool.false_eq_true, if_false] at tail
          obtain ⟨captured, captureRead, tail⟩ := bind_ok_inv _ _ _ tail
          cases captured with
          | none => simp [core.option.Option.and_then] at tail
          | some before =>
            simp only [core.option.Option.and_then,
              P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffOptionCaptureHandoff.call_once] at tail
            obtain ⟨observed, observation, tail⟩ := bind_ok_inv _ _ _ tail
            cases observed with
            | none => simp at tail
            | some after =>
              obtain ⟨work, sourceAbsent, unregistered, eligibility⟩ :=
                caller_observation_success_preconditions before after observation
              obtain ⟨operation, currentOperation, keyEq, storedPlan⟩ :=
                current_plan_success_rows world sandbox record key plan planRead
              obtain ⟨kind, operationEpoch, planKey, planEpoch, planGeneration, capturedPlan⟩ :=
                current_capture_bound_plan world sandbox record operation key plan before
                  currentOperation keyEq storedPlan captureRead
              exact ⟨stored, record, vm, key, plan, operation, before, after,
                rfl, rfl, epochOk, generationOk, manifestOk, sandboxOk,
                phaseOk, currentVm, sourceOk, workspaceOk, stageOk, planRead,
                currentOperation, keyEq, storedPlan, planVm, target, captureRead,
                kind, operationEpoch, planKey, planEpoch, planGeneration, capturedPlan,
                observation, work, sourceAbsent, unregistered, eligibility,
                (by simpa using tail)⟩

end SandboxModel.Lifecycle.PauseStop
