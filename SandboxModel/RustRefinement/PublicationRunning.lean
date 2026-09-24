import SandboxModel.RustRefinement.PublicationRunningChecks

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance : DecidableEq SandboxControllerRust.model.NetworkPolicy := by
  intro left right
  if same : networkPolicy left = networkPolicy right then
    exact isTrue (networkPolicy_injective same)
  else exact isFalse (fun equal => same (congrArg networkPolicy equal))

@[simp] theorem running_vm_owner (value : SandboxControllerRust.model.Vm) : (vm value).owner = tenantId value.owner := rfl
@[simp] theorem running_vm_sandbox (value : SandboxControllerRust.model.Vm) : (vm value).sandbox = sandboxId value.sandbox := rfl
@[simp] theorem running_vm_generation (value : SandboxControllerRust.model.Vm) : (vm value).generation = u64ToNat value.generation := rfl
@[simp] theorem running_vm_status (value : SandboxControllerRust.model.Vm) : (vm value).status = vmStatus value.status := rfl
@[simp] theorem running_slot_owner (value : SandboxControllerRust.model.NetSlot) : (netSlot value).owner = tenantId value.owner := rfl
@[simp] theorem running_slot_sandbox (value : SandboxControllerRust.model.NetSlot) : (netSlot value).sandbox = sandboxId value.sandbox := rfl
@[simp] theorem running_slot_generation (value : SandboxControllerRust.model.NetSlot) : (netSlot value).generation = u64ToNat value.generation := rfl
@[simp] theorem running_slot_vm (value : SandboxControllerRust.model.NetSlot) : (netSlot value).vm = vmId value.vm := rfl
@[simp] theorem running_slot_policy (value : SandboxControllerRust.model.NetSlot) : (netSlot value).policy = networkPolicy value.policy := rfl
@[simp] theorem running_slot_configured (value : SandboxControllerRust.model.NetSlot) : (netSlot value).configured = value.configured := rfl
@[simp] theorem running_slot_active (value : SandboxControllerRust.model.NetSlot) : (netSlot value).active = value.active := rfl

