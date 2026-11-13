import SandboxModel.Lifecycle.PausePublication.Guards
import SandboxModel.Lifecycle.PausePublication.Resources
import SandboxModel.Lifecycle.CaptureHandoff.CallerCurrent

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

@[simp] theorem option_vm_eq (left right : Option ids.VmId) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq
      ids.VmId.Insts.CoreCmpPartialEqVmId left right = .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq,
      ids.VmId.Insts.CoreCmpPartialEqVmId.eq]

theorem publish_success_checks (world : model.World) (sandbox : ids.SandboxId)
    (source : model.RestoreSource) (decision : publication.PublicationDecision)
    (evaluated : publication.handle_publish_paused world sandbox source = .ok (.Ok decision)) :
    ∃ record snapshot vm,
      world.controller_up = true ∧
      tableLookup sandbox world.sandboxes.entries.val = some record ∧
      record.phase = .Pausing ∧ record.terminal = false ∧
      tableLookup source.snapshot world.snapshots.entries.val = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sandbox ∧
      snapshot.source_generation = record.generation ∧ snapshot.workspace = record.workspace ∧
      snapshot.manifest = source.manifest ∧ snapshot.stage = .Durable ∧ snapshot.immutable = true ∧
      tableLookup sandbox world.worker_fences.entries.val = some ⟨record.epoch, false⟩ ∧
      record.current_vm = some vm ∧
      effect.effect_succeeded_check world (.InstallFence sandbox record.epoch false) = .ok true ∧
      effect.effect_succeeded_check world (.FreezeVm sandbox record.epoch record.generation vm) = .ok true ∧
      effect.effect_succeeded_check world
        (.SealSnapshot sandbox record.epoch record.generation vm record.workspace source.snapshot source.manifest) = .ok true ∧
      effect.effect_succeeded_check world
        (.CommitSnapshot sandbox record.epoch record.generation source.snapshot source.manifest) = .ok true ∧
      effect.effect_succeeded_check world (.StopVm sandbox record.epoch record.generation vm) = .ok true ∧
      ∃ operation ledger plan handoff,
        record.current_operation = some operation ∧
        tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val = some ledger ∧
        ledger.sandbox = sandbox ∧ ledger.operation = operation ∧ ledger.result = none ∧
        tableLookup ⟨sandbox, operation.id⟩ world.operation_plans.entries.val = some plan ∧
        plan.target_snapshot = some source ∧ plan.vm = some vm ∧
        currentCaptureView world sandbox record = some handoff ∧
        operation.kind = .Pause ∧ operation.epoch = record.epoch ∧
        plan.key = ⟨sandbox, operation.id⟩ ∧ plan.epoch = record.epoch ∧
        plan.generation = record.generation ∧ plan.capture = some handoff ∧
        handoff.commit = .Registered ∧ handoff.eligibility = .Recoverable ∧ handoff.unload_source = true ∧
        publication.indexed_vms_closed world sandbox = .ok true ∧
        publication.indexed_slots_closed world sandbox = .ok true ∧
        publication.publish_paused world sandbox record source = .ok decision.next ∧
        decision.event = .PublishPaused sandbox source.snapshot := by
  cases up : world.controller_up with
  | false => simp [publication.handle_publish_paused, up] at evaluated
  | true =>
    simp only [publication.handle_publish_paused, up, if_true,
      get_exact _ caller_index_sandbox_eq_lawful,
      get_exact _ snapshot_id_lawful, bind_tc_ok] at evaluated
    cases stored : tableLookup sandbox world.sandboxes.entries.val with
    | none => simp [stored] at evaluated
    | some record =>
      simp only [stored] at evaluated
      by_cases phase : record.phase = .Pausing
      ·
        have checked : core.cmp.PartialEq.ne.trait_default model.Phase.Insts.CoreCmpPartialEqPhase record.phase .Pausing = .ok false := by
          simp [phase, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
        rw [checked] at evaluated
        simp only [bind_tc_ok, Bool.false_eq_true, if_false] at evaluated
        by_cases terminal : record.terminal = false
        ·
          have checked : record.terminal = false := by
            simp [terminal]
          rw [checked] at evaluated
          simp only [Bool.false_eq_true, if_false] at evaluated
          cases snapshotStored : tableLookup source.snapshot world.snapshots.entries.val with
          | none => simp only [snapshotStored, Result.ok.injEq, reduceCtorEq] at evaluated
          | some snapshot =>
            simp only [snapshotStored] at evaluated
            by_cases owner : snapshot.owner = record.owner
            ·
              have checked : core.cmp.PartialEq.ne.trait_default ids.TenantId.Insts.CoreCmpPartialEqTenantId snapshot.owner record.owner = .ok false := by
                simp [owner, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq]
              rw [checked] at evaluated
              simp only [bind_tc_ok, Bool.false_eq_true, if_false] at evaluated
              by_cases session : snapshot.sandbox = sandbox
              ·
                have checked : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId snapshot.sandbox sandbox = .ok false := by
                  simp [session, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
                rw [checked] at evaluated
                simp only [bind_tc_ok, Bool.false_eq_true, if_false] at evaluated
                by_cases generation : snapshot.source_generation = record.generation
                ·
                  have checked : (snapshot.source_generation != record.generation) = false := by
                    simp [generation]
                  rw [checked] at evaluated
                  simp only [Bool.false_eq_true, if_false] at evaluated
                  by_cases workspace : snapshot.workspace = record.workspace
                  ·
                    have checked : core.cmp.PartialEq.ne.trait_default ids.FsId.Insts.CoreCmpPartialEqFsId snapshot.workspace record.workspace = .ok false := by
                      simp [workspace, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.FsId.Insts.CoreCmpPartialEqFsId.eq]
                    rw [checked] at evaluated
                    simp only [bind_tc_ok, Bool.false_eq_true, if_false] at evaluated
                    by_cases manifest : snapshot.manifest = source.manifest
                    ·
                      have checked : core.cmp.PartialEq.ne.trait_default model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef snapshot.manifest source.manifest = .ok false := by
                        simp [manifest, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                      rw [checked] at evaluated
                      simp only [bind_tc_ok, Bool.false_eq_true, if_false] at evaluated
                      by_cases stage : snapshot.stage = .Durable
                      ·
                        have checked : core.cmp.PartialEq.ne.trait_default model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage snapshot.stage .Durable = .ok false := by
                          simp [stage, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                        rw [checked] at evaluated
                        simp only [bind_tc_ok, Bool.false_eq_true, if_false] at evaluated
                        by_cases immutable : snapshot.immutable = true
                        ·
                          have checked : snapshot.immutable = true := by
                            simp [immutable]
                          rw [checked] at evaluated
                          simp only [if_true] at evaluated
                          by_cases fence : tableLookup sandbox world.worker_fences.entries.val = some ⟨record.epoch, false⟩
                          ·
                            have checked : core.cmp.PartialEq.ne.trait_default (core.option.Option.Insts.CoreCmpPartialEqOption (core.cmp.PartialEqShared model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence)) (tableLookup sandbox world.worker_fences.entries.val) (some ⟨record.epoch, false⟩) = .ok false := by
                              simp [fence, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                            rw [checked] at evaluated
                            simp only [bind_tc_ok, Bool.false_eq_true, if_false] at evaluated
                            cases currentVm : record.current_vm with
                            | none => simp [currentVm] at evaluated
                            | some vm =>
                              simp only [currentVm] at evaluated
                              obtain ⟨fenceOk, fenceRun, tail⟩ := bind_ok_inv _ _ _ evaluated
                              cases fenceOk with
                              | false => simp at tail
                              | true =>
                                simp only [if_true] at tail
                                obtain ⟨freezeOk, freezeRun, tail⟩ := bind_ok_inv _ _ _ tail
                                cases freezeOk with
                                | false => simp at tail
                                | true =>
                                  simp only [if_true] at tail
                                  obtain ⟨sealOk, sealRun, tail⟩ := bind_ok_inv _ _ _ tail
                                  cases sealOk with
                                  | false => simp at tail
                                  | true =>
                                    simp only [if_true] at tail
                                    obtain ⟨commitOk, commitRun, tail⟩ := bind_ok_inv _ _ _ tail
                                    cases commitOk with
                                    | false => simp at tail
                                    | true =>
                                      simp only [if_true] at tail
                                      obtain ⟨stopOk, stopRun, tail⟩ := bind_ok_inv _ _ _ tail
                                      cases stopOk with
                                      | false => simp at tail
                                      | true =>
                                        simp only [if_true] at tail
                                        obtain ⟨vmsOk, vmsRun, tail⟩ := bind_ok_inv _ _ _ tail
                                        cases vmsOk with
                                        | false => simp at tail
                                        | true =>
                                          simp only [if_true] at tail
                                          obtain ⟨slotsOk, slotsRun, tail⟩ := bind_ok_inv _ _ _ tail
                                          cases slotsOk with
                                          | false => simp at tail
                                          | true =>
                                            simp only [if_true] at tail
                                            cases currentOp : record.current_operation with
                                            | none => simp [currentOp] at tail
                                            | some operation =>
                                              simp only [currentOp, transition.operation_key, bind_tc_ok,
                                                get_exact _ operation_key_lawful] at tail
                                              cases ledgerLookup : tableLookup ⟨sandbox, operation.id⟩ world.operations.entries.val with
                                              | none => simp [ledgerLookup] at tail
                                              | some ledger =>
                                                simp only [ledgerLookup] at tail
                                                by_cases ledgerSandbox : ledger.sandbox = sandbox
                                                ·
                                                  have checked : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId ledger.sandbox sandbox = .ok false := by
                                                    simp [ledgerSandbox, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
                                                  rw [checked] at tail
                                                  simp only [bind_tc_ok, Bool.false_eq_true, if_false] at tail
                                                  by_cases ledgerOperation : ledger.operation = operation
                                                  ·
                                                    have checked : core.cmp.PartialEq.ne.trait_default model.Operation.Insts.CoreCmpPartialEqOperation ledger.operation operation = .ok false := by
                                                      simp [ledgerOperation, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                                                    rw [checked] at tail
                                                    simp only [bind_tc_ok, Bool.false_eq_true, if_false] at tail
                                                    by_cases noResult : ledger.result = none
                                                    ·
                                                      have checked : core.option.Option.is_some ledger.result = false := by
                                                        simp [noResult, core.option.Option.is_some]
                                                      rw [checked] at tail
                                                      simp only [Bool.false_eq_true, if_false] at tail
                                                      cases planLookup : tableLookup ⟨sandbox, operation.id⟩ world.operation_plans.entries.val with
                                                      | none => simp [planLookup] at tail
                                                      | some plan =>
                                                        simp only [planLookup] at tail
                                                        by_cases target : plan.target_snapshot = some source
                                                        ·
                                                          have checked : core.cmp.PartialEq.ne.trait_default (core.option.Option.Insts.CoreCmpPartialEqOption model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource) plan.target_snapshot (some source) = .ok false := by
                                                            simp [target, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                                                          rw [checked] at tail
                                                          simp only [bind_tc_ok, Bool.false_eq_true, if_false] at tail
                                                          by_cases planVm : plan.vm = some vm
                                                          ·
                                                            have checked : core.cmp.PartialEq.ne.trait_default (core.option.Option.Insts.CoreCmpPartialEqOption ids.VmId.Insts.CoreCmpPartialEqVmId) plan.vm (some vm) = .ok false := by
                                                              simp [planVm, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                                                            rw [checked] at tail
                                                            simp only [bind_tc_ok, Bool.false_eq_true, if_false] at tail
                                                            rw [current_capture_exact] at tail
                                                            simp only [bind_tc_ok] at tail
                                                            cases captured : currentCaptureView world sandbox record with
                                                            | none => simp [captured, SandproofFoundationsRust.core.option.Option.is_some_and] at tail
                                                            | some handoff =>
                                                              simp only [captured, SandproofFoundationsRust.core.option.Option.is_some_and,
                                                                P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once] at tail
                                                              obtain ⟨allowed, mayStop, tail⟩ := bind_ok_inv _ _ _ tail
                                                              cases allowed with
                                                              | false => simp at tail
                                                              | true =>
                                                                simp only [if_true] at tail
                                                                obtain ⟨registered, recoverable, hasSource⟩ :=
                                                                  (stop_requires_complete_registration handoff).1 mayStop
                                                                have identity : operation.kind = .Pause ∧ operation.epoch = record.epoch ∧
                                                                    plan.key = ⟨sandbox, operation.id⟩ ∧ plan.epoch = record.epoch ∧
                                                                    plan.generation = record.generation ∧ plan.capture = some handoff := by
                                                                  have viewed := captured
                                                                  simp [currentCaptureView, currentOp, planLookup] at viewed
                                                                  exact viewed
                                                                obtain ⟨next, published, shape⟩ := bind_ok_inv _ _ _ tail
                                                                have same : ({event := publication.PublicationEvent.PublishPaused sandbox source.snapshot, next := next} : publication.PublicationDecision) = decision := by
                                                                  simpa only [Result.ok.injEq, core.result.Result.Ok.injEq] using shape
                                                                subst decision
                                                                exact ⟨record, snapshot, vm, rfl, rfl, phase, terminal,
                                                                  rfl, owner, session, generation, workspace, manifest, stage,
                                                                  immutable, fence, currentVm, fenceRun, freezeRun, sealRun, commitRun, stopRun,
                                                                  operation, ledger, plan, handoff, currentOp, ledgerLookup, ledgerSandbox,
                                                                  ledgerOperation, noResult, planLookup, target, planVm, captured,
                                                                  identity.1, identity.2.1, identity.2.2.1, identity.2.2.2.1,
                                                                  identity.2.2.2.2.1, identity.2.2.2.2.2,
                                                                  registered, recoverable, hasSource, vmsRun, slotsRun, published, rfl⟩
                                                          ·
                                                            have checked : core.cmp.PartialEq.ne.trait_default (core.option.Option.Insts.CoreCmpPartialEqOption ids.VmId.Insts.CoreCmpPartialEqVmId) plan.vm (some vm) = .ok true := by
                                                              simp [planVm, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                                                            rw [checked] at tail
                                                            simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at tail
                                                        ·
                                                          have checked : core.cmp.PartialEq.ne.trait_default (core.option.Option.Insts.CoreCmpPartialEqOption model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource) plan.target_snapshot (some source) = .ok true := by
                                                            simp [target, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                                                          rw [checked] at tail
                                                          simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at tail
                                                    ·
                                                      have checked : core.option.Option.is_some ledger.result = true := by
                                                        cases result : ledger.result with
                                                        | none => simp [result] at noResult
                                                        | some value => simp [core.option.Option.is_some]
                                                      rw [checked] at tail
                                                      simp only [if_true, Result.ok.injEq, reduceCtorEq] at tail
                                                  ·
                                                    have checked : core.cmp.PartialEq.ne.trait_default model.Operation.Insts.CoreCmpPartialEqOperation ledger.operation operation = .ok true := by
                                                      simp [ledgerOperation, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                                                    rw [checked] at tail
                                                    simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at tail
                                                ·
                                                  have checked : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId ledger.sandbox sandbox = .ok true := by
                                                    simp [ledgerSandbox, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
                                                  rw [checked] at tail
                                                  simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at tail

                          ·
                            have checked : core.cmp.PartialEq.ne.trait_default (core.option.Option.Insts.CoreCmpPartialEqOption (core.cmp.PartialEqShared model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence)) (tableLookup sandbox world.worker_fences.entries.val) (some ⟨record.epoch, false⟩) = .ok true := by
                              simp [fence, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                            rw [checked] at evaluated
                            simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at evaluated
                        ·
                          have checked : snapshot.immutable = false := by
                            simp [immutable]
                          rw [checked] at evaluated
                          simp only [Bool.false_eq_true, if_false, Result.ok.injEq, reduceCtorEq] at evaluated
                      ·
                        have checked : core.cmp.PartialEq.ne.trait_default model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage snapshot.stage .Durable = .ok true := by
                          simp [stage, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                        rw [checked] at evaluated
                        simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at evaluated
                    ·
                      have checked : core.cmp.PartialEq.ne.trait_default model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef snapshot.manifest source.manifest = .ok true := by
                        simp [manifest, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
                      rw [checked] at evaluated
                      simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at evaluated
                  ·
                    have checked : core.cmp.PartialEq.ne.trait_default ids.FsId.Insts.CoreCmpPartialEqFsId snapshot.workspace record.workspace = .ok true := by
                      simp [workspace, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.FsId.Insts.CoreCmpPartialEqFsId.eq]
                    rw [checked] at evaluated
                    simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at evaluated
                ·
                  have checked : (snapshot.source_generation != record.generation) = true := by
                    simp [generation]
                  rw [checked] at evaluated
                  simp only [if_true, Result.ok.injEq, reduceCtorEq] at evaluated
              ·
                have checked : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId snapshot.sandbox sandbox = .ok true := by
                  simp [session, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
                rw [checked] at evaluated
                simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at evaluated
            ·
              have checked : core.cmp.PartialEq.ne.trait_default ids.TenantId.Insts.CoreCmpPartialEqTenantId snapshot.owner record.owner = .ok true := by
                simp [owner, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq]
              rw [checked] at evaluated
              simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at evaluated
        ·
          have checked : record.terminal = true := by
            simp [terminal]
          rw [checked] at evaluated
          simp only [if_true, Result.ok.injEq, reduceCtorEq] at evaluated
      ·
        have checked : core.cmp.PartialEq.ne.trait_default model.Phase.Insts.CoreCmpPartialEqPhase record.phase .Pausing = .ok true := by
          simp [phase, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]
        rw [checked] at evaluated
        simp only [bind_tc_ok, if_true, Result.ok.injEq, reduceCtorEq] at evaluated


end SandboxModel.Lifecycle.PausePublication
