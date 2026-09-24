import SandboxModel.RustRefinement.SchedulerReservePauseUpdate

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance schedulerReservePauseOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerReservePauseLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerReservePauseEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem scheduler_key_sandbox (key : SandboxControllerRust.model.OperationKey) :
    (operationKey key).sandbox = sandboxId key.sandbox := rfl
@[simp] theorem scheduler_key_operation (key : SandboxControllerRust.model.OperationKey) :
    (operationKey key).operation = operationId key.operation := rfl

macro "reserve_pause_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_pausing, lifecycle_record_vm,
    lifecycle_record_operation, publication_record_epoch, publication_record_generation,
    operation_id_field, scheduler_key_sandbox, scheduler_key_operation, operationId_injective.eq_iff,
    scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation,
    scheduler_plan_sealed, scheduler_plan_vm, scheduler_plan_target,
    Option.map_eq_none_iff, Option.map_some, Option.map_none, reduceCtorEq, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "reserve_pause_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | reserve_pause_guard)
    | (rw [dif_neg]; rotate_left; solve | reserve_pause_guard)))
macro "finish_reserve_pause" up:ident found:ident current:ident plan:ident vid:ident snap:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, bind_tc_ok, publication_pausing, core.option.Option.is_some]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handleReservePauseSnapshot
  rw [dif_pos $up]
  erw [publication_sandbox_some $found]
  reserve_pause_steps
  try erw [publication_operation_some $current]
  try erw [publication_operation_none $current]
  reserve_pause_steps
  try erw [scheduler_plan_some $plan]
  try erw [scheduler_plan_none $plan]
  reserve_pause_steps
  try erw [publication_vmId_some $vid]
  try erw [publication_vmId_none $vid]
  try erw [scheduler_snapshot_fresh_some $snap]
  try erw [scheduler_snapshot_fresh_none $snap]
  first | exact ⟨by simp [modelEvent, *], by assumption⟩ | rfl))

theorem handleReservePauseSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (snapshotRoom : U64Headroom rust.next_snapshot_id)
    (manifestRoom : U64Headroom rust.next_manifest_digest)
    (planRoom : StoreHeadroom rust.operation_plans)
    (key : SandboxControllerRust.model.OperationKey) :
    ∃ result, SandboxControllerRust.scheduler.handle_reserve_pause_snapshot rust key = ok result ∧
      SchedulerResultRep model result (Controller.handleReservePauseSnapshot model (operationKey key)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes key.sandbox
  have sandboxLookup := storeRep_lookup_map rep.sandboxes key.sandbox
  have getPlan := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans key
  have planLookup := storeRep_lookup_map rep.operationPlans key
  have containsSnapshot := generated_contains_key_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
    snapshotId_partialEq_lawful rust.snapshots rust.next_snapshot_id
  have snapshotLookup := storeRep_lookup_map rep.snapshots rust.next_snapshot_id
  have cursor : snapshotId rust.next_snapshot_id = ⟨model.nextSnapshotId⟩ := by
    simp [snapshotId, rep.nextSnapshotId]
  have manifestCursor : snapshotManifestRef ⟨rust.next_manifest_digest⟩ = ⟨⟨model.nextManifestDigest⟩⟩ := by
    simp [snapshotManifestRef, manifestDigest, rep.nextManifestDigest]
  rw [cursor] at snapshotLookup
  unfold SandboxControllerRust.scheduler.handle_reserve_pause_snapshot
  simp_rw [getSandbox, getPlan, containsSnapshot, generated_phase_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    scheduler_key_eq, SandboxControllerRust.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handleReservePauseSnapshot
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases found : storeLookup rust.sandboxes key.sandbox with
  | none =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handleReservePauseSnapshot
    rw [dif_pos modelUp]
    erw [publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    by_cases pausing : phase record.phase = .pausing
    case' pos =>
      cases terminal : record.terminal
      case' false =>
        cases current : record.current_operation
        all_goals have modelCurrent := congrArg (Option.map operation) current
        case' some op =>
          by_cases currentId : op.id = key.operation
          case' pos =>
            cases planFound : storeLookup rust.operation_plans key
            all_goals have modelPlan := planLookup.trans (congrArg (Option.map operationPlan) planFound)
            case' some plan =>
              by_cases sameKey : operationKey plan.key = operationKey key
              case' pos =>
                by_cases sameEpoch : plan.epoch = record.epoch
                case' pos =>
                  by_cases sameGeneration : plan.generation = record.generation
                  case' pos =>
                    cases sealed : plan.sealed
                    case' false =>
                      cases planVm : plan.vm
                      case' none =>
                        cases target : plan.target_snapshot
                        case' none =>
                          cases currentVm : record.current_vm
                          all_goals have modelVid := congrArg (Option.map vmId) currentVm
                          case' some vid =>
                            obtain ⟨next, nextEval, nextRep⟩ :=
                              generated_reservePauseSnapshot_refines rust model rep snapshotRoom manifestRoom planRoom key plan vid
                            cases snapFound : storeLookup rust.snapshots rust.next_snapshot_id
                            all_goals have modelSnap := snapshotLookup.trans (congrArg (Option.map snapshot) snapFound)
    all_goals (focus finish_reserve_pause modelUp modelFound modelCurrent modelPlan modelVid modelSnap)

end SandboxModel.RustRefinement
