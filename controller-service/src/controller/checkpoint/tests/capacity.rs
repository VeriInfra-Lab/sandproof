use super::*;
use crate::{CapacityConfig, ConsolePolicy, WorkerTransportError};
use sandproof_protocol::WorkReceipt;

pub(super) fn full_config() -> MaintenanceConfig {
    MaintenanceConfig {
        checkpoint_profile: CheckpointProfile::CreateKill,
        retention_ms: 100,
        max_checkpoint_bytes: 4 * 1024 * 1024,
        max_records: 10_000,
        max_references: 1_000_000,
        max_tail_inputs: 8,
        max_tail_bytes: 64 * 1024,
        max_input_bytes: 8192,
        prune_batch_rows: 3,
        capacity: Some(CapacityConfig {
            max_live_sandboxes: 4,
            max_sql_rows: 1024,
            max_sql_bytes: 16 * 1024 * 1024,
            max_sql_value_bytes: 256 * 1024,
            max_receipt_bytes: 16 * 1024,
            max_transaction_inputs: 4,
            max_transaction_bytes: 32 * 1024,
            max_database_bytes: 512 * 1024 * 1024,
            max_wal_bytes: 520 * 1024 * 1024,
            max_shm_bytes: 2 * 1024 * 1024,
            checkpoint_scratch_bytes: 8 * 1024 * 1024,
            max_online_storage_bytes: 1042 * 1024 * 1024,
        }),
    }
}
pub(super) fn host() -> BoundedHostConfig {
    let mut host = host_config(4);
    host.console_policy = Some(ConsolePolicy::Discard);
    host.maintenance.max_retained_results = 8;
    host
}
fn open_full(path: &std::path::Path) -> ControllerJournal {
    ControllerJournal::open_complete(path, NS, host(), full_config(), 1000).unwrap()
}

