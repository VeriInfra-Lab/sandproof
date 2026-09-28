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

fn limits() -> AdmissionLimits {
    AdmissionLimits {
        max_active_global: 2,
        max_active_per_tenant: 1,
        max_disk_bytes_global: 3_000,
        max_disk_bytes_per_tenant: 1_000,
        cleanup_reserve_bytes: 1_000,
        max_timeout_secs: 86_400,
        max_total_lifetime_secs: 86_400,
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
    let shortened = journal.set_lifetime(&owner, sandbox, 12_000, 5).unwrap();
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
fn default_connect_and_total_lifetime_survive_restart_without_restarting_age() {
    let (directory, mut journal) = setup();
    let sandbox = core::SandboxId(10);
    let owner = core::Actor::Tenant(core::TenantId(1));
    let mut policy = limits();
    policy.max_timeout_secs = 300;
    policy.max_total_lifetime_secs = 600;
    journal
        .admit_create(request(10, 1), 1_000, 300, 1_000, policy, &mut NeverCrash)
        .unwrap();
    running(&mut journal, sandbox, core::OperationId(100));
    assert_eq!(
        journal
            .live_lifetime(&owner, sandbox, 2_000)
            .unwrap()
            .max_end_ms,
        601_000
    );
    assert_eq!(
        journal
            .connect_lifetime(&owner, sandbox, 100_000, None, policy)
            .unwrap()
            .deadline_ms,
        400_000
    );
    assert_eq!(
        journal
            .connect_lifetime(&owner, sandbox, 350_000, None, policy)
            .unwrap()
            .deadline_ms,
        601_000
    );
    assert!(matches!(
        journal.connect_lifetime(&owner, sandbox, 350_000, Some(301), policy),
        Err(AdmissionError::InvalidTimeout)
    ));
    drop(journal);
    let mut reopened =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let preserved = reopened.live_lifetime(&owner, sandbox, 590_000).unwrap();
    assert_eq!(
        (preserved.deadline_ms, preserved.max_end_ms),
        (601_000, 601_000)
    );
    assert_eq!(
        reopened
            .set_lifetime(&owner, sandbox, 590_000, i64::MAX)
            .unwrap()
            .deadline_ms,
        601_000
    );
    assert_eq!(
        reopened
            .connect_lifetime(&owner, sandbox, 590_000, None, policy)
            .unwrap()
            .deadline_ms,
        601_000
    );
    assert!(matches!(
        reopened.connect_lifetime(&owner, sandbox, 601_001, None, policy),
        Err(AdmissionError::InvalidTimeout)
    ));
    assert_eq!(reopened.due_lifetimes(601_000).unwrap(), vec![sandbox]);
}

#[test]
fn nonpositive_timeout_revokes_authority_before_physical_expiry_finishes() {
    for value in [0, -5] {
        let (_directory, mut journal) = setup();
        let sandbox = core::SandboxId(10);
        let owner = core::Actor::Tenant(core::TenantId(1));
        journal
            .admit_create(request(10, 1), 1_000, 300, 1_000, limits(), &mut NeverCrash)
            .unwrap();
        running(&mut journal, sandbox, core::OperationId(100));
        let result = journal.set_lifetime(&owner, sandbox, 2_000, value).unwrap();
        assert_eq!(result.deadline_ms, 2_000);
        assert!(matches!(
            journal.live_lifetime(&owner, sandbox, 2_000),
            Err(AdmissionError::Expired)
        ));
        assert!(matches!(
            journal.connect_lifetime(&owner, sandbox, 2_000, None, limits()),
            Err(AdmissionError::Expired)
        ));
        assert_eq!(journal.due_lifetimes(2_000).unwrap(), vec![sandbox]);
        journal
            .accept_expiry(sandbox, core::OperationId(101), 2_000, &mut NeverCrash)
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
    }
}

#[test]
fn initial_lease_cannot_exceed_total_budget_even_with_larger_single_call_limit() {
    let (_directory, mut journal) = setup();
    let mut policy = limits();
    policy.max_timeout_secs = 600;
    policy.max_total_lifetime_secs = 300;
    assert!(matches!(
        journal.admit_create(request(10, 1), 1_000, 301, 1_000, policy, &mut NeverCrash),
        Err(AdmissionError::InvalidTimeout)
    ));
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(10))
            .is_none()
    );
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
fn version_four_lease_migrates_without_inventing_new_total_life() {
    let (directory, mut journal) = setup();
    journal
        .admit_create(request(10, 1), 1_000, 300, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    running(&mut journal, core::SandboxId(10), core::OperationId(100));
    drop(journal);
    let path = directory.path().join("controller.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch(
        "CREATE TABLE legacy_lifetimes (sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id), owner_id TEXT NOT NULL, deadline_ms INTEGER NOT NULL, disk_bytes INTEGER NOT NULL, started_at_ms INTEGER);
         INSERT INTO legacy_lifetimes SELECT sandbox_id, owner_id, deadline_ms, disk_bytes, started_at_ms FROM sandbox_lifetimes;
         DROP TABLE sandbox_lifetimes;
         ALTER TABLE legacy_lifetimes RENAME TO sandbox_lifetimes;
         UPDATE controller_meta SET version=4 WHERE id=1;",
    ).unwrap();
    drop(connection);
    let mut migrated = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let owner = core::Actor::Tenant(core::TenantId(1));
    let sandbox = core::SandboxId(10);
    let lease = migrated.live_lifetime(&owner, sandbox, 2_000).unwrap();
    assert_eq!(lease.max_end_ms, lease.deadline_ms);
    assert_eq!(
        migrated
            .connect_lifetime(&owner, sandbox, 2_000, None, limits())
            .unwrap()
            .deadline_ms,
        lease.deadline_ms
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
