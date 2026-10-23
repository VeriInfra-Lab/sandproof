//! Exact current terminal confirmation/retirement over real mTLS and SQLite.
use super::{Arc, AtomicI64, Mutex, facade, local, tls};
use axum::{body::Body, http::{Request, Response}};
use sandproof_host_service::HostFacade;
use sandproof_internal::{
    ClientTls, DataReceiver, InternalClient, InternalServer, LifecycleReceiver, Limits, ReceiveError,
    RegisteredEndpoint, ServerTls, ValidationReceiver, lifecycle::CenterPeerRegistration,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, ClosedLifecycleWindow,
    ConfirmedLifecycleResult, ConfirmedRenewal, CreateObservation, LifecycleAccessIdentity,
    LifecycleWindowClose, RecordedLifecycleIntent, RenewalConfirmation,
    TerminalLifecycleObservation, TerminalStatus,
};
use std::{sync::atomic::{AtomicBool, Ordering}, time::Duration};
use tokio::{net::TcpListener, sync::oneshot};

struct LostCloseAck { original: HostFacade<local::TestHost>, lose_once: Arc<AtomicBool> }
impl LifecycleReceiver for LostCloseAck {
    async fn record_lifecycle(&self, request: AcceptedLifecycleIntent, context: AuthenticatedCenterContext) -> Result<RecordedLifecycleIntent, ReceiveError> { self.original.record_lifecycle(request, context).await }
    async fn observe_create(&self, request: AcceptedLifecycleIntent, context: AuthenticatedCenterContext) -> Result<CreateObservation, ReceiveError> { self.original.observe_create(request, context).await }
    async fn observe_terminal(&self, request: AcceptedLifecycleIntent, context: AuthenticatedCenterContext) -> Result<TerminalLifecycleObservation, ReceiveError> { self.original.observe_terminal(request, context).await }
    async fn confirm_terminal(&self, result: TerminalLifecycleObservation, context: AuthenticatedCenterContext) -> Result<ConfirmedLifecycleResult, ReceiveError> { self.original.confirm_terminal(result, context).await }
    async fn confirm_renewal(&self, result: RenewalConfirmation, context: AuthenticatedCenterContext) -> Result<ConfirmedRenewal, ReceiveError> { self.original.confirm_renewal(result, context).await }
    async fn close_window(&self, request: LifecycleWindowClose, context: AuthenticatedCenterContext) -> Result<ClosedLifecycleWindow, ReceiveError> {
        let answer = self.original.close_window(request, context).await?;
        if self.lose_once.swap(false, Ordering::SeqCst) { return Err(ReceiveError::Unavailable); }
        Ok(answer)
    }
}
impl DataReceiver for LostCloseAck {
    async fn receive_data(&self, identity: LifecycleAccessIdentity, context: AuthenticatedCenterContext, request: Request<Body>) -> Response<Body> { self.original.receive_data(identity, context, request).await }
}
impl ValidationReceiver for LostCloseAck {
    async fn validation_evidence(&self, request: sandproof_internal::template::ValidationRequest, context: AuthenticatedCenterContext) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> { self.original.validation_evidence(request, context).await }
}

async fn successor_survives_old_close(client: &InternalClient, create: &AcceptedLifecycleIntent, close: &LifecycleWindowClose, ack: ClosedLifecycleWindow) {
    let mut successor = create.clone(); successor.retention.window = 2.try_into().unwrap();
    successor.request.binding.session = "session-2".try_into().unwrap(); successor.request.binding.key.sequence = 3.try_into().unwrap();
    let recorded = client.record_lifecycle(&successor).await.unwrap(); assert!(recorded.matches(&successor)); assert!(recorded.current);
    assert_eq!(client.close_lifecycle_window(close).await.unwrap(), ack);
    assert_eq!(client.record_lifecycle(&successor).await.unwrap(), recorded);
    let mut future = close.clone(); future.terminal.request.retention.window = 3.try_into().unwrap();
    assert!(client.close_lifecycle_window(&future).await.is_err());
    let mut foreign = close.clone(); foreign.terminal.request.request.binding.node.node_generation = 2.try_into().unwrap();
    assert!(client.close_lifecycle_window(&foreign).await.is_err());
    assert!(client.record_lifecycle(create).await.is_err());
}