fn limits() -> crate::AdmissionLimits {
    let mut value = super::support::limits();
    value.cleanup_reserve_bytes = full_config().capacity.unwrap().max_online_storage_bytes;
    value.max_disk_bytes_global = value.cleanup_reserve_bytes + 100_000;
    value
}
fn admit(
    journal: &mut ControllerJournal,
    id: u64,
    crash: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, AdmissionError> {
    journal.admit_create(request(id), 1000, 60, 1000, limits(), crash)
}
fn create(journal: &mut ControllerJournal, id: u64) {
    admit(journal, id, &mut NeverCrash).unwrap();
    crate::finish_admitted_create_v1(
        journal,
        core::SandboxId(id),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
}

#[test]
fn full_mode_rejects_unusable_atomic_tail_and_missing_discard() {
    for inputs in [1, 2, 3] {
        let dir = TempDir::new().unwrap();
        let mut policy = full_config();
        policy.max_tail_inputs = inputs;
        assert!(matches!(
            ControllerJournal::open_complete(dir.path().join("db"), NS, host(), policy, 1000),
            Err(JournalError::UnsupportedStore)
        ));
    }
    let dir = TempDir::new().unwrap();
    let mut host = host();
    host.console_policy = None;
    assert!(matches!(
        ControllerJournal::open_complete(dir.path().join("db"), NS, host, full_config(), 1000),
        Err(JournalError::UnsupportedStore)
    ));
}

#[test]
fn declared_tail_reserve_boundary_rotates_only_after_the_original_trigger() {
    fn generation(journal: &ControllerJournal) -> i64 {
        journal
            .journal
            .connection
            .query_row(
                "SELECT generation FROM controller_checkpoint WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
    fn advance_clock(journal: &mut ControllerJournal, now_ms: u64) {
        let tx = crate::controller::transaction::WorldTransaction::begin(
            &mut journal.journal.connection,
            &journal.committed_world,
        )
        .unwrap();
        crate::controller::storage::apply_maintenance(
            &tx,
            core::maintenance::MaintenanceInput::Window(
                core::maintenance::windows::WindowInput::AdvanceTime { now_ms },
            ),
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let dir = TempDir::new().unwrap();
    let mut policy = full_config();
    policy.max_tail_inputs = 128;
    let mut journal =
        ControllerJournal::open_complete(dir.path().join("db"), NS, host(), policy, 1000).unwrap();
    let initial = journal.checkpoint_progress().unwrap().unwrap();
    let initial_generation = generation(&journal);
    for now_ms in 1001..=1124 {
        advance_clock(&mut journal, now_ms);
    }
    // The actual writer permits tail 124 plus its four-input atomic reserve.
    journal.expire_retained(1124, &mut NeverCrash).unwrap();
    let boundary = journal.checkpoint_progress().unwrap().unwrap();
    assert_eq!(generation(&journal), initial_generation);
    assert_eq!(boundary.input_cut, initial.input_cut);
    assert_eq!(boundary.input_high_water - boundary.input_cut, 124);
    advance_clock(&mut journal, 1125);
    journal.expire_retained(1125, &mut NeverCrash).unwrap();
    let rotated = journal.checkpoint_progress().unwrap().unwrap();
    assert!(generation(&journal) > initial_generation);
    assert_eq!(rotated.input_cut, boundary.input_high_water + 1);
    assert_eq!(rotated.deleted_through, rotated.input_cut);
}

#[test]
fn saturation_cannot_spend_accepted_cleanup_or_result_retention() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open_full(&path);
    for id in 1..=4 {
        create(&mut journal, id);
    }
    let high = allocators::input_high_water(&journal.journal.connection).unwrap();
    assert!(admit(&mut journal, 5, &mut NeverCrash).is_err());
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        high
    );
    for id in 1..=4 {
        destroy(&mut journal, id);
        release(&mut journal, id);
        retire(&mut journal, id);
    }
    assert_eq!(
        journal
            .world()
            .unwrap()
            .maintenance_state()
            .unwrap()
            .retained_count(),
        8
    );
    assert!(admit(&mut journal, 5, &mut NeverCrash).is_err());
    assert_eq!(journal.expire_retained(1100, &mut NeverCrash).unwrap(), 0);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .maintenance_state()
            .unwrap()
            .retained_count(),
        8
    );
    assert_eq!(journal.expire_retained(1101, &mut NeverCrash).unwrap(), 8);
    create(&mut journal, 5);
    drop(journal);
    let journal = open_full(&path);
    assert_eq!(journal.world().unwrap().sandbox_records().len(), 1);
}

#[test]
fn long_churn_keeps_old_running_and_unknown_but_bounds_tail_sql_and_pages() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open_full(&path);
    create(&mut journal, 1);
    admit(&mut journal, 2, &mut NeverCrash).unwrap();
    journal
        .dispatch_next_for(
            core::SandboxId(2),
            &mut |_| -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
                Err(WorkerTransportError::ConnectionLost)
            },
            &mut NeverCrash,
        )
        .unwrap();
    let old = journal
        .world()
        .unwrap()
        .sandbox_record(core::SandboxId(1))
        .unwrap()
        .clone();
    let mut previous = allocators::input_high_water(&journal.journal.connection).unwrap();
    let mut peak = 0_u64;
    for id in 3..=258 {
        create(&mut journal, id);
        destroy(&mut journal, id);
        release(&mut journal, id);
        let now = 1000 + id * 101;
        journal
            .retire_reclaimed(core::SandboxId(id), now, |_| Ok(()), &mut NeverCrash)
            .unwrap();
        journal.expire_retained(now + 100, &mut NeverCrash).unwrap();
        let high = allocators::input_high_water(&journal.journal.connection).unwrap();
        assert!(high > previous);
        previous = high;
        assert!(count(&journal, "controller_inputs") <= 8);
        assert_eq!(count(&journal, "controller_capacity_reservations"), 2);
        assert_eq!(
            journal.world().unwrap().sandbox_record(core::SandboxId(1)),
            Some(&old)
        );
        assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
        let pages: i64 = journal
            .journal
            .connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        peak = peak.max(u64::try_from(pages).unwrap());
        if [18, 66, 130, 258].contains(&id) {
            let progress = journal.checkpoint_progress().unwrap().unwrap();
            let world = journal.world().unwrap();
            let core = serde_json::to_value(
                core::checkpoint::capture(&world, full_config().core_limits()).unwrap(),
            )
            .unwrap();
            let records = core_records(&core, &world);
            let refs = core_references(&core);
            let wal =
                std::fs::metadata(format!("{}-wal", path.display())).map_or(0, |value| value.len());
            let shm =
                std::fs::metadata(format!("{}-shm", path.display())).map_or(0, |value| value.len());
            let database = std::fs::metadata(&path).unwrap().len();
            let cap = full_config().capacity.unwrap();
            assert!(wal <= cap.max_wal_bytes && shm <= cap.max_shm_bytes);
            let sample = serde_json::json!({"cycles":id-2,"core_records":records,"core_references":refs,"sandboxes":count(&journal,"sandboxes"),"operations":count(&journal,"operations"),"outbox":count(&journal,"outbox"),"receipt_history":count(&journal,"controller_receipt_history"),"effect_keys":count(&journal,"controller_effect_keys"),"allocator_rows":count(&journal,"controller_id_highwaters"),"reservation_rows":count(&journal,"controller_capacity_reservations"),"input_rows":count(&journal,"controller_inputs"),"input_tail":progress.input_high_water-progress.input_cut,"input_covered_prefix":progress.input_cut-progress.deleted_through,"input_high_water":progress.input_high_water,"database_bytes":database,"wal_bytes":wal,"shm_bytes":shm,"pages":pages});
            drop(journal);
            let start = std::time::Instant::now();
            journal = open_full(&path);
            println!(
                "capacity_churn_fixture {} reopen_micros={}",
                sample,
                start.elapsed().as_micros()
            );
        }
    }
    // Configured file ceilings and a much tighter deterministic fixture plateau.
    assert!(peak < 256);
    assert!(
        std::fs::metadata(&path).unwrap().len()
            <= full_config().capacity.unwrap().max_database_bytes
    );
    assert_eq!(count(&journal, "sandboxes"), 2);
}

#[test]
fn unknown_commit_preserves_reservation_and_contiguous_ids() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_full(&path);
        super::super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            admit(&mut journal, 1, &mut NeverCrash),
            Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
        ));
        drop(journal);
        let mut journal = open_full(&path);
        assert_eq!(
            count(&journal, "controller_capacity_reservations"),
            i64::from(durable)
        );
        if durable {
            crate::finish_admitted_create_v1(
                &mut journal,
                core::SandboxId(1),
                core::OperationId(1),
                &mut success,
            )
            .unwrap();
            destroy(&mut journal, 1);
            release(&mut journal, 1);
            retire(&mut journal, 1);
        }
    }
}

