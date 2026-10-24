use super::*;
use crate::{AdmissionError, BoundedHostConfig, CrashPoint, NeverCrash};
use sandbox_controller_core as core;
use tempfile::TempDir;
mod complete;
mod support;
mod unload;
mod validation_certificates;
use support::*;

fn maintenance_config() -> MaintenanceConfig {
    crate::controller::tests_support::fixture::maintenance()
}
fn open(path: &std::path::Path) -> ControllerJournal {
    ControllerJournal::open_complete(path, NS, host_config(4), maintenance_config(), 1000).unwrap()
}
fn count(journal: &ControllerJournal, table: &str) -> i64 {
    journal
        .journal
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
fn checkpoint_and_prune(journal: &mut ControllerJournal) {
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    // The next trusted transaction drains only an already committed cut.
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    let progress = journal.checkpoint_progress().unwrap().unwrap();
    assert_eq!(progress.deleted_through, progress.input_cut);
}
fn release(journal: &mut ControllerJournal, sandbox: u64) {
    let context = journal
        .host_slot_context(core::SandboxId(sandbox))
        .unwrap()
        .unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
}
fn retire(journal: &mut ControllerJournal, sandbox: u64) {
    journal
        .retire_reclaimed(core::SandboxId(sandbox), 1001, |_| Ok(()), &mut NeverCrash)
        .unwrap();
}

#[test]
fn retirement_closes_exact_sql_rows_keeps_query_then_expiry_and_never_reuses_scope() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    create(&mut journal, 1);
    destroy(&mut journal, 1);
    release(&mut journal, 1);
    assert_eq!(count(&journal, "controller_release_witnesses"), 1);
    retire(&mut journal, 1);
    for table in [
        "sandboxes",
        "operations",
        "outbox",
        "controller_receipt_history",
        "controller_effect_keys",
        "sandbox_lifetimes",
        "sandbox_create_options",
        "controller_release_witnesses",
    ] {
        assert_eq!(count(&journal, table), 0, "{table}");
    }
    assert_eq!(count(&journal, "controller_id_highwaters"), 5);
    let query = core::ControllerInput::Request(core::Request::Query(core::QueryRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        sandbox: core::SandboxId(1),
        operation: core::OperationId(1),
    }));
    journal.check(query.clone()).unwrap();
    checkpoint_and_prune(&mut journal);
    drop(journal);
    let mut journal = open(&path);
    journal.check(query.clone()).unwrap();
    journal.expire_retained(1101, &mut NeverCrash).unwrap();
    assert!(journal.check(query.clone()).is_err());
    checkpoint_and_prune(&mut journal);
    drop(journal);
    let journal = open(&path);
    assert!(journal.check(query).is_err());
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
    assert!(journal.next_operation_id(core::SandboxId(1)).is_err());
}

#[test]
fn current_release_witness_survives_rebind_and_restart_without_prefix_dependency() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = ControllerJournal::open_complete(
        &path,
        NS,
        host_config(4),
        crate::controller::tests_support::fixture::maintenance(),
        1000,
    )
    .unwrap();
    create(&mut journal, 1);
    destroy(&mut journal, 1);
    release(&mut journal, 1);
    let old = journal
        .host_slot_context(core::SandboxId(1))
        .unwrap()
        .unwrap();
    create(&mut journal, 2);
    assert!(
        journal
            .host_slot_context(core::SandboxId(1))
            .unwrap()
            .is_none()
    );
    drop(journal);
    let mut journal = open(&path);
    let candidate = journal
        .retirement_candidates()
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.sandbox == core::SandboxId(1))
        .unwrap();
    assert_eq!(candidate.release.unwrap().context, old);
    checkpoint_and_prune(&mut journal);
    drop(journal);
    let mut journal = open(&path);
    retire(&mut journal, 1);
    checkpoint_and_prune(&mut journal);
    drop(journal);
    let journal = open(&path);
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .is_none()
    );
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(2))
            .unwrap()
            .phase,
        core::Phase::Running
    );
}

