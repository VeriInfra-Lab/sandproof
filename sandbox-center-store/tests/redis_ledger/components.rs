//! Current authenticated lifecycle consumers; Redis and `SQLite` remain real.
use super::*;
use sandproof_internal::{LifecycleReceiver, ReceiveError};
#[path = "../../../sandbox-internal/tests/support/tls.rs"]
mod tls_fixture;

struct LoseFirst {
    inbox: sandproof_host_service::HostLifecycle<host_application::TestHost>,
    lose: AtomicBool,
}
impl sandproof_internal::ValidationReceiver for LoseFirst {
    async fn validation_evidence(
        &self,
        request: sandproof_internal::template::ValidationRequest,
        context: AuthenticatedCenterContext,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        self.inbox.validation_evidence(request, context).await
    }
}
impl LifecycleReceiver for LoseFirst {
    async fn observe_resume(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, ReceiveError> {
        self.inbox.observe_resume(intent, context).await
    }

    async fn record_lifecycle(
        &self,
        request: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, ReceiveError> {
        let response = self.inbox.record_lifecycle(request, context).await?;
        // The original SQLite commit succeeds before the receipt is dropped.
        if self.lose.swap(false, Ordering::SeqCst) {
            Err(ReceiveError::Unavailable)
        } else {
            Ok(response)
        }
    }
    async fn observe_create(
        &self,
        request: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::CreateObservation, ReceiveError> {
        self.inbox.observe_create(request, context).await
    }
    async fn observe_terminal(
        &self,
        request: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<TerminalLifecycleObservation, ReceiveError> {
        self.inbox.observe_terminal(request, context).await
    }
    async fn confirm_terminal(
        &self,
        result: TerminalLifecycleObservation,
        context: AuthenticatedCenterContext,
    ) -> Result<ConfirmedLifecycleResult, ReceiveError> {
        self.inbox.confirm_terminal(result, context).await
    }
    async fn close_window(
        &self,
        request: sandproof_protocol::lifecycle::LifecycleWindowClose,
        context: AuthenticatedCenterContext,
    ) -> Result<ClosedLifecycleWindow, ReceiveError> {
        self.inbox.close_window(request, context).await
    }
}
impl sandproof_internal::DataReceiver for LoseFirst {
    async fn receive_data(
        &self,
        _: sandproof_protocol::lifecycle::LifecycleAccessIdentity,
        _: AuthenticatedCenterContext,
        _: axum::http::Request<axum::body::Body>,
    ) -> axum::http::Response<axum::body::Body> {
        let mut response = axum::http::Response::new(axum::body::Body::empty());
        *response.status_mut() = axum::http::StatusCode::SERVICE_UNAVAILABLE;
        response
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one ordered real Redis/HTTPS/SQLite lost-reply scenario"
)]
async fn committed_outbox_real_https_sqlite_and_lost_reply_preserve_one_operation() {
    use sandproof_internal::{
        ClientTls, InternalClient, InternalServer, Limits, RegisteredEndpoint, ServerTls,
        lifecycle::CenterPeerRegistration,
    };
    let fixture = Engine::new();
    let binding = host_application::request().create;
    let resources = binding.resources.checked_add(binding.resources).unwrap();
    let state = center_state_with_resources(resources, &binding);
    let limit = state.required_authority_bytes();
    let mut center = center_authority_with_state(&fixture, fixture.address, &state);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let physical = Arc::new(std::sync::Mutex::new(host_application::State {
        epoch: host_application::TestEpoch::Matching,
        source: Some(
            sandproof_controller_service::capture::SourceProcessIdentity {
                pid: 1234,
                start_time_ticks: 5678,
                boot_id: "fixture-boot".into(),
            },
        ),
        ..host_application::State::default()
    }));
    let host = Arc::new(current_application::application(
        &path,
        Arc::clone(&physical),
        true,
    ));
    host.establish_lifecycle_host_epoch().unwrap();
    let inbox = sandproof_host_service::HostLifecycle::new(Arc::clone(&host));
    let certs = tls_fixture::certificates();
    let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap();
    let server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        lifecycle_fixture::node("host-a"),
        limits,
        sandproof_internal::diagnostic::RejectTemplateCache::new(LoseFirst {
            inbox: inbox.clone(),
            lose: AtomicBool::new(true),
        }),
    )
    .with_center_peer(
        CenterPeerRegistration::new(
            current_application::config().center,
            lifecycle_fixture::node("host-a"),
            vec![certs.center.pin()],
        )
        .unwrap(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = InternalClient::new(
        ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(
            listener.local_addr().unwrap(),
            "host.test".into(),
            lifecycle_fixture::node("host-a"),
        )
        .unwrap(),
        limits,
    );
    let (stop, shutdown) = tokio::sync::oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let create = accept_create(&mut center, binding, 1);
    let pending = center.lifecycle_dispatch_batch().unwrap().pop().unwrap();
    assert_eq!(pending, create);
    assert!(client.record_lifecycle(&pending).await.is_err());
    assert_eq!(
        center.state().unwrap().pending_lifecycle(),
        vec![create.clone()]
    );
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(create_resources(&create))
    );
    assert_eq!(
        inbox
            .pending(lifecycle_fixture::node("host-a"))
            .await
            .unwrap(),
        vec![create.clone()]
    );
    let response = client
        .record_lifecycle(&center.lifecycle_dispatch_batch().unwrap().pop().unwrap())
        .await
        .unwrap();
    assert_eq!(response.intent, create);
    let replay = client.record_lifecycle(&create).await.unwrap();
    assert_eq!(response, replay);
    assert!(physical.lock().unwrap().actions.is_empty());
    let before = client.observe_lifecycle_create(&create).await.unwrap();
    assert!(!matches!(
        before.readiness,
        CurrentCreateReadiness::Observed { .. }
    ));
    let work = inbox
        .advance(
            create.clone(),
            Some(host_application::template()),
            host_application::compute(),
        )
        .await
        .unwrap();
    assert!(work.local.is_some());
    let ready = client.observe_lifecycle_create(&create).await.unwrap();
    assert!(matches!(
        ready.readiness,
        CurrentCreateReadiness::Observed { .. }
    ));
    drop(center);
    let (ledger, _) = RedisLedger::<CenterState>::open(
        endpoint(fixture.address),
        Engine::config(limit),
        fixture.lease(),
    )
    .unwrap();
    let mut center = CenterAuthority::attach(ledger).unwrap();
    assert_eq!(
        center.lifecycle_dispatch_batch().unwrap(),
        vec![create.clone()]
    );
    let replay_ready = client.observe_lifecycle_create(&create).await.unwrap();
    assert_eq!(replay_ready, ready);
    center
        .accept_lifecycle_create_observation(&replay_ready)
        .unwrap();
    assert_eq!(
        physical
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
    let destroy = accept_destroy(&mut center, &create, 2);
    assert_eq!(
        client.record_lifecycle(&destroy).await.unwrap().intent,
        destroy
    );
    assert!(
        center
            .accept_lifecycle_create_observation(&replay_ready)
            .is_err()
    );
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(create_resources(&create))
    );
    host.advance_current_workflow(work.local.unwrap().sandbox)
        .unwrap();
    let cleaned = client.observe_lifecycle_terminal(&destroy).await.unwrap();
    assert_eq!(cleaned.status, TerminalStatus::Cleaned);
    center
        .accept_lifecycle_terminal_observation(&cleaned, 100)
        .unwrap();
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(sandproof_protocol::internal::lifecycle::Resources::ZERO)
    );
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(inbox.active_work(), 0);
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
}
fn create_resources(
    intent: &AcceptedLifecycleIntent,
) -> sandproof_protocol::internal::lifecycle::Resources {
    let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
        unreachable!()
    };
    create.resources
}

#[test]
fn real_redis_commit_dispatch_sqlite_record_and_replay_are_one_identity_not_vm_success() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let mut host = current_application::open(&path);
    host.initialize_lifecycle_inbox(
        current_application::config(),
        current_application::source_limits(),
        &mut sandproof_controller_service::NeverCrash,
    )
    .unwrap();
    let create = accept_create(&mut center, lifecycle_fixture::binding(), 1);
    let batch = center.lifecycle_dispatch_batch().unwrap();
    assert_eq!(batch, vec![create.clone()]);
    let answer = host
        .record_lifecycle_intent(
            &batch[0],
            &current_application::context(),
            &mut sandproof_controller_service::NeverCrash,
        )
        .unwrap();
    assert_eq!(answer.intent, create);
    assert_eq!(answer.current_session_version, 1);
    assert!(answer.current && answer.local.is_none() && !answer.terminal);
    let replay = host
        .record_lifecycle_intent(
            &center.lifecycle_dispatch_batch().unwrap()[0],
            &current_application::context(),
            &mut sandproof_controller_service::NeverCrash,
        )
        .unwrap();
    assert_eq!(answer, replay);
    assert!(host.world().unwrap().sandbox_records().is_empty());
    assert_eq!(
        center.state().unwrap().pending_lifecycle(),
        vec![create.clone()]
    );
    let destroy = accept_destroy(&mut center, &create, 2);
    let answer = host
        .record_lifecycle_intent(
            &destroy,
            &current_application::context(),
            &mut sandproof_controller_service::NeverCrash,
        )
        .unwrap();
    assert!(answer.current && answer.terminal && answer.local.is_none());
    assert_eq!(center.state().unwrap().pending_lifecycle(), vec![destroy]);
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(lifecycle_fixture::resources(1))
    );
    let old = host
        .record_lifecycle_intent(
            &create,
            &current_application::context(),
            &mut sandproof_controller_service::NeverCrash,
        )
        .unwrap();
    assert_eq!(old.intent, create);
    assert_eq!(old.current_session_version, 2);
    assert!(!old.current);
    assert!(
        center
            .accept_lifecycle_create_observation(&lifecycle::observed(&create))
            .is_err()
    );
    assert!(host.world().unwrap().sandbox_records().is_empty());
    assert_eq!(center.state().unwrap().identity_frontiers(), (1, 2));
}

