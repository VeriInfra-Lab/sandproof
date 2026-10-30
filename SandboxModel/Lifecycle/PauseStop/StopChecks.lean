import SandboxModel.Lifecycle.PauseStop.Admission
import SandboxModel.Lifecycle.PauseStop.VmUpdate
import SandboxModel.Lifecycle.PausePublication.Checks
import SandboxModel.Lifecycle.PausePublication.Guards

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller
open SandboxModel.Lifecycle.PausePublication

theorem stop_reject_true {A E : Type} (condition : Bool)
    (continuation : Result (core.result.Result A E)) (error : E) (out : A)
    (evaluated : (if condition then .ok (.Err error) else continuation) = .ok (.Ok out)) :
    condition = false := by
  cases condition with
  | false => rfl
  | true => simp at evaluated

theorem stop_reject_false {A E : Type} (condition : Bool)
    (continuation : Result (core.result.Result A E)) (error : E) (out : A)
    (evaluated : (if condition then continuation else .ok (.Err error)) = .ok (.Ok out)) :
    condition = true := by
  cases condition with
  | true => rfl
  | false => simp at evaluated

/-- A successful ordinary Pause Stop must pass the real capture permission call.
    The premise fixes only the stored sandbox and ordinary Pause phase; the
    registration, eligibility and original-source permission are conclusions. -/
