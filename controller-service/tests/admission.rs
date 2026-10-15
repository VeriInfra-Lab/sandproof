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
    let journal = open_complete(directory.path().join("controller.db"), NAMESPACE).unwrap();
    (directory, journal)
}

#[test]
fn omitted_create_options_do_not_invent_original_network_parameters() {
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
        max_disk_bytes_global: CLEANUP_BYTES + 2_000,
        max_disk_bytes_per_tenant: 1_000,
        cleanup_reserve_bytes: CLEANUP_BYTES,
        max_timeout_secs: 86_400,
        max_lifetime_secs: 86_400,
        startup_timeout_secs: 300,
    }
}

fn template_options() -> CreateOptions {
    use sandproof_controller_service::templates::{
        ResourceShape, TemplateArtifact, TemplateBinding,
    };
    CreateOptions {
        template: Some(TemplateBinding {
            guest: None,
            startup: None,
            template_id: "ubuntu-r1".into(),
            revision: "r1".into(),
            owner: None,
            kernel: TemplateArtifact {
                path: "/opt/images/vmlinux".into(),
                sha256: "a".repeat(64),
                size_bytes: 1024,
            },
            rootfs: TemplateArtifact {
                path: "/opt/images/rootfs.ext4".into(),
                sha256: "b".repeat(64),
                size_bytes: 1_000,
            },
            resources: ResourceShape {
                cpu_count: 2,
                memory_mib: 128,
                disk_bytes: 1_000,
                host_memory_bytes: 256 * 1024 * 1024,
            },
            envd_version: "0.9.0".into(),
        }),
        ..CreateOptions::default()
    }
}

fn template_admission(options: CreateOptions) -> CreateAdmission {
    CreateAdmission {
        now_ms: 1_000,
        timeout_secs: 60,
        disk_bytes: 1_000,
        options,
        limits: AdmissionLimits {
            max_active_global: 10,
            max_active_per_tenant: 10,
            max_disk_bytes_global: CLEANUP_BYTES + 19_000,
            max_disk_bytes_per_tenant: 20_000,
            ..limits()
        },
    }
}

fn compute_limits() -> sandproof_controller_service::templates::ComputeLimits {
    sandproof_controller_service::templates::ComputeLimits {
        max_vcpus_global: 8,
        max_vcpus_per_tenant: 8,
        max_memory_bytes_global: 1024 * 1024 * 1024,
        max_memory_bytes_per_tenant: 1024 * 1024 * 1024,
    }
}

