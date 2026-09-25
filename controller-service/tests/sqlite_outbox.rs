use sandproof_controller_service::{
    AcceptedWrite, CrashInjector, CrashPoint, DispatchResult, DispatchState, Journal, JournalError,
    NeverCrash, RecordedReceipt, WorkerTransportError,
};
use sandproof_protocol::{
    ActionIdentity, FailureCode, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceEvidence,
    ResourceId, ResourceKind, ResourceRef, SandboxId, TenantId, UnknownReason, WorkAction,
    WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tempfile::TempDir;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum TestCommand {
    StartVm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Observation {
    resource: ResourceRef,
    running: bool,
}

impl ResourceEvidence for Observation {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

struct FailAt(CrashPoint);

impl CrashInjector for FailAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

fn id<T: for<'a> TryFrom<&'a str>>(value: &str) -> T
where
    for<'a> <T as TryFrom<&'a str>>::Error: std::fmt::Debug,
{
    T::try_from(value).unwrap()
}

fn action() -> WorkAction<TestCommand> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: id::<TenantId>("tenant-a"),
            sandbox: id::<SandboxId>("sandbox-a"),
            operation: id::<OperationId>("operation-a"),
            epoch: 1,
            generation: 1,
            logical_action: id::<LogicalActionId>("start-vm"),
            attempt: 0,
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: id::<ResourceId>("vm-a"),
            },
        },
        command: TestCommand::StartVm,
    }
}

fn accepted(expected_revision: Option<u64>) -> AcceptedWrite<TestCommand> {
    AcceptedWrite {
        owner: id::<TenantId>("tenant-a"),
        sandbox: id::<SandboxId>("sandbox-a"),
        operation: id::<OperationId>("operation-a"),
        expected_revision,
        sandbox_state: b"starting".to_vec(),
        operation_state: b"pending".to_vec(),
        actions: if expected_revision.is_none() {
            vec![action()]
        } else {
            vec![]
        },
    }
}

fn success_receipt(action: &WorkAction<TestCommand>) -> WorkReceipt<Observation> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(Observation {
            resource: action.identity.resource.clone(),
            running: true,
        }),
    }
}

fn journal_path(directory: &TempDir) -> impl AsRef<Path> {
    directory.path().join("controller.sqlite3")
}

#[test]
fn sqlite_settings_are_pinned_and_observable() {
    let directory = TempDir::new().unwrap();
    let journal = Journal::open(journal_path(&directory)).unwrap();
    let settings = journal.storage_settings().unwrap();
    assert!(!settings.sqlite_version.is_empty());
    assert_eq!(settings.journal_mode, "wal");
    assert_eq!(settings.synchronous, 2);
    assert!(settings.foreign_keys);
    assert!(!settings.trusted_schema);
}