#[test]
fn held_reader_stops_writer_before_new_effect_and_resumes_after_release() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open_full(&path);
    create(&mut journal, 1);
    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM sandboxes", [], |row| row.get(0))
        .unwrap();
    // The snapshot holds existing WAL frames. Zero busy timeout makes this a
    // deterministic contention test rather than waiting for a random release.
    journal
        .journal
        .connection
        .busy_timeout(std::time::Duration::ZERO)
        .unwrap();
    let world = journal.world().unwrap();
    assert!(matches!(
        admit(&mut journal, 2, &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::MaintenanceCapacity))
    ));
    assert_eq!(journal.world().unwrap(), world);
    reader.execute_batch("ROLLBACK;").unwrap();
    create(&mut journal, 2);
}

#[test]
fn all_create_commands_retry_then_kill_consumes_only_its_lease() {
    use sandproof_protocol::{FailureCode, WorkOutcome};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open_full(&path);
    admit(&mut journal, 1, &mut NeverCrash).unwrap();
    let mut observed = std::collections::BTreeSet::new();
    crate::finish_admitted_create_v1(
        &mut journal,
        core::SandboxId(1),
        core::OperationId(1),
        &mut |action: &sandproof_protocol::WorkAction<core::EffectId>| {
            let name = match action.command.command() {
                core::Command::InstallFence { .. } => "fence",
                core::Command::PrepareVm { .. } => "prepare",
                core::Command::ConfigureNetwork { .. } => "network",
                core::Command::ApplyFilePolicy { .. } => "file-policy",
                core::Command::ActivateVm { .. } => "activate",
                _ => panic!("new create command needs reservation review"),
            };
            observed.insert(name);
            if action.command.attempt < core::DEFAULT_RETRY_LIMIT {
                Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::DefinitiveFailure {
                        code: FailureCode::ResourceUnavailable,
                        retryable: true,
                        partial: None,
                    },
                })
            } else {
                success(action)
            }
        },
    )
    .unwrap();
    assert_eq!(
        observed,
        std::collections::BTreeSet::from([
            "fence",
            "prepare",
            "network",
            "file-policy",
            "activate"
        ])
    );
    destroy(&mut journal, 1);
    release(&mut journal, 1);
    retire(&mut journal, 1);
    assert_eq!(count(&journal, "controller_capacity_reservations"), 0);
}

