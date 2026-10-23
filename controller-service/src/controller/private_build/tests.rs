use super::*;
use crate::{
    AdmissionLimits, BoundedHostConfig, CreateAdmission, MaintenanceConfig, NeverCrash,
    finish_admitted_create, kill,
};
use tempfile::TempDir;

const NS: &str = "0123456789abcdef";
const BUILD: &str = "11111111111111111111111111111111";
fn host() -> BoundedHostConfig {
    let mut host = crate::controller::tests_support::host(1);
    host.maintenance.max_retained_results = 32;
    host
}
fn maintenance() -> MaintenanceConfig {
    crate::controller::tests_support::fixture::maintenance()
}
fn open(path: &std::path::Path) -> ControllerJournal {
    ControllerJournal::open_complete(path, NS, host(), maintenance(), 1000).unwrap()
}
fn setup(path: &std::path::Path) -> ControllerJournal {
    let mut journal = open(path);
    journal
        .enable_build_retry_windows(BuildRetryConfig {
            window_duration_ms: 100,
            max_anchors: 1,
        })
        .unwrap();
    journal
}
fn admit(
    journal: &mut ControllerJournal,
    window: Option<&str>,
    id: u64,
    crashes: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, crate::AdmissionError> {
    admit_at(journal, window, id, 1000, crashes)
}
fn admit_at(
    journal: &mut ControllerJournal,
    window: Option<&str>,
    id: u64,
    now_ms: i64,
    crashes: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, crate::AdmissionError> {
    let mut options = CreateOptions::default();
    options
        .metadata
        .insert("_sandproof_build_id".into(), BUILD.into());
    options
        .metadata
        .insert("_sandproof_build_request".into(), "60:false".into());
    if let Some(window) = window {
        options
            .metadata
            .insert("_sandproof_build_window".into(), window.into());
    }
    journal.admit_create_with_options(
        core::CreateRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            owner: core::TenantId(1),
            sandbox: core::SandboxId(id),
            workspace: core::FsId(id),
            file_policy: core::FilePolicyRef {
                digest: core::PolicyDigest(1),
            },
            policy: core::NetworkPolicy::DenyAll,
            operation: core::OperationId(1),
        },
        CreateAdmission {
            now_ms,
            timeout_secs: 60,
            disk_bytes: 1000,
            limits: AdmissionLimits {
                max_active_global: 1,
                max_active_per_tenant: 1,
                max_disk_bytes_global: crate::controller::tests_support::fixture::CLEANUP_BYTES
                    + 100_000,
                max_disk_bytes_per_tenant: 10_000,
                cleanup_reserve_bytes: crate::controller::tests_support::fixture::CLEANUP_BYTES,
                max_timeout_secs: 600,
                max_lifetime_secs: 600,
                startup_timeout_secs: 60,
            },
            options,
        },
        crashes,
    )
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Evidence {
    resource: sandproof_protocol::ResourceRef,
    command: core::Command,
}
impl sandproof_protocol::ResourceEvidence for Evidence {
    fn resource(&self) -> &sandproof_protocol::ResourceRef {
        &self.resource
    }
}
impl crate::HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.command == *command
    }
}
fn success(
    action: &sandproof_protocol::WorkAction<core::EffectId>,
) -> Result<sandproof_protocol::WorkReceipt<Evidence>, crate::WorkerTransportError> {
    action
        .identity
        .validate_version()
        .map_err(|_| crate::WorkerTransportError::ConnectionLost)?;
    Ok(sandproof_protocol::WorkReceipt {
        identity: action.identity.clone(),
        outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(Evidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
    })
}
fn destroy(journal: &mut ControllerJournal, id: u64) {
    finish_admitted_create(
        journal,
        core::SandboxId(id),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
    kill(
        journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: core::SandboxId(id),
            operation: core::OperationId(2),
        },
        &mut success,
    )
    .unwrap();
}
fn issue(journal: &mut ControllerJournal) -> String {
    journal
        .issue_build_window(core::TenantId(1), 1000, &mut NeverCrash)
        .unwrap()
        .request_window
}
fn lookup(journal: &ControllerJournal, window: &str) -> Option<BuildAnchor> {
    journal
        .build_anchor(core::TenantId(1), window, BUILD)
        .unwrap()
}
fn settle(
    journal: &mut ControllerJournal,
    window: &str,
    id: Option<u64>,
) -> Result<(), JournalError> {
    journal.settle_build_anchor(
        core::TenantId(1),
        window,
        BUILD,
        id.map(core::SandboxId),
        &mut NeverCrash,
    )
}
fn rows(journal: &ControllerJournal, table: &str) -> i64 {
    journal
        .journal
        .connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn closed_absence_is_a_fence_and_delayed_post_never_creates() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = setup(&path);
    let window = issue(&mut journal);
    assert!(lookup(&journal, &window).is_none());
    assert!(settle(&mut journal, &window, None).is_err());
    journal
        .close_build_window(core::TenantId(1), &window, 1001, &mut NeverCrash)
        .unwrap();
    assert!(lookup(&journal, &window).is_none());
    settle(&mut journal, &window, None).unwrap();
    assert!(admit(&mut journal, Some(&window), 1, &mut NeverCrash).is_err());
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    drop(journal);
    let mut journal = open(&path);
    assert!(admit(&mut journal, Some(&window), 1, &mut NeverCrash).is_err());
    assert!(admit(&mut journal, None, 1, &mut NeverCrash).is_err());
}

#[test]
fn accepted_tuple_survives_close_until_exact_settlement_and_can_retire() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = setup(&path);
    let window = issue(&mut journal);
    admit(&mut journal, Some(&window), 1, &mut NeverCrash).unwrap();
    let original = lookup(&journal, &window).unwrap();
    journal
        .close_build_window(core::TenantId(1), &window, 1001, &mut NeverCrash)
        .unwrap();
    assert_eq!(lookup(&journal, &window), Some(original.clone()));
    assert!(settle(&mut journal, &window, Some(1)).is_err());
    destroy(&mut journal, 1);
    assert!(
        journal
            .host_slot_release_pinned(core::SandboxId(1))
            .unwrap()
    );
    assert!(settle(&mut journal, &window, None).is_err());
    assert!(settle(&mut journal, &window, Some(2)).is_err());
    settle(&mut journal, &window, Some(1)).unwrap();
    assert!(
        !journal
            .host_slot_release_pinned(core::SandboxId(1))
            .unwrap()
    );
    assert_eq!(
        journal.create_options(core::SandboxId(1)).unwrap(),
        original.options
    );
    let context = journal
        .host_slot_context(core::SandboxId(1))
        .unwrap()
        .unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    journal
        .retire_reclaimed(core::SandboxId(1), 1010, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    let progress = journal.checkpoint_progress().unwrap().unwrap();
    assert_eq!(progress.deleted_through, progress.input_cut);
    drop(journal);
    let mut journal = open(&path);
    settle(&mut journal, &window, Some(1)).unwrap();
    assert!(lookup(&journal, &window).is_none());
    assert!(admit(&mut journal, Some(&window), 2, &mut NeverCrash).is_err());
    let next = issue(&mut journal);
    admit(&mut journal, Some(&next), 2, &mut NeverCrash).unwrap();
}

#[test]
fn lost_admission_commits_have_exactly_the_original_anchor() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = setup(&path);
        let window = issue(&mut journal);
        super::super::transaction::fail_next_commit(durable);
        assert!(admit(&mut journal, Some(&window), 1, &mut NeverCrash).is_err());
        assert_eq!(lookup(&journal, &window).is_some(), durable);
        assert_eq!(
            journal.world().unwrap().sandbox_records().len(),
            usize::from(durable)
        );
        drop(journal);
        let mut journal = open(&path);
        journal
            .close_build_window(core::TenantId(1), &window, 1001, &mut NeverCrash)
            .unwrap();
        assert_eq!(lookup(&journal, &window).is_some(), durable);
        if durable {
            destroy(&mut journal, 1);
            settle(&mut journal, &window, Some(1)).unwrap();
        } else {
            settle(&mut journal, &window, None).unwrap();
        }
        assert!(admit(&mut journal, Some(&window), 2, &mut NeverCrash).is_err());
    }
}