#[test]
fn center_actual_commit_reply_loss_blocks_dispatch_then_recovers_same_outbox() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let mut center = center_authority(&fixture, proxy.address);
    proxy.lose_after(0);
    assert_eq!(
        center.accept_lifecycle(
            &"1".try_into().unwrap(),
            &create_submission(lifecycle_fixture::binding(), 1),
            10
        ),
        Err(AuthorityError::Store(StoreError::OutcomeUnknown))
    );
    assert!(matches!(
        center.lifecycle_dispatch_batch(),
        Err(AuthorityError::RecoveryRequired)
    ));
    assert!(center.state().is_err());
    center.recover().unwrap();
    let pending = center.state().unwrap().pending_lifecycle();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].request.binding.session.as_protocol_id().as_str(),
        "sandbox-1"
    );
    assert_eq!(pending[0].accepted_version, 1);
    assert_eq!(center.lifecycle_dispatch_batch().unwrap(), pending);
}

#[tokio::test]
async fn host_transport_failure_and_old_fact_do_not_release_or_reroute() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let create = accept_create(&mut center, lifecycle_fixture::binding(), 1);
    let pending = center.lifecycle_dispatch_batch().unwrap().pop().unwrap();
    // A bound listener without an accept loop deterministically reaches the
    // real client's TLS/request deadline; no host can acknowledge this intent.
    let unavailable = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let certs = tls_fixture::certificates();
    let client = sandproof_internal::InternalClient::new(
        sandproof_internal::ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        sandproof_internal::RegisteredEndpoint::new(
            unavailable.local_addr().unwrap(),
            "host.test".into(),
            lifecycle_fixture::node("host-a"),
        )
        .unwrap(),
        sandproof_internal::Limits::new(1, Duration::from_millis(100), Duration::from_millis(200))
            .unwrap(),
    );
    assert!(client.record_lifecycle(&pending).await.is_err());
    assert_eq!(client.active_calls(), 0);
    drop(unavailable);
    assert_eq!(pending, create);
    assert_eq!(
        center.state().unwrap().pending_lifecycle(),
        vec![create.clone()]
    );
    let destroy = accept_destroy(&mut center, &create, 2);
    assert!(matches!(
        destroy.request.operation,
        LifecycleOperation::Destroy {}
    ));
    assert!(matches!(
        center.accept_lifecycle_create_observation(&lifecycle::observed(&create)),
        Err(AuthorityError::Decision(_))
    ));
    assert_eq!(center.state().unwrap().pending_lifecycle(), vec![destroy]);
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(lifecycle_fixture::resources(1))
    );
    fixture.field("payload", b"corrupted");
    assert!(center.lifecycle_dispatch_batch().is_err());
    assert!(center.lifecycle_dispatch_batch().is_err());
}

