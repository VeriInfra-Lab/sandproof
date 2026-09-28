mod support;
use support::*;

use sandbox_controller_core as core;
use sandproof_controller_service::{
    AdmissionError, AdmissionLimits, ControllerJournal, CrashInjector, CrashPoint, CreateAdmission,
    CreateOptions, JournalError, NeverCrash,
};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn setup() -> (TempDir, ControllerJournal) {
    let directory = TempDir::new().unwrap();
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    (directory, journal)
}

#[test]
fn legacy_create_options_do_not_invent_original_network_parameters() {
    let options: CreateOptions = serde_json::from_str(r#"{"env_vars":{},"metadata":{}}"#).unwrap();
    assert_eq!(options.networking, None);
    assert!(
        serde_json::to_value(&options)
            .unwrap()
            .get("networking")
            .is_none()
    );
    let input = r#"{"env_vars":{},"metadata":{},"networking":{"allow_internet_access":null,"network":{"allowOut":[],"denyOut":["8.8.8.8","1.2.3.4/24"]}}}"#;
    let options: CreateOptions = serde_json::from_str(input).unwrap();
    assert_eq!(
        serde_json::to_value(options).unwrap(),
        serde_json::from_str::<serde_json::Value>(input).unwrap()
    );
}

fn limits() -> AdmissionLimits {
    AdmissionLimits {
        max_active_global: 2,
        max_active_per_tenant: 1,
        max_disk_bytes_global: 3_000,
        max_disk_bytes_per_tenant: 1_000,
        cleanup_reserve_bytes: 1_000,
        max_timeout_secs: 86_400,
    }
}

struct BeforeCommit;

impl CrashInjector for BeforeCommit {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == CrashPoint::BeforeCommit {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

fn request(sandbox: u64, tenant: u64) -> core::CreateRequest {
    let core::ControllerInput::Request(core::Request::Create(mut request)) = create() else {
        unreachable!()
    };
    request.sandbox = core::SandboxId(sandbox);
    request.owner = core::TenantId(tenant);
    request.actor = core::Actor::Tenant(request.owner);
    request.workspace = core::FsId(sandbox);
    request.operation = core::OperationId(sandbox * 10);
    request
}

#[test]
fn create_options_share_the_admission_commit_and_survive_reopen() {
    let (directory, mut journal) = setup();
    let options = CreateOptions {
        env_vars: BTreeMap::from([("TASK_ID".into(), "unit-7".into())]),
        metadata: BTreeMap::from([("job".into(), "unit-7".into())]),
        networking: Some(sandproof_controller_service::CreateNetworking {
            allow_internet_access: Some(false),
            network: None,
        }),
    };
    assert!(matches!(
        journal.admit_create_with_options(
            request(10, 1),
            CreateAdmission {
                now_ms: 1_000,
                timeout_secs: 60,
                disk_bytes: 1_000,
                limits: limits(),
                options: options.clone()
            },
            &mut BeforeCommit,
        ),
        Err(AdmissionError::Journal(JournalError::InjectedCrash(
            CrashPoint::BeforeCommit
        )))
    ));
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(10))
            .is_none()
    );
    assert_eq!(
        journal.create_options(core::SandboxId(10)).unwrap(),
        CreateOptions::default()
    );
    journal
        .admit_create_with_options(
            request(10, 1),
            CreateAdmission {
                now_ms: 1_000,
                timeout_secs: 60,
                disk_bytes: 1_000,
                limits: limits(),
                options: options.clone(),
            },
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal);
    let reopened =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(
        reopened.create_options(core::SandboxId(10)).unwrap(),
        options
    );
}

fn running(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    operation: core::OperationId,
) {
    let reserved = journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(
                core::OperationKey { sandbox, operation },
            )),
            &mut NeverCrash,
        )
        .unwrap();
    dispatch_all(journal);
    let plan = reserved
        .next
        .operation_plan(core::OperationKey { sandbox, operation })
        .unwrap();
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
                sandbox,
                plan.vm.unwrap(),
                plan.slot.unwrap(),
            )),
            &mut NeverCrash,
        )
        .unwrap();
    dispatch_all(journal);
    journal
        .apply(
            core::ControllerInput::Publication(core::PublicationInput::Running(sandbox)),
            &mut NeverCrash,
        )
        .unwrap();
}

#[test]
fn admission_is_atomic_and_reserves_cleanup_capacity() {
    let (_directory, mut journal) = setup();
    journal
        .admit_create(request(10, 1), 1_000, 60, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    let same_tenant =
        journal.admit_create(request(11, 1), 1_000, 60, 1_000, limits(), &mut NeverCrash);
    assert!(matches!(same_tenant, Err(AdmissionError::QuotaExceeded)));
    journal
        .admit_create(request(20, 2), 1_000, 60, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    let exhausted =
        journal.admit_create(request(30, 3), 1_000, 60, 1_000, limits(), &mut NeverCrash);
    assert!(matches!(exhausted, Err(AdmissionError::QuotaExceeded)));
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(30))
            .is_none()
    );
}