#[test]
fn template_commit_freezes_identity_and_rejects_legacy_writers_after_reopen() {
    let (directory, mut journal) = setup();
    let options = template_options();
    assert!(
        journal
            .admit_template_create(
                request(10, 1),
                template_admission(options.clone()),
                compute_limits(),
                &mut BeforeCommit
            )
            .is_err()
    );
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    // The failed transaction must not freeze its ID or upgrade the schema.
    let connection = rusqlite::Connection::open(directory.path().join("controller.db")).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT version FROM controller_meta", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert!(matches!(
        journal.admit_template_create(
            request(10, 1),
            template_admission(options.clone()),
            compute_limits(),
            &mut AfterCommit,
        ),
        Err(AdmissionError::Journal(JournalError::InjectedCrash(
            CrashPoint::AfterCommit
        )))
    ));
    drop(journal);
    let mut reopened = open_complete(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(
        reopened.create_options(core::SandboxId(10)).unwrap(),
        options
    );
    assert_eq!(
        connection
            .query_row("SELECT version FROM controller_meta", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        8
    );
    let mut changed = options;
    changed.template.as_mut().unwrap().rootfs.sha256 = "c".repeat(64);
    assert!(matches!(
        reopened.admit_template_create(
            request(11, 1),
            template_admission(changed),
            compute_limits(),
            &mut NeverCrash
        ),
        Err(AdmissionError::InvalidCreateOptions)
    ));
    assert!(matches!(
        reopened.admit_create(request(11, 1), 1_000, 60, 1_000, limits(), &mut NeverCrash),
        Err(AdmissionError::InvalidCreateOptions)
    ));
    assert_eq!(reopened.world().unwrap().sandbox_records().len(), 1);
}

#[test]
fn startup_binding_is_frozen_and_schema_guard_never_downgrades() {
    use sandproof_controller_service::templates::TemplateStartup;
    let (directory, mut journal) = setup();
    let mut options = template_options();
    options.template.as_mut().unwrap().startup = Some(TemplateStartup {
        env_vars: BTreeMap::new(),
        user: "root".into(),
        workdir: "/".into(),
        argv: vec!["/bin/app".into(), "literal arg".into()],
    });
    journal
        .admit_template_create(
            request(10, 1),
            template_admission(options.clone()),
            compute_limits(),
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal);
    let mut journal = open_complete(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(
        journal.create_options(core::SandboxId(10)).unwrap(),
        options
    );
    let mut changed = options;
    changed
        .template
        .as_mut()
        .unwrap()
        .startup
        .as_mut()
        .unwrap()
        .argv
        .push("different".into());
    assert!(matches!(
        journal.admit_template_create(
            request(11, 1),
            template_admission(changed),
            compute_limits(),
            &mut NeverCrash
        ),
        Err(AdmissionError::InvalidCreateOptions)
    ));
    let mut legacy = template_options();
    legacy.template.as_mut().unwrap().template_id = "legacy-other".into();
    journal
        .admit_template_create(
            request(11, 1),
            template_admission(legacy),
            compute_limits(),
            &mut NeverCrash,
        )
        .unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("controller.db")).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT version FROM controller_meta", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        9
    );
}

#[test]
fn guest_identity_survives_unknown_commit_and_reopen_without_same_id_replacement() {
    let (directory, mut journal) = setup();
    let mut options = template_options();
    options.template.as_mut().unwrap().guest = Some(
        serde_json::from_str(include_str!(
            "../../tests/compat/guest-template-identity.json"
        ))
        .unwrap(),
    );
    assert!(
        journal
            .admit_template_create(
                request(10, 1),
                template_admission(options.clone()),
                compute_limits(),
                &mut BeforeCommit,
            )
            .is_err()
    );
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    assert!(matches!(
        journal.admit_template_create(
            request(10, 1),
            template_admission(options.clone()),
            compute_limits(),
            &mut AfterCommit,
        ),
        Err(AdmissionError::Journal(JournalError::InjectedCrash(
            CrashPoint::AfterCommit
        )))
    ));
    drop(journal);
    let mut journal = open_complete(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(
        journal.create_options(core::SandboxId(10)).unwrap(),
        options
    );
    for case in 0..4 {
        let mut changed = options.clone();
        let template = changed.template.as_mut().unwrap();
        let guest = template.guest.as_mut().unwrap();
        match case {
            0 => guest.configuration.sha256 = "a".repeat(64),
            1 => guest.artifacts.service.sha256 = "b".repeat(64),
            2 => guest.manifest_sha256 = "c".repeat(64),
            _ => template.guest = None,
        }
        assert!(
            matches!(
                journal.admit_template_create(
                    request(11, 1),
                    template_admission(changed),
                    compute_limits(),
                    &mut NeverCrash,
                ),
                Err(AdmissionError::InvalidCreateOptions)
            ),
            "mutation {case}"
        );
    }
    assert_eq!(journal.world().unwrap().sandbox_records().len(), 1);
    let mut replacement = options;
    replacement.template.as_mut().unwrap().template_id = "different-runtime-template".into();
    replacement.template.as_mut().unwrap().guest = None;
    journal
        .admit_template_create(
            request(11, 1),
            template_admission(replacement),
            compute_limits(),
            &mut NeverCrash,
        )
        .unwrap();
}

#[test]
fn compute_budget_checks_global_and_tenant_exact_boundaries_before_commit() {
    for case in 0..4 {
        let (_directory, mut journal) = setup();
        let mut compute = compute_limits();
        match case {
            0 => {
                compute.max_vcpus_global = 2;
                compute.max_vcpus_per_tenant = 2;
            }
            1 => compute.max_vcpus_per_tenant = 2,
            2 => {
                compute.max_memory_bytes_global = 256 * 1024 * 1024;
                compute.max_memory_bytes_per_tenant = compute.max_memory_bytes_global;
            }
            _ => compute.max_memory_bytes_per_tenant = 256 * 1024 * 1024,
        }
        journal
            .admit_template_create(
                request(10, 1),
                template_admission(template_options()),
                compute,
                &mut NeverCrash,
            )
            .unwrap();
        let world = journal.world().unwrap();
        let second_owner = if case % 2 == 0 { 2 } else { 1 };
        assert!(matches!(
            journal.admit_template_create(
                request(11, second_owner),
                template_admission(template_options()),
                compute,
                &mut NeverCrash
            ),
            Err(AdmissionError::QuotaExceeded)
        ));
        assert_eq!(journal.world().unwrap(), world);
    }
}

#[test]
fn templates_refuse_unaccounted_legacy_resources_and_mismatched_owner_or_disk() {
    let (_directory, mut journal) = setup();
    journal
        .admit_create(request(10, 1), 1_000, 60, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        journal.admit_template_create(
            request(11, 2),
            template_admission(template_options()),
            compute_limits(),
            &mut NeverCrash
        ),
        Err(AdmissionError::UnmanagedActive)
    ));
    let (_directory, mut journal) = setup();
    let mut admission = template_admission(template_options());
    admission.disk_bytes += 1;
    assert!(matches!(
        journal.admit_template_create(request(10, 1), admission, compute_limits(), &mut NeverCrash),
        Err(AdmissionError::InvalidCreateOptions)
    ));
    let mut admission = template_admission(template_options());
    admission.options.template.as_mut().unwrap().owner = Some(2);
    assert!(matches!(
        journal.admit_template_create(request(10, 1), admission, compute_limits(), &mut NeverCrash),
        Err(AdmissionError::InvalidCreateOptions)
    ));
    assert!(journal.world().unwrap().sandbox_records().is_empty());
}

