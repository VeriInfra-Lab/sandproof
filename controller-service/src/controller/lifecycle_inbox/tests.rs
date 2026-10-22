use super::*;
use crate::templates::{ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding};
use crate::{InternalTemplate, NeverCrash};
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::lifecycle::{
    CenterIdentity, ExpectedSession, IncarnationId, LifecycleRequest, OperationBinding,
    OperationKey, SessionId,
};
use sandproof_protocol::{ResourceId, internal::NodeIdentity};

const NS: &str = "0123456789abcdef";
fn config() -> LifecycleInboxConfig {
    LifecycleInboxConfig {
        center: CenterIdentity {
            deployment: "deployment".try_into().unwrap(),
            center: "primary".try_into().unwrap(),
        },
        node: NodeIdentity {
            deployment_id: "deployment".try_into().unwrap(),
            node_id: "host-a".try_into().unwrap(),
            node_generation: 1.try_into().unwrap(),
        },
        host_epoch: Some(crate::capture::HostEpochRegistration {
            version: 1,
            boot_id: "fixture-boot".into(),
            policy: sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32]),
        }),
        max_sessions: 2,
        max_options_bytes: 1024,
        max_profile_bytes: 4096,
        max_manifest_bytes: 16384,
    }
}
fn context() -> AuthenticatedCenterContext {
    config().context()
}
fn admission() -> InternalAdmission {
    let template = TemplateBinding {
        guest: None,
        template_id: "protected-base".into(),
        revision: "r1".into(),
        owner: None,
        kernel: TemplateArtifact {
            path: "/protected/kernel".into(),
            sha256: "a".repeat(64),
            size_bytes: 100,
        },
        rootfs: TemplateArtifact {
            path: "/protected/rootfs".into(),
            sha256: "b".repeat(64),
            size_bytes: 4096,
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            host_memory_bytes: 129 * 1024 * 1024,
            disk_bytes: 4096,
        },
        envd_version: "fixed".into(),
        startup: None,
    };
    InternalAdmission {
        now_ms: 10,
        limits: super::super::tests_support::limits(),
        compute: ComputeLimits {
            max_vcpus_global: 2,
            max_vcpus_per_tenant: 2,
            max_memory_bytes_global: 512 * 1024 * 1024,
            max_memory_bytes_per_tenant: 512 * 1024 * 1024,
        },
        template: InternalTemplate {
            revision: ResourceId::try_from("template-r1").unwrap(),
            digest: template.canonical_digest().unwrap(),
            registered: template.clone(),
            local: template,
        },
        file_policy_digest: core::PolicyDigest(1),
    }
}
fn create(owner: &str, session: &str) -> AcceptedLifecycleIntent {
    let a = admission();
    AcceptedLifecycleIntent {
        center: config().center,
        accepted_version: 1,
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: 3,
                node: config().node,
                session: SessionId::try_from(session).unwrap(),
                key: OperationKey {
                    deployment: "deployment".try_into().unwrap(),
                    owner: owner.try_into().unwrap(),
                    window: 10.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                create: CreateBinding {
                    template_revision: a.template.revision,
                    template_digest: a.template.digest,
                    resources: Resources {
                        instances: 1,
                        cpu: 1,
                        memory_bytes: 129 * 1024 * 1024,
                        disk_bytes: 4096,
                    },
                    options: serde_json::to_vec(&crate::CreateOptions::default()).unwrap(),
                    startup_deadline_ms: 1000,
                    timeout_ms: 60_000,
                    max_lifetime_ms: 600_000,
                },
                target_incarnation: IncarnationId::try_from(101).unwrap(),
            },
        },
    }
}
fn open(path: &std::path::Path) -> ControllerJournal {
    let mut maintenance = super::super::tests_support::fixture::maintenance();
    maintenance.checkpoint_profile = CheckpointProfile::FirstUnload;
    ControllerJournal::open_complete(
        path,
        NS,
        super::super::tests_support::host(4),
        maintenance,
        0,
    )
    .unwrap()
}
fn initialize(path: &std::path::Path) -> ControllerJournal {
    let mut j = open(path);
    j.initialize_lifecycle_inbox(config(), &mut NeverCrash)
        .unwrap();
    refresh_epoch(&mut j);
    j
}
fn refresh_epoch(j: &mut ControllerJournal) {
    j.establish_host_epoch(
        &mut |requirement| {
            Ok(crate::capture::HostEpochObservation {
                node: requirement.node().clone(),
                registration: requirement.registration().clone(),
            })
        },
        &mut NeverCrash,
    )
    .unwrap();
}
fn dispatch_counted(
    j: &mut ControllerJournal,
    sandbox: core::SandboxId,
    calls: &mut usize,
) -> crate::DispatchResult<super::super::tests_support::TestHostEvidence> {
    j.dispatch_next_for(
        sandbox,
        &mut |action| {
            *calls += 1;
            super::super::tests_support::worker(action)
        },
        &mut NeverCrash,
    )
    .unwrap()
}
fn destroy(create: &AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    let mut intent = create.clone();
    intent.accepted_version = 2;
    intent.request.binding.expected = ExpectedSession::Version(1);
    intent.request.binding.key.sequence = 2.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Destroy {};
    intent
}