#[test]
fn failed_prepare_and_failed_activation_have_reserved_terminal_paths() {
    use sandproof_protocol::{FailureCode, WorkOutcome};
    for fail_prepare in [true, false] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_full(&path);
        admit(&mut journal, 1, &mut NeverCrash).unwrap();
        let result = crate::finish_admitted_create_v1(
            &mut journal,
            core::SandboxId(1),
            core::OperationId(1),
            &mut |action: &sandproof_protocol::WorkAction<core::EffectId>| {
                if (fail_prepare
                    && matches!(action.command.command(), core::Command::PrepareVm { .. }))
                    || (!fail_prepare
                        && matches!(action.command.command(), core::Command::ActivateVm { .. }))
                {
                    Ok(WorkReceipt::<Evidence> {
                        identity: action.identity.clone(),
                        outcome: WorkOutcome::DefinitiveFailure {
                            code: FailureCode::PolicyRejected,
                            retryable: false,
                            partial: None,
                        },
                    })
                } else {
                    success(action)
                }
            },
        );
        assert!(
            matches!(result, Err(JournalError::OperationFailed)),
            "{result:?}"
        );
        release(&mut journal, 1);
        retire(&mut journal, 1);
        assert_eq!(count(&journal, "controller_capacity_reservations"), 0);
        drop(journal);
        assert_eq!(
            open_full(&path)
                .world()
                .unwrap()
                .maintenance_state()
                .unwrap()
                .retained_count(),
            1
        );
    }
}

#[test]
fn terminal_supersession_and_noop_queries_cannot_grow_an_operation_chain() {
    let dir = TempDir::new().unwrap();
    let mut journal = open_full(&dir.path().join("db"));
    admit(&mut journal, 1, &mut NeverCrash).unwrap();
    journal
        .apply(
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox: core::SandboxId(1),
                operation: core::OperationId(2),
            })),
            &mut NeverCrash,
        )
        .unwrap();
    let high = allocators::input_high_water(&journal.journal.connection).unwrap();
    for operation in 3..103 {
        assert!(
            journal
                .apply(
                    core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                        actor: core::Actor::Tenant(core::TenantId(1)),
                        sandbox: core::SandboxId(1),
                        operation: core::OperationId(operation)
                    })),
                    &mut NeverCrash
                )
                .is_err()
        );
    }
    for _ in 0..100 {
        journal
            .check(core::ControllerInput::Request(core::Request::Query(
                core::QueryRequest {
                    actor: core::Actor::Tenant(core::TenantId(1)),
                    sandbox: core::SandboxId(1),
                    operation: core::OperationId(1),
                },
            )))
            .unwrap();
    }
    assert_eq!(count(&journal, "operations"), 2);
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        high
    );
}

