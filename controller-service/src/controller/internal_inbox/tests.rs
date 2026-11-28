use super::super::tests_support as workflow_fixture;
use super::super::tests_support::lifecycle_application::fixture::wire as fixture;
use super::*;
use crate::templates::{ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding};
use crate::{AdmissionError, InternalAdmission, InternalTemplate};
use crate::{CrashPoint, NeverCrash};
use fixture::*;
use sandbox_controller_core as core;
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
#[path = "tests/capacity_rejection.rs"]
mod capacity_rejection;
#[path = "tests/renewal.rs"]
mod renewal;
#[path = "tests/starting_destroy.rs"]
mod starting_destroy;
#[path = "tests/window.rs"]
mod window;

fn local_admission() -> InternalAdmission {
    let local = local_template();
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
            revision: "template-revision".try_into().unwrap(),
            digest: local.canonical_digest().unwrap(),
            registered: local.clone(),
            local,
        },
        file_policy_digest: core::PolicyDigest(1),
    }
}
fn local_template() -> TemplateBinding {
    TemplateBinding {
        guest: None,
        template_id: "base-fixed".into(),
        revision: "r1".into(),
        owner: None,
        kernel: TemplateArtifact {
            path: "/protected-cache/kernel".into(),
            sha256: "a".repeat(64),
            size_bytes: 100,
        },
        rootfs: TemplateArtifact {
            path: "/protected-cache/rootfs".into(),
            sha256: "b".repeat(64),
            size_bytes: 4096,
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            host_memory_bytes: 129 * 1024 * 1024,
            disk_bytes: 4096,
        },
        envd_version: "fixed-envd".into(),
        startup: None,
    }
}
fn local_request() -> LifecycleRequest {
    let mut r = request(1, 1, Intent::Create);
    r.create.timeout_ms = 60_000;
    r.create.max_lifetime_ms = 600_000;
    r.create.template_digest = local_admission().template.digest;
    r.create.resources.memory_bytes = local_admission().template.local.resources.host_memory_bytes;
    r.create.options = serde_json::to_vec(&crate::CreateOptions {
        networking: Some(crate::CreateNetworking {
            allow_internet_access: Some(false),
            network: None,
        }),
        ..crate::CreateOptions::default()
    })
    .unwrap();
    r
}

