import SandboxModel.RustRefinement.LifecycleChecks

namespace SandboxModel.RustRefinement
set_option maxHeartbeats 4000000
open Aeneas Aeneas.Std Result
local instance lifecycleAdmissionOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance lifecycleAdmissionLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance lifecycleAdmissionEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem handleCreate_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (input : SandboxControllerRust.request.CreateRequest) :
    ∃ result, SandboxControllerRust.request.handle_create rust input = ok result ∧
      RequestResultRep model result (SandboxModel.Controller.handleCreate model (actor input.actor)
        (tenantId input.owner) (sandboxId input.sandbox) (fsId input.workspace) (filePolicyRef input.file_policy)
        (networkPolicy input.policy) (operationId input.operation)) := by
  obtain ⟨next, nextEval, nextRep⟩ := generated_beginCreate_refines rust model rep room capacity
    input.sandbox input.owner input.workspace input.file_policy input.policy input.operation
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes input.sandbox
  have getFs := generated_get_eq SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId
    fsId_partialEq_lawful rust.filesystems input.workspace
  have getOp := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operations ⟨input.sandbox, input.operation⟩
  have getPlan := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans ⟨input.sandbox, input.operation⟩
  have sandboxLookup := storeRep_lookup_map rep.sandboxes input.sandbox
  have fsLookup := storeRep_lookup_map rep.filesystems input.workspace
  have opLookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  have planLookup := storeRep_lookup_map rep.operationPlans (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  change model.operations (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at opLookup
  change model.operationPlans (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at planLookup
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  all_goals simp only [SandboxControllerRust.request.handle_create, up]
  · exact ⟨_, rfl, by simp [RequestResultRep, SandboxModel.Controller.handleCreate, modelUp, reject?]⟩
  cases s : storeLookup rust.sandboxes input.sandbox
  case' none =>
    cases f : storeLookup rust.filesystems input.workspace
    case' none =>
      cases o : storeLookup rust.operations ⟨input.sandbox, input.operation⟩
      case' none =>
        cases p : storeLookup rust.operation_plans ⟨input.sandbox, input.operation⟩
        case' none =>
          by_cases auth : SandboxModel.authorized (actor input.actor) (tenantId input.owner)
  all_goals simp [*, SandboxControllerRust.store.Store.contains_key,
    SandboxControllerRust.transition.operation_key, generated_authorized_eq,
    network_policy_clone_eq, RequestResultRep, SandboxModel.Controller.handleCreate,
    reject?, modelEvent]
  all_goals repeat' (split at * <;> simp_all [RequestResultRep, reject?, modelEvent] <;> subst_vars)
  all_goals simp_all

macro "finish_lifecycle" : tactic => `(tactic| (
  simp [*, SandboxControllerRust.request.handle_pause, SandboxModel.Controller.handlePause,
    SandboxControllerRust.request.handle_resume, SandboxModel.Controller.handleResume,
    SandboxControllerRust.request.handle_kill, SandboxModel.Controller.handleKill,
    SandboxControllerRust.store.Store.contains_key, SandboxControllerRust.transition.operation_key,
    generated_phase_eq, generated_phase_ne, generated_authorized_eq]
  all_goals repeat' (split <;> (try simp_all) <;> subst_vars)
  all_goals try simp_all [RequestResultRep, reject?, modelEvent]
  all_goals simp [lifecycle_record_vm] at *
  all_goals subst_vars
  all_goals simp_all))

theorem handlePause_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 2)
    (input : SandboxControllerRust.request.PauseRequest) :
    ∃ result, SandboxControllerRust.request.handle_pause rust input = ok result ∧
      RequestResultRep model result (SandboxModel.Controller.handlePause model (actor input.actor)
        (sandboxId input.sandbox) (operationId input.operation)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes input.sandbox
  have getOp := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operations ⟨input.sandbox, input.operation⟩
  have getPlan := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans ⟨input.sandbox, input.operation⟩
  have sandboxLookup := storeRep_lookup_map rep.sandboxes input.sandbox
  have opLookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  have planLookup := storeRep_lookup_map rep.operationPlans (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  change model.operations (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at opLookup
  change model.operationPlans (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at planLookup
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · finish_lifecycle
  cases found : storeLookup rust.sandboxes input.sandbox with
  | none => finish_lifecycle
  | some record =>
    obtain ⟨notMax, epoch, increment, epochNext⟩ := lifecycle_increment record.epoch
      (room.sandboxEpoch (input.sandbox, record) (storeLookup_some_mem found))
    by_cases running : phase record.phase = .running
    · cases terminal : record.terminal
      · cases current : record.current_vm with
        | none => finish_lifecycle
        | some vid =>
          obtain ⟨next, nextEval, nextRep⟩ := generated_beginPause_refines rust model rep room capacity
            input.sandbox record input.operation vid epoch epochNext
          cases o : storeLookup rust.operations ⟨input.sandbox, input.operation⟩
          case' none =>
            cases p : storeLookup rust.operation_plans ⟨input.sandbox, input.operation⟩
            case' none =>
              by_cases auth : SandboxModel.authorized (actor input.actor) (tenantId record.owner)
          all_goals finish_lifecycle
      · finish_lifecycle
    · finish_lifecycle

theorem handleResume_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (input : SandboxControllerRust.request.ResumeRequest) :
    ∃ result, SandboxControllerRust.request.handle_resume rust input = ok result ∧
      RequestResultRep model result (SandboxModel.Controller.handleResume model (actor input.actor)
        (sandboxId input.sandbox) (operationId input.operation)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes input.sandbox
  have getOp := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operations ⟨input.sandbox, input.operation⟩
  have getPlan := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans ⟨input.sandbox, input.operation⟩
  have sandboxLookup := storeRep_lookup_map rep.sandboxes input.sandbox
  have opLookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  have planLookup := storeRep_lookup_map rep.operationPlans (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  change model.operations (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at opLookup
  change model.operationPlans (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at planLookup
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · finish_lifecycle
  cases found : storeLookup rust.sandboxes input.sandbox with
  | none => finish_lifecycle
  | some record =>
    obtain ⟨notMax, epoch, increment, epochNext⟩ := lifecycle_increment record.epoch
      (room.sandboxEpoch (input.sandbox, record) (storeLookup_some_mem found))
    obtain ⟨generationNotMax, generation, generationIncrement, generationNext⟩ := lifecycle_increment record.generation
      (room.sandboxGeneration (input.sandbox, record) (storeLookup_some_mem found))
    by_cases paused : phase record.phase = .paused
    · cases terminal : record.terminal
      · cases current : record.durable_snapshot with
        | none => finish_lifecycle
        | some source =>
          have check := generated_usableSnapshot_refines rust model rep input.sandbox record source
          obtain ⟨next, nextEval, nextRep⟩ := generated_beginResume_refines rust model rep room capacity
            input.sandbox record input.operation source epoch generation epochNext generationNext
          cases usable : SandboxModel.Controller.usableSnapshotCheck model (sandboxId input.sandbox)
            (sandboxRecord record) (restoreSource source)
          case' true =>
            cases o : storeLookup rust.operations ⟨input.sandbox, input.operation⟩
            case' none =>
              cases p : storeLookup rust.operation_plans ⟨input.sandbox, input.operation⟩
              case' none =>
                by_cases auth : SandboxModel.authorized (actor input.actor) (tenantId record.owner)
          all_goals finish_lifecycle
      · finish_lifecycle
    · finish_lifecycle

theorem handleKill_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (input : SandboxControllerRust.request.KillRequest) :
    ∃ result, SandboxControllerRust.request.handle_kill rust input = ok result ∧
      RequestResultRep model result (SandboxModel.Controller.handleKill model (actor input.actor)
        (sandboxId input.sandbox) (operationId input.operation)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes input.sandbox
  have getOp := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operations ⟨input.sandbox, input.operation⟩
  have getPlan := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans ⟨input.sandbox, input.operation⟩
  have sandboxLookup := storeRep_lookup_map rep.sandboxes input.sandbox
  have opLookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  have planLookup := storeRep_lookup_map rep.operationPlans (SandboxControllerRust.model.OperationKey.mk input.sandbox input.operation)
  change model.operations (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at opLookup
  change model.operationPlans (SandboxModel.operationKey (sandboxId input.sandbox) (operationId input.operation)) = _ at planLookup
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · finish_lifecycle
  cases found : storeLookup rust.sandboxes input.sandbox with
  | none => finish_lifecycle
  | some record =>
    obtain ⟨notMax, epoch, increment, epochNext⟩ := lifecycle_increment record.epoch
      (room.sandboxEpoch (input.sandbox, record) (storeLookup_some_mem found))
    have check := generated_supersedeReady_refines rust model rep input.sandbox record.current_operation
    obtain ⟨next, nextEval, nextRep⟩ := generated_beginKill_refines rust model rep room capacity
      input.sandbox record input.operation epoch epochNext
    by_cases destroyed : phase record.phase = .destroyed
    · finish_lifecycle
    · cases terminal : record.terminal
      · cases ready : SandboxModel.Controller.supersedeReadyCheck model (sandboxId input.sandbox)
          (record.current_operation.map operation)
        case' true =>
          cases o : storeLookup rust.operations ⟨input.sandbox, input.operation⟩
          case' none =>
            cases p : storeLookup rust.operation_plans ⟨input.sandbox, input.operation⟩
            case' none =>
              by_cases auth : SandboxModel.authorized (actor input.actor) (tenantId record.owner)
        all_goals finish_lifecycle
      · finish_lifecycle

end SandboxModel.RustRefinement