#[test]
fn lease_updates_and_expiry_recheck_use_the_server_clock() {
    let (_directory, mut journal) = setup();
    let sandbox = core::SandboxId(10);
    journal
        .admit_create(request(10, 1), 1_000, 30, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    running(&mut journal, sandbox, core::OperationId(100));
    let owner = core::Actor::Tenant(core::TenantId(1));
    let foreign = core::Actor::Tenant(core::TenantId(2));
    assert!(matches!(
        journal.live_lifetime(&foreign, sandbox, 2_000),
        Err(AdmissionError::Unauthorized)
    ));
    let unchanged = journal
        .connect_lifetime(&owner, sandbox, 10_000, Some(10), limits())
        .unwrap();
    assert_eq!(unchanged.deadline_ms, 31_000);
    let extended = journal
        .connect_lifetime(&owner, sandbox, 10_000, Some(60), limits())
        .unwrap();
    assert_eq!(extended.deadline_ms, 70_000);
    assert!(matches!(
        journal.accept_expiry(sandbox, core::OperationId(101), 31_000, &mut NeverCrash),
        Err(AdmissionError::NotDue)
    ));
    assert!(journal.due_lifetimes(31_000).unwrap().is_empty());
    let shortened = journal
        .set_lifetime(&owner, sandbox, 12_000, 5, limits())
        .unwrap();
    assert_eq!(shortened.deadline_ms, 17_000);
    assert_eq!(journal.due_lifetimes(17_000).unwrap(), vec![sandbox]);
    assert!(matches!(
        journal.live_lifetime(&core::Actor::Tenant(core::TenantId(1)), sandbox, 17_000),
        Err(AdmissionError::Expired)
    ));
    journal
        .accept_expiry(sandbox, core::OperationId(101), 17_000, &mut NeverCrash)
        .unwrap();
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Revoking
    );
    assert!(matches!(
        journal.accept_expiry(sandbox, core::OperationId(102), 17_000, &mut NeverCrash),
        Err(AdmissionError::NotRunning)
    ));
}

#[test]
fn invalid_timeout_and_crash_before_commit_never_consume_quota() {
    let (_directory, mut journal) = setup();
    assert!(matches!(
        journal.admit_create(request(10, 1), 1_000, 0, 1_000, limits(), &mut NeverCrash),
        Err(AdmissionError::InvalidTimeout)
    ));
    assert!(matches!(
        journal.admit_create(
            request(10, 1),
            i64::MAX - 1,
            10,
            1_000,
            limits(),
            &mut NeverCrash
        ),
        Err(AdmissionError::TimeOverflow)
    ));
    assert!(matches!(
        journal.admit_create(
            request(10, 1),
            1_000,
            60,
            1_000,
            limits(),
            &mut BeforeCommit
        ),
        Err(AdmissionError::Journal(JournalError::InjectedCrash(
            CrashPoint::BeforeCommit
        )))
    ));
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(10))
            .is_none()
    );
    journal
        .admit_create(request(10, 1), 1_000, 60, 1_000, limits(), &mut NeverCrash)
        .unwrap();
}

#[test]
fn unmanaged_old_active_state_fails_closed() {
    let (_directory, mut journal) = setup();
    apply(&mut journal, create());
    assert!(matches!(
        journal.admit_create(request(20, 2), 1_000, 60, 1_000, limits(), &mut NeverCrash),
        Err(AdmissionError::UnmanagedActive)
    ));
}

#[test]
fn version_one_store_migrates_without_inventing_leases() {
    let (directory, journal) = setup();
    drop(journal);
    let path = directory.path().join("controller.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE sandbox_lifetimes; UPDATE controller_meta SET version=1 WHERE id=1;",
        )
        .unwrap();
    drop(connection);
    let mut migrated = ControllerJournal::open(&path, NAMESPACE).unwrap();
    migrated
        .admit_create(request(10, 1), 1_000, 60, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    drop(migrated);
    let reopened = ControllerJournal::open(&path, NAMESPACE).unwrap();
    assert!(
        reopened
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(10))
            .is_some()
    );
}

#[test]
fn destroyed_sandbox_keeps_disk_budget_until_exact_cleanup_receipts() {
    let (_directory, mut journal) = setup();
    let mut one = limits();
    one.max_active_global = 1;
    journal
        .admit_create(request(10, 1), 1_000, 60, 1_000, one, &mut NeverCrash)
        .unwrap();
    running(&mut journal, core::SandboxId(10), core::OperationId(100));
    apply(&mut journal, kill(1));
    dispatch_all(&mut journal);
    let world = journal.world().unwrap();
    let record = world.sandbox_record(core::SandboxId(10)).unwrap();
    for vm in &world
        .operation_plan(core::OperationKey {
            sandbox: core::SandboxId(10),
            operation: core::OperationId(101),
        })
        .unwrap()
        .stop_vms
    {
        journal
            .apply(
                core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
                    core::SandboxId(10),
                    record.generation,
                    *vm,
                )),
                &mut NeverCrash,
            )
            .unwrap();
        dispatch_all(&mut journal);
    }
    journal
        .apply(
            core::ControllerInput::Publication(core::PublicationInput::Destroyed(core::SandboxId(
                10,
            ))),
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(
        journal.admit_create(request(20, 2), 2_000, 60, 1_000, one, &mut NeverCrash),
        Err(AdmissionError::QuotaExceeded)
    ));
    let resources = journal.world().unwrap().resource_index(core::SandboxId(10));
    for vm in resources.vms {
        journal
            .apply(
                core::ControllerInput::Scheduler(core::SchedulerInput::CleanupVm(
                    core::SandboxId(10),
                    1,
                    vm,
                )),
                &mut NeverCrash,
            )
            .unwrap();
        dispatch_all(&mut journal);
    }
    for slot in resources.slots {
        journal
            .apply(
                core::ControllerInput::Scheduler(core::SchedulerInput::CleanupSlot(
                    core::SandboxId(10),
                    1,
                    slot,
                )),
                &mut NeverCrash,
            )
            .unwrap();
        dispatch_all(&mut journal);
    }
    journal
        .admit_create(request(20, 2), 2_000, 60, 1_000, one, &mut NeverCrash)
        .unwrap();
}