#[test]
fn crash_before_commit_rolls_back_every_row_and_never_calls_worker() {
    let directory = TempDir::new().unwrap();
    let path = journal_path(&directory);
    let mut journal = Journal::open(&path).unwrap();
    assert!(matches!(
        journal.commit(&accepted(None), &mut FailAt(CrashPoint::BeforeCommit)),
        Err(JournalError::InjectedCrash(CrashPoint::BeforeCommit))
    ));
    drop(journal);

    let mut journal = Journal::open(&path).unwrap();
    assert!(
        journal
            .sandbox(&id::<SandboxId>("sandbox-a"))
            .unwrap()
            .is_none()
    );
    assert!(
        journal
            .operation(&id::<OperationId>("operation-a"))
            .unwrap()
            .is_none()
    );
    let mut calls = 0;
    let result = journal
        .dispatch_next::<TestCommand, Observation, _>(
            &mut |_| {
                calls += 1;
                Err(WorkerTransportError::ConnectionLost)
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert_eq!(result, DispatchResult::NoReadyAction);
    assert_eq!(calls, 0);
}

#[test]
fn crash_after_commit_recovers_accepted_state_and_dispatches_once() {
    let directory = TempDir::new().unwrap();
    let path = journal_path(&directory);
    let mut journal = Journal::open(&path).unwrap();
    assert!(matches!(
        journal.commit(&accepted(None), &mut FailAt(CrashPoint::AfterCommit)),
        Err(JournalError::InjectedCrash(CrashPoint::AfterCommit))
    ));
    drop(journal);

    let mut journal = Journal::open(&path).unwrap();
    let sandbox = journal
        .sandbox(&id::<SandboxId>("sandbox-a"))
        .unwrap()
        .unwrap();
    assert_eq!(sandbox.revision, 1);
    assert_eq!(sandbox.state, b"starting");
    assert_eq!(
        journal
            .operation(&id::<OperationId>("operation-a"))
            .unwrap()
            .unwrap()
            .state,
        b"pending"
    );
    assert_eq!(
        journal.dispatch_state(&action().identity).unwrap(),
        Some(DispatchState::Ready)
    );

    let mut calls = 0;
    let result = journal
        .dispatch_next::<TestCommand, Observation, _>(
            &mut |work| {
                calls += 1;
                Ok(success_receipt(work))
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(result, DispatchResult::ReceiptRecorded(_)));
    assert_eq!(calls, 1);
    assert_eq!(
        journal.dispatch_state(&action().identity).unwrap(),
        Some(DispatchState::Receipted)
    );
    assert_eq!(
        journal
            .dispatch_next::<TestCommand, Observation, _>(
                &mut |_| panic!("must not run a second VM"),
                &mut NeverCrash
            )
            .unwrap(),
        DispatchResult::NoReadyAction
    );
    drop(journal);
    let journal = Journal::open(&path).unwrap();
    assert_eq!(
        journal
            .recorded_receipts::<TestCommand, Observation>()
            .unwrap(),
        vec![RecordedReceipt {
            action: action(),
            receipt: success_receipt(&action())
        }]
    );
}

#[test]
fn crash_before_claim_keeps_action_ready() {
    let directory = TempDir::new().unwrap();
    let path = journal_path(&directory);
    let mut journal = Journal::open(&path).unwrap();
    journal.commit(&accepted(None), &mut NeverCrash).unwrap();
    assert!(matches!(
        journal.dispatch_next::<TestCommand, Observation, _>(
            &mut |_| panic!("claim was not committed"),
            &mut FailAt(CrashPoint::BeforeDispatchClaim)
        ),
        Err(JournalError::InjectedCrash(CrashPoint::BeforeDispatchClaim))
    ));
    drop(journal);
    let journal = Journal::open(&path).unwrap();
    assert_eq!(
        journal.dispatch_state(&action().identity).unwrap(),
        Some(DispatchState::Ready)
    );
}

#[test]
fn crash_after_claim_never_blindly_redispatches() {
    let directory = TempDir::new().unwrap();
    let path = journal_path(&directory);
    let mut journal = Journal::open(&path).unwrap();
    journal.commit(&accepted(None), &mut NeverCrash).unwrap();
    assert!(matches!(
        journal.dispatch_next::<TestCommand, Observation, _>(
            &mut |_| panic!("worker must not be called"),
            &mut FailAt(CrashPoint::AfterDispatchClaim)
        ),
        Err(JournalError::InjectedCrash(CrashPoint::AfterDispatchClaim))
    ));
    drop(journal);
    let mut journal = Journal::open(&path).unwrap();
    assert_eq!(
        journal.uncertain_actions::<TestCommand>().unwrap(),
        vec![action()]
    );
    assert_eq!(
        journal
            .dispatch_next::<TestCommand, Observation, _>(
                &mut |_| panic!("ambiguous action must not be sent again"),
                &mut NeverCrash
            )
            .unwrap(),
        DispatchResult::NoReadyAction
    );
}

#[test]
fn vm_started_but_receipt_lost_is_unknown_until_reconciled() {
    let directory = TempDir::new().unwrap();
    let path = journal_path(&directory);
    let mut journal = Journal::open(&path).unwrap();
    journal.commit(&accepted(None), &mut NeverCrash).unwrap();
    let mut calls = 0;
    assert!(matches!(
        journal.dispatch_next::<TestCommand, Observation, _>(
            &mut |work| {
                calls += 1;
                Ok(success_receipt(work))
            },
            &mut FailAt(CrashPoint::AfterWorkerCall)
        ),
        Err(JournalError::InjectedCrash(CrashPoint::AfterWorkerCall))
    ));
    assert_eq!(calls, 1);
    drop(journal);

    let mut journal = Journal::open(&path).unwrap();
    assert_eq!(
        journal.uncertain_actions::<TestCommand>().unwrap(),
        vec![action()]
    );
    assert_eq!(
        journal
            .dispatch_next::<TestCommand, Observation, _>(
                &mut |_| panic!("must reconcile actual VM before new attempt"),
                &mut NeverCrash
            )
            .unwrap(),
        DispatchResult::NoReadyAction
    );
    assert!(journal.record_receipt(&success_receipt(&action())).unwrap());
    assert!(
        journal
            .uncertain_actions::<TestCommand>()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn transport_loss_without_process_crash_stays_uncertain() {
    let directory = TempDir::new().unwrap();
    let mut journal = Journal::open(journal_path(&directory)).unwrap();
    journal.commit(&accepted(None), &mut NeverCrash).unwrap();
    let result = journal
        .dispatch_next::<TestCommand, Observation, _>(
            &mut |_| Err(WorkerTransportError::ConnectionLost),
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(result, DispatchResult::OutcomeUnknown { .. }));
    assert_eq!(
        journal.uncertain_actions::<TestCommand>().unwrap(),
        vec![action()]
    );
}

#[test]
fn explicit_unknown_receipt_is_durable_but_never_redispatched() {
    let directory = TempDir::new().unwrap();
    let path = journal_path(&directory);
    let mut journal = Journal::open(&path).unwrap();
    journal.commit(&accepted(None), &mut NeverCrash).unwrap();
    let result = journal
        .dispatch_next::<TestCommand, Observation, _>(
            &mut |work| {
                Ok(WorkReceipt {
                    identity: work.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(result, DispatchResult::ReceiptRecorded(_)));
    drop(journal);
    let mut journal = Journal::open(&path).unwrap();
    let receipts = journal
        .recorded_receipts::<TestCommand, Observation>()
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(matches!(
        &receipts[0].receipt.outcome,
        WorkOutcome::Unknown { .. }
    ));
    assert_eq!(
        journal
            .dispatch_next::<TestCommand, Observation, _>(
                &mut |_| panic!("unknown outcome must be reconciled, not reissued"),
                &mut NeverCrash
            )
            .unwrap(),
        DispatchResult::NoReadyAction
    );
}

#[test]
fn receipts_require_claim_exact_identity_and_exact_resource() {
    let directory = TempDir::new().unwrap();
    let mut journal = Journal::open(journal_path(&directory)).unwrap();
    journal.commit(&accepted(None), &mut NeverCrash).unwrap();
    assert!(matches!(
        journal.record_receipt(&success_receipt(&action())),
        Err(JournalError::ReceiptBeforeDispatch)
    ));
    journal
        .dispatch_next::<TestCommand, Observation, _>(
            &mut |_| Err(WorkerTransportError::ReplyLost),
            &mut NeverCrash,
        )
        .unwrap();

    let mut stale = success_receipt(&action());
    stale.identity.attempt = 1;
    assert!(journal.record_receipt(&stale).is_err());
    let mut wrong_resource = success_receipt(&action());
    if let WorkOutcome::ObservedSuccess(observation) = &mut wrong_resource.outcome {
        observation.resource.id = id::<ResourceId>("vm-other");
    }
    assert!(journal.record_receipt(&wrong_resource).is_err());
    assert_eq!(
        journal.dispatch_state(&action().identity).unwrap(),
        Some(DispatchState::Dispatched)
    );

    let receipt = success_receipt(&action());
    assert!(journal.record_receipt(&receipt).unwrap());
    assert!(!journal.record_receipt(&receipt).unwrap());
    let mut conflicting = receipt;
    conflicting.outcome = WorkOutcome::DefinitiveFailure {
        code: FailureCode::ExternalFailure,
        retryable: true,
        partial: None,
    };
    assert!(matches!(
        journal.record_receipt(&conflicting),
        Err(JournalError::ConflictingReceipt)
    ));
}

#[test]
fn revision_conflict_and_duplicate_action_roll_back_all_changes() {
    let directory = TempDir::new().unwrap();
    let mut journal = Journal::open(journal_path(&directory)).unwrap();
    journal.commit(&accepted(None), &mut NeverCrash).unwrap();
    assert!(matches!(
        journal.commit(&accepted(None), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let mut stale = accepted(Some(0));
    stale.sandbox_state = b"incorrect".to_vec();
    assert!(matches!(
        journal.commit(&stale, &mut NeverCrash),
        Err(JournalError::RevisionConflict)
    ));

    let mut duplicate = accepted(Some(1));
    duplicate.sandbox_state = b"incorrect".to_vec();
    duplicate.actions = vec![action()];
    assert!(matches!(
        journal.commit(&duplicate, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let sandbox = journal
        .sandbox(&id::<SandboxId>("sandbox-a"))
        .unwrap()
        .unwrap();
    assert_eq!(sandbox.revision, 1);
    assert_eq!(sandbox.state, b"starting");

    let mut next = accepted(Some(1));
    next.sandbox_state = b"running".to_vec();
    next.operation_state = b"succeeded".to_vec();
    assert_eq!(journal.commit(&next, &mut NeverCrash).unwrap(), 2);
    assert_eq!(
        journal
            .sandbox(&id::<SandboxId>("sandbox-a"))
            .unwrap()
            .unwrap()
            .state,
        b"running"
    );
    assert_eq!(
        journal
            .operation(&id::<OperationId>("operation-a"))
            .unwrap()
            .unwrap()
            .state,
        b"succeeded"
    );
}

#[test]
fn foreign_owner_or_unsupported_protocol_cannot_commit_an_action() {
    let directory = TempDir::new().unwrap();
    let mut journal = Journal::open(journal_path(&directory)).unwrap();
    let mut foreign = accepted(None);
    foreign.actions[0].identity.owner = id::<TenantId>("tenant-b");
    assert!(matches!(
        journal.commit(&foreign, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let mut unknown_version = accepted(None);
    unknown_version.actions[0].identity.protocol_version = 99;
    assert!(matches!(
        journal.commit(&unknown_version, &mut NeverCrash),
        Err(JournalError::Contract(_))
    ));
    assert!(
        journal
            .sandbox(&id::<SandboxId>("sandbox-a"))
            .unwrap()
            .is_none()
    );
}
