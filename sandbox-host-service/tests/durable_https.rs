//! Real mTLS and the current original Journal; controlled host effects, not KVM.
#[path = "durable_https/execution.rs"]
mod execution;
#[path = "durable_https/internal_data.rs"]
mod internal_data;
#[allow(
    dead_code,
    reason = "shared component assembly and original host fixture"
)]
#[path = "durable_https/support.rs"]
mod local;
#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod tls;
#[path = "durable_https/validation.rs"]
mod validation;
#[path = "durable_https/window.rs"]
mod window;
use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits};
use sandproof_data_transport::TransferLimits;
use sandproof_host_service::{HostFacade, HostLifecycle};
use sandproof_internal::{
    ClientTls, InternalClient, InternalServer, LifecycleReceiver, Limits, RegisteredEndpoint,
    ServerTls, lifecycle::CenterPeerRegistration,
};
use sandproof_protocol::lifecycle::{CurrentCreateReadiness, TerminalStatus};
use std::{
    sync::{Arc, Mutex, atomic::AtomicI64},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};

fn facade(
    application: Arc<
        sandproof_controller_service::application::ControllerApplication<local::TestHost>,
    >,
) -> HostFacade<local::TestHost> {
    HostFacade::new(
        application,
        TransferLimits {
            file_upload_bytes: 1024,
            file_download_bytes: 1024,
            command_input_bytes: 1024,
            rpc_request_bytes: 1024,
            rpc_response_bytes: 1024,
            max_transfers: 3,
            max_transfers_per_vm: 2,
            control_reserve: 1,
            request_idle_seconds: 5,
            file_idle_seconds: 5,
            write_idle_seconds: 5,
        },
        RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 4,
            control_reserve: 1,
        })
        .unwrap(),
    )
    .unwrap()
}

async fn changed_requests_are_refused(
    client: &InternalClient,
    create: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) {
    for variant in 0..7 {
        let mut changed = create.clone();
        match variant {
            0 => local::create_mut(&mut changed).timeout_ms += 1,
            1 => changed.retention.slot = 2,
            2 => changed.retention.window = 2.try_into().unwrap(),
            3 => changed.center.center = "other-center".try_into().unwrap(),
            4 => changed.accepted_version += 1,
            5 => changed.request.binding.key.owner = "other-owner".try_into().unwrap(),
            _ => changed.request.binding.node.node_generation = 2.try_into().unwrap(),
        }
        assert!(
            client.record_lifecycle(&changed).await.is_err(),
            "changed request variant {variant}"
        );
    }
}