#[test]
fn checkpoint_lost_commit_reply_recovers_both_durable_outcomes_without_deleting_prefix() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        create(&mut journal, 1);
        let world = journal.world().unwrap();
        let inputs = count(&journal, "controller_inputs");
        crate::controller::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.publish_checkpoint(&mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        assert_eq!(count(&journal, "controller_inputs"), inputs);
        assert_eq!(journal.world().unwrap(), world);
        drop(journal);
        assert_eq!(open(&path).world().unwrap(), world);
    }
}

#[test]
fn retirement_rollback_and_unknown_commit_never_mix_retained_and_deleted_state() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        create(&mut journal, 1);
        destroy(&mut journal, 1);
        release(&mut journal, 1);
        crate::controller::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.retire_reclaimed(core::SandboxId(1), 1001, |_| Ok(()), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        let world = journal.world().unwrap();
        assert_eq!(world.sandbox_record(core::SandboxId(1)).is_none(), durable);
        assert_eq!(count(&journal, "sandboxes"), i64::from(!durable));
        assert_eq!(
            count(&journal, "controller_release_witnesses"),
            i64::from(!durable)
        );
        drop(journal);
        assert_eq!(open(&path).world().unwrap(), world);
    }
}

#[test]
fn damaged_root_or_missing_tail_fails_closed_even_with_valid_remaining_maximum() {
    for corruption in [0, 1, 2, 3] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        create(&mut journal, 1);
        checkpoint_and_prune(&mut journal);
        create(&mut journal, 2);
        drop(journal);
        let raw = Connection::open(&path).unwrap();
        match corruption {
            0 => {
                raw.execute("UPDATE controller_checkpoint SET sha256=zeroblob(32)", [])
                    .unwrap();
            }
            1 => {
                raw.execute("DELETE FROM controller_checkpoint", [])
                    .unwrap();
            }
            2 => {
                raw.execute("DELETE FROM controller_inputs WHERE sequence=(SELECT MIN(sequence)+1 FROM controller_inputs)",[]).unwrap();
            }
            _ => {
                raw.execute("DELETE FROM controller_inputs WHERE sequence=(SELECT MAX(sequence) FROM controller_inputs)",[]).unwrap();
            }
        }
        drop(raw);
        assert!(
            ControllerJournal::open_complete(&path, NS, host_config(4), maintenance_config(), 1000)
                .is_err()
        );
    }
}