struct BeforeCommit;

struct AfterCommit;

impl CrashInjector for AfterCommit {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == CrashPoint::AfterCommit {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

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
        template: None,
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
    let reopened = open_complete(directory.path().join("controller.db"), NAMESPACE).unwrap();
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
    dispatch_all(journal);
    let world = journal.world().unwrap();
    let plan = world
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
fn physical_slot_capacity_is_checked_before_admission_and_survives_restart() {
    let directory = TempDir::new().unwrap();
    let mut journal =
        open_complete_with_slots(directory.path().join("controller.db"), NAMESPACE, 2).unwrap();
    let mut policy = limits();
    policy.max_active_global = 3;
    policy.max_active_per_tenant = 3;
    policy.max_disk_bytes_global = CLEANUP_BYTES + 9_000;
    policy.max_disk_bytes_per_tenant = 10_000;

    for sandbox in [10, 11] {
        let decision = journal
            .admit_create(
                request(sandbox, 1),
                1_000,
                60,
                1_000,
                policy,
                &mut NeverCrash,
            )
            .unwrap();
        let key = core::OperationKey {
            sandbox: core::SandboxId(sandbox),
            operation: core::OperationId(sandbox * 10),
        };
        let plan = decision.next.operation_plan(key).unwrap();
        assert_eq!(plan.vm, Some(core::VmId(sandbox - 10)));
        assert_eq!(plan.slot, Some(core::NetSlotId(sandbox - 10)));
    }
    let before_rejection = journal.world().unwrap();
    assert!(matches!(
        journal.admit_create(request(12, 1), 1_000, 60, 1_000, policy, &mut NeverCrash),
        Err(AdmissionError::QuotaExceeded)
    ));
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(12))
            .is_none()
    );
    assert_eq!(journal.world().unwrap(), before_rejection);