#[test]
fn noops_do_not_spend_inputs_receipts_revisions_or_checkpoint_generations() {
    let dir = TempDir::new().unwrap();
    let mut journal = open_full(&dir.path().join("db"));
    admit(&mut journal, 1, &mut NeverCrash).unwrap();
    let mut receipt = None;
    journal
        .dispatch_next_for(
            core::SandboxId(1),
            &mut |action| {
                let value = success(action)?;
                receipt = Some(value.clone());
                Ok(value)
            },
            &mut NeverCrash,
        )
        .unwrap();
    let record = journal
        .world()
        .unwrap()
        .sandbox_record(core::SandboxId(1))
        .unwrap()
        .clone();
    let plan = journal
        .world()
        .unwrap()
        .operation_plan(core::OperationKey {
            sandbox: core::SandboxId(1),
            operation: core::OperationId(1),
        })
        .unwrap()
        .clone();
    let input = core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
        core::SandboxId(1),
        plan.vm.unwrap(),
        plan.slot.unwrap(),
    ));
    journal.apply(input.clone(), &mut NeverCrash).unwrap();
    let before = allocators::input_high_water(&journal.journal.connection).unwrap();
    let receipts = count(&journal, "controller_receipt_history");
    let revision: i64 = journal
        .journal
        .connection
        .query_row("SELECT revision FROM sandboxes", [], |row| row.get(0))
        .unwrap();
    let credit: i64 = journal
        .journal
        .connection
        .query_row(
            "SELECT input_left FROM controller_capacity_reservations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for _ in 0..16 {
        journal.apply(input.clone(), &mut NeverCrash).unwrap();
        journal
            .complete(receipt.as_ref().unwrap(), &mut NeverCrash)
            .unwrap();
    }
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        before
    );
    assert_eq!(count(&journal, "controller_receipt_history"), receipts);
    assert_eq!(
        journal
            .journal
            .connection
            .query_row("SELECT revision FROM sandboxes", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        revision
    );
    assert_eq!(
        journal
            .journal
            .connection
            .query_row(
                "SELECT input_left FROM controller_capacity_reservations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        credit
    );
    crate::finish_admitted_create_v1(
        &mut journal,
        core::SandboxId(1),
        record.current_operation.unwrap().id,
        &mut success,
    )
    .unwrap();
    destroy(&mut journal, 1);
    release(&mut journal, 1);
    assert_release_checkpoint_noops(&mut journal);
}
fn assert_release_checkpoint_noops(journal: &mut ControllerJournal) {
    let witness = journal.retirement_candidates().unwrap()[0].release;
    let before = allocators::input_high_water(&journal.journal.connection).unwrap();
    for _ in 0..16 {
        release(journal, 1);
    }
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        before
    );
    assert_eq!(journal.retirement_candidates().unwrap()[0].release, witness);
    let root = journal.publish_checkpoint(&mut NeverCrash).unwrap();
    for _ in 0..16 {
        assert_eq!(journal.publish_checkpoint(&mut NeverCrash).unwrap(), root);
    }
    let progress = journal.checkpoint_progress().unwrap().unwrap();
    assert_eq!(progress.deleted_through, progress.input_cut);
    assert_eq!(progress.input_cut, before);
}

#[test]
fn future_wrapper_and_maximal_operation_are_rejected_before_admission() {
    let dir = TempDir::new().unwrap();
    let mut journal = open_full(&dir.path().join("db"));
    let mut maximum = request(1);
    maximum.operation = core::OperationId(u64::MAX);
    assert!(matches!(
        journal.admit_create(maximum, 1000, 60, 1000, limits(), &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::IdentifierExhausted))
    ));
    let mut large = request(1);
    large.policy = core::NetworkPolicy::StaticAllowlist(vec![core::Endpoint {
        address: "x".repeat(6000),
        prefix_length: 32,
        protocol: core::Protocol::Tcp,
        port: 443,
    }]);
    assert!(matches!(
        journal.admit_create(large, 1000, 60, 1000, limits(), &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::MaintenanceCapacity))
    ));
    assert_eq!(count(&journal, "sandboxes"), 0);
    assert_eq!(count(&journal, "controller_capacity_reservations"), 0);
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Request(core::Request::Create(request(1))),
            &mut NeverCrash
        ),
        Err(JournalError::UnsupportedInput)
    ));
}

#[test]
fn sqlite_full_refuses_new_work_without_changing_accepted_cleanup() {
    use crate::{CreateAdmission, CreateOptions};
    let dir = TempDir::new().unwrap();
    let mut journal = open_full(&dir.path().join("db"));
    create(&mut journal, 1);
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    let pages: i64 = journal
        .journal
        .connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    journal
        .journal
        .connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    let mut options = CreateOptions::default();
    options
        .metadata
        .insert("payload".into(), "x".repeat(60 * 1024));
    let before = journal.world().unwrap();
    let result = journal.admit_create_with_options(
        request(2),
        CreateAdmission {
            now_ms: 1000,
            timeout_secs: 60,
            disk_bytes: 1000,
            limits: limits(),
            options,
        },
        &mut NeverCrash,
    );
    assert!(
        matches!(result,Err(AdmissionError::Journal(JournalError::Sqlite(rusqlite::Error::SqliteFailure(code,_)))) if code.code==rusqlite::ErrorCode::DiskFull)
    );
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(count(&journal, "controller_capacity_reservations"), 1);
    journal
        .journal
        .connection
        .pragma_update(
            None,
            "max_page_count",
            i64::try_from(full_config().capacity.unwrap().max_database_bytes / 4096).unwrap(),
        )
        .unwrap();
    destroy(&mut journal, 1);
    release(&mut journal, 1);
    retire(&mut journal, 1);
}

