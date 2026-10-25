//! Real driver/`SQLite` paths; physical deletion and quiescence are controlled.
use super::*;
use sandproof_controller_service::capture::MaterialRetirementObservation;
use sandproof_protocol::lifecycle::ContentDigest;
#[path = "material_fixture.rs"]
mod material_fixture;
use material_fixture::{PLAN, terminal};

fn reopen(
    path: &Path,
    physical: &Arc<Mutex<local::State>>,
) -> Arc<ControllerApplication<local::TestHost>> {
    let app = Arc::new(current::application(path, physical.clone(), false));
    app.establish_lifecycle_host_epoch().unwrap();
    assert!(app.pending_workflows().unwrap().is_empty());
    app
}

fn material_rows(path: &Path) -> Vec<serde_json::Value> {
    let c = rusqlite::Connection::open_with_flags(
        path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    let mut query = c.prepare("SELECT state_json FROM controller_source_work_records WHERE work_key IS NOT NULL ORDER BY id").unwrap();
    query.query_map([], |row| row.get::<_, Vec<u8>>(0)).unwrap()
        .map(|row| serde_json::from_slice(&row.unwrap()).unwrap()).collect()
}

fn configure_delete(physical: &Mutex<local::State>, reply: MaterialRetirementObservation) {
    let mut state = physical.lock().unwrap();
    state.material_plan = Some(PLAN.to_vec());
    state.material_observation = Some(reply);
}

#[tokio::test]
async fn idle_driver_retries_original_material_plan_after_restart_then_confirms_cleaned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let physical = state();
    let (destroy, reply) = terminal(&path, &physical);
    let original = material_rows(&path);
    assert_eq!(original.len(), 1);
    let charge = original[0]["capture"]["reserved_material_bytes"].as_u64().unwrap();
    assert!(charge > 0);
    let mut wrong = reply.clone();
    wrong.plan_binding = ContentDigest::from_sha256([0; 32]);
    configure_delete(&physical, wrong);
    {
        let host = HostLifecycle::new(reopen(&path, &physical));
        assert_eq!(host.drive_once(config(), Arc::new(Missing)).await.unwrap(),
            sandproof_host_service::DriveReport::default());
        assert_eq!(physical.lock().unwrap().material_prepare_calls, 1,
            "idle driver must invoke the original maintenance path after native work ends");
        let held = material_rows(&path);
        assert_eq!(held[0]["retirement"]["state"], "retiring");
        assert_eq!(held[0]["capture"]["reserved_material_bytes"].as_u64(), Some(charge));
        assert_eq!(host.observe_terminal(destroy.clone(), current::context()).await.unwrap().status,
            TerminalStatus::Pending);
        assert_eq!(host.active_work(), 0);
    }
    configure_delete(&physical, reply);
    let host = HostLifecycle::new(reopen(&path, &physical));
    host.drive_once(config(), Arc::new(Missing)).await.unwrap();
    assert!(material_rows(&path).is_empty());
    let cleaned = host.observe_terminal(destroy, current::context()).await.unwrap();
    assert_eq!(cleaned.status, TerminalStatus::Cleaned);
    assert!(host.confirm_terminal(cleaned.clone(), current::context()).await.unwrap().matches(&cleaned));
    let state = physical.lock().unwrap();
    assert_eq!(state.material_prepare_calls, 1);
    assert_eq!(state.material_delete_plans, vec![PLAN.to_vec(), PLAN.to_vec()]);
    assert_eq!(host.active_work(), 0);
}

async fn independent_session_progresses(host: &HostLifecycle<local::TestHost>) -> AcceptedLifecycleIntent {
    let mut peer = second(current::request());
    peer.request.binding.key.owner = "2".try_into().unwrap();
    peer.request.binding.key.sequence = 1.try_into().unwrap();
    host.record_lifecycle(peer.clone(), current::context()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), host.drive_once(config(), Arc::new(Templates)))
        .await.unwrap().unwrap();
    assert_eq!(host.active_work(), 1, "the original physical delete still owns cleanup");
    let ready = host.observe_create(peer.clone(), current::context()).await.unwrap();
    assert!(matches!(ready.readiness,
        sandproof_protocol::lifecycle::CurrentCreateReadiness::Observed { .. }));
    peer.accepted_version = 2;
    peer.request.binding.expected = ExpectedSession::Version(1);
    peer.request.binding.key.sequence = 2.try_into().unwrap();
    peer.request.operation = LifecycleOperation::Destroy {};
    host.record_lifecycle(peer.clone(), current::context()).await.unwrap();
    host.drive_once(config(), Arc::new(Missing)).await.unwrap();
    assert_eq!(host.observe_terminal(peer.clone(), current::context()).await.unwrap().status,
        TerminalStatus::Pending);
    peer
}

async fn blocked_maintenance(cancel: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let physical = state();
    let (destroy, reply) = terminal(&path, &physical);
    configure_delete(&physical, reply);
    let (entered, observed) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let mut release = Release(Some(release));
    physical.lock().unwrap().block_material_delete = Some((entered, blocked));
    let host = HostLifecycle::new(reopen(&path, &physical));
    let worker = host.clone();
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { worker.run_driver(config(), Arc::new(Missing), shutdown).await });
    tokio::task::spawn_blocking(move || observed.recv_timeout(Duration::from_secs(5)).unwrap())
        .await.unwrap();
    assert_eq!(host.active_work(), 1);
    if cancel {
        task.abort();
    } else {
        stop.send(true).unwrap();
    }
    assert_eq!(host.active_work(), 1, "cancel/shutdown cannot refund actual blocking work");
    let peer = independent_session_progresses(&host).await;
    assert_eq!(physical.lock().unwrap().material_delete_plans.len(), 1,
        "driver clones cannot start a second material pass while cleanup is held");
    if !cancel { assert!(!task.is_finished(), "shutdown must await actual maintenance exit"); }
    release.0.take().unwrap().send(()).unwrap();
    if cancel {
        assert!(task.await.unwrap_err().is_cancelled());
    } else {
        task.await.unwrap().unwrap();
    }
    work_drained(&host).await;
    for _ in 0..4 {
        host.drive_once(config(), Arc::new(Missing)).await.unwrap();
    }
    assert_eq!(host.observe_terminal(destroy, current::context()).await.unwrap().status,
        TerminalStatus::Cleaned);
    assert_eq!(host.observe_terminal(peer, current::context()).await.unwrap().status,
        TerminalStatus::Cleaned, "terminal work must obtain its turn after maintenance releases cleanup");
    assert!(material_rows(&path).is_empty());
    assert_eq!(host.active_work(), 0);
}

#[tokio::test]
async fn cancelling_driver_keeps_material_cleanup_held_and_other_session_progresses() {
    blocked_maintenance(true).await;
}

#[tokio::test]
async fn driver_shutdown_joins_real_maintenance_and_terminal_work_gets_a_turn() {
    blocked_maintenance(false).await;
}
