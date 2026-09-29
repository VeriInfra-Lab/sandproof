mod support;
use support::*;

use sandbox_controller_core as core;
use sandproof_controller_service::{
    AdmissionError, AdmissionLimits, ControllerJournal, CrashInjector, CrashPoint, JournalError,
    NeverCrash, finish_admitted_create_v1, finish_compensating_create_v1,
};
use sandproof_protocol::{UnknownReason, WorkAction, WorkOutcome, WorkReceipt};
use tempfile::TempDir;

struct CrashAt(CrashPoint);
impl CrashInjector for CrashAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

fn limits() -> AdmissionLimits {
    AdmissionLimits {
        max_vm_id: 10_000,
        max_active_global: 1,
        max_active_per_tenant: 1,
        max_disk_bytes_global: 10_000,
        max_disk_bytes_per_tenant: 5_000,
        cleanup_reserve_bytes: 1_000,
        max_timeout_secs: 300,
        max_lifetime_secs: 600,
        startup_timeout_secs: 1,
    }
}

fn unknown(action: &WorkAction<core::EffectId>) -> WorkReceipt<FakeHostEvidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::Unknown {
            reason: UnknownReason::ObservationUnavailable,
            last_observation: None,
        },
    }
}

fn setup() -> (TempDir, ControllerJournal, WorkAction<core::EffectId>) {
    let directory = TempDir::new().unwrap();
    let mut journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let core::ControllerInput::Request(core::Request::Create(request)) = create() else {
        unreachable!()
    };
    journal
        .admit_create(request, 0, 300, 2_000, limits(), &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        finish_admitted_create_v1(&mut journal, SANDBOX, CREATE_OP, &mut |action| {
            if matches!(action.command.command(), core::Command::ActivateVm { .. }) {
                Ok(unknown(action))
            } else {
                worker(action)
            }
        }),
        Err(JournalError::UnresolvedAction)
    ));
    let actions = journal.uncertain_actions().unwrap();
    assert_eq!(actions.len(), 1);
    (directory, journal, actions[0].clone())
}

fn schedule_stop(journal: &mut ControllerJournal) -> core::EffectId {
    // The activation is still unknown; only the terminal fence is dispatched.
    assert_eq!(dispatch_all(journal), 1);
    let world = journal.world().unwrap();
    let record = world.sandbox_record(SANDBOX).unwrap();
    let command = core::Command::StopVm {
        sandbox: SANDBOX,
        epoch: record.epoch,
        generation: record.generation,
        vm: record.current_vm.unwrap(),
    };
    apply(
        journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
            SANDBOX,
            record.generation,
            record.current_vm.unwrap(),
        )),
    );
    journal
        .world()
        .unwrap()
        .effect_ids()
        .iter()
        .find(|id| id.command() == &command)
        .unwrap()
        .clone()
}

fn assert_reclaimed(journal: &ControllerJournal, activation: &core::EffectId) {
    let world = journal.world().unwrap();
    assert!(journal.uncertain_actions().unwrap().is_empty());
    assert!(matches!(
        world.effect_status(activation),
        Some(core::EffectStatus::RetiredByStop(_))
    ));
    assert!(!core::effect_succeeded_check(&world, activation.command()));
    let record = world.sandbox_record(SANDBOX).unwrap();
    assert_eq!(record.phase, core::Phase::Destroyed);
    assert_eq!(
        record.current_operation.unwrap().status,
        core::OperationStatus::Failed(core::FailureKind::DeadlineExceeded)
    );
    assert_eq!(record.effective, core::Authority::Closed);
    let index = world.resource_index(SANDBOX);
    assert!(
        index
            .vms
            .iter()
            .all(|vm| world.vm_record(*vm).unwrap().status == core::VmStatus::Reclaimed)
    );
    assert!(
        index
            .slots
            .iter()
            .all(|slot| world.slot_record(*slot).unwrap().reclaimed)
    );
}

