//! Checked read-only snapshots of the actual original journal/application.
#[allow(
    dead_code,
    reason = "shared original application fixture supports several suites"
)]
#[path = "support/internal_application.rs"]
mod fixture;
#[allow(dead_code, reason = "shared current lifecycle application fixture")]
#[path = "support/lifecycle_application.rs"]
mod current;
use fixture::{State, TestClock, complete_fixture, wire};
use sandproof_controller_service::{ControllerJournal, InternalInboxConfig, NeverCrash};
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
use sandproof_protocol::internal::window::WindowCloseRequest;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

const NAMESPACE: &str = "0123456789abcdef";

fn inbox() -> InternalInboxConfig {
    InternalInboxConfig {
        node: wire::node("host-a"),
        max_sandboxes: 2,
        operations_per_sandbox: 4,
        max_options_bytes: 128,
    }
}

fn host(path: &Path) -> sandproof_controller_service::BoundedHostConfig {
    complete_fixture::host(
        path,
        NAMESPACE,
        64,
        2 * 1024 * 1024,
        complete_fixture::CLEANUP_BYTES + 1000,
    )
}

fn files(root: &Path) -> BTreeMap<PathBuf, (usize, [u8; 32])> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            assert!(
                path.is_file(),
                "fixture must not create host resource directories"
            );
            let bytes = fs::read(&path).unwrap();
            (path, (bytes.len(), Sha256::digest(&bytes).into()))
        })
        .collect()
}

fn assert_no_authority_write(root: &Path, mut before: BTreeMap<PathBuf, (usize, [u8; 32])>) {
    let mut after = files(root);
    // SQLite's actual READ_ONLY WAL reader can create an empty WAL and create
    // or update SHM reader coordination. They are not control-state writes.
    // Never use immutable=1: doing so would silently ignore live WAL records.
    let shm = root.join("controller.db-shm");
    if let Some((size, _)) = after.remove(&shm) {
        assert!(
            size <= 2 * 1024 * 1024,
            "reader SHM exceeds original fixture cap"
        );
    }
    before.remove(&shm);
    let wal = root.join("controller.db-wal");
    if !before.contains_key(&wal)
        && let Some((size, _)) = after.remove(&wal)
    {
        assert_eq!(size, 0, "read-only audit must not append WAL frames");
    }
    assert_eq!(
        after, before,
        "control database and all other fixture files must remain unchanged"
    );
}