fn config() -> InternalInboxConfig {
    InternalInboxConfig {
        node: node("host-a"),
        max_sandboxes: 2,
        operations_per_sandbox: 3,
        max_options_bytes: 128,
    }
}
fn open(path: &std::path::Path) -> ControllerJournal {
    super::super::tests_support::open(path, "0123456789abcdef").unwrap()
}
struct Stop(CrashPoint);
impl CrashInjector for Stop {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}
#[test]
fn exact_duplicate_survives_restart_and_conflicting_parameters_never_execute() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let r = request(1, 1, Intent::Create);
    let original = j.record_internal_intent(&r, &mut NeverCrash).unwrap();
    assert_eq!(original.fact, LifecycleFact::Recorded);
    assert!(j.world().unwrap().sandbox_records().is_empty());
    drop(j);
    let mut j = open(&path);
    assert_eq!(
        j.record_internal_intent(&r, &mut NeverCrash).unwrap(),
        original
    );
    let mut changed = r.clone();
    changed.create.timeout_ms += 1;
    assert!(matches!(
        j.record_internal_intent(&changed, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert_eq!(j.internal_pending(&config().node).unwrap(), vec![r]);
}
#[test]
fn terminal_first_and_after_recording_create_fence_delayed_or_duplicate_create() {
    for first in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut j = open(&dir.path().join("db"));
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let create = request(1, 1, Intent::Create);
        let original = if first {
            None
        } else {
            Some(j.record_internal_intent(&create, &mut NeverCrash).unwrap())
        };
        let destroy = request(2, 2, Intent::Destroy);
        let result = j.record_internal_intent(&destroy, &mut NeverCrash).unwrap();
        let late = j.record_internal_intent(&create, &mut NeverCrash);
        if let Some(original) = original {
            assert_eq!(late.unwrap(), original);
        } else {
            assert!(matches!(late, Err(JournalError::Conflict)));
        }
        assert!(j.world().unwrap().sandbox_records().is_empty());
        assert_eq!(
            j.record_internal_intent(&destroy, &mut NeverCrash).unwrap(),
            result
        );
        assert_eq!(j.internal_pending(&config().node).unwrap(), vec![destroy]);
    }
}
#[test]
fn before_commit_rolls_back_after_commit_restart_finds_exact_request() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let r = request(1, 1, Intent::Create);
        assert!(matches!(
            j.record_internal_intent(&r, &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(j);
        let mut j = open(&path);
        assert_eq!(
            j.internal_pending(&config().node).unwrap().len(),
            usize::from(point == CrashPoint::AfterCommit)
        );
        let result = j.record_internal_intent(&r, &mut NeverCrash).unwrap();
        assert_eq!(result.identity, r.identity);
        assert!(j.world().unwrap().sandbox_records().is_empty());
    }
}
#[test]
fn unknown_sql_commit_cannot_mint_a_new_operation_or_response() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        super::super::transaction::fail_next_commit(durable);
        let r = request(1, 1, Intent::Create);
        assert!(matches!(
            j.record_internal_intent(&r, &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        assert_eq!(
            j.internal_pending(&config().node).unwrap().len(),
            usize::from(durable)
        );
        assert_eq!(
            j.record_internal_intent(&r, &mut NeverCrash)
                .unwrap()
                .identity,
            r.identity
        );
    }
}
#[test]
fn ownership_node_window_and_noncanonical_options_fail_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = open(&dir.path().join("db"));
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let r = request(1, 1, Intent::Create);
    j.record_internal_intent(&r, &mut NeverCrash).unwrap();
    for mutate in [0, 1, 2, 3] {
        let mut bad = r.clone();
        match mutate {
            0 => bad.identity.tenant_id = "other".try_into().unwrap(),
            1 => bad.identity.node.node_generation = 2.try_into().unwrap(),
            2 => bad.identity.request_window = 2.try_into().unwrap(),
            _ => bad.create.options.push(b' '),
        }
        assert!(matches!(
            j.record_internal_intent(&bad, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
    }
    assert_eq!(j.internal_pending(&config().node).unwrap(), vec![r]);
}
#[test]
fn normal_slots_full_leave_one_destroy_slot_and_future_sql_space_is_charged() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = open(&dir.path().join("db"));
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    j.record_internal_intent(&request(1, 1, Intent::Create), &mut NeverCrash)
        .unwrap();
    j.record_internal_intent(
        &request(2, 2, Intent::Renew { end_at_ms: 400 }),
        &mut NeverCrash,
    )
    .unwrap();
    assert!(matches!(
        j.record_internal_intent(
            &request(3, 3, Intent::Renew { end_at_ms: 500 }),
            &mut NeverCrash
        ),
        Err(JournalError::MaintenanceCapacity)
    ));
    j.record_internal_intent(&request(3, 3, Intent::Destroy), &mut NeverCrash)
        .unwrap();
    let mut too_large = config();
    too_large.max_options_bytes = 65_536;
    let dir = tempfile::tempdir().unwrap();
    let mut j = open(&dir.path().join("db"));
    assert!(matches!(
        j.initialize_internal_inbox(too_large, &mut NeverCrash),
        Err(JournalError::MaintenanceCapacity)
    ));
}

#[test]
fn internal_inbox_local_admission_mapping_quota_slot_and_outbox_commit_together() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let r = local_request();
        j.record_internal_intent(&r, &mut NeverCrash).unwrap();
        assert!(matches!(
            j.admit_internal_create(&r, local_admission(), &mut Stop(point)),
            Err(AdmissionError::Journal(JournalError::InjectedCrash(_)))
        ));
        drop(j);
        let mut j = open(&path);
        let committed = point == CrashPoint::AfterCommit;
        assert_eq!(
            j.world().unwrap().sandbox_records().len(),
            usize::from(committed)
        );
        let inbox = codec::load(&j.journal.connection).unwrap().unwrap();
        assert_eq!(inbox.entries[0].local.is_some(), committed);
        assert_eq!(inbox.tenants.len(), usize::from(committed));
        let result = j
            .admit_internal_create(&r, local_admission(), &mut NeverCrash)
            .unwrap();
        assert_eq!(result.local.is_some(), !committed);
        assert_eq!(j.world().unwrap().sandbox_records().len(), 1);
        let local = codec::load(&j.journal.connection).unwrap().unwrap().entries[0]
            .local
            .unwrap();
        let lifetime = j
            .journal
            .connection
            .query_row(
                "SELECT startup_deadline_ms FROM sandbox_lifetimes",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(lifetime, 1000);
        assert!(j.host_slot_context(local.sandbox).unwrap().is_some());
        assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(2));
        assert_eq!(
            j.capture_internal_progress(&r, &mut NeverCrash)
                .unwrap()
                .fact,
            LifecycleFact::Recorded
        );
    }
}

#[test]
fn internal_inbox_unknown_admission_commit_reopens_same_mapping_never_allocates_second_vm() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let r = local_request();
        j.record_internal_intent(&r, &mut NeverCrash).unwrap();
        super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            j.admit_internal_create(&r, local_admission(), &mut NeverCrash),
            Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
        ));
        drop(j);
        let mut j = open(&path);
        assert_eq!(
            j.world().unwrap().sandbox_records().len(),
            usize::from(durable)
        );
        let answer = j
            .admit_internal_create(&r, local_admission(), &mut NeverCrash)
            .unwrap();
        assert_eq!(answer.local.is_some(), !durable);
        assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(2));
    }
}