macro "running_guard" validity:ident : tactic => `(tactic| (simp only [*, -$validity,  bind_tc_ok, lifecycle_record_phase, lifecycle_record_owner,
    lifecycle_record_terminal, publication_record_epoch, publication_record_generation, publication_record_workspace,
    lifecycle_record_vm, lifecycle_record_operation, publication_starting, publication_executing,
    publication_record_slot, publication_record_policy, publication_record_filePolicy, publication_record_restore,
    operation_id_field, compensation_ledger_sandbox, compensation_ledger_operation,
    compensation_ledger_result, running_vm_owner, running_vm_sandbox, running_vm_generation, running_vm_status,
    running_slot_owner, running_slot_sandbox, running_slot_generation, running_slot_vm, running_slot_policy,
    running_slot_configured, running_slot_active, Option.map_some, Option.map_none, reduceCtorEq,
    tenantId_injective.eq_iff, sandboxId_eq_iff, vmId_injective.eq_iff, u64ToNat_eq_iff, networkPolicy_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "running_model_steps" validity:ident : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | running_guard $validity)
    | (rw [dif_neg]; rotate_left; solve | running_guard $validity)))
macro "finish_running" validity:ident up:ident found:ident vid:ident vmfound:ident slotid:ident slotfound:ident current:ident ledger:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, -$validity,  bind_tc_ok, lifecycle_record_phase, lifecycle_record_owner,
    lifecycle_record_terminal, publication_record_epoch, publication_record_generation, publication_record_workspace,
    lifecycle_record_vm, lifecycle_record_operation, publication_starting, publication_executing,
    publication_record_slot, publication_record_policy, publication_record_filePolicy, publication_record_restore,
    operation_id_field, compensation_ledger_sandbox, compensation_ledger_operation,
    compensation_ledger_result, running_vm_owner, running_vm_sandbox, running_vm_generation, running_vm_status,
    running_slot_owner, running_slot_sandbox, running_slot_generation, running_slot_vm, running_slot_policy,
    running_slot_configured, running_slot_active, Option.map_some, Option.map_none,
    tenantId_injective.eq_iff, sandboxId_eq_iff, vmId_injective.eq_iff, u64ToNat_eq_iff, networkPolicy_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handlePublishRunning
  rw [dif_pos $up, publication_sandbox_some $found]
  running_model_steps $validity
  try rw [lifecycle_record_vm]
  try erw [publication_vmId_some $vid]
  try erw [publication_vmId_none $vid]
  try erw [publication_vm_some $vmfound]
  try erw [publication_vm_none $vmfound]
  running_model_steps $validity
  try rw [publication_record_slot]
  try erw [publication_slotId_some $slotid]
  try erw [publication_slotId_none $slotid]
  try erw [publication_slot_some $slotfound]
  try erw [publication_slot_none $slotfound]
  running_model_steps $validity
  try rw [lifecycle_record_operation]
  try erw [publication_operation_some $current]
  try erw [publication_operation_none $current]
  try erw [publication_ledger_some $ledger]
  try erw [publication_ledger_none $ledger]
  running_model_steps $validity
  first | exact ⟨rfl, by assumption⟩ | rfl))

theorem handlePublishRunning_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : PublicationWorldRep rust model)
    (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (slotRoom : StoreHeadroom rust.net_slots)
    (sid : SandboxControllerRust.ids.SandboxId) :
    ∃ result, SandboxControllerRust.publication.handle_publish_running rust sid = ok result ∧
      PublicationResultRep model result (Controller.handlePublishRunning model valid (sandboxId sid)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have allVms := fun id => generated_get_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
    vmId_partialEq_lawful rust.vms id
  have allSlots := fun id => generated_get_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
    netSlotId_partialEq_lawful rust.net_slots id
  have allFences := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.worker_fences sid
  have allPolicies := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.applied_file_policies sid
  have allOperations := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operations key
  have allEffects := fun cmd => generated_effectSucceeded_refines rust model rep cmd
  have slotsCheck := generated_indexedSlotsClosed_refines rust model rep sid
  unfold SandboxControllerRust.publication.handle_publish_running
  simp_rw [getSandbox, allVms, allSlots, allFences, allPolicies, allOperations, network_policy_clone_eq, allEffects, slotsCheck]
  simp only [bind_tc_ok, command]
  simp_rw [publication_fence_store_ne rust model rep, publication_applied_store_ne rust model rep,
    generated_phase_ne, generated_operation_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
    publication_vmStatus_eq, networkPolicy_eq_lawful,
    SandboxControllerRust.transition.operation_key, core.option.Option.is_some, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handlePublishRunning
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases found : storeLookup rust.sandboxes sid with
  | none =>
    have modelFound : model.sandboxes (sandboxId sid) = none := by simpa only [found, Option.map_none] using sandboxLookup
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handlePublishRunning
    rw [dif_pos modelUp, publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound : model.sandboxes (sandboxId sid) = some (sandboxRecord record) := by
      simpa only [found, Option.map_some] using sandboxLookup
    by_cases starting : phase record.phase = .starting
    case' pos =>
      cases terminal : record.terminal
      case' false =>
        cases currentVm : record.current_vm
        all_goals have modelVmId := congrArg (Option.map vmId) currentVm
        case' some vid =>
          have vmLookup := storeRep_lookup_map rep.vms vid
          cases foundVm : storeLookup rust.vms vid
          all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) foundVm)
          case' some machine =>
            by_cases vmOwner : machine.owner = record.owner
            case' pos =>
              by_cases vmSandbox : machine.sandbox = sid
              case' pos =>
                by_cases vmGeneration : machine.generation = record.generation
                case' pos =>
                  by_cases executing : vmStatus machine.status = .executing
                  case' pos =>
                    cases currentSlot : record.net_slot
                    all_goals have modelSlotId := congrArg (Option.map netSlotId) currentSlot
                    case' some slotId =>
                      have slotLookup := storeRep_lookup_map rep.netSlots slotId
                      cases foundSlot : storeLookup rust.net_slots slotId
                      all_goals have modelSlot := slotLookup.trans (congrArg (Option.map netSlot) foundSlot)
                      case' some slot =>
                        have indexed := valid.2.2.2.1 (sandboxId slot.sandbox) (netSlotId slotId) (netSlot slot) modelSlot rfl
                        obtain ⟨next, nextEval, nextRep⟩ := generated_publishRunning_refines rust model rep sandboxRoom operationRoom slotRoom sid record slotId slot indexed
                        by_cases slotOwner : slot.owner = record.owner
                        case' pos =>
                          by_cases slotSandbox : slot.sandbox = sid
                          case' pos =>
                            by_cases slotGeneration : slot.generation = record.generation
                            case' pos =>
                              by_cases slotVm : slot.vm = vid
                              case' pos =>
                                by_cases policy : slot.policy = record.policy
                                case' pos =>
                                  cases configured : slot.configured
                                  case' true =>
                                    cases active : slot.active
                                    case' false =>
                                      cases slotsClosed : indexedSlotsClosedCheck model (sandboxId sid)
                                      case' true =>
                                        cases applied : Controller.appliedPolicyCheck model (sandboxId sid) (u64ToNat record.generation) (fsId record.workspace) (filePolicyRef record.file_policy)
                                        case' true =>
                                          cases fenceSucceeded : effectSucceededCheck model (.installFence (sandboxId sid) (u64ToNat record.epoch) false)
                                          case' true =>
                                            cases prepareSucceeded : effectSucceededCheck model (.prepareVm (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (vmId vid) (netSlotId slotId) (record.restore_from.map restoreSource))
                                            case' true =>
                                              cases networkSucceeded : effectSucceededCheck model (.configureNetwork (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (vmId vid) (netSlotId slotId) (networkPolicy record.policy))
                                              case' true =>
                                                cases activateSucceeded : effectSucceededCheck model (.activateVm (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (vmId vid))
                                                case' true =>
                                                  cases fileSucceeded : effectSucceededCheck model (.applyFilePolicy (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (fsId record.workspace) (filePolicyRef record.file_policy))
                                                  case' true =>
                                                    cases current : record.current_operation
                                                    all_goals have modelCurrent := congrArg (Option.map operation) current
                                                    case' some op =>
                                                      have ledgerLookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk sid op.id)
                                                      change model.operations (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at ledgerLookup
                                                      cases ledgerFound : storeLookup rust.operations ⟨sid, op.id⟩
                                                      all_goals have modelLedger := ledgerLookup.trans (congrArg (Option.map operationLedger) ledgerFound)
                                                      case' some ledger =>
                                                        by_cases sameSid : ledger.sandbox = sid
                                                        case' pos =>
                                                          by_cases sameOp : operation ledger.operation = operation op
                                                          case' pos =>
                                                            cases finalResult : ledger.result
                                                            case' none =>
                                                              by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
    all_goals (focus finish_running valid modelUp modelFound modelVmId modelVm modelSlotId modelSlot modelCurrent modelLedger)

end SandboxModel.RustRefinement