theorem stop_success_permission (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (vm : ids.VmId) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.stop_vm world sandbox epoch generation vm = .ok (.Ok decision)) :
    ∃ handoff,
      transition.current_capture world sandbox record = .ok (some handoff) ∧
      handoff.commit = .Registered ∧ handoff.eligibility = .Recoverable ∧
      handoff.unload_source = true := by
  have rowRead : store.Store.get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      world.sandboxes sandbox = .ok (some record) := by
    simp only [get_exact _ caller_index_sandbox_eq_lawful, stored]
  simp only [worker.pause_stop.stop_vm, rowRead, bind_tc_ok] at evaluated
  obtain ⟨vmRow, _vmRead, tail⟩ := bind_ok_inv _ _ _ evaluated
  cases vmRow with
  | none => simp at tail
  | some storedVm =>
    simp only at tail
    have epochOk := stop_reject_true _ _ _ _ tail
    simp only [epochOk, Bool.false_eq_true, if_false] at tail
    have generationOk := stop_reject_true _ _ _ _ tail
    simp only [generationOk, Bool.false_eq_true, if_false] at tail
    obtain ⟨different, _sandboxRead, tail⟩ := bind_ok_inv _ _ _ tail
    have sameSandbox := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨stoppable, _stoppableRead, tail⟩ := bind_ok_inv _ _ _ tail
    have canStop := stop_reject_false _ _ _ _ tail
    subst stoppable
    simp only [if_true] at tail
    have phaseRead : model.Phase.Insts.CoreCmpPartialEqPhase.eq record.phase .Pausing = .ok true := by
      simp [phase]
    rw [phaseRead] at tail
    simp only [bind_tc_ok, if_true] at tail
    obtain ⟨different, _currentVmRead, tail⟩ := bind_ok_inv _ _ _ tail
    have sameVm := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    have nonterminal := stop_reject_true _ _ _ _ tail
    simp only [nonterminal, Bool.false_eq_true, if_false] at tail
    obtain ⟨different, _frozenRead, tail⟩ := bind_ok_inv _ _ _ tail
    have frozen := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨fence, _fenceRead, tail⟩ := bind_ok_inv _ _ _ tail
    obtain ⟨different, _fenceCompare, tail⟩ := bind_ok_inv _ _ _ tail
    have sameFence := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨planResult, _planRead, tail⟩ := bind_ok_inv _ _ _ tail
    cases planResult with
    | Err error =>
      simp [core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at tail
    | Ok pair =>
      rcases pair with ⟨key, plan⟩
      simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at tail
      obtain ⟨different, _planVmRead, tail⟩ := bind_ok_inv _ _ _ tail
      have samePlanVm := stop_reject_true _ _ _ _ tail
      subst different
      simp only [Bool.false_eq_true, if_false] at tail
      obtain ⟨handoff, handoffRead, tail⟩ := bind_ok_inv _ _ _ tail
      cases handoff with
      | none =>
        simp [SandproofFoundationsRust.core.option.Option.is_some_and] at tail
      | some handoff =>
        simp only [SandproofFoundationsRust.core.option.Option.is_some_and,
          P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once] at tail
        obtain ⟨allowed, permitRead, tail⟩ := bind_ok_inv _ _ _ tail
        have allowedTrue := stop_reject_false _ _ _ _ tail
        subst allowed
        exact ⟨handoff, handoffRead, (stop_requires_complete_registration handoff).1 permitRead⟩

theorem stop_checked_same [DecidableEq T] (eq : core.cmp.PartialEq T T)
    (lawful : TableLawfulEq eq) (left right : T)
    (evaluated : core.cmp.PartialEq.ne.trait_default eq left right = .ok false) :
    left = right := by
  have compared := lawful left right
  simpa [core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,compared] using evaluated

theorem stop_frozen_checked (status : model.VmStatus)
    (evaluated : core.cmp.PartialEq.ne.trait_default
      model.VmStatus.Insts.CoreCmpPartialEqVmStatus status .Frozen = .ok false) :
    status = .Frozen := by
  cases status <;> simp [core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
    model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq,model.VmStatus.read_discriminant] at evaluated ⊢

/-- Every ordinary-Stop guard and the exact final write are consequences of
    real successful execution. No VM-owner or VM/record-generation invariant
    is silently added: those are not guards of this independent entry. -/
theorem stop_success_facts (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (vm : ids.VmId) (record : model.SandboxRecord)
    (decision : worker.WorkerDecision)
    (stored : tableLookup sandbox world.sandboxes.entries.val = some record)
    (phase : record.phase = .Pausing)
    (evaluated : worker.pause_stop.stop_vm world sandbox epoch generation vm = .ok (.Ok decision)) :
    ∃ storedVm key plan handoff target snapshot,
      tableLookup vm world.vms.entries.val = some storedVm ∧
      epoch = record.epoch ∧ generation = storedVm.generation ∧ storedVm.sandbox = sandbox ∧
      record.current_vm = some vm ∧ record.terminal = false ∧ storedVm.status = .Frozen ∧
      tableLookup sandbox world.worker_fences.entries.val = some ⟨record.epoch,false⟩ ∧
      worker.current_plan world sandbox record = .ok (.Ok (key,plan)) ∧ plan.vm = some vm ∧
      transition.current_capture world sandbox record = .ok (some handoff) ∧
      (handoff.commit = .Registered ∧ handoff.eligibility = .Recoverable ∧ handoff.unload_source = true) ∧
      plan.target_snapshot = some target ∧
      tableLookup target.snapshot world.snapshots.entries.val = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sandbox ∧
      snapshot.source_generation = record.generation ∧ snapshot.workspace = record.workspace ∧
      snapshot.manifest = target.manifest ∧ snapshot.stage = .Durable ∧ snapshot.immutable = true ∧
      worker.put_vm world vm {storedVm with status := .Stopped} = .ok decision.next ∧
      decision.event = .StopVm sandbox vm := by
  have rowRead : store.Store.get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      world.sandboxes sandbox = .ok (some record) := by
    simp only [get_exact _ caller_index_sandbox_eq_lawful, stored]
  simp only [worker.pause_stop.stop_vm, rowRead, bind_tc_ok] at evaluated
  obtain ⟨vmRow, vmRead, tail⟩ := bind_ok_inv _ _ _ evaluated
  cases vmRow with
  | none => simp at tail
  | some storedVm =>
    simp only at tail
    have epochOk := stop_reject_true _ _ _ _ tail
    simp only [epochOk, Bool.false_eq_true, if_false] at tail
    have generationOk := stop_reject_true _ _ _ _ tail
    simp only [generationOk, Bool.false_eq_true, if_false] at tail
    obtain ⟨different, sandboxRead, tail⟩ := bind_ok_inv _ _ _ tail
    have sameSandbox := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨stoppable, _stoppableRead, tail⟩ := bind_ok_inv _ _ _ tail
    have canStop := stop_reject_false _ _ _ _ tail
    subst stoppable
    simp only [if_true] at tail
    have phaseRead : model.Phase.Insts.CoreCmpPartialEqPhase.eq record.phase .Pausing = .ok true := by
      simp [phase]
    rw [phaseRead] at tail
    simp only [bind_tc_ok, if_true] at tail
    obtain ⟨different, currentVmRead, tail⟩ := bind_ok_inv _ _ _ tail
    have sameVm := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    have nonterminal := stop_reject_true _ _ _ _ tail
    simp only [nonterminal, Bool.false_eq_true, if_false] at tail
    obtain ⟨different, frozenRead, tail⟩ := bind_ok_inv _ _ _ tail
    have frozen := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨fence, fenceRead, tail⟩ := bind_ok_inv _ _ _ tail
    obtain ⟨different, fenceCompare, tail⟩ := bind_ok_inv _ _ _ tail
    have sameFence := stop_reject_true _ _ _ _ tail
    subst different
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨planResult, planRead, tail⟩ := bind_ok_inv _ _ _ tail
    cases planResult with
    | Err error =>
      simp [core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at tail
    | Ok pair =>
      rcases pair with ⟨key, plan⟩
      simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at tail
      obtain ⟨different, planVmRead, tail⟩ := bind_ok_inv _ _ _ tail
      have samePlanVm := stop_reject_true _ _ _ _ tail
      subst different
      simp only [Bool.false_eq_true, if_false] at tail
      obtain ⟨handoff, handoffRead, tail⟩ := bind_ok_inv _ _ _ tail
      cases handoff with
      | none =>
        simp [SandproofFoundationsRust.core.option.Option.is_some_and] at tail
      | some handoff =>
        simp only [SandproofFoundationsRust.core.option.Option.is_some_and,
          P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once] at tail
        obtain ⟨allowed, permitRead, tail⟩ := bind_ok_inv _ _ _ tail
        have allowedTrue := stop_reject_false _ _ _ _ tail
        subst allowed
        simp only [if_true] at tail
        cases targetRead : plan.target_snapshot with
        | none => simp [targetRead] at tail
        | some target =>
          simp only [targetRead] at tail
          obtain ⟨snapshotRow, snapshotRead, tail⟩ := bind_ok_inv _ _ _ tail
          cases snapshotRow with
          | none => simp at tail
          | some snapshot =>
            simp only at tail
            obtain ⟨different, ownerRead, tail⟩ := bind_ok_inv _ _ _ tail
            have sameOwner := stop_reject_true _ _ _ _ tail
            subst different
            simp only [Bool.false_eq_true, if_false] at tail
            obtain ⟨different, snapshotSandboxRead, tail⟩ := bind_ok_inv _ _ _ tail
            have sameSandbox := stop_reject_true _ _ _ _ tail
            subst different
            simp only [Bool.false_eq_true, if_false] at tail
            have snapshotGeneration := stop_reject_true _ _ _ _ tail
            simp only [snapshotGeneration, Bool.false_eq_true, if_false] at tail
            obtain ⟨different, workspaceRead, tail⟩ := bind_ok_inv _ _ _ tail
            have sameWorkspace := stop_reject_true _ _ _ _ tail
            subst different
            simp only [Bool.false_eq_true, if_false] at tail
            obtain ⟨different, manifestRead, tail⟩ := bind_ok_inv _ _ _ tail
            have sameManifest := stop_reject_true _ _ _ _ tail
            subst different
            simp only [Bool.false_eq_true, if_false] at tail
            obtain ⟨different, stageRead, tail⟩ := bind_ok_inv _ _ _ tail
            have durable := stop_reject_true _ _ _ _ tail
            subst different
            simp only [Bool.false_eq_true, if_false] at tail
            have immutable := stop_reject_false _ _ _ _ tail
            simp only [immutable, if_true, model_World_clone_exact, bind_tc_ok] at tail
            obtain ⟨next, updated, returned⟩ := bind_ok_inv _ _ _ tail
            have same : ({event := worker.WorkerEvent.StopVm sandbox vm, next := next} :
                worker.WorkerDecision) = decision := by
              simpa only [Result.ok.injEq, core.result.Result.Ok.injEq] using returned
            subst decision
            have vmStored : tableLookup vm world.vms.entries.val = some storedVm := by
              simpa only [get_exact _ vm_id_lawful, Result.ok.injEq] using vmRead
            have fenceValue : fence = some ⟨record.epoch, false⟩ :=
              stop_checked_same _ (fun left right => option_fence_eq left right) _ _ fenceCompare
            have fenceStored : tableLookup sandbox world.worker_fences.entries.val =
                some ⟨record.epoch, false⟩ := by
              simpa only [get_exact _ caller_index_sandbox_eq_lawful, Result.ok.injEq, fenceValue] using fenceRead
            have snapshotStored : tableLookup target.snapshot world.snapshots.entries.val = some snapshot := by
              simpa only [get_exact _ snapshot_id_lawful, Result.ok.injEq] using snapshotRead
            exact ⟨storedVm,key,plan,handoff,target,snapshot,
              vmStored,(by simpa using epochOk),(by simpa using generationOk),
              stop_checked_same _ caller_index_sandbox_eq_lawful _ _ sandboxRead,
              stop_checked_same _ (fun left right => option_vm_eq left right) _ _ currentVmRead,
              nonterminal,stop_frozen_checked _ frozenRead,fenceStored,planRead,
              stop_checked_same _ (fun left right => option_vm_eq left right) _ _ planVmRead,
              handoffRead,(stop_requires_complete_registration handoff).1 permitRead,
              targetRead,snapshotStored,
              stop_checked_same _ (fun _ _ => rfl) _ _ ownerRead,
              stop_checked_same _ caller_index_sandbox_eq_lawful _ _ snapshotSandboxRead,
              (by simpa using snapshotGeneration),
              stop_checked_same _ (fun _ _ => rfl) _ _ workspaceRead,
              stop_checked_same model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef (fun left right => snapshot_manifest_eq_exact left right) _ _ manifestRead,
              stop_checked_same model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage (fun left right => snapshot_stage_eq_exact left right) _ _ stageRead,
              immutable,updated,rfl⟩

end SandboxModel.Lifecycle.PauseStop