#[test]
fn internal_inbox_destroy_fence_blocks_admission_and_empty_cleanup_is_durable() {
    for recorded in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let create = local_request();
        if recorded {
            j.record_internal_intent(&create, &mut NeverCrash).unwrap();
        }
        let destroy = LifecycleRequest {
            intent: Intent::Destroy,
            identity: request(2, 2, Intent::Destroy).identity,
            ..create.clone()
        };
        j.record_internal_intent(&destroy, &mut NeverCrash).unwrap();
        let final_fact = j
            .capture_internal_progress(&destroy, &mut NeverCrash)
            .unwrap();
        assert_eq!(final_fact.fact, LifecycleFact::Cleaned);
        drop(j);
        let mut j = open(&path);
        if recorded {
            assert!(
                j.admit_internal_create(&create, local_admission(), &mut NeverCrash)
                    .unwrap()
                    .local
                    .is_none()
            );
        } else {
            assert!(j.record_internal_intent(&create, &mut NeverCrash).is_err());
        }
        assert_eq!(
            j.record_internal_intent(&destroy, &mut NeverCrash).unwrap(),
            final_fact
        );
        assert!(j.world().unwrap().sandbox_records().is_empty());
    }
}

#[test]
fn internal_inbox_template_shape_time_policy_and_quota_refusals_have_no_mapping_or_effects() {
    for case in 0..9 {
        let dir = tempfile::tempdir().unwrap();
        let mut j = open(&dir.path().join("db"));
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let r = local_request();
        j.record_internal_intent(&r, &mut NeverCrash).unwrap();
        let mut a = local_admission();
        match case {
            0 => a.template.digest[0] = 2,
            1 => a.template.revision = "wrong-revision".try_into().unwrap(),
            2 => a.template.local.resources.cpu_count = 2,
            3 => a.template.local.owner = Some(2),
            4 => a.now_ms = 1000,
            5 => a.now_ms = -1,
            6 => a.limits.max_active_global = 0,
            7 => a.compute.max_memory_bytes_global = 1,
            _ => a.limits.max_lifetime_secs = 599,
        }
        assert!(
            j.admit_internal_create(&r, a, &mut NeverCrash).is_err(),
            "case {case}"
        );
        assert!(j.world().unwrap().sandbox_records().is_empty());
        let inbox = codec::load(&j.journal.connection).unwrap().unwrap();
        assert!(inbox.entries[0].local.is_none());
        assert!(inbox.tenants.is_empty());
        assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(1));
    }
}