#[test]
fn terminal_activation_stops_without_initializing_and_releases_quota_only_after_cleanup() {
    let (_directory, mut journal, activation) = setup();
    assert_eq!(
        journal.abort_due_startups(1_000, &mut NeverCrash).unwrap(),
        1
    );
    let mut calls = Vec::new();
    finish_compensating_create_v1(&mut journal, SANDBOX, CREATE_OP, &mut |action| {
        assert!(!matches!(
            action.command.command(),
            core::Command::ActivateVm { .. } | core::Command::PrepareVm { .. }
        ));
        calls.push(action.command.command().clone());
        worker(action)
    })
    .unwrap();
    assert_reclaimed(&journal, &activation.command);
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, core::Command::StopVm { .. }))
            .count(),
        1
    );
    let before = journal.world().unwrap();
    assert!(matches!(
        journal.complete(&success(&activation), &mut NeverCrash),
        Err(JournalError::ObsoleteReceipt)
    ));
    assert_eq!(journal.world().unwrap(), before);
    let core::ControllerInput::Request(core::Request::Create(mut request)) = create() else {
        unreachable!()
    };
    request.sandbox = core::SandboxId(11);
    request.workspace = core::FsId(11);
    request.operation = core::OperationId(102);
    journal
        .admit_create(request, 2_000, 300, 2_000, limits(), &mut NeverCrash)
        .unwrap();
}

#[test]
fn uncertain_or_mismatched_stop_keeps_activation_terminal_and_quota_held() {
    let (_directory, mut journal, activation) = setup();
    journal.abort_due_startups(1_000, &mut NeverCrash).unwrap();
    let mut stopped = None;
    assert!(matches!(
        finish_compensating_create_v1(&mut journal, SANDBOX, CREATE_OP, &mut |action| {
            if matches!(action.command.command(), core::Command::StopVm { .. }) {
                stopped = Some(action.clone());
                return Ok(unknown(action));
            }
            worker(action)
        }),
        Err(JournalError::UnresolvedAction)
    ));
    let stop = stopped.unwrap();
    let before = journal.world().unwrap();
    assert!(matches!(
        journal.apply(kill(1), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(
        before.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Revoking
    );
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Effect(core::EffectInput::RetireActivation {
                activation: activation.command.clone(),
                stop: stop.command.clone(),
            }),
            &mut NeverCrash
        ),
        Err(JournalError::UnresolvedAction)
    ));
    let mut wrong = success(&stop);
    wrong.identity.owner = "2".try_into().unwrap();
    assert!(journal.complete(&wrong, &mut NeverCrash).is_err());
    assert_eq!(journal.world().unwrap(), before);
    let core::ControllerInput::Request(core::Request::Create(mut request)) = create() else {
        unreachable!()
    };
    request.sandbox = core::SandboxId(11);
    request.operation = core::OperationId(102);
    assert!(matches!(
        journal.admit_create(request, 2_000, 300, 2_000, limits(), &mut NeverCrash),
        Err(AdmissionError::QuotaExceeded)
    ));
    journal.complete(&success(&stop), &mut NeverCrash).unwrap();
    finish_compensating_create_v1(&mut journal, SANDBOX, CREATE_OP, &mut worker).unwrap();
    assert_reclaimed(&journal, &activation.command);
}