#[test]
fn retained_expiry_credits_are_spendable_at_the_input_frontier() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open_full(&path);
    for (id, now) in [(1, 1001), (2, 1051)] {
        admit(&mut journal, id, &mut NeverCrash).unwrap();
        let result = crate::finish_admitted_create_v1(
            &mut journal,
            core::SandboxId(id),
            core::OperationId(1),
            &mut |action: &sandproof_protocol::WorkAction<core::EffectId>| {
                if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                    Ok(WorkReceipt::<Evidence> {
                        identity: action.identity.clone(),
                        outcome: sandproof_protocol::WorkOutcome::DefinitiveFailure {
                            code: sandproof_protocol::FailureCode::PolicyRejected,
                            retryable: false,
                            partial: None,
                        },
                    })
                } else {
                    success(action)
                }
            },
        );
        assert!(matches!(result, Err(JournalError::OperationFailed)));
        release(&mut journal, id);
        journal
            .retire_reclaimed(core::SandboxId(id), now, |_| Ok(()), &mut NeverCrash)
            .unwrap();
    }
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    assert_eq!(count(&journal, "controller_inputs"), 0);
    let high = i64::MAX as u64 - 1 - 2;
    move_empty_input_frontier(&journal, high);
    drop(journal);
    let mut journal = open_full(&path);
    assert_eq!(journal.expire_retained(1101, &mut NeverCrash).unwrap(), 1);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .maintenance_state()
            .unwrap()
            .retained_count(),
        1
    );
    assert_eq!(journal.expire_retained(1151, &mut NeverCrash).unwrap(), 1);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .maintenance_state()
            .unwrap()
            .retained_count(),
        0
    );
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        i64::MAX as u64 - 1
    );
    assert!(matches!(
        admit(&mut journal, 3, &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::IdentifierExhausted))
    ));
}

pub(super) fn move_empty_input_frontier(journal: &ControllerJournal, high: u64) {
    let bytes: Vec<u8> = journal
        .journal
        .connection
        .query_row("SELECT payload FROM controller_checkpoint", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut envelope: Envelope = serde_json::from_slice(&bytes).unwrap();
    envelope.input_cut = high;
    for cursor in &mut envelope.allocators {
        if cursor.kind == "input" {
            cursor.high_water = high;
        }
    }
    let bytes = serde_json::to_vec(&envelope).unwrap();
    let digest = Sha256::digest(&bytes);
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_checkpoint SET input_cut=?1,payload=?2,sha256=?3",
            params![sql(high).unwrap(), bytes, digest.as_slice()],
        )
        .unwrap();
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_checkpoint_gc SET deleted_through=?1",
            [sql(high).unwrap()],
        )
        .unwrap();
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_id_highwaters SET high_water=?1 WHERE kind='input'",
            [high.to_string()],
        )
        .unwrap();
}

