use sandbox_controller_core as core;
use sandproof_controller_service::{
    ControllerJournal, CrashInjector, CrashPoint, DispatchResult, HostEvidence, JournalError,
    NeverCrash, WorkerTransportError,
};
use sandproof_protocol::{
    FailureCode, ResourceEvidence, ResourceRef, UnknownReason, WorkAction, WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

const NAMESPACE: &str = "0123456789abcdef";
const SANDBOX: core::SandboxId = core::SandboxId(10);
const CREATE_OP: core::OperationId = core::OperationId(100);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FakeHostEvidence {
    resource: ResourceRef,
    command: core::Command,
    established: bool,
}

impl ResourceEvidence for FakeHostEvidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

impl HostEvidence for FakeHostEvidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.established && self.command == *command
    }
}

fn success(action: &WorkAction<core::EffectId>) -> WorkReceipt<FakeHostEvidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(FakeHostEvidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            established: true,
        }),
    }
}

// Keep the production transport signature for this deliberately infallible fake.
#[allow(clippy::unnecessary_wraps)]
fn worker(
    action: &WorkAction<core::EffectId>,
) -> Result<WorkReceipt<FakeHostEvidence>, WorkerTransportError> {
    Ok(success(action))
}

fn create() -> core::ControllerInput {
    core::ControllerInput::Request(core::Request::Create(core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: SANDBOX,
        workspace: core::FsId(10),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: CREATE_OP,
    }))
}

fn kill(owner: u64) -> core::ControllerInput {
    core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
        actor: core::Actor::Tenant(core::TenantId(owner)),
        sandbox: SANDBOX,
        operation: core::OperationId(101),
    }))
}

fn setup() -> (TempDir, ControllerJournal) {
    let directory = TempDir::new().unwrap();
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    (directory, journal)
}

fn apply(journal: &mut ControllerJournal, input: core::ControllerInput) -> core::World {
    journal.apply(input, &mut NeverCrash).unwrap().next
}

fn dispatch_all(journal: &mut ControllerJournal) -> usize {
    let mut count = 0;
    loop {
        match journal.dispatch_next(&mut worker, &mut NeverCrash).unwrap() {
            DispatchResult::NoReadyAction => return count,
            DispatchResult::ReceiptRecorded(_) => count += 1,
            DispatchResult::OutcomeUnknown { .. } => panic!("fake worker should return evidence"),
        }
        assert!(count < 20);
    }
}

fn prepare_ready(journal: &mut ControllerJournal) -> core::OperationPlan {
    apply(journal, create());
    let key = core::OperationKey {
        sandbox: SANDBOX,
        operation: CREATE_OP,
    };
    let world = apply(
        journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(key)),
    );
    let plan = world.operation_plan(key).unwrap().clone();
    assert_eq!(dispatch_all(journal), 1); // installation fence
    apply(
        journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
            SANDBOX,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    plan
}

struct FailAt(CrashPoint);
impl CrashInjector for FailAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if self.0 == point {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn real_reducer_drives_create_kill_and_replays_the_same_world() {
    let (directory, mut journal) = setup();
    let plan = prepare_ready(&mut journal);
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Publication(core::PublicationInput::Running(SANDBOX)),
            &mut NeverCrash
        ),
        Err(JournalError::ControllerRejected(_))
    ));
    assert_eq!(dispatch_all(&mut journal), 4); // prepare, network, activate, file policy
    let running = apply(
        &mut journal,
        core::ControllerInput::Publication(core::PublicationInput::Running(SANDBOX)),
    );
    assert_eq!(
        running.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Running
    );
    assert!(
        journal
            .check(core::ControllerInput::Access(core::AccessInput::Exec(
                core::Actor::Tenant(core::TenantId(1)),
                SANDBOX,
                plan.vm.unwrap(),
            )))
            .is_ok()
    );
    assert!(matches!(
        journal.apply(kill(2), &mut NeverCrash),
        Err(JournalError::ControllerRejected(_))
    ));
    assert_eq!(journal.world().unwrap(), running);
    apply(&mut journal, kill(1));
    assert_eq!(dispatch_all(&mut journal), 1); // terminal fence
    assert!(
        journal
            .check(core::ControllerInput::Access(core::AccessInput::Exec(
                core::Actor::Tenant(core::TenantId(1)),
                SANDBOX,
                plan.vm.unwrap(),
            )))
            .is_err()
    );
    apply(
        &mut journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
            SANDBOX,
            1,
            plan.vm.unwrap(),
        )),
    );
    assert_eq!(dispatch_all(&mut journal), 1);
    let destroyed = apply(
        &mut journal,
        core::ControllerInput::Publication(core::PublicationInput::Destroyed(SANDBOX)),
    );
    assert_eq!(
        destroyed.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Destroyed
    );
    apply(
        &mut journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::CleanupVm(
            SANDBOX,
            1,
            plan.vm.unwrap(),
        )),
    );
    apply(
        &mut journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::CleanupSlot(
            SANDBOX,
            1,
            plan.slot.unwrap(),
        )),
    );
    assert_eq!(dispatch_all(&mut journal), 2);
    let cleaned = journal.world().unwrap();
    drop(journal);
    let reopened =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(reopened.world().unwrap(), cleaned);
    assert!(reopened.uncertain_actions().unwrap().is_empty());
}