#[test]
fn lost_close_and_settlement_answers_retry_same_identity() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = setup(&path);
        let window = issue(&mut journal);
        admit(&mut journal, Some(&window), 1, &mut NeverCrash).unwrap();
        destroy(&mut journal, 1);
        super::super::transaction::fail_next_commit(durable);
        assert!(
            journal
                .close_build_window(core::TenantId(1), &window, 1001, &mut NeverCrash)
                .is_err()
        );
        assert!(lookup(&journal, &window).is_some());
        journal
            .close_build_window(core::TenantId(1), &window, 1001, &mut NeverCrash)
            .unwrap();
        super::super::transaction::fail_next_commit(durable);
        assert!(settle(&mut journal, &window, Some(1)).is_err());
        assert_eq!(lookup(&journal, &window).is_none(), durable);
        drop(journal);
        let mut journal = open(&path);
        settle(&mut journal, &window, Some(1)).unwrap();
        assert!(lookup(&journal, &window).is_none());
        assert!(admit(&mut journal, Some(&window), 2, &mut NeverCrash).is_err());
    }
}

#[test]
fn expiry_persists_and_repeated_closed_operations_do_not_grow_tail() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = setup(&path);
    let window = issue(&mut journal);
    journal
        .advance_build_windows(1100, &mut NeverCrash)
        .unwrap();
    let count = rows(&journal, "controller_inputs");
    for now in 1101..1110 {
        journal
            .close_build_window(core::TenantId(1), &window, now, &mut NeverCrash)
            .unwrap();
        settle(&mut journal, &window, None).unwrap();
    }
    assert_eq!(rows(&journal, "controller_inputs"), count);
    drop(journal);
    let mut journal = open(&path);
    assert!(admit(&mut journal, Some(&window), 1, &mut NeverCrash).is_err());
    assert!(
        journal
            .close_build_window(core::TenantId(2), &window, 1000, &mut NeverCrash)
            .is_err()
    );
    assert!(
        journal
            .close_build_window(
                core::TenantId(1),
                "bw-0123456789abcdef-1-99",
                1000,
                &mut NeverCrash
            )
            .is_err()
    );
}