    drop(journal);
    let mut reopened =
        open_complete_with_slots(directory.path().join("controller.db"), NAMESPACE, 2).unwrap();
    assert!(matches!(
        reopened.admit_create(request(12, 1), 1_000, 60, 1_000, policy, &mut NeverCrash),
        Err(AdmissionError::QuotaExceeded)
    ));
    assert!(
        reopened
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(12))
            .is_none()
    );
    assert_eq!(reopened.world().unwrap(), before_rejection);
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
fn default_connect_and_repeated_renewals_keep_the_original_total_limit_across_restart() {
    let (directory, mut journal) = setup();
    let mut policy = limits();
    policy.max_timeout_secs = 600;
    policy.max_lifetime_secs = 600;
    let sandbox = core::SandboxId(10);
    let owner = core::Actor::Tenant(core::TenantId(1));
    journal
        .admit_create(request(10, 1), 1_000, 20, 1_000, policy, &mut NeverCrash)
        .unwrap();
    running(&mut journal, sandbox, core::OperationId(100));
    let default = journal
        .connect_lifetime(&owner, sandbox, 2_000, None, policy)
        .unwrap();
    assert_eq!(
        (default.deadline_ms, default.max_deadline_ms),
        (302_000, 601_000)
    );
    assert_eq!(
        journal
            .connect_lifetime(&owner, sandbox, 3_000, Some(1), policy)
            .unwrap(),
        default
    );
    assert!(matches!(
        journal.connect_lifetime(&owner, sandbox, 3_000, Some(601), policy),
        Err(AdmissionError::InvalidTimeout)
    ));
    assert_eq!(
        journal
            .set_lifetime(&owner, sandbox, 4_000, i64::MAX)
            .unwrap()
            .deadline_ms,
        601_000
    );
    assert_eq!(
        journal
            .set_lifetime(&owner, sandbox, 5_000, 15)
            .unwrap()
            .deadline_ms,
        20_000
    );
    drop(journal);
    let mut journal = open_complete(directory.path().join("controller.db"), NAMESPACE).unwrap();
    policy.max_timeout_secs = 3_600;
    policy.max_lifetime_secs = 3_600;
    for now in [6_000, 300_000, 600_999] {
        let renewed = journal
            .connect_lifetime(&owner, sandbox, now, Some(3_600), policy)
            .unwrap();
        assert_eq!(renewed.started_at_ms, Some(1_000));
        assert_eq!(
            (renewed.deadline_ms, renewed.max_deadline_ms),
            (601_000, 601_000)
        );
    }
    assert!(matches!(
        journal.set_lifetime(&owner, sandbox, 601_000, 300),
        Err(AdmissionError::Expired)
    ));
    assert!(matches!(
        journal.set_lifetime(&owner, sandbox, 601_001, 300),
        Err(AdmissionError::MaxLifetimeExceeded)
    ));
    assert!(matches!(
        journal.connect_lifetime(&owner, sandbox, 601_001, None, policy),
        Err(AdmissionError::MaxLifetimeExceeded)
    ));
    assert!(matches!(
        journal.set_lifetime(
            &core::Actor::Tenant(core::TenantId(2)),
            sandbox,
            601_001,
            300
        ),
        Err(AdmissionError::Unauthorized)
    ));
}

