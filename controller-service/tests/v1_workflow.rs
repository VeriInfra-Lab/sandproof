mod support;
use support::*;

use sandbox_controller_core as core;
use sandproof_controller_service::{
    AdmissionLimits, ControllerJournal, CrashInjector, CrashPoint, JournalError, NeverCrash,
    create_v1, finish_accepted_kill_v1, finish_admitted_create_v1, finish_admitted_create_v1_timed,
    finish_compensating_create_v1, kill_v1,
};
use sandproof_protocol::{FailureCode, UnknownReason, WorkAction, WorkOutcome, WorkReceipt};
use tempfile::TempDir;

fn setup() -> (TempDir, ControllerJournal) {
    let directory = TempDir::new().unwrap();
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    (directory, journal)
}

fn create_request() -> core::CreateRequest {
    let core::ControllerInput::Request(core::Request::Create(request)) = create() else {
        unreachable!()
    };
    request
}

fn kill_request() -> core::KillRequest {
    let core::ControllerInput::Request(core::Request::Kill(request)) = kill(1) else {
        unreachable!()
    };
    request
}

#[test]
fn create_publishes_after_all_results_and_kill_publishes_after_stop() {
    let (directory, mut journal) = setup();
    let running = create_v1(&mut journal, create_request(), &mut worker).unwrap();
    assert_eq!(
        running.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Running
    );
    assert_eq!(running.resource_index(SANDBOX).vms.len(), 1);
    let destroyed = kill_v1(&mut journal, kill_request(), &mut worker).unwrap();
    assert_eq!(
        destroyed.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Destroyed
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
    drop(journal);
    let reopened =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(reopened.world().unwrap(), destroyed);
}

#[test]
fn quota_admission_and_expiry_reuse_the_same_physical_workflow() {
    let (_directory, mut journal) = setup();
    let limits = AdmissionLimits {
        max_active_global: 2,
        max_active_per_tenant: 1,
        max_disk_bytes_global: 10_000,
        max_disk_bytes_per_tenant: 5_000,
        cleanup_reserve_bytes: 1_000,
        max_timeout_secs: 86_400,
        max_lifetime_secs: 86_400,
    };
    let request = create_request();
    journal
        .admit_create(request.clone(), 1_000, 60, 2_000, limits, &mut NeverCrash)
        .unwrap();
    let running = finish_admitted_create_v1(
        &mut journal,
        request.sandbox,
        request.operation,
        &mut worker,
    )
    .unwrap();
    assert_eq!(
        running.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Running
    );
    assert_eq!(
        journal
            .live_lifetime(&request.actor, SANDBOX, 1_001)
            .unwrap()
            .deadline_ms,
        61_000
    );

    let expiry_operation = core::OperationId(102);
    journal
        .accept_expiry(SANDBOX, expiry_operation, 61_000, &mut NeverCrash)
        .unwrap();
    let destroyed =
        finish_accepted_kill_v1(&mut journal, SANDBOX, expiry_operation, &mut worker).unwrap();
    assert_eq!(
        destroyed.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Destroyed
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

struct BeforePublicationCommit;

impl CrashInjector for BeforePublicationCommit {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == CrashPoint::BeforeCommit {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn running_publication_resets_usable_lifetime_atomically_across_restart() {
    let (directory, mut journal) = setup();
    let request = create_request();
    let limits = AdmissionLimits {
        max_active_global: 2,
        max_active_per_tenant: 1,
        max_disk_bytes_global: 10_000,
        max_disk_bytes_per_tenant: 5_000,
        cleanup_reserve_bytes: 1_000,
        max_timeout_secs: 300,
        max_lifetime_secs: 600,
    };
    journal
        .admit_create(request.clone(), 1_000, 60, 2_000, limits, &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        finish_admitted_create_v1_timed(
            &mut journal,
            request.sandbox,
            request.operation,
            &mut worker,
            &mut || Err(JournalError::InjectedCrash(CrashPoint::BeforeCommit)),
        ),
        Err(JournalError::InjectedCrash(CrashPoint::BeforeCommit))
    ));
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(SANDBOX)
            .unwrap()
            .phase,
        core::Phase::Starting
    );
    assert!(matches!(
        journal.publish_running_at(SANDBOX, 5_000, &mut BeforePublicationCommit),
        Err(JournalError::InjectedCrash(CrashPoint::BeforeCommit))
    ));
    drop(journal);
    let mut reopened =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(
        reopened
            .world()
            .unwrap()
            .sandbox_record(SANDBOX)
            .unwrap()
            .phase,
        core::Phase::Starting
    );
    let running = finish_admitted_create_v1_timed(
        &mut reopened,
        request.sandbox,
        request.operation,
        &mut |_action| -> Result<WorkReceipt<FakeHostEvidence>, _> {
            panic!("recovery must not dispatch acknowledged host work")
        },
        &mut || Ok(6_000),
    )
    .unwrap();
    assert_eq!(
        running.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Running
    );
    let lifetime = reopened
        .live_lifetime(&request.actor, SANDBOX, 6_001)
        .unwrap();
    assert_eq!(
        (
            lifetime.started_at_ms,
            lifetime.deadline_ms,
            lifetime.max_deadline_ms
        ),
        (Some(6_000), 66_000, 606_000)
    );
    assert!(matches!(
        reopened.accept_expiry(SANDBOX, core::OperationId(102), 61_000, &mut NeverCrash),
        Err(sandproof_controller_service::AdmissionError::NotDue)
    ));
}

#[test]
fn unknown_host_result_prevents_running_publication() {
    let (_directory, mut journal) = setup();
    let result = create_v1(
        &mut journal,
        create_request(),
        &mut |action: &WorkAction<core::EffectId>| {
            if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                Ok(WorkReceipt::<FakeHostEvidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            } else {
                worker(action)
            }
        },
    );
    assert!(matches!(result, Err(JournalError::UnresolvedAction)));
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(SANDBOX)
            .unwrap()
            .phase,
        core::Phase::Starting
    );
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
}

#[test]
fn definitive_create_failure_revokes_stops_and_publishes_failed() {
    let (_directory, mut journal) = setup();
    let mut stopped = false;
    let result = create_v1(&mut journal, create_request(), &mut |action| {
        let outcome = if matches!(
            action.command.command(),
            core::Command::ConfigureNetwork { .. }
        ) {
            WorkOutcome::DefinitiveFailure {
                code: FailureCode::PolicyRejected,
                retryable: false,
                partial: None,
            }
        } else {
            stopped |= matches!(action.command.command(), core::Command::StopVm { .. });
            return worker(action);
        };
        Ok(WorkReceipt::<FakeHostEvidence> {
            identity: action.identity.clone(),
            outcome,
        })
    });
    assert!(
        matches!(result, Err(JournalError::OperationFailed)),
        "{result:?}"
    );
    assert!(stopped);
    let world = journal.world().unwrap();
    let record = world.sandbox_record(SANDBOX).unwrap();
    assert_eq!(record.phase, core::Phase::Destroyed);
    assert_eq!(record.effective, core::Authority::Closed);
    assert_eq!(
        record.current_operation.unwrap().status,
        core::OperationStatus::Failed(core::FailureKind::PolicyFailure)
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[test]
fn clean_retryable_create_failure_uses_next_attempt_once() {
    let (_directory, mut journal) = setup();
    let mut network_attempts = Vec::new();
    let result = create_v1(&mut journal, create_request(), &mut |action| {
        if matches!(
            action.command.command(),
            core::Command::ConfigureNetwork { .. }
        ) {
            network_attempts.push(action.command.attempt);
            if network_attempts.len() == 1 {
                return Ok(WorkReceipt::<FakeHostEvidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::DefinitiveFailure {
                        code: FailureCode::ResourceUnavailable,
                        retryable: true,
                        partial: None,
                    },
                });
            }
        }
        worker(action)
    })
    .unwrap();
    assert_eq!(
        result.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Running
    );
    assert_eq!(network_attempts, vec![0, 1]);
}

#[test]
fn compensation_stop_reply_lost_requires_observation_before_failed_publication() {
    let (directory, mut journal) = setup();
    let lost = create_v1(&mut journal, create_request(), &mut |action| {
        if matches!(
            action.command.command(),
            core::Command::ConfigureNetwork { .. }
        ) {
            return Ok(WorkReceipt::<FakeHostEvidence> {
                identity: action.identity.clone(),
                outcome: WorkOutcome::DefinitiveFailure {
                    code: FailureCode::PolicyRejected,
                    retryable: false,
                    partial: None,
                },
            });
        }
        if matches!(action.command.command(), core::Command::StopVm { .. }) {
            return Ok(WorkReceipt::<FakeHostEvidence> {
                identity: action.identity.clone(),
                outcome: WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            });
        }
        worker(action)
    });
    assert!(matches!(lost, Err(JournalError::UnresolvedAction)));
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(SANDBOX)
            .unwrap()
            .phase,
        core::Phase::Revoking
    );
    drop(journal);

    let mut reopened =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let uncertain = reopened.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::StopVm { .. }
    ));
    assert_eq!(
        reopened
            .reconcile_uncertain(&mut |action| Some(success(action)))
            .unwrap(),
        1
    );
    let completed =
        finish_compensating_create_v1(&mut reopened, SANDBOX, CREATE_OP, &mut worker).unwrap();
    assert_eq!(
        completed.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(
        completed
            .sandbox_record(SANDBOX)
            .unwrap()
            .current_operation
            .unwrap()
            .status,
        core::OperationStatus::Failed(core::FailureKind::PolicyFailure)
    );
}

#[test]
fn one_sandbox_workflow_does_not_dispatch_another_tenants_actions() {
    let (_directory, mut journal) = setup();
    let mut other = create_request();
    other.actor = core::Actor::Tenant(core::TenantId(2));
    other.owner = core::TenantId(2);
    other.sandbox = core::SandboxId(20);
    other.workspace = core::FsId(20);
    other.operation = core::OperationId(200);
    apply(
        &mut journal,
        core::ControllerInput::Request(core::Request::Create(other)),
    );
    let mut seen = Vec::new();
    create_v1(
        &mut journal,
        create_request(),
        &mut |action: &WorkAction<core::EffectId>| {
            seen.push(action.command.command().sandbox());
            worker(action)
        },
    )
    .unwrap();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|sandbox| *sandbox == SANDBOX));
    let other_fence = journal
        .world()
        .unwrap()
        .effect_ids()
        .iter()
        .find(|effect| effect.command().sandbox() == core::SandboxId(20))
        .unwrap()
        .clone();
    assert_eq!(
        journal.world().unwrap().effect_status(&other_fence),
        Some(core::EffectStatus::Committed)
    );
    assert_eq!(dispatch_all(&mut journal), 1);
}