#[test]
fn internal_inbox_multitenant_mapping_is_stable_injective_and_charges_original_compute_quota() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    let mut cfg = config();
    cfg.max_sandboxes = 3;
    j.initialize_internal_inbox(cfg, &mut NeverCrash).unwrap();
    let first = local_request();
    j.record_internal_intent(&first, &mut NeverCrash).unwrap();
    let local = j
        .admit_internal_create(&first, local_admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    let mut second = first.clone();
    second.identity.tenant_id = "researcher-b".try_into().unwrap();
    second.identity.sandbox_id = "sandbox-2".try_into().unwrap();
    second.identity.retention_slot = 2;
    second.identity.operation_id = "operation-2".try_into().unwrap();
    j.record_internal_intent(&second, &mut NeverCrash).unwrap();
    let other = j
        .admit_internal_create(&second, local_admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    assert_ne!(local.owner, other.owner);
    assert_ne!(local.sandbox, other.sandbox);
    let mut third = first.clone();
    third.identity.sandbox_id = "sandbox-3".try_into().unwrap();
    third.identity.retention_slot = 3;
    third.identity.operation_id = "operation-3".try_into().unwrap();
    j.record_internal_intent(&third, &mut NeverCrash).unwrap();
    assert!(matches!(
        j.admit_internal_create(&third, local_admission(), &mut NeverCrash),
        Err(AdmissionError::QuotaExceeded)
    ));
    drop(j);
    let mut j = open(&path);
    // Replay is independent of a newly broken cache or later time.
    let mut invalid = local_admission();
    invalid.now_ms = i64::MAX;
    invalid.template.digest = [9; 32];
    assert!(
        j.admit_internal_create(&first, invalid, &mut NeverCrash)
            .unwrap()
            .local
            .is_none()
    );
    assert_eq!(
        codec::load(&j.journal.connection).unwrap().unwrap().entries[0].local,
        Some(local)
    );
    assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(3));
}

#[test]
fn internal_inbox_ready_fact_uses_original_publication_time_and_survives_lost_response() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let r = local_request();
        j.record_internal_intent(&r, &mut NeverCrash).unwrap();
        let local = j
            .admit_internal_create(&r, local_admission(), &mut NeverCrash)
            .unwrap()
            .local
            .unwrap();
        let mut calls = 0;
        crate::finish_admitted_create_timed(
            &mut j,
            local.sandbox,
            local.operation,
            &mut |action| {
                calls += 1;
                workflow_fixture::worker(action)
            },
            &mut || Ok(20),
        )
        .unwrap();
        assert!(calls > 0);
        assert!(matches!(
            j.capture_internal_progress(&r, &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(j);
        let mut j = open(&path);
        let answer = j.capture_internal_progress(&r, &mut NeverCrash).unwrap();
        assert_eq!(
            answer.fact,
            LifecycleFact::Ready {
                started_at_ms: 20,
                end_at_ms: 60_020,
                max_end_at_ms: 600_020
            }
        );
        assert_eq!(
            j.record_internal_intent(&r, &mut NeverCrash).unwrap(),
            answer
        );
        assert_eq!(
            j.capture_internal_progress(&r, &mut NeverCrash).unwrap(),
            answer
        );
        assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(2));
    }
}