#[tokio::test]
async fn actual_https_resume_forwards_original_pending_through_facade_and_wrapper() {
    use sandproof_protocol::lifecycle::{ContentDigest, CurrentResumeReadiness, LifecycleOperation};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(
        &path,
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let facade = facade(app);
    let inbox = facade.lifecycle();
    let (server, listener, client, foreign) = validation::validation_transport(facade).await;
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let create = local::request();
    client.record_lifecycle(&create).await.unwrap();
    local::drive(&inbox).await;
    assert!(matches!(
        client
            .observe_lifecycle_create(&create)
            .await
            .unwrap()
            .readiness,
        CurrentCreateReadiness::Observed { .. }
    ));
    let mut unload = local::destroy(&create);
    unload.request.operation = LifecycleOperation::Unload {
        source_incarnation: 101.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    client.record_lifecycle(&unload).await.unwrap();
    let mut resume = local::destroy(&unload);
    resume.request.operation = LifecycleOperation::Resume {
        source_incarnation: 101.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        target_incarnation: 102.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([7; 32]),
    };
    client.record_lifecycle(&resume).await.unwrap();
    // Only reception is complete; no Unload/Resume worker ran. With fresh
    // physical observations unavailable, forwarding cannot manufacture Ready.
    state.lock().unwrap().observe_unavailable = true;
    let expected = inbox
        .observe_resume(resume.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(expected.readiness, CurrentResumeReadiness::Pending);
    assert!(expected.matches_request(&resume));
    let before = local::audit(&path);
    let actions = state.lock().unwrap().actions.clone();
    let actual = client.observe_lifecycle_resume(&resume).await.unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.request, resume);
    assert_eq!(actual.current_request, resume);
    assert_eq!(local::audit(&path), before);
    assert_eq!(state.lock().unwrap().actions, actions);
    assert!(foreign.observe_lifecycle_resume(&resume).await.is_err());
    let mut altered = resume.clone();
    altered.retention.window = 2.try_into().unwrap();
    assert!(client.observe_lifecycle_resume(&altered).await.is_err());
    assert_eq!(
        client.observe_lifecycle_resume(&resume).await.unwrap(),
        expected
    );
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
    assert_eq!(inbox.active_work(), 0);
}

#[tokio::test]
async fn actual_https_records_original_journal_and_replays_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::state()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let destroy = local::destroy(&create);
    for provision in [true, false] {
        let application = local::application(&path, state.clone(), clock.clone(), provision);
        let facade = facade(application.clone());
        let inbox = facade.lifecycle();
        let (server, listener, client, foreign) = validation::validation_transport(facade).await;
        let (stop, shutdown) = oneshot::channel();
        let run = server.clone();
        let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
        assert!(foreign.record_lifecycle(&create).await.is_err());
        let recorded = client.record_lifecycle(&create).await.unwrap();
        assert!(recorded.matches(&create));
        assert_eq!(recorded.current, provision);
        changed_requests_are_refused(&client, &create).await;
        assert!(
            client
                .record_lifecycle(&destroy)
                .await
                .unwrap()
                .matches(&destroy)
        );
        // No local admission took place. The original durable Destroy fence,
        // rather than the transport, establishes that no native work is owed.
        local::drive(&inbox).await;
        let cleaned = client.observe_lifecycle_terminal(&destroy).await.unwrap();
        assert_eq!(cleaned.status, TerminalStatus::Cleaned);
        assert_eq!(
            client.observe_lifecycle_terminal(&destroy).await.unwrap(),
            cleaned
        );
        let old_create = client.record_lifecycle(&create).await.unwrap();
        assert_eq!(old_create.intent, create);
        assert!(!old_create.current);
        assert!(old_create.terminal);
        assert_eq!(old_create.current_session_version, 2);
        let mut late = create.clone();
        late.request.binding.key.sequence = 3.try_into().unwrap();
        assert!(client.record_lifecycle(&late).await.is_err());
        assert!(
            application
                .lifecycle_pending_creates(&local::config().node)
                .unwrap()
                .is_empty()
        );
        assert!(state.lock().unwrap().actions.is_empty());
        assert_eq!(client.active_calls(), 0);
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(server.active_connections(), 0);
        assert_eq!(inbox.active_work(), 0);
    }
}

#[tokio::test]
async fn cancelled_caller_keeps_actual_blocking_work_budget_until_host_exits() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::state()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), state.clone(), clock, true);
    let inbox = HostLifecycle::new(app.clone());
    let create = local::request();
    inbox
        .record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state.lock().unwrap().block_prepare = Some((entered, wait));
    let worker = inbox.clone();
    let current = create.clone();
    let caller = tokio::spawn(async move {
        worker
            .advance(current, Some(local::template()), local::compute())
            .await
    });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(15)).unwrap())
        .await
        .unwrap();
    caller.abort();
    let _ = caller.await;
    assert_eq!(inbox.active_work(), 1);
    // The real callback still holds its work permit. Independent control
    // capacity nevertheless accepts Destroy; recording cannot free that work.
    let destroy = local::destroy(&create);
    assert!(
        inbox
            .record_lifecycle(destroy.clone(), local::context())
            .await
            .unwrap()
            .matches(&destroy)
    );
    assert_eq!(inbox.active_work(), 1);
    assert_eq!(
        inbox
            .observe_terminal(destroy.clone(), local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Pending
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while inbox.active_work() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        state
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|a| matches!(
                a.command.command(),
                sandbox_controller_core::Command::PrepareVm { .. }
            ))
            .count(),
        1
    );
    local::drive(&inbox).await;
    assert_eq!(
        inbox
            .observe_terminal(destroy, local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Cleaned
    );
}