#[test]
fn original_host_health_counts_fresh_positions_but_retains_unknown_and_unretired_responsibility() {
    use current::fixture::{State, TestClock, TestEpoch};
    use sandproof_protocol::lifecycle::{ExpectedSession, LifecycleOperation};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("controller.db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let mut inbox = current::config();
    inbox.max_sessions = 1;
    let mut bounded = host(&path);
    bounded.maintenance.max_host_slots = 1;
    bounded.physical_storage.as_mut().unwrap().pools[0].max_host_slots = 1;
    let app = current::application_with_host_configuration(
        &path,
        state.clone(),
        true,
        inbox.clone(),
        bounded.clone(),
    )
    .with_clock(Arc::new(TestClock(AtomicI64::new(100))));
    let audit = || {
        ControllerJournal::audit_lifecycle_complete(
            &path, NAMESPACE, &bounded, &current::maintenance(), &inbox,
        )
        .unwrap()
    };
    let free = || {
        let (observed_at_ms, resources) = app
            .internal_free_resources(&inbox.node, current::fixture::compute())
            .unwrap();
        assert_eq!(observed_at_ms, 100);
        resources.instances
    };
    assert!(audit().host_slots.is_empty());
    assert_eq!(free(), 1, "an initialized empty host must accept its first VM");
    let create = current::request();
    let context = current::context();
    app.record_lifecycle_intent(&create, &context).unwrap();
    assert_eq!(free(), 0, "Recorded without a native mapping still occupies its lane");
    app.establish_lifecycle_host_epoch().unwrap();
    let local = current::advance(&app).unwrap().local.unwrap();
    assert!(matches!(
        app.observe_lifecycle_create(&create, &context).unwrap().readiness,
        sandproof_controller_service::CreateReadiness::Observed(_)
    ));
    assert_eq!(free(), 0);
    let mut destroy = create;
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    app.record_lifecycle_intent(&destroy, &context).unwrap();
    assert_current_health_retirement(&app, &state, &free, &audit, destroy, &context, local.sandbox);
    let mut next = current::request();
    next.retention.window = 2.try_into().unwrap();
    next.request.binding.key.window = 2.try_into().unwrap();
    next.request.binding.session = "successor".try_into().unwrap();
    app.record_lifecycle_intent(&next, &context).unwrap();
    app.advance_lifecycle_create_with_verification(
        &next, &context, Some(current::fixture::template()), current::fixture::compute(), None,
    ).unwrap();
    assert!(matches!(
        app.observe_lifecycle_create(&next, &context).unwrap().readiness,
        sandproof_controller_service::CreateReadiness::Observed(_)
    ));
    assert_eq!(free(), 0);
}

fn assert_current_health_retirement(
    app: &sandproof_controller_service::application::ControllerApplication<current::fixture::TestHost>,
    state: &Mutex<current::fixture::State>,
    free: &impl Fn() -> u64,
    audit: &impl Fn() -> sandproof_controller_service::LifecycleAudit,
    destroy: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    context: &sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    sandbox: sandbox_controller_core::SandboxId,
) {
    use sandproof_protocol::lifecycle::{
        LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
    };
    state.lock().unwrap().release_unknown = true;
    assert_eq!(
        app.advance_current_workflow(sandbox),
        Err(sandproof_controller_service::application::ApplicationError::Busy)
    );
    assert!(!app.lifecycle_destroy_cleaned(&destroy, context).unwrap());
    assert_eq!(
        audit().host_slots[0].1,
        sandbox_controller_core::maintenance::slots::HostSlotPhase::Releasing
    );
    assert_eq!(free(), 0, "unknown physical release cannot advertise capacity");
    state.lock().unwrap().release_unknown = false;
    assert!(app.advance_current_workflow(sandbox).unwrap());
    assert!(app.lifecycle_destroy_cleaned(&destroy, context).unwrap());
    assert_eq!(free(), 0, "physical Free does not retire the retained lane");
    let cleaned = TerminalLifecycleObservation { request: destroy, status: TerminalStatus::Cleaned };
    app.confirm_lifecycle_terminal(&cleaned, context).unwrap();
    assert_eq!(free(), 0, "confirmation alone does not close the lane");
    app.close_lifecycle_window(&LifecycleWindowClose { terminal: cleaned }, context).unwrap();
    assert_eq!(free(), 1);
    let actions = state.lock().unwrap().actions.len();
    assert_eq!(free(), 1);
    assert_eq!(state.lock().unwrap().actions.len(), actions, "health is read-only");
}

#[test]
fn audit_projects_exact_global_local_mapping_and_latest_fact_without_host_work() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("controller.db");
    let state = Arc::new(Mutex::new(State::default()));
    let clock = Arc::new(TestClock(AtomicI64::new(100)));
    let app =
        fixture::application_with_inbox_config(&path, state.clone(), clock.clone(), true, inbox());
    let mut create = fixture::request();
    // Global tenant 2 arrives first and maps to local owner 1, not local 2.
    create.identity.tenant_id = "2".try_into().unwrap();
    app.record_internal_intent(&create).unwrap();
    let received = app.internal_audit().unwrap();
    assert_eq!(received.lanes.len(), usize::from(inbox().max_sandboxes));
    assert!(received.lanes.iter().all(|lane| {
        lane.retired_through == 0
            && lane.current_window.unwrap().get() == 1
            && lane.last_retired.is_none()
    }));
    assert_eq!(received.operations[0].confirmed, None);
    assert!(received.native.is_empty());
    assert_eq!(received.operations[0].local, None);
    assert_eq!(
        received.operations[0].response.fact,
        LifecycleFact::Recorded
    );
    app.advance_internal_lifecycle(&create, Some(fixture::template()), fixture::compute())
        .unwrap();
    let mut renew = create.clone();
    renew.identity.operation_id = "operation-2".try_into().unwrap();
    renew.identity.intent_version = 2.try_into().unwrap();
    renew.intent = Intent::Renew { end_at_ms: 120_100 };
    app.record_internal_intent(&renew).unwrap();
    app.advance_internal_lifecycle(&renew, None, fixture::compute())
        .unwrap();
    let actions_before = state.lock().unwrap().actions.clone();
    // Even an expired physical-time reading must not let audit expire/recover.
    clock.0.store(500_000, Ordering::SeqCst);
    let audit = app.internal_audit().unwrap();
    assert_eq!(audit.node, inbox().node);
    assert_eq!(audit.operations.len(), 2);
    assert!(!audit.operations[0].current);
    assert!(audit.operations[1].current);
    assert_eq!(audit.operations[1].identity, renew.identity);
    assert_eq!(
        audit.operations[1].response.fact,
        LifecycleFact::Renewed { end_at_ms: 120_100 }
    );
    let local = audit.operations[1].local.unwrap();
    assert_eq!(local.owner.0, 1);
    let native = &audit.native[0];
    assert_eq!(native.sandbox, local.sandbox);
    assert_eq!(native.owner, local.owner);
    assert_eq!(native.phase, sandbox_controller_core::Phase::Running);
    assert!(native.resources.vms.contains(&native.current_vm.unwrap()));
    assert_eq!(audit.host_slots[0].0.binding.sandbox, native.sandbox);
    assert_eq!(state.lock().unwrap().actions, actions_before);
    assert_eq!(app.internal_audit().unwrap(), audit);
    let encoded = String::from_utf8(serde_json::to_vec(&audit).unwrap()).unwrap();
    for forbidden in [
        "options",
        "env_vars",
        "metadata",
        "secret",
        "/protected-cache",
        "kernel",
        "rootfs",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "audit must not serialize {forbidden}"
        );
    }
    let live_before = files(directory.path());
    // The committed create/renew lives in the real WAL, so a checked reader
    // must return this SAME mapping rather than a stale immutable DB view.
    assert_eq!(
        ControllerJournal::audit_complete(
            &path,
            NAMESPACE,
            &host(&path),
            &complete_fixture::maintenance(),
            &inbox()
        )
        .unwrap(),
        audit
    );
    assert_no_authority_write(directory.path(), live_before);
    drop(app);
    let before = files(directory.path());
    let readonly = ControllerJournal::audit_complete(
        &path,
        NAMESPACE,
        &host(&path),
        &complete_fixture::maintenance(),
        &inbox(),
    )
    .unwrap();
    assert_eq!(readonly, audit);
    assert_no_authority_write(directory.path(), before);
}

