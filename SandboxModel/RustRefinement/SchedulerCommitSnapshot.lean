import SandboxModel.RustRefinement.SchedulerStopVm

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance schedulerCommitSnapshotOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerCommitSnapshotLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerCommitSnapshotEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem scheduler_localSealed : snapshotStage .LocalSealed = .localSealed := rfl
@[simp] theorem scheduler_snapshot_sandbox (value : SandboxControllerRust.model.Snapshot) :
    (snapshot value).sandbox = sandboxId value.sandbox := rfl
@[simp] theorem scheduler_snapshot_generation (value : SandboxControllerRust.model.Snapshot) :
    (snapshot value).sourceGeneration = u64ToNat value.source_generation := rfl
@[simp] theorem scheduler_snapshot_workspace (value : SandboxControllerRust.model.Snapshot) :
    (snapshot value).workspace = fsId value.workspace := rfl
@[simp] theorem scheduler_snapshot_stage (value : SandboxControllerRust.model.Snapshot) :
    (snapshot value).stage = snapshotStage value.stage := rfl
@[simp] theorem scheduler_snapshot_manifest (value : SandboxControllerRust.model.Snapshot) :
    (snapshot value).manifest = snapshotManifestRef value.manifest := rfl

macro "commit_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_vm, publication_pausing,
    publication_record_epoch, publication_record_generation, publication_record_workspace,
    scheduler_snapshot_sandbox, scheduler_snapshot_generation, scheduler_snapshot_workspace,
    scheduler_snapshot_stage, scheduler_snapshot_manifest,
    scheduler_localSealed, Option.map_some, Option.map_none, reduceCtorEq,
    sandboxId_eq_iff, fsId_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true,
    true_and, and_true, false_and, and_false]))
macro "commit_model_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | commit_guard)
    | (rw [dif_neg]; rotate_left; solve | commit_guard)))
macro "finish_commit" up:ident snap:ident found:ident current:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, bind_tc_ok, scheduler_localSealed, publication_pausing]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handleScheduleCommitSnapshot
  rw [dif_pos $up, publication_snapshot_some $snap, publication_sandbox_some $found]
  commit_model_steps
  try erw [publication_vmId_some $current]
  try erw [publication_vmId_none $current]
  commit_model_steps
  first | exact ⟨rfl, by assumption⟩ | rfl))

theorem handleScheduleCommitSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (snapshotId : SandboxControllerRust.ids.SnapshotId) :
    ∃ result, SandboxControllerRust.scheduler.handle_schedule_commit_snapshot rust sid snapshotId = ok result ∧
      SchedulerResultRep model result
        (Controller.handleScheduleCommitSnapshot model (sandboxId sid) (RustRefinement.snapshotId snapshotId)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have getSnapshot := generated_get_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
    snapshotId_partialEq_lawful rust.snapshots snapshotId
  have snapshotLookup := storeRep_lookup_map rep.snapshots snapshotId
  have allEffects := fun cmd => generated_effectSucceeded_refines rust model rep cmd
  unfold SandboxControllerRust.scheduler.handle_schedule_commit_snapshot
  simp_rw [getSandbox, getSnapshot, allEffects, world_clone_eq]
  simp only [bind_tc_ok, command]
  simp_rw [generated_phase_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    publication_snapshotStage_eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handleScheduleCommitSnapshot
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases snapFound : storeLookup rust.snapshots snapshotId with
  | none =>
    have modelSnap := snapshotLookup.trans (congrArg (Option.map snapshot) snapFound)
    refine ⟨.Err .SnapshotNotFound, rfl, ?_⟩
    unfold Controller.handleScheduleCommitSnapshot
    rw [dif_pos modelUp, publication_snapshot_none modelSnap]
    rfl
  | some snap =>
    have modelSnap := snapshotLookup.trans (congrArg (Option.map snapshot) snapFound)
    cases found : storeLookup rust.sandboxes sid with
    | none =>
      have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
      refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
      unfold Controller.handleScheduleCommitSnapshot
      rw [dif_pos modelUp, publication_snapshot_some modelSnap, publication_sandbox_none modelFound]
      rfl
    | some record =>
      have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
      obtain ⟨next, nextEval, nextRep, _⟩ :=
        generated_enqueue_refines rust model rep capacity
          (.CommitSnapshot sid record.epoch record.generation snapshotId snap.manifest)
      by_cases owner : snap.sandbox = sid
      case' pos =>
        by_cases pausing : phase record.phase = .pausing
        case' pos =>
          cases current : record.current_vm
          all_goals have modelCurrent := congrArg (Option.map vmId) current
          case' some vid =>
            by_cases generation : snap.source_generation = record.generation
            case' pos =>
              by_cases workspace : snap.workspace = record.workspace
              case' pos =>
                by_cases sealed : snapshotStage snap.stage = .localSealed
                case' pos =>
                  cases succeeded : effectSucceededCheck model
                    (.sealSnapshot (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation)
                      (vmId vid) (fsId record.workspace) (RustRefinement.snapshotId snapshotId) (snapshotManifestRef snap.manifest))
      all_goals (focus finish_commit modelUp modelSnap modelFound modelCurrent)

end SandboxModel.RustRefinement