#[test]
fn full_future_envelope_is_required_before_any_admission() {
    let fixture = Engine::new();
    let (ledger, _) = RedisLedger::initialize(
        endpoint(fixture.address),
        Engine::config(16_384),
        fixture.lease(),
        &center_state(),
    )
    .unwrap();
    assert!(matches!(
        CenterAuthority::attach(ledger),
        Err(AuthorityError::Store(StoreError::Configuration))
    ));
}

#[test]
fn slow_detached_transport_does_not_hold_center_decision_lock() {
    use std::sync::{Mutex, mpsc};
    let fixture = Engine::new();
    let center = Arc::new(Mutex::new(center_authority(&fixture, fixture.address)));
    let create = accept_create(&mut center.lock().unwrap(), lifecycle_fixture::binding(), 1);
    let dispatch = center
        .lock()
        .unwrap()
        .lifecycle_dispatch_batch()
        .unwrap()
        .pop()
        .unwrap();
    let (entered, wait) = mpsc::sync_channel(0);
    let (release, gate) = mpsc::sync_channel(0);
    let worker = thread::spawn(move || {
        entered.send(()).unwrap();
        gate.recv_timeout(Duration::from_secs(5)).unwrap();
        lifecycle::observed(&dispatch)
    });
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut authority = center
        .try_lock()
        .expect("no authority borrow/lock spans detached RPC");
    let destroy = accept_destroy(&mut authority, &create, 2);
    release.send(()).unwrap();
    let stale = worker.join().unwrap();
    assert!(
        authority
            .accept_lifecycle_create_observation(&stale)
            .is_err()
    );
    assert_eq!(
        authority.state().unwrap().pending_lifecycle(),
        vec![destroy]
    );
}
