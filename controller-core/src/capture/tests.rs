use super::{CaptureCommit, CaptureHandoff, RestoreEligibility};
use crate::*;
mod caller_identity;

const SID: SandboxId = SandboxId(10);
const OWNER: TenantId = TenantId(1);
const PAUSE: OperationId = OperationId(2);

fn step(world: &World, input: ControllerInput) -> World {
    handle_controller(world, input)
        .expect("valid controller input")
        .next
}

fn effect(world: &World, predicate: impl Fn(&Command) -> bool) -> EffectId {
    world
        .effect_ids()
        .iter()
        .find(|id| predicate(id.command()))
        .expect("original effect")
        .clone()
}

fn finish(world: &World, id: EffectId) -> World {
    let dispatched = step(
        world,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let observed = step(&dispatched, ControllerInput::Worker(id.clone()));
    step(
        &observed,
        ControllerInput::Effect(EffectInput::Receipt(
            id,
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    )
}

fn drain(world: &World) -> World {
    let mut next = world.clone();
    for _ in 0..32 {
        let ready = next
            .effect_ids()
            .iter()
            .find(|id| {
                if next.effect_status(id) != Some(EffectStatus::Committed) {
                    return false;
                }
                let Ok(dispatched) = handle_controller(
                    &next,
                    ControllerInput::Effect(EffectInput::Dispatch((*id).clone())),
                ) else {
                    return false;
                };
                handle_controller(&dispatched.next, ControllerInput::Worker((*id).clone())).is_ok()
            })
            .cloned();
        let Some(id) = ready else {
            return next;
        };
        next = finish(&next, id);
    }
    panic!("finite fixture did not drain");
}

fn running() -> World {
    let created = step(
        &initial_world(),
        ControllerInput::Request(Request::Create(CreateRequest {
            actor: Actor::Tenant(OWNER),
            owner: OWNER,
            sandbox: SID,
            workspace: FsId(1),
            file_policy: FilePolicyRef {
                digest: PolicyDigest(1),
            },
            policy: NetworkPolicy::DenyAll,
            operation: OperationId(1),
        })),
    );
    let reserved = step(
        &drain(&created),
        ControllerInput::Scheduler(SchedulerInput::ReserveStartup(OperationKey {
            sandbox: SID,
            operation: OperationId(1),
        })),
    );
    let plan = reserved
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: OperationId(1),
        })
        .unwrap();
    let prepared = step(
        &reserved,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    step(
        &drain(&prepared),
        ControllerInput::Publication(PublicationInput::Running(SID)),
    )
}

fn pausing() -> World {
    let accepted = step(
        &running(),
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: PAUSE,
        })),
    );
    step(
        &accepted,
        ControllerInput::Scheduler(SchedulerInput::ReservePauseSnapshot(OperationKey {
            sandbox: SID,
            operation: PAUSE,
        })),
    )
}

fn capture(world: &World) -> CaptureHandoff {
    world
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: PAUSE,
        })
        .unwrap()
        .capture
        .unwrap()
}

fn source(world: &World) -> RestoreSource {
    world
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: PAUSE,
        })
        .unwrap()
        .target_snapshot
        .unwrap()
}

fn sealed() -> World {
    let frozen = drain(&pausing());
    let target = source(&frozen);
    let planned = step(
        &frozen,
        ControllerInput::Scheduler(SchedulerInput::SealSnapshot(
            SID,
            target.snapshot,
            target.manifest,
        )),
    );
    drain(&planned)
}

fn commit_planned() -> (World, EffectId) {
    let sealed = sealed();
    let planned = step(
        &sealed,
        ControllerInput::Scheduler(SchedulerInput::CommitSnapshot(
            SID,
            source(&sealed).snapshot,
        )),
    );
    let id = effect(&planned, |c| matches!(c, Command::CommitSnapshot { .. }));
    (planned, id)
}

fn fail(world: &World) -> World {
    step(
        world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SID,
            failure: FailureKind::Cancelled,
            evidence: None,
        })),
    )
}

