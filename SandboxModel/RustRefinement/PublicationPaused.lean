import SandboxModel.RustRefinement.PublicationPausedChecks

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
local instance pausedOpDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance pausedLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance pausedEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance pausedManifestDecEq : DecidableEq SandboxControllerRust.model.SnapshotManifestRef := by
  intro left right
  if same : snapshotManifestRef left = snapshotManifestRef right then
    exact isTrue (snapshotManifestRef_injective same)
  else
    exact isFalse (fun equal => same (congrArg snapshotManifestRef equal))

macro "simplify_paused" validity:ident : tactic => `(tactic| (
  try dsimp only
  try simp only [*, -$validity, bind_tc_ok, lifecycle_record_phase, lifecycle_record_owner,
    publication_record_epoch, publication_record_generation, publication_record_workspace,
    lifecycle_record_vm, lifecycle_record_operation, publication_pausing, publication_durable,
    operation_id_field, compensation_ledger_sandbox, compensation_ledger_operation,
    compensation_ledger_result, restoreSource, snapshot, Option.map_some, Option.map_none,
    tenantId_injective.eq_iff, sandboxId_eq_iff, fsId_eq_iff, u64ToNat_eq_iff, snapshotManifestRef_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]
  try simp
  try rfl))

macro "finish_paused" validity:ident up:ident found:ident snap:ident vid:ident current:ident ledger:ident : tactic => `(tactic| (
  simplify_paused $validity
  all_goals unfold Controller.handlePublishPaused
  all_goals rw [dif_pos $up]
  all_goals try rw [publication_sandbox_some $found]
  all_goals try rw [publication_sandbox_none $found]
  all_goals simplify_paused $validity
  all_goals try rw [publication_snapshot_some $snap]
  all_goals try rw [publication_snapshot_none $snap]
  all_goals simplify_paused $validity
  all_goals try rw [publication_vmId_some $vid]
  all_goals try rw [publication_vmId_none $vid]
  all_goals simplify_paused $validity
  all_goals try rw [publication_operation_some $current]
  all_goals try rw [publication_operation_none $current]
  all_goals simplify_paused $validity
  all_goals try rw [publication_ledger_some $ledger]
  all_goals try rw [publication_ledger_none $ledger]
  all_goals simplify_paused $validity
  all_goals try exact ⟨rfl, by assumption⟩
  all_goals try rfl))

theorem handlePublishPaused_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : PublicationWorldRep rust model)
    (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId) (source : SandboxControllerRust.model.RestoreSource) :
    ∃ result, SandboxControllerRust.publication.handle_publish_paused rust sid source = ok result ∧
      PublicationResultRep model result (Controller.handlePublishPaused model valid (sandboxId sid) (restoreSource source)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have getSnapshot := generated_get_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
    snapshotId_partialEq_lawful rust.snapshots source.snapshot
  have snapshotLookup := storeRep_lookup_map rep.snapshots source.snapshot
  have allFences := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.worker_fences sid
  have allOperations := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operations key
  have allEffects := fun cmd => generated_effectSucceeded_refines rust model rep cmd
  have vmsCheck := generated_indexedVmsClosed_refines rust model rep sid
  have slotsCheck := generated_indexedSlotsClosed_refines rust model rep sid
  unfold SandboxControllerRust.publication.handle_publish_paused
  simp_rw [getSandbox, getSnapshot, allFences, allOperations, allEffects, vmsCheck, slotsCheck]
  simp only [bind_tc_ok, command]
  simp_rw [publication_fence_store_ne rust model rep, generated_phase_ne, generated_operation_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
    snapshotManifestRef_eq_lawful, publication_snapshotStage_eq,
    SandboxControllerRust.transition.operation_key, core.option.Option.is_some, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · simp only [Controller.handlePublishPaused, modelUp, Bool.false_eq_true, dif_neg, not_false_eq_true,
        PublicationResultRep, publicationReject]
  cases found : storeLookup rust.sandboxes sid with
  | none =>
    have modelFound : model.sandboxes (sandboxId sid) = none := by simpa only [found, Option.map_none] using sandboxLookup
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handlePublishPaused
    rw [dif_pos modelUp, publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound : model.sandboxes (sandboxId sid) = some (sandboxRecord record) := by
      simpa only [found, Option.map_some] using sandboxLookup
    obtain ⟨next, nextEval, nextRep⟩ := generated_publishPaused_refines rust model rep sandboxRoom operationRoom sid record source
    by_cases pausing : phase record.phase = .pausing
    case' pos =>
      cases foundSnapshot : storeLookup rust.snapshots source.snapshot
      all_goals have modelSnapshot := snapshotLookup.trans (congrArg (Option.map snapshot) foundSnapshot)
      case' some snap =>
        by_cases owner : snap.owner = record.owner
        case' pos =>
          by_cases sandbox : snap.sandbox = sid
          case' pos =>
            by_cases generation : snap.source_generation = record.generation
            case' pos =>
              by_cases workspace : snap.workspace = record.workspace
              case' pos =>
                by_cases manifest : snap.manifest = source.manifest
                case' pos =>
                  by_cases durable : snapshotStage snap.stage = .durable
                  case' pos =>
                    cases immutable : snap.immutable
                    case' true =>
                      by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
                      case' pos =>
                        cases currentVm : record.current_vm
                        all_goals have modelVm := congrArg (Option.map vmId) currentVm
                        case' some vid =>
                          cases fenceSucceeded : effectSucceededCheck model (.installFence (sandboxId sid) (u64ToNat record.epoch) false)
                          case' true =>
                            cases freezeSucceeded : effectSucceededCheck model (.freezeVm (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (vmId vid))
                            case' true =>
                              cases sealSucceeded : effectSucceededCheck model (.sealSnapshot (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (vmId vid) (fsId record.workspace) (snapshotId source.snapshot) (snapshotManifestRef source.manifest))
                              case' true =>
                                cases commitSucceeded : effectSucceededCheck model (.commitSnapshot (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (snapshotId source.snapshot) (snapshotManifestRef source.manifest))
                                case' true =>
                                  cases stopSucceeded : effectSucceededCheck model (.stopVm (sandboxId sid) (u64ToNat record.epoch) (u64ToNat record.generation) (vmId vid))
                                  case' true =>
                                    cases vmsClosed : indexedVmsClosedCheck model (sandboxId sid)
                                    case' true =>
                                      cases slotsClosed : indexedSlotsClosedCheck model (sandboxId sid)
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
    all_goals (focus finish_paused valid modelUp modelFound modelSnapshot modelVm modelCurrent modelLedger)

end SandboxModel.RustRefinement
