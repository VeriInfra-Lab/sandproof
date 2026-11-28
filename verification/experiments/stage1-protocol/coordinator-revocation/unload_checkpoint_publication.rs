//! Hit automatic checkpoint publication before the public Unload can open its
//! inbox transaction. The deployment tail is deliberately small.
use super::super::support::NS;
use super::{assert_unrecorded, lifecycle, next_unload, record_with_fault, retry_unload};
use super::{start_sessions, stored_inbox};
use crate::application::{ApplicationConfig, ControllerApplication};
use crate::controller::{storage, transaction::WorldTransaction};
use crate::{AdmissionLimits, ControllerJournal, NeverCrash};
use lifecycle::fixture::{State, TestClock, TestEpoch, TestHost, complete_fixture};
use sandbox_controller_core as core;
use std::path::Path;
use std::sync::{Arc, Mutex, atomic::AtomicI64};

const DISK: u64 = 2 * 1024 * 1024;
const RESERVE: u64 = lifecycle::CLEANUP_BYTES + 1000;

fn open(path: &Path) -> ControllerJournal {
    let mut policy = lifecycle::maintenance();
    policy.max_tail_inputs = 8;
    ControllerJournal::open_complete(
        path,
        NS,
        complete_fixture::host(path, NS, 64, DISK, RESERVE),
        policy,
        0,
    )
    .unwrap()
}

fn application(
    journal: ControllerJournal,
    host: &Arc<Mutex<State>>,
) -> ControllerApplication<TestHost> {
    let config = ApplicationConfig {
        public_ipv4_enabled: false,
        limits: AdmissionLimits {
            max_disk_bytes_global: RESERVE + DISK,
            max_disk_bytes_per_tenant: DISK,
            cleanup_reserve_bytes: RESERVE,
            ..lifecycle::limits()
        },
        disk_bytes: 4096,
        file_policy_digest: core::PolicyDigest(1),
        cpu_count: 1,
        memory_mb: 128,
        disk_size_mb: 1,
        envd_version: "fixed-envd".into(),
        domain: "example.invalid".into(),
    };
    ControllerApplication::new(journal, TestHost(Arc::clone(host)), config)
        .with_clock(Arc::new(TestClock(AtomicI64::new(10))))
}

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

fn check_publication(path: &Path, durable: bool) {
    let host = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let mut journal = open(path);
    journal
        .initialize_lifecycle_inbox(
            lifecycle::config(),
            lifecycle::source_limits(),
            &mut NeverCrash,
        )
        .unwrap();
    let app = application(journal, &host);
    let creates = start_sessions(&app);
    let unload = next_unload(&creates[0]);
    drop(app);

    // Fill the tail with five genuine, monotonically advancing maintenance
    // inputs. Each clock change goes through the normal checked transaction.
    let mut journal = open(path);
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    let start = journal
        .world()
        .unwrap()
        .maintenance_state()
        .unwrap()
        .time_floor();
    for now_ms in (start + 1)..=(start + 5) {
        let tx = WorldTransaction::begin(&mut journal.journal.connection, &journal.committed_world)
            .unwrap();
        storage::apply_maintenance(
            &tx,
            core::maintenance::MaintenanceInput::Window(
                core::maintenance::windows::WindowInput::AdvanceTime { now_ms },
            ),
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let before = journal.checkpoint_progress().unwrap().unwrap();
    let policy = journal.maintenance_config().unwrap().unwrap();
    assert_eq!(before.deleted_through, before.input_cut);
    let tail = before.input_high_water - before.input_cut;
    assert_eq!(tail, 5);
    assert!(tail + policy.capacity.unwrap().max_transaction_inputs > policy.max_tail_inputs);
    let old_generation = generation(&journal);
    let world = journal.world().unwrap();
    let inbox = stored_inbox(&journal);

    let app = application(journal, &host);
    record_with_fault(&app, &creates, &unload, durable);
    drop(app);

    let journal = open(path);
    let after = journal.checkpoint_progress().unwrap().unwrap();
    assert_eq!(after.deleted_through, before.deleted_through);
    assert_eq!(after.input_high_water, before.input_high_water);
    assert_eq!(
        after.input_cut,
        if durable {
            before.input_high_water
        } else {
            before.input_cut
        }
    );
    assert_eq!(generation(&journal), old_generation + i64::from(durable));
    assert_eq!(journal.world().unwrap(), world);
    assert_eq!(stored_inbox(&journal), inbox);
    assert_unrecorded(&journal, &creates, &unload);

    let app = application(journal, &host);
    retry_unload(&app, &creates, &unload);
    drop(app);
    let recovered = open(path);
    assert_eq!(generation(&recovered), old_generation + 1);
    let final_progress = recovered.checkpoint_progress().unwrap().unwrap();
    assert_eq!(final_progress.input_cut, before.input_high_water);
    assert_eq!(final_progress.deleted_through, final_progress.input_cut);
}

#[test]
fn uncertain_root_publication_revokes_both_routes_before_unload_is_recorded() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        check_publication(&dir.path().join("db"), durable);
    }
}