fn assert_original_kept(before: &World, after: &World) {
    let original = before.sandbox_record(SID).unwrap();
    let held = after.sandbox_record(SID).unwrap();
    assert_eq!(held.current_vm, original.current_vm);
    assert_eq!(held.generation, original.generation);
    assert_eq!(held.phase, Phase::Pausing);
    assert!(
        !held.terminal,
        "ordinary pause failure is not owner destroy"
    );
    assert_eq!(
        after.vm_record(original.current_vm.unwrap()),
        before.vm_record(original.current_vm.unwrap())
    );
    assert_eq!(after.resource_index(SID), before.resource_index(SID));
    assert_eq!(
        held.effective, original.effective,
        "failure is not a physical fence receipt"
    );
    assert_eq!(
        after.worker_fences.get(&SID),
        before.worker_fences.get(&SID)
    );
    if let Some(slot) = original.net_slot {
        assert_eq!(after.slot_record(slot), before.slot_record(slot));
    }
    assert_eq!(
        capture(after).has_capture_work(),
        capture(before).has_capture_work()
    );
    assert_eq!(
        capture(after).has_unload_source(),
        capture(before).has_unload_source()
    );
    assert_eq!(
        after.effect_ids(),
        before.effect_ids(),
        "failure must not invent terminal fence or stop"
    );
}

#[test]
fn ordinary_failure_keeps_live_original_and_invalidates_already_queued_freeze() {
    let before = pausing();
    let freeze = effect(&before, |c| matches!(c, Command::FreezeVm { .. }));
    let after = fail(&before);
    assert_original_kept(&before, &after);
    assert_eq!(capture(&after).eligibility(), RestoreEligibility::Revoked);
    assert_eq!(
        handle_effect_dispatch(&after, freeze),
        Err(EffectReject::NotDispatchable)
    );
    let row = before.sandbox_record(SID).unwrap();
    assert!(matches!(row.effective, Authority::Running { .. }));
    let fence = effect(
        &after,
        |c| matches!(c, Command::InstallFence { epoch, terminal: false, .. } if *epoch == row.epoch),
    );
    let revoked = finish(&after, fence);
    assert_eq!(
        revoked.sandbox_record(SID).unwrap().effective,
        Authority::Closed
    );
    assert_eq!(
        revoked.worker_fences.get(&SID),
        Some(&WorkerFence {
            epoch: row.epoch,
            terminal: false
        })
    );
    assert!(!revoked.slot_record(row.net_slot.unwrap()).unwrap().active);
    assert_eq!(
        revoked.vm_record(row.current_vm.unwrap()),
        before.vm_record(row.current_vm.unwrap())
    );
    assert_eq!(capture(&revoked).eligibility(), RestoreEligibility::Revoked);
}

#[test]
fn ordinary_failure_keeps_frozen_original_and_rejects_late_seal_commit_and_stop() {
    let before = sealed();
    let target = source(&before);
    let after = fail(&before);
    assert_original_kept(&before, &after);
    assert_eq!(
        handle_schedule_commit_snapshot(&after, SID, target.snapshot),
        Err(SchedulerReject::PlanNotOpen)
    );
    assert_eq!(
        handle_schedule_seal_snapshot(&after, SID, target.snapshot, target.manifest),
        Err(SchedulerReject::PlanNotOpen)
    );
    assert!(!capture(&after).may_stop_original());
    assert!(
        !capture(&after).may_reactivate_original(),
        "unknown work has no fabricated exit receipt"
    );
}

