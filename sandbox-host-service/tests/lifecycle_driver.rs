//! Current original application and `SQLite`; host effects/epoch are controlled.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/lifecycle_application.rs"]
mod local;

use sandproof_controller_service::{
    CreateReadiness, InternalTemplate, application::ApplicationError,
};
use sandproof_host_service::{DriveReport, DriverConfig, HostLifecycle, TemplateSource};
use sandproof_internal::LifecycleReceiver;
use sandproof_protocol::{
    internal::lifecycle::CreateBinding,
    lifecycle::{AcceptedLifecycleIntent, LifecycleOperation},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct Templates;
impl TemplateSource for Templates {
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        let original = local::fixture::template();
        if binding.template_revision != original.revision
            || binding.template_digest != original.digest
        {
            return Err(ApplicationError::NotFound);
        }
        Ok(original)
    }
}
fn config() -> DriverConfig {
    DriverConfig {
        node: local::config().node,
        compute: local::fixture::compute(),
        interval: Duration::from_millis(100),
    }
}
fn matching_host() -> Arc<Mutex<local::fixture::State>> {
    Arc::new(Mutex::new(local::fixture::State {
        epoch: local::fixture::TestEpoch::Matching,
        source: Some(
            sandproof_controller_service::capture::SourceProcessIdentity {
                pid: 1234,
                start_time_ticks: 5678,
                boot_id: "fixture-boot".into(),
            },
        ),
        ..local::fixture::State::default()
    }))
}

struct HeldMissing {
    first_pass: std::sync::Barrier,
    arrivals: std::sync::atomic::AtomicUsize,
}