#[test]
fn internal_inbox_stopped_is_not_cleaned_until_exact_physical_release_and_details_remain_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let r = local_request();
    j.record_internal_intent(&r, &mut NeverCrash).unwrap();
    let local = j
        .admit_internal_create(&r, local_admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    crate::finish_admitted_create_timed(
        &mut j,
        local.sandbox,
        local.operation,
        &mut workflow_fixture::worker,
        &mut || Ok(20),
    )
    .unwrap();
    let ready = j.capture_internal_progress(&r, &mut NeverCrash).unwrap();
    let destroy = LifecycleRequest {
        intent: Intent::Destroy,
        identity: request(2, 2, Intent::Destroy).identity,
        ..r.clone()
    };
    j.record_internal_intent(&destroy, &mut NeverCrash).unwrap();
    crate::finish_accepted_kill(
        &mut j,
        local.sandbox,
        core::OperationId(2),
        &mut workflow_fixture::worker,
    )
    .unwrap();
    assert_eq!(
        j.capture_internal_progress(&destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
    let context = j.host_slot_context(local.sandbox).unwrap().unwrap();
    assert!(
        j.release_host_slot(
            context,
            |_| Err(crate::WorkerTransportError::ReplyLost),
            &mut NeverCrash
        )
        .is_err()
    );
    assert_eq!(
        j.capture_internal_progress(&destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
    j.release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    let cleaned = j
        .capture_internal_progress(&destroy, &mut NeverCrash)
        .unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    assert!(j.retirement_candidates().unwrap().is_empty());
    assert!(
        j.retire_reclaimed(local.sandbox, 700_000, |_| Ok(()), &mut NeverCrash)
            .is_err()
    );
    drop(j);
    let mut j = open(&path);
    assert_eq!(
        j.record_internal_intent(&destroy, &mut NeverCrash).unwrap(),
        cleaned
    );
    assert_eq!(
        j.record_internal_intent(&r, &mut NeverCrash).unwrap(),
        ready
    );
    assert_eq!(
        j.capture_internal_progress(&r, &mut NeverCrash).unwrap(),
        ready
    );
}

#[test]
fn internal_inbox_destroy_and_native_terminal_commit_together_even_during_uncertain_prepare() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let create = local_request();
        j.record_internal_intent(&create, &mut NeverCrash).unwrap();
        let local = j
            .admit_internal_create(&create, local_admission(), &mut NeverCrash)
            .unwrap()
            .local
            .unwrap();
        let result = crate::finish_admitted_create_timed(
            &mut j,
            local.sandbox,
            local.operation,
            &mut |action| {
                if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                    Err(crate::WorkerTransportError::ReplyLost)
                } else {
                    workflow_fixture::worker(action)
                }
            },
            &mut || Ok(20),
        );
        assert!(matches!(result, Err(JournalError::UnresolvedAction)));
        let destroy = LifecycleRequest {
            intent: Intent::Destroy,
            identity: request(2, 2, Intent::Destroy).identity,
            ..create.clone()
        };
        assert!(matches!(
            j.record_internal_intent(&destroy, &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(j);
        let mut j = open(&path);
        let terminal = point == CrashPoint::AfterCommit;
        assert_eq!(
            j.world()
                .unwrap()
                .sandbox_record(local.sandbox)
                .unwrap()
                .terminal,
            terminal
        );
        assert_eq!(
            j.internal_pending(&config().node).unwrap(),
            if terminal {
                vec![destroy.clone()]
            } else {
                vec![create.clone()]
            }
        );
        j.record_internal_intent(&destroy, &mut NeverCrash).unwrap();
        assert!(
            j.world()
                .unwrap()
                .sandbox_record(local.sandbox)
                .unwrap()
                .terminal
        );
        assert_eq!(
            j.next_operation_id(local.sandbox).unwrap(),
            core::OperationId(2)
        );
        let before_start_retry = j.world().unwrap();
        assert!(matches!(
            crate::finish_admitted_create_timed(
                &mut j,
                local.sandbox,
                local.operation,
                &mut workflow_fixture::worker,
                &mut || panic!("a terminal create must not resample Running time")
            ),
            Err(JournalError::UnsupportedInput)
        ));
        assert_eq!(j.world().unwrap(), before_start_retry);
        assert_eq!(
            j.capture_internal_progress(&destroy, &mut NeverCrash)
                .unwrap()
                .fact,
            LifecycleFact::Recorded
        );
    }
}

#[test]
fn internal_inbox_old_unbound_format_is_refused_without_upgrade_or_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    drop(j);
    let db = rusqlite::Connection::open(&path).unwrap();
    for version in [12, 13] {
        db.execute(
            "UPDATE controller_meta SET version=?1 WHERE id=1",
            [version],
        )
        .unwrap();
        assert!(matches!(
            super::super::tests_support::open(&path, "0123456789abcdef"),
            Err(JournalError::UnsupportedStore)
        ));
        assert_eq!(
            db.query_row("SELECT version FROM controller_meta", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
}

#[test]
fn internal_inbox_recovery_refuses_saved_ready_without_native_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let r = local_request();
    j.record_internal_intent(&r, &mut NeverCrash).unwrap();
    j.admit_internal_create(&r, local_admission(), &mut NeverCrash)
        .unwrap();
    // Deliberate SQL-only fault: a structurally valid response/digest is not
    // evidence of native Running. Recovery must check the actual projection.
    let mut inbox = codec::load(&j.journal.connection).unwrap().unwrap();
    inbox.observation = 3.try_into().unwrap();
    inbox.entries[0].response.observation_version = inbox.observation;
    inbox.entries[0].response.fact = LifecycleFact::Ready {
        started_at_ms: 10,
        end_at_ms: 60_010,
        max_end_at_ms: 600_010,
    };
    codec::save(&j.journal.connection, &inbox).unwrap();
    drop(j);
    assert!(super::super::tests_support::open(&path, "0123456789abcdef").is_err());
}