#[test]
fn current_sql_only_lifetime_and_registry_changes_survive_older_checkpoint() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    create(&mut journal, 1);
    checkpoint_and_prune(&mut journal);
    let cut = allocators::input_high_water(&journal.journal.connection).unwrap();
    let lifetime = journal
        .set_lifetime(
            &core::Actor::Tenant(core::TenantId(1)),
            core::SandboxId(1),
            2000,
            100,
        )
        .unwrap();
    journal
        .enable_template_registry(&registry_config(&dir))
        .unwrap();
    let publication = publication('a');
    journal
        .begin_template_publication(&publication, &registry_config(&dir))
        .unwrap();
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        cut
    );
    drop(journal);
    let journal = open(&path);
    let restored = journal
        .live_lifetime(
            &core::Actor::Tenant(core::TenantId(1)),
            core::SandboxId(1),
            2000,
        )
        .unwrap();
    assert_eq!(restored, lifetime);
    assert_eq!(
        journal
            .template_publication(1, &publication.candidate_id)
            .unwrap(),
        Some(publication)
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the three publication states share one complete real admission/cleanup/pin/reopen matrix"
)]
fn only_unsettled_template_validation_pins_release_and_retirement() {
    use crate::{
        CreateAdmission, CreateOptions, template_registry::PublicationState,
        templates::ComputeLimits,
    };
    for state in [
        PublicationState::Checking,
        PublicationState::Ready,
        PublicationState::Failed,
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .enable_template_registry(&registry_config(&dir))
            .unwrap();
        let publication = publication('a');
        journal
            .begin_template_publication(&publication, &registry_config(&dir))
            .unwrap();
        journal
            .admit_template_validation(
                request(1),
                CreateAdmission {
                    now_ms: 1000,
                    timeout_secs: 60,
                    disk_bytes: 1000,
                    limits: limits(),
                    options: CreateOptions {
                        template: Some(publication.binding.clone()),
                        ..CreateOptions::default()
                    },
                },
                ComputeLimits {
                    max_vcpus_global: 10,
                    max_vcpus_per_tenant: 10,
                    max_memory_bytes_global: 1_000_000_000,
                    max_memory_bytes_per_tenant: 1_000_000_000,
                },
                &publication.candidate_id,
                &mut NeverCrash,
            )
            .unwrap();
        crate::finish_admitted_create(
            &mut journal,
            core::SandboxId(1),
            core::OperationId(1),
            &mut success,
        )
        .unwrap();
        if state == PublicationState::Ready {
            journal
                .mark_template_checked(1, &publication.candidate_id)
                .unwrap();
        }
        destroy(&mut journal, 1);
        if state != PublicationState::Checking {
            journal.reconcile_template_publications(false).unwrap();
        }
        assert_eq!(
            journal
                .template_publication(1, &publication.candidate_id)
                .unwrap()
                .unwrap()
                .state,
            state
        );
        if state != PublicationState::Checking {
            assert!(
                !journal
                    .host_slot_release_pinned(core::SandboxId(1))
                    .unwrap()
            );
            release(&mut journal, 1);
            retire(&mut journal, 1);
            checkpoint_and_prune(&mut journal);
            drop(journal);
            let journal = open(&path);
            assert!(
                journal
                    .world()
                    .unwrap()
                    .sandbox_record(core::SandboxId(1))
                    .is_none()
            );
            assert_eq!(
                journal
                    .template_publication(1, &publication.candidate_id)
                    .unwrap()
                    .unwrap()
                    .state,
                state
            );
            continue;
        }
        let context = journal
            .host_slot_context(core::SandboxId(1))
            .unwrap()
            .unwrap();
        let mut called = false;
        assert!(
            journal
                .release_host_slot(
                    context,
                    |_| {
                        called = true;
                        Ok(())
                    },
                    &mut NeverCrash
                )
                .is_err()
        );
        assert!(!called);
        assert!(journal.retirement_candidates().unwrap().is_empty());
        assert!(
            journal
                .retire_reclaimed(
                    core::SandboxId(1),
                    1001,
                    |_| {
                        called = true;
                        Ok(())
                    },
                    &mut NeverCrash
                )
                .is_err()
        );
        assert!(!called);
        checkpoint_and_prune(&mut journal);
        drop(journal);
        let journal = open(&path);
        assert!(
            journal
                .world()
                .unwrap()
                .sandbox_record(core::SandboxId(1))
                .is_some()
        );
        assert!(
            journal
                .host_slot_release_pinned(core::SandboxId(1))
                .unwrap()
        );
    }
}

#[test]
fn receipt_history_is_initial_unknown_then_final_without_unknown_poll_growth() {
    use sandproof_protocol::{UnknownReason, WorkOutcome, WorkReceipt};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    admit(&mut journal, 1, &mut NeverCrash).unwrap();
    let action = journal.claim_next(None, &mut NeverCrash).unwrap().unwrap();
    let unknown = WorkReceipt::<support::Evidence> {
        identity: action.identity.clone(),
        outcome: WorkOutcome::Unknown {
            reason: UnknownReason::ObservationUnavailable,
            last_observation: None,
        },
    };
    journal.complete(&unknown, &mut NeverCrash).unwrap();
    let inputs = count(&journal, "controller_inputs");
    for _ in 0..8 {
        journal.complete(&unknown, &mut NeverCrash).unwrap();
    }
    assert_eq!(count(&journal, "controller_inputs"), inputs);
    assert_eq!(count(&journal, "controller_receipt_history"), 1);
    let mut changed = unknown.clone();
    changed.outcome = WorkOutcome::Unknown {
        reason: UnknownReason::WorkerRestarted,
        last_observation: None,
    };
    assert!(journal.complete(&changed, &mut NeverCrash).is_err());
    assert_eq!(count(&journal, "controller_receipt_history"), 1);
    journal
        .complete(&success(&action).unwrap(), &mut NeverCrash)
        .unwrap();
    assert_eq!(count(&journal, "controller_receipt_history"), 2);
    checkpoint_and_prune(&mut journal);
    drop(journal);
    let journal = open(&path);
    assert_eq!(count(&journal, "controller_receipt_history"), 2);
}