async fn transport(original: HostFacade<local::TestHost>, lose_once: Arc<AtomicBool>) -> (InternalServer<sandproof_internal::diagnostic::RejectTemplateCache<LostCloseAck>>, TcpListener, InternalClient) {
    let certs = tls::certificates(); let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap(); let cfg = local::config();
    let server = InternalServer::new(ServerTls::new(certs.host.material(), vec![certs.ca.der().clone()], vec![certs.center.pin()]).unwrap(), cfg.node.clone(), limits,
        sandproof_internal::diagnostic::RejectTemplateCache::new(LostCloseAck { original, lose_once }))
        .with_center_peer(CenterPeerRegistration::new(cfg.center, cfg.node, vec![certs.center.pin()]).unwrap()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = InternalClient::new(ClientTls::new(certs.center.material(), vec![certs.ca.der().clone()], vec![certs.host.pin()]).unwrap(),
        RegisteredEndpoint::new(listener.local_addr().unwrap(), "host.test".into(), local::config().node).unwrap(), limits);
    (server, listener, client)
}

#[tokio::test]
async fn exact_terminal_confirmation_and_lost_close_ack_survive_real_https_and_reopen() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("db"); let state = Arc::new(Mutex::new(local::state())); let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request(); let destroy = local::destroy(&create); let lose_once = Arc::new(AtomicBool::new(true)); let mut saved_terminal = None;
    for provision in [true, false] {
        let app = local::application(&path, state.clone(), clock.clone(), provision); let facade = facade(app); let worker = facade.lifecycle();
        let (server, listener, client) = transport(facade, lose_once.clone()).await;
        let (stop, shutdown) = oneshot::channel(); let run = server.clone(); let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
        if provision {
            client.record_lifecycle(&create).await.unwrap(); worker.advance(create.clone(), Some(local::template()), local::compute()).await.unwrap();
            assert!(matches!(client.observe_lifecycle_create(&create).await.unwrap().readiness, sandproof_protocol::lifecycle::CurrentCreateReadiness::Observed { .. }));
            let forged = TerminalLifecycleObservation { request: destroy.clone(), status: TerminalStatus::Cleaned }; let close = LifecycleWindowClose { terminal: forged };
            assert!(client.close_lifecycle_window(&close).await.is_err()); // running is not terminal
            client.record_lifecycle(&destroy).await.unwrap(); local::drive(&worker).await;
            let cleaned = client.observe_lifecycle_terminal(&destroy).await.unwrap(); assert_eq!(cleaned.status, TerminalStatus::Cleaned);
            let close = LifecycleWindowClose { terminal: cleaned.clone() };
            assert!(client.close_lifecycle_window(&close).await.is_err()); // cleanup is not confirmation
            let mut altered = cleaned.clone(); altered.request.accepted_version += 1;
            assert!(client.confirm_lifecycle_terminal(&altered).await.is_err());
            let mut altered = cleaned.clone(); altered.request.retention.window = 2.try_into().unwrap();
            assert!(client.confirm_lifecycle_terminal(&altered).await.is_err());
            assert!(client.confirm_lifecycle_terminal(&cleaned).await.unwrap().matches(&cleaned));
            assert!(client.close_lifecycle_window(&close).await.is_err()); // original commit succeeded, only its reply lost
            assert!(!lose_once.load(Ordering::SeqCst)); assert!(client.record_lifecycle(&create).await.is_err());
            saved_terminal = Some(cleaned);
        } else {
            let close = LifecycleWindowClose { terminal: saved_terminal.clone().unwrap() }; let ack = client.close_lifecycle_window(&close).await.unwrap(); assert!(ack.matches(&close));
            assert!(client.confirm_lifecycle_terminal(saved_terminal.as_ref().unwrap()).await.is_err());
            successor_survives_old_close(&client, &create, &close, ack).await;
        }
        assert_eq!(client.active_calls(), 0); stop.send(()).unwrap(); task.await.unwrap().unwrap(); assert_eq!(server.active_connections(), 0); assert_eq!(worker.active_work(), 0);
    }
}