fn export_closed_fixture(path: &std::path::Path) {
    let Some(output) = std::env::var_os("SANDPROOF_V3_OLD_READER_FIXTURE") else {
        return;
    };
    let output = std::path::PathBuf::from(output);
    std::fs::create_dir(&output).unwrap();
    // Every connection has closed. Never export a live database without WAL.
    assert!(!path.with_file_name("db-wal").exists());
    std::fs::copy(path, output.join("db")).unwrap();
    let mut maintenance = super::super::tests_support::fixture::maintenance();
    maintenance.checkpoint_profile = CheckpointProfile::FirstUnload;
    std::fs::write(
        output.join("maintenance.json"),
        serde_json::to_vec(&maintenance).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("host.json"),
        serde_json::to_vec(&super::super::tests_support::host(4)).unwrap(),
    )
    .unwrap();
}

#[test]
fn v3_original_create_admission_full_replay_and_restart_keep_distinct_roles_and_owners() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = initialize(&path);
    for (owner, session) in [("alpha", "session-a"), ("beta", "session-b")] {
        let request = create(owner, session);
        j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
            .unwrap();
        let accepted = j
            .admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
            .unwrap();
        assert!(accepted.local.is_some());
        assert_eq!(accepted.current_session_version, 1);
        let before = j.world().unwrap();
        let mut expired = admission();
        expired.now_ms = 100_000;
        let replay = j
            .admit_lifecycle_create(&request, &context(), expired, &mut NeverCrash)
            .unwrap();
        assert_eq!(replay, accepted);
        assert_eq!(j.world().unwrap(), before);
    }
    j.publish_checkpoint(&mut NeverCrash).unwrap();
    drop(j);
    let j = open(&path);
    let a = j
        .lifecycle_work_state(&create("alpha", "session-a"), &context())
        .unwrap()
        .local
        .unwrap();
    let b = j
        .lifecycle_work_state(&create("beta", "session-b"), &context())
        .unwrap()
        .local
        .unwrap();
    assert_ne!(a.owner, b.owner);
    assert_ne!(a.sandbox, b.sandbox);
    assert_eq!(
        j.world()
            .unwrap()
            .sandbox_record(a.sandbox)
            .unwrap()
            .generation,
        1
    );
    assert_eq!(
        create("alpha", "session-a").request.operation,
        create("beta", "session-b").request.operation
    );
    drop(j);
    export_closed_fixture(&path);
}

#[test]
fn v3_create_lost_commit_reply_retries_the_same_native_mapping() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = initialize(&path);
        let request = create("alpha", "session-a");
        j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
            .unwrap();
        super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            j.admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash),
            Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
        ));
        drop(j);
        let mut j = open(&path);
        assert_eq!(
            j.lifecycle_work_state(&request, &context())
                .unwrap()
                .local
                .is_some(),
            durable
        );
        if !durable {
            j.establish_host_epoch(
                &mut |r| {
                    Ok(crate::capture::HostEpochObservation {
                        node: r.node().clone(),
                        registration: r.registration().clone(),
                    })
                },
                &mut NeverCrash,
            )
            .unwrap();
        }
        let accepted = j
            .admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
            .unwrap();
        assert_eq!(accepted.local.unwrap().sandbox, core::SandboxId(1));
        assert_eq!(j.world().unwrap().sandbox_records().len(), 1);
    }
}