#[test]
fn incomplete_or_changed_configuration_never_rewrites_current_store() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    create(&mut journal, 1);
    let world = journal.world().unwrap();
    drop(journal);
    let before = std::fs::read(&path).unwrap();
    for capacity in [false, true] {
        let mut policy = maintenance_config();
        if capacity {
            policy.max_checkpoint_bytes = 16;
        } else {
            policy.capacity = None;
        }
        assert!(ControllerJournal::open_complete(&path, NS, host_config(4), policy, 1000).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    assert_eq!(open(&path).world().unwrap(), world);
}

#[test]
fn release_witness_and_global_frontiers_cannot_disappear_when_rows_are_pruned() {
    for sql in [
        "DELETE FROM controller_release_witnesses",
        "UPDATE controller_id_highwaters SET high_water='0' WHERE kind='sandbox'",
        "UPDATE controller_id_highwaters SET high_water='0' WHERE kind='input'",
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        create(&mut journal, 1);
        destroy(&mut journal, 1);
        release(&mut journal, 1);
        checkpoint_and_prune(&mut journal);
        drop(journal);
        let raw = Connection::open(&path).unwrap();
        raw.execute(sql, []).unwrap();
        drop(raw);
        assert!(
            ControllerJournal::open_complete(&path, NS, host_config(4), maintenance_config(), 1000)
                .is_err(),
            "{sql}"
        );
    }
}

#[test]
fn no_release_witness_is_invented_from_incomplete_current_release() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    create(&mut journal, 1);
    destroy(&mut journal, 1);
    let context = journal
        .host_slot_context(core::SandboxId(1))
        .unwrap()
        .unwrap();
    assert!(
        journal
            .release_host_slot(
                context,
                |_| Err(crate::WorkerTransportError::ConnectionLost),
                &mut NeverCrash
            )
            .is_err()
    );
    drop(journal);
    let journal = open(&path);
    assert_eq!(count(&journal, "controller_release_witnesses"), 0);
    assert!(journal.retirement_candidates().unwrap().is_empty());
}

#[test]
fn unchanged_foreign_write_and_single_writer_rules_apply_to_checkpoint_mode() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    create(&mut journal, 1);
    checkpoint_and_prune(&mut journal);
    assert!(matches!(
        ControllerJournal::open_complete(&path, NS, host_config(4), maintenance_config(), 1000),
        Err(JournalError::ControllerBusy)
    ));
    assert!(matches!(
        crate::Journal::open(&path),
        Err(JournalError::UnsupportedStore)
    ));
    let external = Connection::open(&path).unwrap();
    external
        .execute("UPDATE sandbox_lifetimes SET deadline_ms=deadline_ms+1", [])
        .unwrap();
    assert!(matches!(journal.world(), Err(JournalError::CorruptStore)));
    assert!(journal.publish_checkpoint(&mut NeverCrash).is_err());
}

#[test]
fn retirement_fault_after_sql_deletion_rolls_back_but_post_commit_fault_preserves_retirement() {
    for point in [
        CrashPoint::AfterRetirementDelete,
        CrashPoint::BeforeCommit,
        CrashPoint::AfterCommit,
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        create(&mut journal, 1);
        destroy(&mut journal, 1);
        release(&mut journal, 1);
        assert!(
            matches!(journal.retire_reclaimed(core::SandboxId(1),1001,|_|Ok(()),&mut FailAt(point)),Err(JournalError::InjectedCrash(found)) if found==point)
        );
        let committed = point == CrashPoint::AfterCommit;
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(core::SandboxId(1))
                .is_none(),
            committed
        );
        assert_eq!(
            count(&journal, "controller_release_witnesses"),
            i64::from(!committed)
        );
        drop(journal);
        let journal = open(&path);
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(core::SandboxId(1))
                .is_none(),
            committed
        );
    }
}

mod capacity;

mod capacity_protocol;

mod build_capacity;
