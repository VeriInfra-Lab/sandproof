//! Differential checks compare checkpoint/tail recovery with the committed memo.
use super::*;
use crate::{AdmissionError, AdmissionLimits, NeverCrash, finish_admitted_create_v1_timed};
use rusqlite::types::Value;
use sandproof_protocol::{ResourceRef, UnknownReason};
use serde::Deserialize;
use tempfile::TempDir;

const NS: &str = "0123456789abcdef";

fn setup() -> (TempDir, ControllerJournal) {
    let dir = TempDir::new().unwrap();
    let journal =
        crate::controller::tests_support::open(dir.path().join("controller.db"), NS).unwrap();
    (dir, journal)
}

fn request(id: u64) -> core::CreateRequest {
    core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: core::SandboxId(id),
        workspace: core::FsId(id),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(id),
    }
}

fn create(id: u64) -> core::ControllerInput {
    core::ControllerInput::Request(core::Request::Create(request(id)))
}

fn limits() -> AdmissionLimits {
    AdmissionLimits {
        max_active_global: 10,
        max_active_per_tenant: 10,
        max_disk_bytes_global: crate::controller::tests_support::fixture::CLEANUP_BYTES + 100_000,
        max_disk_bytes_per_tenant: 100_000,
        cleanup_reserve_bytes: crate::controller::tests_support::fixture::CLEANUP_BYTES,
        max_timeout_secs: 600,
        max_lifetime_secs: 600,
        startup_timeout_secs: 60,
    }
}