#[test]
fn v3_destroy_first_and_late_create_never_allocate_and_mapped_destroy_is_one_transaction() {
    for admitted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = initialize(&path);
        let request = create("alpha", "session-a");
        let stop = destroy(&request);
        if admitted {
            j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
                .unwrap();
            j.admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
                .unwrap();
        }
        let result = j
            .record_lifecycle_intent(&stop, &context(), &mut NeverCrash)
            .unwrap();
        assert!(result.terminal);
        if admitted {
            let retry = j
                .admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
                .unwrap();
            assert!(!retry.current);
            assert!(retry.terminal);
            assert!(
                j.world()
                    .unwrap()
                    .sandbox_record(result.local.unwrap().sandbox)
                    .unwrap()
                    .terminal
            );
        } else {
            assert!(
                j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
                    .is_err()
            );
            assert!(j.world().unwrap().sandbox_records().is_empty());
        }
        drop(j);
        let j = open(&path);
        assert!(j.lifecycle_work_state(&stop, &context()).unwrap().terminal);
    }
}

#[test]
fn v3_changed_full_request_foreign_context_and_old_native_admission_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = initialize(&path);
    let request = create("alpha", "session-a");
    j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
        .unwrap();
    let before = j.world().unwrap();
    let mut changed = request.clone();
    changed.request.binding.session = "other-session".try_into().unwrap();
    assert!(matches!(
        j.record_lifecycle_intent(&changed, &context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let mut node = config().node;
    node.node_generation = 2.try_into().unwrap();
    let foreign =
        AuthenticatedCenterContext::from_verified_peer_registration(config().center, node);
    assert!(j.lifecycle_work_state(&request, &foreign).is_err());
    let old = core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: core::SandboxId(1),
        workspace: core::FsId(1),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(1),
    };
    assert!(
        j.admit_create(
            old,
            0,
            60,
            4096,
            super::super::tests_support::limits(),
            &mut NeverCrash
        )
        .is_err()
    );
    assert_eq!(j.world().unwrap(), before);
}

#[test]
fn v3_empty_store_cannot_downgrade_into_the_old_inbox_or_create_without_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    let mut no_epoch = config();
    no_epoch.host_epoch = None;
    j.initialize_lifecycle_inbox(no_epoch, &mut NeverCrash)
        .unwrap();
    let legacy = crate::InternalInboxConfig {
        node: config().node,
        max_sandboxes: 2,
        operations_per_sandbox: 3,
        max_options_bytes: 128,
    };
    assert!(matches!(
        j.initialize_internal_inbox(legacy, &mut NeverCrash),
        Err(JournalError::UnsupportedStore)
    ));
    let version: i64 = j
        .journal
        .connection
        .query_row("SELECT version FROM controller_meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(version, crate::LIFECYCLE_INBOX_FORMAT_VERSION);
    let request = create("alpha", "session-a");
    j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
        .unwrap();
    assert!(
        j.admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
            .is_err()
    );
    assert!(j.world().unwrap().sandbox_records().is_empty());
    drop(j);
    let j = open(&path);
    assert!(
        j.lifecycle_work_state(&request, &context())
            .unwrap()
            .local
            .is_none()
    );
}

#[test]
fn v3_missing_required_table_or_relabelled_feature_refuses_reopen() {
    for corruption in [
        "DROP TABLE controller_lifecycle_inbox",
        "UPDATE controller_meta SET version=16 WHERE id=1",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let j = initialize(&path);
        drop(j);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(corruption).unwrap();
        drop(connection);
        let mut maintenance = super::super::tests_support::fixture::maintenance();
        maintenance.checkpoint_profile = CheckpointProfile::FirstUnload;
        assert!(
            ControllerJournal::preflight_complete(
                &path,
                NS,
                &super::super::tests_support::host(4),
                &maintenance
            )
            .is_err()
        );
        assert!(
            ControllerJournal::open_complete(
                &path,
                NS,
                super::super::tests_support::host(4),
                maintenance,
                0
            )
            .is_err()
        );
    }
}

#[test]
fn v3_epoch_reference_alone_does_not_admit_and_wrong_observation_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    j.initialize_lifecycle_inbox(config(), &mut NeverCrash)
        .unwrap();
    let request = create("alpha", "session-a");
    j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
        .unwrap();
    assert!(
        j.admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
            .is_err()
    );
    assert!(matches!(
        j.establish_host_epoch(
            &mut |requirement| {
                let mut registration = requirement.registration().clone();
                registration.boot_id = "other-boot".into();
                Ok(crate::capture::HostEpochObservation {
                    node: requirement.node().clone(),
                    registration,
                })
            },
            &mut NeverCrash
        ),
        Err(JournalError::InvalidHostEvidence)
    ));
    assert!(j.world().unwrap().sandbox_records().is_empty());
    for durable in [false, true] {
        super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            j.establish_host_epoch(
                &mut |r| Ok(crate::capture::HostEpochObservation {
                    node: r.node().clone(),
                    registration: r.registration().clone(),
                }),
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        assert!(
            j.admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
                .is_err()
        );
        assert!(j.world().unwrap().sandbox_records().is_empty());
    }
}

#[test]
fn v3_epoch_provider_runs_without_an_open_sql_transaction_then_commits_original_ref() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    j.initialize_lifecycle_inbox(config(), &mut NeverCrash)
        .unwrap();
    let mut observer = rusqlite::Connection::open(&path).unwrap();
    observer.busy_timeout(std::time::Duration::ZERO).unwrap();
    j.establish_host_epoch(
        &mut |requirement| {
            // No data mutation by a second writer: an immediate lock followed by
            // rollback deterministically fails if provider I/O holds the SQL lock.
            let tx = observer
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            tx.rollback().unwrap();
            Ok(crate::capture::HostEpochObservation {
                node: requirement.node().clone(),
                registration: requirement.registration().clone(),
            })
        },
        &mut NeverCrash,
    )
    .unwrap();
    drop(observer);
    drop(j);
    let mut j = open(&path);
    let request = create("alpha", "session-a");
    j.record_lifecycle_intent(&request, &context(), &mut NeverCrash)
        .unwrap();
    assert!(
        j.admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
            .is_err()
    );
    refresh_epoch(&mut j);
    let local = j
        .admit_lifecycle_create(&request, &context(), admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    drop(j);
    let mut j = open(&path);
    let mut calls = 0;
    let outcome = dispatch_counted(&mut j, local.sandbox, &mut calls);
    assert!(matches!(outcome, crate::DispatchResult::NoReadyAction));
    assert_eq!(calls, 0);
    refresh_epoch(&mut j);
    let outcome = dispatch_counted(&mut j, local.sandbox, &mut calls);
    assert!(matches!(outcome, crate::DispatchResult::ReceiptRecorded(_)));
    assert_eq!(calls, 1);
    assert!(
        j.establish_host_epoch(
            &mut |_| Err(crate::WorkerTransportError::ConnectionLost),
            &mut NeverCrash
        )
        .is_err()
    );
    let outcome = dispatch_counted(&mut j, local.sandbox, &mut calls);
    assert!(matches!(outcome, crate::DispatchResult::NoReadyAction));
    assert_eq!(calls, 1);
    refresh_epoch(&mut j);
    assert!(
        j.establish_host_epoch(
            &mut |r| {
                let mut registration = r.registration().clone();
                registration.boot_id = "wrong-boot".into();
                Ok(crate::capture::HostEpochObservation {
                    node: r.node().clone(),
                    registration,
                })
            },
            &mut NeverCrash
        )
        .is_err()
    );
    let outcome = dispatch_counted(&mut j, local.sandbox, &mut calls);
    assert!(matches!(outcome, crate::DispatchResult::NoReadyAction));
    assert_eq!(calls, 1);
}
