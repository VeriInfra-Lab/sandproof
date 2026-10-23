//! The original startup abort owns a partially launched, unregistered VM.
use super::*;
use crate::WorkerTransportError;
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};

fn partial_prepare(
    journal: &mut ControllerJournal,
) -> (
    LifecycleRequest,
    InternalLocalBinding,
    WorkAction<core::EffectId>,
) {
    let create = local_request();
    journal
        .record_internal_intent(&create, &mut NeverCrash)
        .unwrap();
    let local = journal
        .admit_internal_create(&create, local_admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    assert!(matches!(
        crate::finish_admitted_create_timed(
            journal,
            local.sandbox,
            local.operation,
            &mut |action| {
                if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                    Ok(WorkReceipt {
                        identity: action.identity.clone(),
                        outcome: WorkOutcome::Unknown {
                            reason: sandproof_protocol::UnknownReason::ObservationUnavailable,
                            last_observation: None,
                        },
                    })
                } else {
                    workflow_fixture::worker(action)
                }
            },
            &mut || Ok(20),
        ),
        Err(JournalError::UnresolvedAction)
    ));
    let prepare = journal.uncertain_actions().unwrap().pop().unwrap();
    assert!(matches!(
        prepare.command.command(),
        core::Command::PrepareVm { .. }
    ));
    let world = journal.world().unwrap();
    assert!(world.resource_index(local.sandbox).vms.is_empty());
    assert!(
        world
            .sandbox_record(local.sandbox)
            .unwrap()
            .current_vm
            .is_none()
    );
    (create, local, prepare)
}

fn destroy(create: &LifecycleRequest) -> LifecycleRequest {
    LifecycleRequest {
        intent: Intent::Destroy,
        identity: request(2, 2, Intent::Destroy).identity,
        ..create.clone()
    }
}

fn assert_compensating(
    journal: &ControllerJournal,
    local: InternalLocalBinding,
    failure: core::FailureKind,
) {
    let world = journal.world().unwrap();
    let record = world.sandbox_record(local.sandbox).unwrap();
    let operation = record.current_operation.unwrap();
    assert_eq!(operation.id, local.operation);
    assert_eq!(operation.kind, core::OperationKind::Create);
    assert_eq!(
        operation.status,
        core::OperationStatus::Compensating(failure)
    );
    assert!(core::scheduler::only_uncertain_prepare_for_abort(
        &world,
        local.sandbox
    ));
}