fn admit(
    j: &mut ControllerJournal,
    id: u64,
    crash: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, AdmissionError> {
    j.admit_create(request(id), 1000, 60, 1000, limits(), crash)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Evidence {
    resource: ResourceRef,
    command: core::Command,
}
impl ResourceEvidence for Evidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}
impl HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.command == *command
    }
}
fn success(action: &WorkAction<core::EffectId>) -> WorkReceipt<Evidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(Evidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
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

fn count(j: &ControllerJournal) -> i64 {
    j.journal
        .connection
        .query_row("SELECT count(*) FROM controller_inputs", [], |r| r.get(0))
        .unwrap()
}

fn parity(j: &ControllerJournal) -> core::World {
    let replay = storage::load_world(&j.journal.connection).unwrap();
    assert_eq!(j.world().unwrap(), replay);
    replay
}

// Include every persisted table, all SQL values and table names. Rowid order is
// deterministic for these fixtures and catches prefixes, IDs and projections.
fn snapshot(j: &ControllerJournal) -> Vec<(String, Vec<Vec<Value>>)> {
    let db = &j.journal.connection;
    let tables: Vec<String> = db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    tables
        .into_iter()
        .map(|name| {
            let mut stmt = db
                .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))
                .unwrap();
            let columns = stmt.column_count();
            let rows = stmt
                .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn ready_to_publish(j: &mut ControllerJournal) {
    admit(j, 1, &mut NeverCrash).unwrap();
    assert!(
        finish_admitted_create_v1_timed(
            j,
            core::SandboxId(1),
            core::OperationId(1),
            &mut |a: &WorkAction<core::EffectId>| Ok(success(a)),
            &mut || Err(JournalError::UnsupportedInput)
        )
        .is_err()
    );
    parity(j);
}

fn running(j: &mut ControllerJournal) {
    ready_to_publish(j);
    j.publish_running_at(core::SandboxId(1), 2000, &mut NeverCrash)
        .unwrap();
    parity(j);
}

#[test]
fn admission_hooks_publish_exact_committed_tip_on_same_object() {
    let (dir, mut j) = setup();
    let initial = parity(&j);
    let before = snapshot(&j);
    assert!(admit(&mut j, 1, &mut FailAt(CrashPoint::BeforeCommit)).is_err());
    assert_eq!(snapshot(&j), before);
    assert_eq!(parity(&j), initial);
    assert!(admit(&mut j, 1, &mut FailAt(CrashPoint::AfterCommit)).is_err());
    assert_eq!(count(&j), 3); // create, reserve startup, bind position.
    let world = parity(&j);
    let after = snapshot(&j);
    assert!(admit(&mut j, 1, &mut NeverCrash).is_err());
    assert_eq!(snapshot(&j), after);
    drop(j);
    let reopened =
        crate::controller::tests_support::open(dir.path().join("controller.db"), NS).unwrap();
    assert_eq!(parity(&reopened), world);
}

#[test]
fn claim_publishes_dispatch_never_worker_preview_and_receipts_are_incremental() {
    let (_dir, mut j) = setup();
    admit(&mut j, 1, &mut NeverCrash).unwrap();
    let before = snapshot(&j);
    assert!(
        j.claim_next(None, &mut FailAt(CrashPoint::BeforeDispatchClaim))
            .is_err()
    );
    assert_eq!(snapshot(&j), before);
    assert!(
        j.claim_next(None, &mut FailAt(CrashPoint::AfterDispatchClaim))
            .is_err()
    );
    let claimed = parity(&j);
    let action = j.uncertain_actions().unwrap().pop().unwrap();
    assert_eq!(
        claimed.effect_status(&action.command),
        Some(core::EffectStatus::Dispatched)
    );
    assert!(!core::effect::command_result_present(
        &claimed,
        action.command.command()
    ));
    assert_eq!(count(&j), 4);
    let before = snapshot(&j);
    assert!(
        j.complete(&success(&action), &mut FailAt(CrashPoint::BeforeCommit))
            .is_err()
    );
    assert_eq!(snapshot(&j), before);
    assert_eq!(parity(&j), claimed);
    assert!(
        j.complete(&success(&action), &mut FailAt(CrashPoint::AfterCommit))
            .is_err()
    );
    assert_eq!(count(&j), 6); // Worker then Receipt, not one cached prefix.
    parity(&j);
    let completed = snapshot(&j);
    j.complete(&success(&action), &mut NeverCrash).unwrap();
    assert_eq!(snapshot(&j), completed);
    parity(&j);
}

#[test]
fn unknown_then_success_and_rejected_evidence_keep_replay_parity() {
    let (_dir, mut j) = setup();
    admit(&mut j, 1, &mut NeverCrash).unwrap();
    let a = j.claim_next(None, &mut NeverCrash).unwrap().unwrap();
    let unknown = WorkReceipt::<Evidence> {
        identity: a.identity.clone(),
        outcome: WorkOutcome::Unknown {
            reason: UnknownReason::ObservationUnavailable,
            last_observation: None,
        },
    };
    j.complete(&unknown, &mut NeverCrash).unwrap();
    assert_eq!(count(&j), 5);
    parity(&j);
    let before = snapshot(&j);
    let mut bad = success(&a);
    bad.identity.epoch += 1;
    assert!(j.complete(&bad, &mut NeverCrash).is_err());
    assert_eq!(snapshot(&j), before);
    parity(&j);
    j.complete(&success(&a), &mut NeverCrash).unwrap();
    assert_eq!(count(&j), 7);
    parity(&j);
}

#[test]
fn sql_failure_after_worker_input_rolls_back_every_projection_and_candidate() {
    let (_dir, mut j) = setup();
    admit(&mut j, 1, &mut NeverCrash).unwrap();
    let a = j.claim_next(None, &mut NeverCrash).unwrap().unwrap();
    let before = snapshot(&j);
    j.journal.connection.execute_batch("CREATE TEMP TRIGGER fail_receipt BEFORE INSERT ON controller_inputs WHEN (SELECT count(*) FROM controller_inputs)=4 BEGIN SELECT RAISE(ABORT, 'test receipt failure'); END;").unwrap();
    assert!(j.complete(&success(&a), &mut NeverCrash).is_err());
    assert_eq!(snapshot(&j), before);
    parity(&j);
    j.journal
        .connection
        .execute_batch("DROP TRIGGER fail_receipt")
        .unwrap();
    j.complete(&success(&a), &mut NeverCrash).unwrap();
    parity(&j);
}

#[test]
fn admission_auxiliary_failure_does_not_publish_create_or_reserved_ids() {
    let (_dir, mut j) = setup();
    let initial = parity(&j);
    let before = snapshot(&j);
    j.journal.connection.execute_batch("CREATE TEMP TRIGGER fail_options BEFORE INSERT ON sandbox_create_options BEGIN SELECT RAISE(ABORT, 'test options failure'); END;").unwrap();
    assert!(admit(&mut j, 1, &mut NeverCrash).is_err());
    assert_eq!(snapshot(&j), before);
    assert_eq!(parity(&j), initial);
    j.journal
        .connection
        .execute_batch("DROP TRIGGER fail_options")
        .unwrap();
    let accepted = admit(&mut j, 1, &mut NeverCrash).unwrap();
    assert_eq!(
        accepted
            .next
            .operation_plan(core::OperationKey {
                sandbox: core::SandboxId(1),
                operation: core::OperationId(1)
            })
            .unwrap()
            .vm,
        Some(core::VmId(0))
    );
    parity(&j);
}

#[test]
fn multi_startup_batch_rolls_back_prefix_and_publishes_only_complete_commit() {
    let (_dir, mut j) = setup();
    admit(&mut j, 1, &mut NeverCrash).unwrap();
    admit(&mut j, 2, &mut NeverCrash).unwrap();
    let before = snapshot(&j);
    j.journal.connection.execute_batch("CREATE TEMP TRIGGER fail_second BEFORE INSERT ON controller_inputs WHEN (SELECT count(*) FROM controller_inputs)=7 BEGIN SELECT RAISE(ABORT, 'test second input'); END;").unwrap();
    assert!(j.abort_due_startups(61_000, &mut NeverCrash).is_err());
    assert_eq!(snapshot(&j), before);
    parity(&j);
    j.journal
        .connection
        .execute_batch("DROP TRIGGER fail_second")
        .unwrap();
    assert!(
        j.abort_due_startups(61_000, &mut FailAt(CrashPoint::AfterCommit))
            .is_err()
    );
    assert_eq!(count(&j), 8);
    assert!(
        parity(&j)
            .sandbox_records()
            .iter()
            .all(|(_, r)| r.phase == core::Phase::Revoking)
    );
    assert_eq!(j.abort_due_startups(61_000, &mut NeverCrash).unwrap(), 0);
    parity(&j);
}

#[test]
fn publication_and_expiry_hooks_keep_lifetime_and_cached_world_atomic() {
    let (_dir, mut j) = setup();
    ready_to_publish(&mut j);
    let before = snapshot(&j);
    assert!(
        j.publish_running_at(
            core::SandboxId(1),
            2000,
            &mut FailAt(CrashPoint::BeforeCommit)
        )
        .is_err()
    );
    assert_eq!(snapshot(&j), before);
    parity(&j);
    assert!(
        j.publish_running_at(
            core::SandboxId(1),
            2000,
            &mut FailAt(CrashPoint::AfterCommit)
        )
        .is_err()
    );
    assert_eq!(
        parity(&j).sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Running
    );
    let actor = core::Actor::Tenant(core::TenantId(1));
    let inputs = count(&j);
    let world = parity(&j);
    j.connect_lifetime(&actor, core::SandboxId(1), 3000, Some(100), limits())
        .unwrap();
    assert!(matches!(
        j.accept_expiry(
            core::SandboxId(1),
            core::OperationId(2),
            62_000,
            &mut NeverCrash
        ),
        Err(AdmissionError::NotDue)
    ));
    assert_eq!(count(&j), inputs);
    assert_eq!(parity(&j), world);
    j.set_lifetime(&actor, core::SandboxId(1), 4000, 0).unwrap();
    assert!(matches!(
        j.live_lifetime(&actor, core::SandboxId(1), 4000),
        Err(AdmissionError::Expired)
    ));
    assert_eq!(count(&j), inputs);
    let before = snapshot(&j);
    assert!(
        j.accept_expiry(
            core::SandboxId(1),
            core::OperationId(2),
            4000,
            &mut FailAt(CrashPoint::BeforeCommit)
        )
        .is_err()
    );
    assert_eq!(snapshot(&j), before);
    parity(&j);
    assert!(
        j.accept_expiry(
            core::SandboxId(1),
            core::OperationId(2),
            4000,
            &mut FailAt(CrashPoint::AfterCommit)
        )
        .is_err()
    );
    assert_eq!(
        parity(&j).sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Revoking
    );
}

fn registry_config(dir: &TempDir) -> crate::template_registry::RegistryConfig {
    crate::template_registry::RegistryConfig {
        root: dir.path().join("registry"),
        max_templates_global: 10,
        max_templates_per_tenant: 10,
        max_artifact_bytes_global: 100_000,
        max_artifact_bytes_per_tenant: 100_000,
        validation_timeout_secs: 60,
    }
}

fn journal_error(error: AdmissionError) -> JournalError {
    match error {
        AdmissionError::Journal(error) => error,
        other => panic!("unexpected admission error: {other:?}"),
    }
}

fn prepare_uncertain_family(
    j: &mut ControllerJournal,
    family: &str,
) -> Option<WorkAction<core::EffectId>> {
    let mut action = None;
    match family {
        "claim" => {
            admit(j, 1, &mut NeverCrash).unwrap();
        }
        "receipt" => {
            admit(j, 1, &mut NeverCrash).unwrap();
            action = j.claim_next(None, &mut NeverCrash).unwrap();
        }
        "publication" => ready_to_publish(j),
        "startup_batch" => {
            admit(j, 1, &mut NeverCrash).unwrap();
            admit(j, 2, &mut NeverCrash).unwrap();
        }
        "expiry" => {
            running(j);
            j.set_lifetime(
                &core::Actor::Tenant(core::TenantId(1)),
                core::SandboxId(1),
                3000,
                0,
            )
            .unwrap();
        }
        "lifetime" => running(j),
        _ => {}
    }
    action
}

const UNCERTAIN_FAMILIES: [&str; 8] = [
    "admission",
    "claim",
    "receipt",
    "publication",
    "startup_batch",
    "expiry",
    "lifetime",
    "registry",
];

#[test]
fn uncertain_commit_reconciles_both_durable_outcomes_for_every_write_family() {
    for durable in [false, true] {
        for family in UNCERTAIN_FAMILIES {
            let (dir, mut j) = setup();
            let action = prepare_uncertain_family(&mut j, family);
            let before = snapshot(&j);
            let inputs = count(&j);
            let world_before = parity(&j);
            let mut host_calls = 0;
            transaction::fail_next_commit(durable);
            let result = match family {
                "admission" => admit(&mut j, 1, &mut NeverCrash)
                    .map(|_| ())
                    .map_err(journal_error),
                "claim" => j
                    .dispatch_next(
                        &mut |a: &WorkAction<core::EffectId>| {
                            host_calls += 1;
                            Ok(success(a))
                        },
                        &mut NeverCrash,
                    )
                    .map(|_| ()),
                "receipt" => j
                    .complete(&success(action.as_ref().unwrap()), &mut NeverCrash)
                    .map(|_| ()),
                "publication" => j
                    .publish_running_at(core::SandboxId(1), 2000, &mut NeverCrash)
                    .map(|_| ()),
                "startup_batch" => j
                    .abort_due_startups(61_000, &mut NeverCrash)
                    .map(|_| ())
                    .map_err(journal_error),
                "expiry" => j
                    .accept_expiry(
                        core::SandboxId(1),
                        core::OperationId(2),
                        3000,
                        &mut NeverCrash,
                    )
                    .map(|_| ())
                    .map_err(journal_error),
                "lifetime" => j
                    .set_lifetime(
                        &core::Actor::Tenant(core::TenantId(1)),
                        core::SandboxId(1),
                        3000,
                        0,
                    )
                    .map(|_| ())
                    .map_err(journal_error),
                "registry" => j.enable_template_registry(&registry_config(&dir)),
                _ => unreachable!(),
            };
            assert!(
                matches!(result, Err(JournalError::CommitUncertain(_))),
                "{family}/{durable}: {result:?}"
            );
            assert_eq!(host_calls, 0, "a lost claim confirmation must not dispatch");
            assert!(j.journal.connection.is_autocommit());
            let world = parity(&j); // Same object must reconcile before any retry.
            if durable {
                assert_ne!(snapshot(&j), before, "{family}");
                if matches!(family, "lifetime" | "registry") {
                    assert_eq!(count(&j), inputs);
                    assert_eq!(world, world_before);
                } else {
                    assert!(count(&j) > inputs);
                }
            } else {
                assert_eq!(snapshot(&j), before, "{family}");
                assert_eq!(world, world_before);
            }
            if family == "lifetime" {
                assert_eq!(
                    j.live_lifetime(
                        &core::Actor::Tenant(core::TenantId(1)),
                        core::SandboxId(1),
                        3000
                    )
                    .is_err(),
                    durable
                );
            }
            if family == "registry" {
                assert_eq!(j.has_template_registry().unwrap(), durable);
            }
            let after = snapshot(&j);
            drop(j);
            let reopened =
                crate::controller::tests_support::open(dir.path().join("controller.db"), NS)
                    .unwrap();
            assert_eq!(parity(&reopened), world);
            assert_eq!(snapshot(&reopened), after);
        }
    }
}

#[test]
fn failed_rollback_and_active_connection_never_serve_uncommitted_candidate() {
    let (_dir, mut j) = setup();
    let initial = parity(&j);
    {
        let tx =
            transaction::WorldTransaction::begin(&mut j.journal.connection, &j.committed_world)
                .unwrap();
        storage::apply(&tx, NS, create(1)).unwrap();
        // SQLite may roll a transaction back itself on certain failures. The
        // wrapper's later ROLLBACK then fails; it must invalidate its memo.
        tx.execute_batch("ROLLBACK").unwrap();
    }
    assert_eq!(parity(&j), initial);
    j.journal
        .connection
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    assert!(matches!(j.world(), Err(JournalError::CorruptStore)));
    assert!(matches!(
        j.has_template_registry(),
        Err(JournalError::CorruptStore)
    ));
    assert!(j.create_options(core::SandboxId(1)).is_err());
    assert!(j.apply(create(1), &mut NeverCrash).is_err());
    j.journal.connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(parity(&j), initial);
    admit(&mut j, 1, &mut NeverCrash).unwrap();
    parity(&j);
}

#[test]
fn foreign_historical_and_sql_only_commits_are_not_silently_reblessed() {
    for sql in [
        "UPDATE controller_inputs SET input_json=X'00' WHERE sequence=(SELECT MAX(sequence) FROM controller_inputs)",
        "UPDATE sandboxes SET state_blob=X'00'",
        "UPDATE sandbox_lifetimes SET deadline_ms=0",
    ] {
        let (dir, mut j) = setup();
        admit(&mut j, 1, &mut NeverCrash).unwrap();
        parity(&j);
        let external = rusqlite::Connection::open(dir.path().join("controller.db")).unwrap();
        external.execute(sql, []).unwrap();
        assert!(matches!(j.world(), Err(JournalError::CorruptStore)));
        assert!(
            matches!(j.world(), Err(JournalError::CorruptStore)),
            "must not absorb foreign data_version"
        );
        assert!(j.create_options(core::SandboxId(1)).is_err());
        assert!(j.apply(create(2), &mut NeverCrash).is_err());
    }
}

#[test]
fn old_and_new_generic_handles_cannot_write_controller_state() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let mut old = Journal {
        connection: rusqlite::Connection::open(&path).unwrap(),
    };
    let mut j = crate::controller::tests_support::open(&path, NS).unwrap();
    admit(&mut j, 1, &mut NeverCrash).unwrap();
    assert!(matches!(
        Journal::open(&path),
        Err(JournalError::UnsupportedStore)
    ));
    let before = snapshot(&j);
    let public = j.sandbox_id(core::SandboxId(1)).unwrap();
    let write = crate::AcceptedWrite::<core::EffectId> {
        owner: identity::owner_id(core::TenantId(1)).unwrap(),
        sandbox: public,
        operation: identity::operation_id(
            NS,
            core::OperationKey {
                sandbox: core::SandboxId(1),
                operation: core::OperationId(1),
            },
        )
        .unwrap(),
        expected_revision: Some(2),
        sandbox_state: vec![],
        operation_state: vec![],
        actions: vec![],
    };
    assert!(matches!(
        old.commit(&write, &mut NeverCrash),
        Err(JournalError::UnsupportedStore)
    ));
    let mut calls = 0;
    assert!(matches!(
        old.dispatch_next(
            &mut |a: &WorkAction<core::EffectId>| {
                calls += 1;
                Ok(success(a))
            },
            &mut NeverCrash
        ),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(calls, 0);
    let a = j.claim_next(None, &mut NeverCrash).unwrap().unwrap();
    assert!(matches!(
        old.record_receipt(&success(&a)),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(count(&j), 4);
    assert_ne!(snapshot(&j), before); // Only the authorized controller claim.
    parity(&j);
}

fn publication(id: char) -> crate::template_registry::Publication {
    use crate::{
        template_registry::{Publication, PublicationState},
        templates::{ResourceShape, TemplateArtifact, TemplateBinding},
    };
    let candidate_id = id.to_string().repeat(32);
    Publication {
        binding: TemplateBinding {
            guest: None,
            template_id: format!("tpl-{candidate_id}"),
            revision: "r1".into(),
            owner: Some(1),
            startup: None,
            kernel: TemplateArtifact {
                path: "/test/kernel".into(),
                sha256: "a".repeat(64),
                size_bytes: 1,
            },
            rootfs: TemplateArtifact {
                path: "/test/rootfs".into(),
                sha256: "b".repeat(64),
                size_bytes: 1000,
            },
            resources: ResourceShape {
                cpu_count: 1,
                memory_mib: 128,
                disk_bytes: 1000,
                host_memory_bytes: 256 * 1024 * 1024,
            },
            envd_version: "0.9.0".into(),
        },
        candidate_id,
        image_digest: format!("sha256:{}", "a".repeat(64)),
        free_bytes: 500,
        min_free_bytes: 100,
        state: PublicationState::Checking,
        checked: false,
        validation_sandbox: None,
        validation_certificate: None,
        failure: None,
    }
}

fn sql_only<T>(
    j: &mut ControllerJournal,
    call: impl FnOnce(&mut ControllerJournal) -> Result<T, JournalError>,
) -> T {
    let world = parity(j);
    let inputs = count(j);
    let result = call(j).unwrap();
    assert_eq!(count(j), inputs);
    assert_eq!(parity(j), world);
    result
}

#[test]
fn every_registry_commit_preserves_world_but_queries_read_current_sql() {
    use crate::{
        CreateAdmission, CreateOptions,
        template_registry::{PublicationFailure, PublicationState},
        templates::ComputeLimits,
    };
    let (dir, mut j) = setup();
    let config = registry_config(&dir);
    sql_only(&mut j, |j| j.enable_template_registry(&config));
    let value = publication('a');
    assert!(sql_only(&mut j, |j| j.begin_template_publication(&value, &config)));
    j.admit_template_validation(
        request(1),
        CreateAdmission {
            now_ms: 1000,
            timeout_secs: 60,
            disk_bytes: 1000,
            limits: limits(),
            options: CreateOptions {
                template: Some(value.binding.clone()),
                ..CreateOptions::default()
            },
        },
        ComputeLimits {
            max_vcpus_global: 10,
            max_vcpus_per_tenant: 10,
            max_memory_bytes_global: 1_000_000_000,
            max_memory_bytes_per_tenant: 1_000_000_000,
        },
        &value.candidate_id,
        &mut NeverCrash,
    )
    .unwrap();
    finish_admitted_create_v1_timed(
        &mut j,
        core::SandboxId(1),
        core::OperationId(1),
        &mut |a: &WorkAction<core::EffectId>| Ok(success(a)),
        &mut || Ok(2000),
    )
    .unwrap();
    parity(&j);
    sql_only(&mut j, |j| j.mark_template_checked(1, &value.candidate_id));
    crate::kill_v1(
        &mut j,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: core::SandboxId(1),
            operation: core::OperationId(2),
        },
        &mut |a: &WorkAction<core::EffectId>| Ok(success(a)),
    )
    .unwrap();
    parity(&j);
    sql_only(&mut j, |j| j.reconcile_template_publications(false));
    assert_eq!(
        j.template_publication(1, &value.candidate_id)
            .unwrap()
            .unwrap()
            .state,
        PublicationState::Ready
    );
    sql_only(&mut j, |j| {
        j.set_template_alias(1, "cache-test", &value.binding.template_id, None)
    });
    assert!(
        j.resolve_registered_template(1, "cache-test")
            .unwrap()
            .is_some()
    );
    assert!(sql_only(&mut j, |j| j.delete_template_alias(1, "cache-test")));
    assert!(
        j.resolve_registered_template(1, "cache-test")
            .unwrap()
            .is_none()
    );
    let rejected = publication('b');
    sql_only(&mut j, |j| j.begin_template_publication(&rejected, &config));
    sql_only(&mut j, |j| {
        j.reject_template_publication(1, &rejected.candidate_id, PublicationFailure::ArtifactCheck)
    });
    assert_eq!(
        j.template_publication(1, &rejected.candidate_id)
            .unwrap()
            .unwrap()
            .state,
        PublicationState::Failed
    );
}

#[test]
fn warm_reads_and_own_sql_only_commits_do_not_replay_history() {
    let (_dir, mut j) = setup();
    running(&mut j);
    let before = storage::FULL_REPLAYS.with(std::cell::Cell::get);
    for _ in 0..5 {
        assert_eq!(j.world().unwrap().sandbox_records().len(), 1);
        j.live_lifetime(
            &core::Actor::Tenant(core::TenantId(1)),
            core::SandboxId(1),
            3000,
        )
        .unwrap();
        j.create_options(core::SandboxId(1)).unwrap();
        assert!(j.due_lifetimes(3000).unwrap().is_empty());
    }
    j.set_lifetime(
        &core::Actor::Tenant(core::TenantId(1)),
        core::SandboxId(1),
        3000,
        100,
    )
    .unwrap();
    assert_eq!(
        j.live_lifetime(
            &core::Actor::Tenant(core::TenantId(1)),
            core::SandboxId(1),
            3000
        )
        .unwrap()
        .deadline_ms,
        103_000
    );
    assert_eq!(storage::FULL_REPLAYS.with(std::cell::Cell::get), before);
    parity(&j);
}

#[test]
fn panic_before_commit_rolls_back_candidate_and_same_object_remains_usable() {
    struct PanicBefore;
    impl CrashInjector for PanicBefore {
        fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
            assert_ne!(point, CrashPoint::BeforeCommit, "test panic before commit");
            Ok(())
        }
    }
    let (_dir, mut j) = setup();
    let initial = parity(&j);
    let before = snapshot(&j);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| admit(
            &mut j,
            1,
            &mut PanicBefore
        )))
        .is_err()
    );
    assert_eq!(snapshot(&j), before);
    assert_eq!(parity(&j), initial);
    admit(&mut j, 1, &mut NeverCrash).unwrap();
    parity(&j);
}