#[test]
fn explicit_failure_retry_keeps_logical_identity_and_allocates_new_attempt() {
    let (_directory, mut journal) = setup();
    apply(&mut journal, create());
    let mut failed_action = None;
    journal
        .dispatch_next(
            &mut |action: &WorkAction<core::EffectId>| {
                failed_action = Some(action.clone());
                Ok(WorkReceipt::<FakeHostEvidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::DefinitiveFailure {
                        code: FailureCode::ResourceUnavailable,
                        retryable: true,
                        partial: None,
                    },
                })
            },
            &mut NeverCrash,
        )
        .unwrap();
    let failed = failed_action.unwrap();
    assert_eq!(dispatch_all(&mut journal), 0);
    apply(
        &mut journal,
        core::ControllerInput::Effect(core::EffectInput::Retry(
            failed.command.logical.clone(),
            core::FailureKind::ResourceFailure,
        )),
    );
    let mut retry = None;
    journal
        .dispatch_next(
            &mut |action: &WorkAction<core::EffectId>| {
                retry = Some(action.clone());
                worker(action)
            },
            &mut NeverCrash,
        )
        .unwrap();
    let retry = retry.unwrap();
    assert_eq!(
        retry.identity.logical_action,
        failed.identity.logical_action
    );
    assert_eq!(retry.identity.attempt, failed.identity.attempt + 1);
    assert_eq!(retry.command.logical, failed.command.logical);
    assert_eq!(dispatch_all(&mut journal), 0);
}

#[test]
fn before_commit_failure_leaves_no_core_state_or_worker_action() {
    let (_directory, mut journal) = setup();
    assert!(matches!(
        journal.apply(create(), &mut FailAt(CrashPoint::BeforeCommit)),
        Err(JournalError::InjectedCrash(_))
    ));
    assert_eq!(journal.world().unwrap(), core::initial_world());
    assert_eq!(dispatch_all(&mut journal), 0);
    apply(&mut journal, create());
    assert_eq!(dispatch_all(&mut journal), 1);
}

#[test]
fn kill_before_startup_dispatch_never_sends_the_old_fence_to_the_host() {
    let (_directory, mut journal) = setup();
    apply(&mut journal, create());
    apply(&mut journal, kill(1));
    let mut commands = Vec::new();
    loop {
        let result = journal
            .dispatch_next(
                &mut |action: &WorkAction<core::EffectId>| {
                    commands.push(action.command.command().clone());
                    worker(action)
                },
                &mut NeverCrash,
            )
            .unwrap();
        if matches!(result, DispatchResult::NoReadyAction) {
            break;
        }
        assert!(commands.len() < 3);
    }
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0],
        core::Command::InstallFence { terminal: true, .. }
    ));
    let world = apply(
        &mut journal,
        core::ControllerInput::Publication(core::PublicationInput::Destroyed(SANDBOX)),
    );
    assert_eq!(
        world.sandbox_record(SANDBOX).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(dispatch_all(&mut journal), 0);
}