#[test]
fn commit_dispatch_is_unknown_and_late_success_tracks_material_without_reviving_capture() {
    let (before, id) = commit_planned();
    let dispatched = step(
        &before,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    assert_eq!(capture(&dispatched).commit(), CaptureCommit::Unknown);
    let after = fail(&dispatched);
    assert_original_kept(&dispatched, &after);
    assert_eq!(capture(&after).commit(), CaptureCommit::Unknown);
    assert_eq!(
        handle_effect_dispatch(&after, id.clone()),
        Err(EffectReject::NotDispatchable)
    );
    assert!(!capture(&after).may_stop_original());
    // This is the original service's observation -> actual Receipt ordering:
    // the already dispatched physical commit succeeded after ordinary failure.
    let observed = step(&after, ControllerInput::Worker(id.clone()));
    let received = step(
        &observed,
        ControllerInput::Effect(EffectInput::Receipt(
            id.clone(),
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    );
    assert_eq!(
        received.effect_status(&id),
        Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(
            ResultDigest(1)
        )))
    );
    assert_eq!(capture(&received).commit(), CaptureCommit::Registered);
    assert_eq!(
        capture(&received).eligibility(),
        RestoreEligibility::Revoked
    );
    assert!(capture(&received).has_capture_work());
    assert!(
        capture(&received).has_unload_source(),
        "late published files have cleanup responsibility"
    );
    assert!(!capture(&received).may_stop_original());
    assert_eq!(
        received.effect_ids(),
        after.effect_ids(),
        "late commit must not enqueue StopVm"
    );
    assert_eq!(
        received.vm_record(received.sandbox_record(SID).unwrap().current_vm.unwrap()),
        after.vm_record(after.sandbox_record(SID).unwrap().current_vm.unwrap())
    );
    assert!(handle_publish_paused(&received, SID, source(&received)).is_err());
    let replayed = step(
        &received,
        ControllerInput::Effect(EffectInput::Receipt(
            id,
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    );
    assert_eq!(
        replayed, received,
        "duplicate late success is exactly idempotent"
    );
}

#[test]
fn registered_handoff_keeps_material_responsibility_and_allows_original_unload() {
    let (before, id) = commit_planned();
    let registered = finish(&before, id);
    assert_eq!(capture(&registered).commit(), CaptureCommit::Registered);
    assert!(capture(&registered).has_unload_source());
    let after = fail(&registered);
    assert_original_kept(&registered, &after);
    assert!(
        capture(&after).may_stop_original(),
        "valid handoff must not be rejected forever"
    );
    let stopped = drain(&after);
    let published = step(
        &stopped,
        ControllerInput::Publication(PublicationInput::Paused(SID, source(&stopped))),
    );
    assert_eq!(published.sandbox_record(SID).unwrap().phase, Phase::Paused);
    assert!(capture(&published).has_unload_source());
}

#[test]
fn owner_destroy_revokes_eligibility_but_never_releases_capture_material() {
    let (before, id) = commit_planned();
    let registered = finish(&before, id);
    let after = step(
        &registered,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(3),
        })),
    );
    assert!(after.sandbox_record(SID).unwrap().terminal);
    assert_eq!(capture(&after).eligibility(), RestoreEligibility::Revoked);
    assert!(capture(&after).has_capture_work());
    assert!(capture(&after).has_unload_source());
}

#[test]
fn early_stop_without_registered_handoff_is_refused_by_real_worker() {
    let mut before = sealed();
    let record = before.sandbox_record(SID).unwrap();
    let command = Command::StopVm {
        sandbox: SID,
        epoch: record.epoch,
        generation: record.generation,
        vm: record.current_vm.unwrap(),
    };
    crate::transition::enqueue(&mut before, command.clone());
    let id = effect(&before, |c| *c == command);
    let dispatched = step(
        &before,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    assert_eq!(
        handle_worker(&dispatched, &id),
        Err(WorkerReject::SnapshotNotReady)
    );
}

#[test]
fn duplicate_and_exhausted_reservation_preserve_original_running_charge() {
    let before = pausing();
    assert_eq!(
        handle_reserve_pause_snapshot(
            &before,
            OperationKey {
                sandbox: SID,
                operation: PAUSE
            }
        ),
        Err(SchedulerReject::PlanAlreadySealed)
    );
    let mut limited = running();
    limited.next_snapshot_id = u64::MAX;
    let accepted = step(
        &limited,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: PAUSE,
        })),
    );
    assert_eq!(
        handle_reserve_pause_snapshot(
            &accepted,
            OperationKey {
                sandbox: SID,
                operation: PAUSE
            }
        ),
        Err(SchedulerReject::CounterExhausted)
    );
    assert_original_kept(&accepted, &fail(&accepted));
}

#[test]
fn rejected_and_duplicate_failure_do_not_mutate_original_ledger() {
    let before = pausing();
    assert_eq!(
        handle_compensation(
            &before,
            CompensationRequest {
                sandbox: SID,
                failure: FailureKind::InternalFailure,
                evidence: None,
            }
        ),
        Err(Reject::FailureUnjustified)
    );
    let after = fail(&before);
    assert_eq!(
        handle_compensation(
            &after,
            CompensationRequest {
                sandbox: SID,
                failure: FailureKind::Cancelled,
                evidence: None,
            }
        ),
        Err(Reject::OperationAlreadyFinal)
    );
}

