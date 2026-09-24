import SandboxModel.RustRefinement.SchedulerPrepareChecks

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance schedulerSealSnapshotOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerSealSnapshotLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerSealSnapshotEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance schedulerSealSnapshotSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource := schedulerRestoreSourceDecEq

theorem scheduler_snapshot_fresh_some {α : Sort u} {value : Option Snapshot}
    {record : Snapshot} (found : value = some record)
    (present : (r : Snapshot) → value = some r → α) (missing : value = none → α) :
    Controller.handleReservePauseSnapshot.match_1 (fun _ => α) value present missing = present record found := by
  cases found
  rfl
theorem scheduler_snapshot_fresh_none {α : Sort u} {value : Option Snapshot}
    (found : value = none)
    (present : (r : Snapshot) → value = some r → α) (missing : value = none → α) :
    Controller.handleReservePauseSnapshot.match_1 (fun _ => α) value present missing = missing found := by
  cases found
  rfl

macro "seal_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_vm, lifecycle_record_owner, publication_pausing,
    lifecycle_record_operation, publication_record_epoch, publication_record_generation, publication_record_workspace,
    operation_id_field, scheduler_key_mk, scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation,
    scheduler_plan_sealed, scheduler_plan_vm_eq, scheduler_plan_target_eq,
    running_vm_owner, running_vm_sandbox, running_vm_generation, running_vm_status,
    scheduler_frozen, Option.map_some, Option.map_none, reduceCtorEq,
    tenantId_injective.eq_iff, sandboxId_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "seal_model_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | seal_guard)
    | (rw [dif_neg]; rotate_left; solve | seal_guard)))
macro "finish_seal" up:ident found:ident current:ident plan:ident vid:ident vmfound:ident snap:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, bind_tc_ok, scheduler_key_mk, scheduler_frozen, publication_pausing]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handleScheduleSealSnapshot
  rw [dif_pos $up, publication_sandbox_some $found]
  seal_model_steps
  try erw [publication_operation_some $current]
  try erw [publication_operation_none $current]
  try erw [scheduler_plan_some $plan]
  try erw [scheduler_plan_none $plan]
  seal_model_steps
  try erw [publication_vmId_some $vid]
  try erw [publication_vmId_none $vid]
  seal_model_steps
  try erw [publication_vm_some $vmfound]
  try erw [publication_vm_none $vmfound]
  seal_model_steps
  try erw [scheduler_snapshot_fresh_some $snap]
  try erw [scheduler_snapshot_fresh_none $snap]
  first | exact ⟨rfl, by assumption⟩ | rfl))

theorem handleScheduleSealSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (snapId : SandboxControllerRust.ids.SnapshotId)
    (manifest : SandboxControllerRust.model.SnapshotManifestRef) :
    ∃ result, SandboxControllerRust.scheduler.handle_schedule_seal_snapshot rust sid snapId manifest = ok result ∧
      SchedulerResultRep model result
        (Controller.handleScheduleSealSnapshot model (sandboxId sid) (snapshotId snapId) (snapshotManifestRef manifest)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have allVms := fun vid => generated_get_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
    vmId_partialEq_lawful rust.vms vid
  have allPlans := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans key
  have allFences := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.worker_fences sid
  have containsSnapshot := generated_contains_key_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
    snapshotId_partialEq_lawful rust.snapshots snapId
  have snapshotLookup := storeRep_lookup_map rep.snapshots snapId
  have allEffects := fun cmd => generated_effectSucceeded_refines rust model rep cmd
  have sourceEq := generated_option_eq
    SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource scheduler_restore_eq_lawful
  have vmEq := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
  unfold SandboxControllerRust.scheduler.handle_schedule_seal_snapshot
  simp_rw [getSandbox, allVms, allPlans, allFences, containsSnapshot, allEffects, world_clone_eq]
  simp only [bind_tc_ok, command]
  simp_rw [publication_fence_store_ne rust model rep, generated_phase_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    scheduler_key_eq, publication_vmStatus_eq, sourceEq, vmEq,
    SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.transition.operation_key, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handleScheduleSealSnapshot
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases found : storeLookup rust.sandboxes sid with
  | none =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handleScheduleSealSnapshot
    rw [dif_pos modelUp, publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    by_cases pausing : phase record.phase = .pausing
    case' pos =>
      cases current : record.current_operation
      all_goals have modelCurrent := congrArg (Option.map operation) current
      case' some op =>
        have planLookup := storeRep_lookup_map rep.operationPlans (SandboxControllerRust.model.OperationKey.mk sid op.id)
        change model.operationPlans (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at planLookup
        cases planFound : storeLookup rust.operation_plans ⟨sid, op.id⟩
        all_goals have modelPlan := planLookup.trans (congrArg (Option.map operationPlan) planFound)
        case' some plan =>
          by_cases sameKey : operationKey plan.key = SandboxModel.operationKey (sandboxId sid) (operationId op.id)
          case' pos =>
            by_cases sameEpoch : plan.epoch = record.epoch
            case' pos =>
              by_cases sameGeneration : plan.generation = record.generation
              case' pos =>
                cases sealed : plan.sealed
                case' true =>
                  by_cases target : plan.target_snapshot = some ⟨snapId, manifest⟩
                  case' pos =>
                    cases currentVm : record.current_vm
                    all_goals have modelVid := congrArg (Option.map vmId) currentVm
                    case' some vid =>
                      obtain ⟨next, nextEval, nextRep, _⟩ :=
                        generated_enqueue_refines rust model rep capacity
                          (.SealSnapshot sid record.epoch record.generation vid record.workspace snapId manifest)
                      by_cases plannedVm : plan.vm = some vid
                      case' pos =>
                        have vmLookup := storeRep_lookup_map rep.vms vid
                        cases vmFound : storeLookup rust.vms vid
                        all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
                        case' some machine =>
                          by_cases owner : machine.owner = record.owner
                          case' pos =>
                            by_cases sandbox : machine.sandbox = sid
                            case' pos =>
                              by_cases generation : machine.generation = record.generation
                              case' pos =>
                                by_cases frozen : vmStatus machine.status = .frozen
                                case' pos =>
                                  by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
                                  case' pos =>
                                    cases succeeded : effectSucceededCheck model (.freezeVm (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (vmId vid))
                                    case' true =>
                                      cases snapFound : storeLookup rust.snapshots snapId
                                      all_goals have modelSnap := snapshotLookup.trans (congrArg (Option.map snapshot) snapFound)
    all_goals (focus finish_seal modelUp modelFound modelCurrent modelPlan modelVid modelVm modelSnap)

end SandboxModel.RustRefinement