#[test]
fn incompatible_retry_flag_is_rejected_without_granting_a_new_attempt() {
    let (_directory, mut journal) = setup();
    apply(&mut journal, create());
    let result = journal.dispatch_next(
        &mut |action: &WorkAction<core::EffectId>| {
            Ok(WorkReceipt::<FakeHostEvidence> {
                identity: action.identity.clone(),
                outcome: WorkOutcome::DefinitiveFailure {
                    code: FailureCode::ResourceUnavailable,
                    retryable: false,
                    partial: None,
                },
            })
        },
        &mut NeverCrash,
    );
    assert!(matches!(result, Err(JournalError::InvalidHostEvidence)));
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    assert_eq!(dispatch_all(&mut journal), 0);
    assert!(matches!(
        journal.apply(kill(1), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
}

#[test]
fn wrong_resource_and_conflicting_final_receipts_are_rejected() {
    let (_directory, mut journal) = setup();
    apply(&mut journal, create());
    let _ = journal.dispatch_next(&mut worker, &mut FailAt(CrashPoint::AfterDispatchClaim));
    let action = journal.uncertain_actions().unwrap().pop().unwrap();
    let receipt = success(&action);
    let mut wrong = receipt.clone();
    if let WorkOutcome::ObservedSuccess(evidence) = &mut wrong.outcome {
        evidence.resource.id = "wrong-resource".try_into().unwrap();
    }
    let before = journal.world().unwrap();
    assert!(journal.complete(&wrong, &mut NeverCrash).is_err());
    assert_eq!(journal.world().unwrap(), before);
    journal.complete(&receipt, &mut NeverCrash).unwrap();
    let failed = WorkReceipt::<FakeHostEvidence> {
        identity: action.identity,
        outcome: WorkOutcome::DefinitiveFailure {
            code: FailureCode::PolicyRejected,
            retryable: false,
            partial: None,
        },
    };
    assert!(matches!(
        journal.complete(&failed, &mut NeverCrash),
        Err(JournalError::ConflictingReceipt)
    ));
}

#[test]
fn exclusive_process_lease_and_namespace_are_enforced() {
    let (directory, journal) = setup();
    let path = directory.path().join("controller.db");
    assert!(matches!(
        ControllerJournal::open(&path, NAMESPACE),
        Err(JournalError::ControllerBusy)
    ));
    drop(journal);
    assert!(matches!(
        ControllerJournal::open(&path, "1111111111111111"),
        Err(JournalError::UnsupportedStore)
    ));
    assert!(ControllerJournal::open(&path, NAMESPACE).is_ok());
}

#[test]
fn crash_after_claim_requires_observation_before_lifecycle_can_advance() {
    let (directory, mut journal) = setup();
    apply(&mut journal, create());
    let mut called = false;
    let result = journal.dispatch_next(
        &mut |action| {
            called = true;
            worker(action)
        },
        &mut FailAt(CrashPoint::AfterDispatchClaim),
    );
    assert!(matches!(result, Err(JournalError::InjectedCrash(_))));
    assert!(!called);
    drop(journal);
    let mut journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(dispatch_all(&mut journal), 0);
    assert!(matches!(
        journal.apply(kill(1), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    let actions = journal.uncertain_actions().unwrap();
    assert_eq!(actions.len(), 1);
    let receipt = success(&actions[0]);
    let resolved = journal.complete(&receipt, &mut NeverCrash).unwrap();
    assert_eq!(
        journal.complete(&receipt, &mut NeverCrash).unwrap(),
        resolved
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
    assert_eq!(dispatch_all(&mut journal), 0);
}

#[test]
fn explicit_unknown_can_be_resolved_without_reexecuting() {
    let (_directory, mut journal) = setup();
    apply(&mut journal, create());
    let result = journal
        .dispatch_next(
            &mut |action: &WorkAction<core::EffectId>| {
                Ok(WorkReceipt::<FakeHostEvidence> {
                    identity: action.identity.clone(),
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
    let pending = journal.uncertain_actions().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(dispatch_all(&mut journal), 0);
    journal
        .complete(&success(&pending[0]), &mut NeverCrash)
        .unwrap();
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[test]
fn success_without_command_evidence_does_not_change_resource_state() {
    let (_directory, mut journal) = setup();
    apply(&mut journal, create());
    let result = journal.dispatch_next(
        &mut |action| {
            let mut receipt = success(action);
            if let WorkOutcome::ObservedSuccess(evidence) = &mut receipt.outcome {
                evidence.established = false;
            }
            Ok(receipt)
        },
        &mut NeverCrash,
    );
    assert!(matches!(result, Err(JournalError::InvalidHostEvidence)));
    let pending = journal.uncertain_actions().unwrap();
    assert_eq!(pending.len(), 1);
    let world = journal.world().unwrap();
    assert!(!core::effect::command_result_present(
        &world,
        pending[0].command.command()
    ));
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Worker(pending[0].command.clone()),
            &mut NeverCrash
        ),
        Err(JournalError::UnsupportedInput)
    ));
}

#[test]
fn lost_prepare_reply_never_creates_followup_actions_until_reconciled() {
    let (directory, mut journal) = setup();
    let _plan = prepare_ready(&mut journal);
    let mut saved = None;
    let result = journal.dispatch_next(
        &mut |action| {
            saved = Some(success(action));
            worker(action)
        },
        &mut FailAt(CrashPoint::AfterWorkerCall),
    );
    assert!(matches!(result, Err(JournalError::InjectedCrash(_))));
    drop(journal);
    let mut journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(dispatch_all(&mut journal), 0);
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(SANDBOX)
            .unwrap()
            .current_vm
            .is_none()
    );
    let receipt = saved.unwrap();
    assert!(matches!(
        journal.complete(&receipt, &mut FailAt(CrashPoint::BeforeCommit)),
        Err(JournalError::InjectedCrash(_))
    ));
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(SANDBOX)
            .unwrap()
            .current_vm
            .is_none()
    );
    journal.complete(&receipt, &mut NeverCrash).unwrap();
    assert_eq!(dispatch_all(&mut journal), 3);
    apply(
        &mut journal,
        core::ControllerInput::Publication(core::PublicationInput::Running(SANDBOX)),
    );
}