#[test]
fn retirement_transaction_is_atomic_replayable_and_preserves_missing_receipt() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let (directory, mut journal, activation) = setup();
        journal.abort_due_startups(1_000, &mut NeverCrash).unwrap();
        let stop = schedule_stop(&mut journal);
        assert_eq!(dispatch_all(&mut journal), 1);
        let input = core::ControllerInput::Effect(core::EffectInput::RetireActivation {
            activation: activation.command.clone(),
            stop,
        });
        assert!(matches!(
            journal.apply(input.clone(), &mut CrashAt(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(journal);
        let mut reopened =
            ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
        let retired = matches!(
            reopened.world().unwrap().effect_status(&activation.command),
            Some(core::EffectStatus::RetiredByStop(_))
        );
        assert_eq!(retired, point == CrashPoint::AfterCommit);
        reopened.apply(input, &mut NeverCrash).unwrap();
        finish_compensating_create_v1(&mut reopened, SANDBOX, CREATE_OP, &mut worker).unwrap();
        assert_reclaimed(&reopened, &activation.command);
        let before = reopened.world().unwrap();
        finish_compensating_create_v1(&mut reopened, SANDBOX, CREATE_OP, &mut |_action| -> Result<
            WorkReceipt<FakeHostEvidence>,
            _,
        > {
            panic!("completed recovery must not execute host work twice")
        })
        .unwrap();
        assert_eq!(reopened.world().unwrap(), before);
        drop(reopened);
        let database = rusqlite::Connection::open(directory.path().join("controller.db")).unwrap();
        let receipts: Vec<Vec<u8>> = database.prepare("SELECT receipt_json FROM outbox WHERE CAST(action_json AS TEXT) LIKE '%ActivateVm%'").unwrap()
            .query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        // The historical unknown receipt is not replaced by fake success.
        assert_eq!(receipts.len(), 1);
        let receipt: WorkReceipt<FakeHostEvidence> = serde_json::from_slice(&receipts[0]).unwrap();
        assert!(matches!(receipt.outcome, WorkOutcome::Unknown { .. }));
        assert_eq!(
            database
                .query_row("SELECT started_at_ms FROM sandbox_lifetimes", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn restart_recovers_terminal_fence_stop_and_cleanup_windows_without_second_activation() {
    for target in ["fence", "stop", "cleanup"] {
        let (directory, mut journal, activation) = setup();
        journal.abort_due_startups(1_000, &mut NeverCrash).unwrap();
        let mut lost = None;
        assert!(matches!(
            finish_compensating_create_v1(&mut journal, SANDBOX, CREATE_OP, &mut |action| {
                let selected = matches!(
                    (target, action.command.command()),
                    ("fence", core::Command::InstallFence { terminal: true, .. })
                        | ("stop", core::Command::StopVm { .. })
                        | ("cleanup", core::Command::CleanupVm { .. })
                );
                if selected {
                    lost = Some(action.clone());
                    return Ok(unknown(action));
                }
                worker(action)
            }),
            Err(JournalError::UnresolvedAction)
        ));
        drop(journal);
        let mut reopened =
            ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
        reopened
            .complete(&success(&lost.unwrap()), &mut NeverCrash)
            .unwrap();
        finish_compensating_create_v1(&mut reopened, SANDBOX, CREATE_OP, &mut |action| {
            assert!(!matches!(
                action.command.command(),
                core::Command::ActivateVm { .. } | core::Command::PrepareVm { .. }
            ));
            worker(action)
        })
        .unwrap();
        assert_reclaimed(&reopened, &activation.command);
    }
}

#[test]
fn version_six_unknown_activation_survives_upgrade_without_rewriting_history() {
    let (directory, journal, activation) = setup();
    let before = journal.world().unwrap();
    drop(journal);
    let path = directory.path().join("controller.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE controller_meta SET version=6", [])
        .unwrap();
    drop(connection);
    let mut reopened = ControllerJournal::open(&path, NAMESPACE).unwrap();
    assert_eq!(reopened.world().unwrap(), before);
    assert_eq!(
        reopened.uncertain_actions().unwrap(),
        vec![activation.clone()]
    );
    reopened.abort_due_startups(1_000, &mut NeverCrash).unwrap();
    finish_compensating_create_v1(&mut reopened, SANDBOX, CREATE_OP, &mut worker).unwrap();
    assert_reclaimed(&reopened, &activation.command);
    drop(reopened);
    let connection = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT version FROM controller_meta", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        7
    );
}

#[test]
fn terminal_intent_is_atomic_and_late_activation_cannot_complete_it() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let (directory, mut journal, activation) = setup();
        assert!(
            journal
                .abort_due_startups(1_000, &mut CrashAt(point))
                .is_err()
        );
        drop(journal);
        let mut reopened =
            ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
        assert_eq!(
            reopened
                .world()
                .unwrap()
                .sandbox_record(SANDBOX)
                .unwrap()
                .terminal,
            point == CrashPoint::AfterCommit
        );
        reopened.abort_due_startups(1_000, &mut NeverCrash).unwrap();
        let before = reopened.world().unwrap();
        assert!(matches!(
            reopened.complete(&success(&activation), &mut NeverCrash),
            Err(JournalError::ObsoleteReceipt)
        ));
        assert_eq!(reopened.world().unwrap(), before);
        finish_compensating_create_v1(&mut reopened, SANDBOX, CREATE_OP, &mut worker).unwrap();
        assert_reclaimed(&reopened, &activation.command);
    }
}