#[test]
fn revoking_c1_keeps_the_original_c0_running_backing_responsibility() {
    let (before, id) = commit_planned();
    let stopped = drain(&finish(&before, id));
    let paused = step(
        &stopped,
        ControllerInput::Publication(PublicationInput::Paused(SID, source(&stopped))),
    );
    let c0 = capture(&paused);
    let resumed = step(
        &paused,
        ControllerInput::Request(Request::Resume(ResumeRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(3),
        })),
    );
    let reserved = step(
        &drain(&resumed),
        ControllerInput::Scheduler(SchedulerInput::ReserveStartup(OperationKey {
            sandbox: SID,
            operation: OperationId(3),
        })),
    );
    let plan = reserved
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: OperationId(3),
        })
        .unwrap();
    let prepared = step(
        &reserved,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    let running = step(
        &drain(&prepared),
        ControllerInput::Publication(PublicationInput::Running(SID)),
    );
    let c1 = step(
        &running,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(4),
        })),
    );
    let failed = fail(&c1);
    assert_eq!(
        capture(&failed),
        c0,
        "c1 revocation must not change c0 material slots"
    );
    assert!(c0.has_unload_source());
    let original = running.sandbox_record(SID).unwrap().current_vm.unwrap();
    assert_eq!(failed.vm_record(original), running.vm_record(original));
}

#[test]
fn finite_capture_state_space_preserves_uses_and_never_revives_failed_registration() {
    for commit in [
        CaptureCommit::NotRegistered,
        CaptureCommit::Unknown,
        CaptureCommit::Registered,
    ] {
        for eligibility in [
            RestoreEligibility::Capturing,
            RestoreEligibility::Recoverable,
            RestoreEligibility::Revoked,
        ] {
            for capture_work in [false, true] {
                for unload_source in [false, true] {
                    let original = CaptureHandoff {
                        commit,
                        eligibility,
                        capture_work,
                        unload_source,
                        original_continuation: None,
                        terminal_drain: None,
                    };
                    let failed = original.ordinary_failure();
                    assert_eq!(failed.commit(), original.commit());
                    assert_eq!(failed.has_capture_work(), capture_work);
                    assert_eq!(failed.has_unload_source(), unload_source);
                    if commit != CaptureCommit::Registered {
                        assert_eq!(failed.eligibility(), RestoreEligibility::Revoked);
                        assert!(failed.register_bundle().is_none());
                        assert!(!failed.may_stop_original());
                        if let Some(observed) = failed.observe_bundle() {
                            assert_eq!(observed.eligibility(), RestoreEligibility::Revoked);
                            assert!(observed.has_capture_work());
                            assert!(observed.has_unload_source());
                            assert!(!observed.may_stop_original());
                        }
                    }
                    let terminated = original.terminate();
                    assert_eq!(terminated.has_capture_work(), capture_work);
                    assert_eq!(terminated.has_unload_source(), unload_source);
                    assert!(!terminated.may_stop_original());
                }
            }
        }
    }
}

#[test]
fn resource_failure_from_original_work_is_nonterminal_after_bounded_retries() {
    let mut world = pausing();
    let epoch = world.sandbox_record(SID).unwrap().epoch;
    let fence = effect(
        &world,
        |c| matches!(c, Command::InstallFence { epoch: e, terminal: false, .. } if *e == epoch),
    );
    world = finish(&world, fence);
    let mut id = effect(&world, |c| matches!(c, Command::FreezeVm { .. }));
    for attempt in 0..=crate::DEFAULT_RETRY_LIMIT {
        world = step(
            &world,
            ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
        );
        world = step(
            &world,
            ControllerInput::Effect(EffectInput::Receipt(
                id.clone(),
                EffectOutcome::Failed(FailureKind::ResourceFailure),
            )),
        );
        if attempt < crate::DEFAULT_RETRY_LIMIT {
            world = step(
                &world,
                ControllerInput::Effect(EffectInput::Retry(
                    id.logical.clone(),
                    FailureKind::ResourceFailure,
                )),
            );
            id.attempt += 1;
        }
    }
    world = step(
        &world,
        ControllerInput::Effect(EffectInput::FinalizeFailure(
            id.logical.clone(),
            FailureKind::ResourceFailure,
        )),
    );
    let failed = step(
        &world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SID,
            failure: FailureKind::ResourceFailure,
            evidence: Some(id),
        })),
    );
    assert_original_kept(&world, &failed);
    assert_eq!(capture(&failed).eligibility(), RestoreEligibility::Revoked);
}
