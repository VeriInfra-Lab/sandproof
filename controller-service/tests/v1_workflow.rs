mod support;
use support::*;

use sandbox_controller_core as core;
use sandproof_controller_service::{ControllerJournal, JournalError, create_v1, kill_v1};
use sandproof_protocol::{UnknownReason, WorkAction, WorkOutcome, WorkReceipt};
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