#[test]
fn audit_exports_original_exact_confirmation_and_retired_floor_after_history_is_removed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("controller.db");
    let app = fixture::application_with_inbox_config(
        &path,
        Arc::new(Mutex::new(State::default())),
        Arc::new(TestClock(AtomicI64::new(100))),
        true,
        inbox(),
    );
    let destroy = wire::request(2, 2, Intent::Destroy);
    app.record_internal_intent(&destroy).unwrap();
    let cleaned = app
        .advance_internal_lifecycle(&destroy, None, fixture::compute())
        .unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    let unconfirmed = app.internal_audit().unwrap();
    assert_eq!(unconfirmed.operations[0].confirmed, None);
    app.confirm_internal_result(&cleaned).unwrap();
    let confirmed = app.internal_audit().unwrap();
    assert_eq!(confirmed.operations[0].confirmed, Some(cleaned.clone()));
    assert_eq!(confirmed.operations[0].response, cleaned);
    app.close_internal_window(&WindowCloseRequest::new(cleaned.clone()))
        .unwrap();
    let retired = app.internal_audit().unwrap();
    assert!(retired.operations.is_empty());
    let lane = retired
        .lanes
        .iter()
        .find(|lane| lane.retention_slot == destroy.identity.retention_slot)
        .unwrap();
    assert_eq!(lane.retired_through, 1);
    assert_eq!(lane.current_window.unwrap().get(), 2);
    assert_eq!(lane.last_retired, Some(cleaned));
    assert!(
        retired
            .lanes
            .iter()
            .filter(|other| other.retention_slot != lane.retention_slot)
            .all(|other| {
                other.retired_through == 0
                    && other.current_window.unwrap().get() == 1
                    && other.last_retired.is_none()
            })
    );
    drop(app);
    let before = files(directory.path());
    assert_eq!(
        ControllerJournal::audit_complete(
            &path,
            NAMESPACE,
            &host(&path),
            &complete_fixture::maintenance(),
            &inbox(),
        )
        .unwrap(),
        retired
    );
    assert_no_authority_write(directory.path(), before);
}

#[test]
fn audit_never_provisions_missing_database_directory_or_inbox() {
    let directory = tempfile::tempdir().unwrap();
    for path in [
        directory.path().join("missing.db"),
        directory.path().join("missing/child.db"),
    ] {
        assert!(
            ControllerJournal::audit_complete(
                &path,
                NAMESPACE,
                &host(&path),
                &complete_fixture::maintenance(),
                &inbox()
            )
            .is_err()
        );
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    let path = directory.path().join("controller.db");
    let journal = ControllerJournal::open_complete(
        &path,
        NAMESPACE,
        host(&path),
        complete_fixture::maintenance(),
        0,
    )
    .unwrap();
    assert!(journal.internal_audit().is_err());
    drop(journal);
    let before = files(directory.path());
    assert!(
        ControllerJournal::audit_complete(
            &path,
            NAMESPACE,
            &host(&path),
            &complete_fixture::maintenance(),
            &inbox()
        )
        .is_err()
    );
    assert_no_authority_write(directory.path(), before);
}

#[test]
fn audit_rejects_foreign_configuration_and_corrupt_root_without_repair() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open_complete(
        &path,
        NAMESPACE,
        host(&path),
        complete_fixture::maintenance(),
        0,
    )
    .unwrap();
    journal
        .initialize_internal_inbox(inbox(), &mut NeverCrash)
        .unwrap();
    drop(journal);
    let before = files(directory.path());
    let mut foreign = inbox();
    foreign.node = wire::node("host-b");
    assert!(
        ControllerJournal::audit_complete(
            &path,
            NAMESPACE,
            &host(&path),
            &complete_fixture::maintenance(),
            &foreign
        )
        .is_err()
    );
    let mut changed = complete_fixture::maintenance();
    changed.retention_ms += 1;
    assert!(
        ControllerJournal::audit_complete(&path, NAMESPACE, &host(&path), &changed, &inbox())
            .is_err()
    );
    assert_no_authority_write(directory.path(), before);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE controller_checkpoint SET sha256=zeroblob(32)", [])
        .unwrap();
    drop(connection);
    let corrupt = files(directory.path());
    assert!(
        ControllerJournal::audit_complete(
            &path,
            NAMESPACE,
            &host(&path),
            &complete_fixture::maintenance(),
            &inbox()
        )
        .is_err()
    );
    assert_no_authority_write(directory.path(), corrupt);
}