#[test]
fn housekeeping_unknown_commit_reloads_either_durable_prefix() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_full(&path);
        create(&mut journal, 1);
        journal.publish_checkpoint(&mut NeverCrash).unwrap();
        let expected = journal.world().unwrap();
        let before = journal.checkpoint_progress().unwrap().unwrap();
        let before_rows = count(&journal, "controller_inputs");
        let reservations = count(&journal, "controller_capacity_reservations");
        assert!(before.deleted_through < before.input_cut);
        super::super::HOUSEKEEPING_FAILURE.with(|slot| slot.set(Some(durable)));
        assert!(matches!(
            journal.publish_checkpoint(&mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        assert_eq!(journal.world().unwrap(), expected);
        let after = journal.checkpoint_progress().unwrap().unwrap();
        let removed = if durable {
            full_config()
                .prune_batch_rows
                .min(before.input_cut - before.deleted_through)
        } else {
            0
        };
        assert_eq!(after.input_cut, before.input_cut);
        assert_eq!(after.input_high_water, before.input_high_water);
        assert_eq!(after.deleted_through, before.deleted_through + removed);
        assert_eq!(
            count(&journal, "controller_inputs"),
            before_rows - i64::try_from(removed).unwrap()
        );
        assert_eq!(
            count(&journal, "controller_capacity_reservations"),
            reservations
        );
        drop(journal);
        let mut journal = open_full(&path);
        assert_eq!(journal.world().unwrap(), expected);
        assert_eq!(journal.checkpoint_progress().unwrap().unwrap(), after);
        journal.publish_checkpoint(&mut NeverCrash).unwrap();
        let completed = journal.checkpoint_progress().unwrap().unwrap();
        assert_eq!(completed.deleted_through, completed.input_cut);
        drop(journal);
        assert_eq!(open_full(&path).world().unwrap(), expected);
    }
}

#[test]
fn full_mode_rejects_unearned_checkpoint_generations() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let journal = open_full(&path);
    journal
        .journal
        .connection
        .execute("UPDATE controller_checkpoint SET generation=?1", [i64::MAX])
        .unwrap();
    drop(journal);
    assert!(matches!(
        ControllerJournal::open_complete(&path, NS, host(), full_config(), 1000),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn tight_sql_and_core_reservations_finish_the_original_workflow() {
    let seed_dir = TempDir::new().unwrap();
    let seed = open_full(&seed_dir.path().join("db"));
    let (rows, bytes, lease_rows, lease_bytes) = super::super::super::capacity::fixture_sql_budget(
        &seed.journal.connection,
        &seed.world().unwrap(),
    )
    .unwrap();
    let mut policy = full_config();
    policy.max_records = 1 + 4 + 2 + 61;
    policy.max_references = 86;
    let cap = policy.capacity.as_mut().unwrap();
    cap.max_sql_rows = rows + lease_rows;
    cap.max_sql_bytes = bytes + lease_bytes + 64;
    let dir = TempDir::new().unwrap();
    let mut journal =
        ControllerJournal::open_complete(dir.path().join("db"), NS, host(), policy.clone(), 1000)
            .unwrap();
    create(&mut journal, 1);
    let charged = super::super::super::capacity::fixture_sql_budget(
        &journal.journal.connection,
        &journal.world().unwrap(),
    )
    .unwrap();
    let limits = policy.capacity.unwrap();
    assert_eq!(charged.0, limits.max_sql_rows);
    assert!(limits.max_sql_bytes - charged.1 < 128);
    assert!(matches!(
        admit(&mut journal, 2, &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::MaintenanceCapacity))
    ));
    destroy(&mut journal, 1);
    release(&mut journal, 1);
    retire(&mut journal, 1);
    assert_eq!(count(&journal, "sandboxes"), 0);
}

fn core_records(value: &serde_json::Value, world: &core::World) -> usize {
    let mut count = 0;
    for key in [
        "sandboxes",
        "vms",
        "filesystems",
        "snapshots",
        "net_slots",
        "applied_file_policies",
        "worker_fences",
        "operations",
        "operation_plans",
        "resource_indexes",
        "logical_effects",
        "effects",
    ] {
        count += value[key].as_array().unwrap().len();
    }
    let state = world.maintenance_state().unwrap();
    count
        + 1
        + state.retained_count()
        + state.host_slots().len()
        + state.windows().open_windows().len()
}
fn core_references(value: &serde_json::Value) -> usize {
    fn policies(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Object(fields) => fields
                .iter()
                .map(|(key, value)| {
                    if ["StaticAllowlist", "allow_out", "deny_out"].contains(&key.as_str()) {
                        value.as_array().map_or(0, Vec::len)
                    } else {
                        policies(value)
                    }
                })
                .sum(),
            serde_json::Value::Array(values) => values.iter().map(policies).sum(),
            _ => 0,
        }
    }
    let mut count = value["effect_keys"].as_array().unwrap().len()
        + value["inflight"].as_array().unwrap().len()
        + policies(value);
    for plan in value["operation_plans"].as_array().unwrap() {
        count += plan[1]["stop_vms"].as_array().unwrap().len()
            + plan[1]["close_slots"].as_array().unwrap().len();
    }
    for row in value["resource_indexes"].as_array().unwrap() {
        for key in ["vms", "slots", "snapshots"] {
            count += row[1][key].as_array().unwrap().len();
        }
    }
    count
}