#[test]
fn nonpositive_timeout_expires_without_synchronous_kill_or_resurrection() {
    for seconds in [0, -1, i64::MIN] {
        let (_directory, mut journal) = setup();
        let sandbox = core::SandboxId(10);
        let owner = core::Actor::Tenant(core::TenantId(1));
        journal
            .admit_create(request(10, 1), 1_000, 60, 1_000, limits(), &mut NeverCrash)
            .unwrap();
        running(&mut journal, sandbox, core::OperationId(100));
        assert_eq!(
            journal
                .set_lifetime(&owner, sandbox, 5_000, seconds)
                .unwrap()
                .deadline_ms,
            5_000
        );
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(sandbox)
                .unwrap()
                .phase,
            core::Phase::Running
        );
        assert_eq!(journal.due_lifetimes(5_000).unwrap(), vec![sandbox]);
        assert!(matches!(
            journal.live_lifetime(&owner, sandbox, 5_000),
            Err(AdmissionError::Expired)
        ));
        assert!(matches!(
            journal.connect_lifetime(&owner, sandbox, 5_000, None, limits()),
            Err(AdmissionError::Expired)
        ));
        assert!(matches!(
            journal.set_lifetime(&owner, sandbox, 5_000, 300),
            Err(AdmissionError::Expired)
        ));
        journal
            .accept_expiry(sandbox, core::OperationId(101), 5_000, &mut NeverCrash)
            .unwrap();
        assert!(matches!(
            journal.set_lifetime(&owner, sandbox, 5_000, 300),
            Err(AdmissionError::ChangingState)
        ));
    }
}

#[test]
fn create_rejects_total_limit_overflow_without_shortening_the_request() {
    let (_directory, mut journal) = setup();
    let mut policy = limits();
    policy.max_lifetime_secs = 60;
    for seconds in [61, 300, 86_400] {
        assert!(matches!(
            journal.admit_create(
                request(10, 1),
                1_000,
                seconds,
                1_000,
                policy,
                &mut NeverCrash
            ),
            Err(AdmissionError::InvalidTimeout)
        ));
        assert!(journal.world().unwrap().sandbox_records().is_empty());
    }
    journal
        .admit_create(request(10, 1), 1_000, 60, 1_000, policy, &mut NeverCrash)
        .unwrap();
    running(&mut journal, core::SandboxId(10), core::OperationId(100));
    let lifetime = journal
        .live_lifetime(
            &core::Actor::Tenant(core::TenantId(1)),
            core::SandboxId(10),
            1_001,
        )
        .unwrap();
    assert_eq!(
        (lifetime.deadline_ms, lifetime.max_deadline_ms),
        (61_000, 61_000)
    );
}

#[test]
fn old_lifetime_schema_is_rejected_without_changing_granted_time() {
    let (directory, mut journal) = setup();
    let sandbox = core::SandboxId(10);
    journal
        .admit_create(request(10, 1), 1_000, 30, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    running(&mut journal, sandbox, core::OperationId(100));
    drop(journal);
    let path = directory.path().join("controller.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("ALTER TABLE sandbox_lifetimes DROP COLUMN startup_deadline_ms; ALTER TABLE sandbox_lifetimes DROP COLUMN max_deadline_ms; UPDATE controller_meta SET version=4 WHERE id=1;").unwrap();
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        open_complete(&path, NAMESPACE),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn old_startup_schema_is_rejected_without_resetting_the_deadline() {
    let (directory, mut journal) = setup();
    journal
        .admit_create(request(10, 1), 1_000, 5, 1_000, limits(), &mut NeverCrash)
        .unwrap();
    drop(journal);
    let path = directory.path().join("controller.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE sandbox_lifetimes DROP COLUMN startup_deadline_ms;
             UPDATE controller_meta SET version=5 WHERE id=1;",
        )
        .unwrap();
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        open_complete(&path, NAMESPACE),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
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
fn missing_accepted_lifetime_fails_closed() {
    let (directory, mut journal) = setup();
    apply(&mut journal, create());
    // Corrupt the accepted lease rather than using a removed raw-create opener.
    rusqlite::Connection::open(directory.path().join("controller.db"))
        .unwrap()
        .execute("DELETE FROM sandbox_lifetimes", [])
        .unwrap();
    assert!(matches!(
        journal.admit_create(request(20, 2), 1_000, 60, 1_000, limits(), &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::CorruptStore))
    ));
}

#[test]
fn old_store_is_rejected_without_inventing_leases() {
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
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        open_complete(&path, NAMESPACE),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
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