#[test]
fn pure_maintenance_is_not_a_production_pruning_entry() {
    let (_dir, mut journal) = setup();
    admit(&mut journal, 1, &mut NeverCrash).unwrap();
    let before = journal.world().unwrap();
    let input_count: i64 = journal
        .journal
        .connection
        .query_row("SELECT COUNT(*) FROM controller_inputs", [], |row| {
            row.get(0)
        })
        .unwrap();
    let input = core::ControllerInput::Maintenance(core::maintenance::MaintenanceInput::Enable {
        limits: core::maintenance::MaintenanceLimits {
            max_retained_results: 4,
            max_host_slots: 2,
            windows: core::maintenance::windows::WindowLimits::new(2, 100).unwrap(),
        },
        now_ms: 0,
    });
    assert!(matches!(
        journal.apply(input, &mut NeverCrash),
        Err(JournalError::UnsupportedInput)
    ));
    assert_eq!(journal.world().unwrap(), before);
    let after_count: i64 = journal
        .journal
        .connection
        .query_row("SELECT COUNT(*) FROM controller_inputs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(input_count, after_count);
}

#[test]
fn serialized_window_limits_cannot_bypass_checked_constructor() {
    for (count, duration) in [(0_u64, 100_u64), (2, 0)] {
        let value = serde_json::json!({"max_retained_results":4,"max_host_slots":2,
            "windows":{"max_open_windows":count,"max_duration_ms":duration}});
        let parsed = serde_json::from_value::<core::maintenance::MaintenanceLimits>(value);
        assert!(parsed.is_err());
    }
}