async fn confirm_original_termination(
    host: &HostLifecycle<local::fixture::TestHost>,
    create: &AcceptedLifecycleIntent,
) -> sandproof_protocol::lifecycle::LifecycleWindowClose {
    use sandproof_protocol::lifecycle::{
        ExpectedSession, LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
    };
    let mut destroy = create.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    host.record_lifecycle(destroy.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.observe_terminal(destroy.clone(), local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Pending
    );
    let forged = TerminalLifecycleObservation {
        request: destroy.clone(),
        status: TerminalStatus::Cleaned,
    };
    assert!(
        host.confirm_terminal(forged.clone(), local::context())
            .await
            .is_err()
    );
    let close = LifecycleWindowClose {
        terminal: forged.clone(),
    };
    assert!(
        host.close_window(close.clone(), local::context())
            .await
            .is_err()
    );
    host.drive_once(config(), Arc::new(Templates))
        .await
        .unwrap();
    assert_eq!(
        host.observe_terminal(destroy, local::context())
            .await
            .unwrap(),
        forged
    );
    assert!(
        host.close_window(close.clone(), local::context())
            .await
            .is_err(),
        "physical release cannot replace durable center confirmation"
    );
    assert_eq!(
        host.free_resources(config().node, config().compute)
            .await
            .unwrap()
            .1
            .instances,
        1
    );
    assert!(
        host.confirm_terminal(forged.clone(), local::context())
            .await
            .unwrap()
            .matches(&forged)
    );
    close
}

#[tokio::test]
async fn exact_durable_confirmation_precedes_reuse_and_old_messages_cannot_touch_successor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let physical = matching_host();
    let app = Arc::new(local::application(&path, physical.clone(), true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let create = local::request();
    host.record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    host.advance(
        create.clone(),
        Some(local::fixture::template()),
        local::fixture::compute(),
    )
    .await
    .unwrap();
    let close = confirm_original_termination(&host, &create).await;
    drop((host, app));
    let app = Arc::new(local::application(&path, physical, false));
    let host = HostLifecycle::new(app.clone());
    assert!(
        host.close_window(close.clone(), local::context())
            .await
            .unwrap()
            .matches(&close)
    );
    assert_eq!(
        host.free_resources(config().node, config().compute)
            .await
            .unwrap()
            .1
            .instances,
        2
    );
    assert!(
        host.record_lifecycle(create.clone(), local::context())
            .await
            .is_err(),
        "retired Create cannot become absent and run again"
    );
    let mut successor = create;
    successor.request.binding.session = "successor".try_into().unwrap();
    successor.request.binding.key.sequence = 3.try_into().unwrap();
    successor.request.binding.key.owner = "account-beta".try_into().unwrap();
    successor.retention.window = 2.try_into().unwrap();
    host.record_lifecycle(successor.clone(), local::context())
        .await
        .unwrap();
    // Exact recent replay remains answerable; it cannot close the newer use.
    assert!(
        host.close_window(close.clone(), local::context())
            .await
            .unwrap()
            .matches(&close)
    );
    assert!(
        app.lifecycle_work_state(&successor, &local::context())
            .unwrap()
            .current
    );
    app.establish_lifecycle_host_epoch().unwrap();
    host.drive_once(config(), Arc::new(Templates))
        .await
        .unwrap();
    assert!(matches!(
        app.observe_lifecycle_create(&successor, &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
}
impl TemplateSource for HeldMissing {
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        if binding.template_revision.as_str() == "missing" {
            if self
                .arrivals
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                < 4
            {
                self.first_pass.wait();
            }
            Err(ApplicationError::NotFound)
        } else {
            Templates.bound(binding)
        }
    }
}

#[tokio::test]
async fn four_unavailable_entries_cannot_starve_the_fifth_real_create() {
    let dir = tempfile::tempdir().unwrap();
    let mut inbox = local::config();
    inbox.max_sessions = 5;
    let physical = matching_host();
    let app = Arc::new(local::application_with_configuration(
        &dir.path().join("db"),
        physical.clone(),
        true,
        inbox,
    ));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let mut requests = Vec::new();
    for index in 0..5 {
        let mut request = local::request();
        request.request.binding.session = format!("session-{index}").as_str().try_into().unwrap();
        request.retention.slot = (index + 1).try_into().unwrap();
        request.request.binding.key.sequence = (index + 1_u64).try_into().unwrap();
        if index < 4 {
            let LifecycleOperation::Create { create, .. } = &mut request.request.operation else {
                panic!()
            };
            create.template_revision = "missing".try_into().unwrap();
        } else {
            request.request.binding.key.owner = "account-beta".try_into().unwrap();
        }
        host.record_lifecycle(request.clone(), local::context())
            .await
            .unwrap();
        requests.push(request);
    }
    let templates = Arc::new(HeldMissing {
        first_pass: std::sync::Barrier::new(5),
        arrivals: std::sync::atomic::AtomicUsize::new(0),
    });
    let first_host = host.clone();
    let first_templates = templates.clone();
    let first = tokio::spawn(async move { first_host.drive_once(config(), first_templates).await });
    let barrier = templates.clone();
    tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || barrier.first_pass.wait()),
    )
    .await
    .unwrap()
    .unwrap();
    let report = first.await.unwrap().unwrap();
    assert_eq!(
        (report.scanned, report.advanced, report.unavailable),
        (5, 0, 5)
    );
    assert!(physical.lock().unwrap().actions.is_empty());
    // The next finite pass rotates the original inbox scan. It does not create
    // another authoritative queue or turn missing material into NotAccepted.
    let report = host.drive_once(config(), templates).await.unwrap();
    assert_eq!(report.advanced, 1);
    assert!(matches!(
        app.observe_lifecycle_create(&requests[4], &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
    for request in &requests[..4] {
        let state = app
            .lifecycle_work_state(request, &local::context())
            .unwrap();
        assert!(state.current && !state.terminal && state.local.is_none());
    }
    assert_eq!(
        physical
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|action| matches!(
                action.command.command(),
                sandbox_controller_core::Command::PrepareVm { .. }
            ))
            .count(),
        1
    );
    assert_eq!(host.active_work(), 0);
}

#[tokio::test]
async fn cancelled_caller_retains_actual_work_permit_and_terminal_registration_still_progresses() {
    use sandproof_protocol::lifecycle::ExpectedSession;
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    physical.lock().unwrap().block_prepare = Some((entered_tx, release_rx));
    let app = Arc::new(local::application(&dir.path().join("db"), physical, true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let request = local::request();
    host.record_lifecycle(request.clone(), local::context())
        .await
        .unwrap();
    let worker_host = host.clone();
    let work = request.clone();
    let worker = tokio::spawn(async move {
        worker_host
            .advance(
                work,
                Some(local::fixture::template()),
                local::fixture::compute(),
            )
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv_timeout(Duration::from_secs(5)).unwrap())
        .await
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    assert_eq!(
        host.active_work(),
        1,
        "HTTP/task cancellation cannot refund a live blocking operation"
    );
    let mut destroy = request;
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    assert!(
        host.record_lifecycle(destroy.clone(), local::context())
            .await
            .unwrap()
            .terminal
    );
    assert!(
        !app.lifecycle_destroy_cleaned(&destroy, &local::context())
            .unwrap()
    );
    assert_eq!(host.active_work(), 1);
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.active_work() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        app.lifecycle_destroy_cleaned(&destroy, &local::context())
            .unwrap()
    );
}

#[tokio::test]
async fn same_original_driver_starts_recorded_create_then_restart_never_recreates_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let physical = matching_host();
    let app = Arc::new(local::application(&path, physical.clone(), true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let request = local::request();
    host.record_lifecycle(request.clone(), local::context())
        .await
        .unwrap();
    assert!(physical.lock().unwrap().actions.is_empty());
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 1,
            advanced: 1,
            unavailable: 0,
        }
    );
    let original = app
        .lifecycle_work_state(&request, &local::context())
        .unwrap();
    assert!(original.local.is_some());
    assert!(matches!(
        app.observe_lifecycle_create(&request, &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
    let calls = physical.lock().unwrap().actions.len();
    assert!(calls > 0);
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport::default()
    );
    assert_eq!(physical.lock().unwrap().actions.len(), calls);
    drop((host, app));
    let app = Arc::new(local::application(&path, physical.clone(), false));
    // The reopened manager checks its original registered requirement again;
    // this fixture supplies controlled matching host facts, not a real payload.
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport::default()
    );
    assert_eq!(
        app.lifecycle_work_state(&request, &local::context())
            .unwrap(),
        original
    );
    assert!(matches!(
        app.observe_lifecycle_create(&request, &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
    assert_eq!(physical.lock().unwrap().actions.len(), calls);
    assert_eq!(host.active_work(), 0);
}

#[tokio::test]
async fn first_missing_material_keeps_responsibility_and_does_not_starve_next_create() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    let app = Arc::new(local::application(&dir.path().join("db"), physical, true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let mut missing = local::request();
    let LifecycleOperation::Create { create, .. } = &mut missing.request.operation else {
        unreachable!()
    };
    create.template_revision = "missing".try_into().unwrap();
    host.record_lifecycle(missing.clone(), local::context())
        .await
        .unwrap();
    let mut healthy = local::request();
    healthy.request.binding.session = "second-session".try_into().unwrap();
    healthy.retention.slot = 2;
    healthy.request.binding.key.sequence = 2.try_into().unwrap();
    host.record_lifecycle(healthy.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 2,
            advanced: 1,
            unavailable: 1,
        }
    );
    let pending = app
        .lifecycle_work_state(&missing, &local::context())
        .unwrap();
    assert!(pending.local.is_none());
    assert!(pending.current);
    assert!(!pending.terminal);
    assert_eq!(
        app.lifecycle_pending_creates(&config().node).unwrap(),
        vec![missing]
    );
    assert!(matches!(
        app.observe_lifecycle_create(&healthy, &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
}

#[tokio::test]
async fn missing_epoch_cannot_start_or_turn_original_accepted_work_into_not_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    let app = Arc::new(local::application(
        &dir.path().join("db"),
        physical.clone(),
        true,
    ));
    let host = HostLifecycle::new(app.clone());
    let request = local::request();
    host.record_lifecycle(request.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 1,
            advanced: 0,
            unavailable: 1,
        }
    );
    let state = app
        .lifecycle_work_state(&request, &local::context())
        .unwrap();
    assert!(state.current && !state.terminal && state.local.is_none());
    assert_eq!(
        app.lifecycle_pending_creates(&config().node).unwrap(),
        vec![request]
    );
    assert!(physical.lock().unwrap().actions.is_empty());
}

#[tokio::test]
async fn wrong_registered_driver_node_refuses_before_original_create() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    let app = Arc::new(local::application(
        &dir.path().join("db"),
        physical.clone(),
        true,
    ));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let request: AcceptedLifecycleIntent = local::request();
    host.record_lifecycle(request.clone(), local::context())
        .await
        .unwrap();
    let mut wrong = config();
    wrong.node.node_generation = 2.try_into().unwrap();
    assert_eq!(
        host.drive_once(wrong, Arc::new(Templates)).await,
        Err(ApplicationError::OutcomeUnknown)
    );
    assert_eq!(
        app.lifecycle_pending_creates(&config().node).unwrap(),
        vec![request]
    );
    assert!(physical.lock().unwrap().actions.is_empty());
}

#[tokio::test]
async fn native_running_without_original_source_stays_historical_and_not_current_ready() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    physical.lock().unwrap().source = None;
    let app = Arc::new(local::application(&dir.path().join("db"), physical, true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let request = local::request();
    host.record_lifecycle(request.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 1,
            advanced: 1,
            unavailable: 0,
        }
    );
    let view = app
        .observe_lifecycle_create(&request, &local::context())
        .unwrap();
    assert_eq!(
        view.historical_result,
        Some(sandbox_controller_core::OperationResult::Running(1))
    );
    assert_eq!(
        view.readiness,
        CreateReadiness::Pending(
            sandproof_controller_service::CreateReadinessPending::OriginalEvidenceMissing,
        )
    );
}