#[test]
fn windowless_private_admission_is_rejected_before_and_after_service_enablement() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    let before = journal.world().unwrap();
    assert!(admit(&mut journal, None, 1, &mut NeverCrash).is_err());
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
    journal
        .enable_build_retry_windows(BuildRetryConfig {
            window_duration_ms: 100,
            max_anchors: 1,
        })
        .unwrap();
    assert!(admit(&mut journal, None, 1, &mut NeverCrash).is_err());
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    assert_eq!(rows(&journal, "private_build_anchors"), 0);
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
}

#[test]
fn lost_issue_response_never_implies_a_builder_was_created() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = setup(&path);
        super::super::transaction::fail_next_commit(durable);
        assert!(
            journal
                .issue_build_window(core::TenantId(1), 1000, &mut NeverCrash)
                .is_err()
        );
        assert_eq!(rows(&journal, "private_build_anchors"), 0);
        assert!(journal.world().unwrap().sandbox_records().is_empty());
        let recovered = journal
            .issue_build_window(core::TenantId(1), 1000, &mut NeverCrash)
            .unwrap();
        assert!(
            recovered
                .request_window
                .ends_with(if durable { "-1" } else { "-0" })
        );
        // A lost issuance may occupy finite open-window budget, then expires.
        journal
            .advance_build_windows(1100, &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .world()
                .unwrap()
                .maintenance_state()
                .unwrap()
                .windows()
                .open_windows()
                .is_empty()
        );
    }
}

#[test]
fn sql_anchor_binding_and_unsettled_absence_are_validated_on_reopen() {
    for remove in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = setup(&path);
        let window = issue(&mut journal);
        admit(&mut journal, Some(&window), 1, &mut NeverCrash).unwrap();
        let mut anchor = lookup(&journal, &window).unwrap();
        journal.publish_checkpoint(&mut NeverCrash).unwrap();
        drop(journal);
        let sql = Connection::open(&path).unwrap();
        if remove {
            sql.execute("DELETE FROM private_build_anchors", [])
                .unwrap();
        } else {
            anchor
                .options
                .metadata
                .insert("changed".into(), "binding".into());
            sql.execute(
                "UPDATE private_build_anchors SET anchor_json=?1",
                [serde_json::to_vec(&anchor).unwrap()],
            )
            .unwrap();
        }
        drop(sql);
        assert!(ControllerJournal::open_complete(&path, NS, host(), maintenance(), 1000).is_err());
    }
}