fn observe_reclaimed_prepare(
    journal: &mut ControllerJournal,
    prepare: &WorkAction<core::EffectId>,
) {
    // This is the original native observation emitted from the exact durable
    // reclaimed-prepare marker, not from absence of a logical VM.
    let aborted = WorkReceipt::<workflow_fixture::TestHostEvidence> {
        identity: prepare.identity.clone(),
        outcome: WorkOutcome::DefinitiveFailure {
            code: sandproof_protocol::FailureCode::InvalidResource,
            retryable: false,
            partial: None,
        },
    };
    assert!(journal.reconcile_observed(prepare, &aborted).unwrap());
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

fn confirm_exact_release(
    journal: &mut ControllerJournal,
    destroy: &LifecycleRequest,
    local: InternalLocalBinding,
) -> LifecycleResponse {
    assert_eq!(
        journal
            .capture_internal_progress(destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
    let context = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    assert!(
        journal
            .release_host_slot(
                context,
                |_| Err(WorkerTransportError::ReplyLost),
                &mut NeverCrash
            )
            .is_err()
    );
    assert_eq!(
        journal
            .capture_internal_progress(destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    let cleaned = journal
        .capture_internal_progress(destroy, &mut NeverCrash)
        .unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    cleaned
}

#[test]
fn destroy_of_partial_prepare_keeps_original_abort_plan_and_requires_exact_release() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (create, local, prepare) = partial_prepare(&mut journal);
    // Restart before receipt of Destroy: no volatile VM identity is needed.
    drop(journal);
    let mut journal = open(&path);
    let destroy = destroy(&create);
    let accepted = journal
        .record_internal_intent(&destroy, &mut NeverCrash)
        .unwrap();
    assert_eq!(accepted.fact, LifecycleFact::Recorded);
    assert_compensating(&journal, local, core::FailureKind::Cancelled);
    assert_eq!(
        journal
            .abort_due_startups(700_000, &mut NeverCrash)
            .unwrap(),
        0
    );
    drop(journal);
    let mut journal = open(&path);
    assert_eq!(
        journal
            .record_internal_intent(&destroy, &mut NeverCrash)
            .unwrap(),
        accepted
    );
    let mut commands = Vec::new();
    crate::finish_compensating_create(
        &mut journal,
        local.sandbox,
        local.operation,
        &mut |action| {
            assert!(!matches!(
                action.command.command(),
                core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }
            ));
            commands.push(action.command.command().clone());
            workflow_fixture::worker(action)
        },
    )
    .unwrap();
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, core::Command::AbortPrepare { .. }))
            .count(),
        1
    );
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(local.sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    // A late original Prepared receipt cannot revive the cancelled VM.
    let before = world;
    assert!(
        journal
            .complete(
                &workflow_fixture::worker(&prepare).unwrap(),
                &mut NeverCrash
            )
            .is_err()
    );
    assert_eq!(journal.world().unwrap(), before);
    observe_reclaimed_prepare(&mut journal, &prepare);
    let cleaned = confirm_exact_release(&mut journal, &destroy, local);
    drop(journal);
    let mut journal = open(&path);
    assert_eq!(
        journal
            .record_internal_intent(&destroy, &mut NeverCrash)
            .unwrap(),
        cleaned
    );
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
}

#[test]
fn destroy_and_original_compensation_are_atomic_across_known_and_unknown_commits() {
    for failure in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (create, local, _) = partial_prepare(&mut journal);
        let original = journal.world().unwrap();
        let destroy = destroy(&create);
        let committed = failure % 2 == 1;
        if failure < 2 {
            let point = if committed {
                CrashPoint::AfterCommit
            } else {
                CrashPoint::BeforeCommit
            };
            assert!(matches!(
                journal.record_internal_intent(&destroy, &mut Stop(point)),
                Err(JournalError::InjectedCrash(_))
            ));
        } else {
            crate::controller::transaction::fail_next_commit(committed);
            assert!(matches!(
                journal.record_internal_intent(&destroy, &mut NeverCrash),
                Err(JournalError::CommitUncertain(_))
            ));
        }
        drop(journal);
        let mut journal = open(&path);
        if committed {
            assert_compensating(&journal, local, core::FailureKind::Cancelled);
            assert_eq!(
                journal.internal_pending(&config().node).unwrap(),
                vec![destroy.clone()]
            );
        } else {
            assert_eq!(journal.world().unwrap(), original);
            assert_eq!(
                journal.internal_pending(&config().node).unwrap(),
                vec![create]
            );
        }
        let receipt = journal
            .record_internal_intent(&destroy, &mut NeverCrash)
            .unwrap();
        assert_eq!(receipt.fact, LifecycleFact::Recorded);
        assert_compensating(&journal, local, core::FailureKind::Cancelled);
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
        assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    }
}

#[test]
fn lost_abort_result_is_observed_after_restart_not_executed_as_a_new_prepare() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (create, local, prepare) = partial_prepare(&mut journal);
    let destroy = destroy(&create);
    journal
        .record_internal_intent(&destroy, &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        crate::finish_compensating_create(
            &mut journal,
            local.sandbox,
            local.operation,
            &mut |action| {
                assert!(!matches!(
                    action.command.command(),
                    core::Command::PrepareVm { .. }
                ));
                if matches!(action.command.command(), core::Command::AbortPrepare { .. }) {
                    Err(WorkerTransportError::ReplyLost)
                } else {
                    workflow_fixture::worker(action)
                }
            }
        ),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(
        journal
            .capture_internal_progress(&destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Recorded
    );
    drop(journal);
    let mut journal = open(&path);
    let abort = journal
        .uncertain_actions()
        .unwrap()
        .into_iter()
        .find(|action| matches!(action.command.command(), core::Command::AbortPrepare { .. }))
        .unwrap();
    assert!(
        journal
            .reconcile_observed(&abort, &workflow_fixture::worker(&abort).unwrap())
            .unwrap()
    );
    crate::finish_compensating_create(
        &mut journal,
        local.sandbox,
        local.operation,
        &mut |action| {
            assert!(!matches!(
                action.command.command(),
                core::Command::PrepareVm { .. } | core::Command::AbortPrepare { .. }
            ));
            workflow_fixture::worker(action)
        },
    )
    .unwrap();
    observe_reclaimed_prepare(&mut journal, &prepare);
    assert_eq!(
        journal
            .capture_internal_progress(&destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
}

#[test]
fn destroy_after_original_startup_abort_reuses_deadline_compensation() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (create, local, _) = partial_prepare(&mut journal);
    assert_eq!(
        journal
            .abort_due_startups(700_000, &mut NeverCrash)
            .unwrap(),
        1
    );
    journal
        .record_internal_intent(&destroy(&create), &mut NeverCrash)
        .unwrap();
    assert_compensating(&journal, local, core::FailureKind::DeadlineExceeded);
}