#[test]
fn private_config_is_explicit_immutable_and_requires_maintenance() {
    let dir = TempDir::new().unwrap();
    let mut incomplete = maintenance();
    incomplete.capacity = None;
    assert!(
        ControllerJournal::open_complete(
            dir.path().join("incomplete"),
            NS,
            host(),
            incomplete,
            1000
        )
        .is_err()
    );
    let mut journal = setup(&dir.path().join("db"));
    for config in [
        BuildRetryConfig {
            window_duration_ms: 0,
            max_anchors: 1,
        },
        BuildRetryConfig {
            window_duration_ms: 100,
            max_anchors: 0,
        },
        BuildRetryConfig {
            window_duration_ms: 1001,
            max_anchors: 1,
        },
        BuildRetryConfig {
            window_duration_ms: 100,
            max_anchors: 2,
        },
    ] {
        assert!(journal.enable_build_retry_windows(config).is_err());
    }
    assert_eq!(
        journal.build_retry_config().unwrap(),
        Some(BuildRetryConfig {
            window_duration_ms: 100,
            max_anchors: 1
        })
    );
    assert!(
        journal
            .issue_build_window(core::TenantId(0), 1000, &mut NeverCrash)
            .is_err()
    );
}

#[test]
fn preflight_creates_nothing_and_refuses_private_domain_rollback() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let value = BuildRetryConfig {
        window_duration_ms: 100,
        max_anchors: 1,
    };
    ControllerJournal::preflight_build_retry_windows(&path, NS, Some(value)).unwrap();
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let journal = setup(&path);
    drop(journal);
    let before = std::fs::metadata(&path).unwrap().len();
    ControllerJournal::preflight_build_retry_windows(&path, NS, Some(value)).unwrap();
    assert!(ControllerJournal::preflight_build_retry_windows(&path, NS, None).is_err());
    assert!(
        ControllerJournal::preflight_build_retry_windows(
            &path,
            NS,
            Some(BuildRetryConfig {
                max_anchors: 2,
                ..value
            })
        )
        .is_err()
    );
    assert!(
        ControllerJournal::preflight_build_retry_windows(&path, "fedcba9876543210", Some(value))
            .is_err()
    );
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
}

#[test]
fn rejected_issue_cannot_roll_back_expired_window_fence() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = setup(&path);
    let window = issue(&mut journal);
    assert!(
        journal
            .issue_build_window(core::TenantId(1), u64::MAX, &mut NeverCrash)
            .is_err()
    );
    let world = journal.world().unwrap();
    let windows = world.maintenance_state().unwrap().windows();
    assert_eq!(windows.time_floor(), u64::MAX);
    assert!(windows.open_windows().is_empty());
    assert_eq!(windows.issued_through(), Some(0));
    drop(journal);
    let mut journal = open(&path);
    assert!(admit(&mut journal, Some(&window), 1, &mut NeverCrash).is_err());
    journal
        .close_build_window(core::TenantId(1), &window, 1000, &mut NeverCrash)
        .unwrap();
    settle(&mut journal, &window, None).unwrap();
}

#[test]
fn anchor_lookup_checks_sql_sandbox_identity_not_only_json() {
    let dir = TempDir::new().unwrap();
    let mut journal = setup(&dir.path().join("db"));
    let window = issue(&mut journal);
    admit(&mut journal, Some(&window), 1, &mut NeverCrash).unwrap();
    journal
        .journal
        .connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    journal
        .journal
        .connection
        .execute(
            "UPDATE private_build_anchors SET sandbox_id='sb-0123456789abcdef-2'",
            [],
        )
        .unwrap();
    assert!(
        journal
            .build_anchor(core::TenantId(1), &window, BUILD)
            .is_err()
    );
}

#[test]
fn actual_admission_expiry_is_fenced_before_a_rejected_create() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = setup(&path);
    let window = issue(&mut journal);
    journal
        .advance_build_windows(1099, &mut NeverCrash)
        .unwrap();
    assert!(admit_at(&mut journal, Some(&window), 1, 1100, &mut NeverCrash).is_err());
    assert_eq!(
        journal
            .world()
            .unwrap()
            .maintenance_state()
            .unwrap()
            .windows()
            .time_floor(),
        1100
    );
    assert!(lookup(&journal, &window).is_none());
    drop(journal);
    let mut journal = open(&path);
    assert!(admit_at(&mut journal, Some(&window), 1, 1099, &mut NeverCrash).is_err());
    assert!(journal.world().unwrap().sandbox_records().is_empty());
}
