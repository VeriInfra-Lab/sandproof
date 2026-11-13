//! Real client/TLS/server forwarding only; this receiver is not a SQL authority.
use axum::body::Body;
use hyper::{Request, Response};
use sandproof_internal::{
    ClientTls, DataReceiver, InternalClient, InternalServer, LifecycleReceiver, Limits,
    ReceiveError, RegisteredEndpoint, RejectLifecycle, ServerTls, TransportError,
    diagnostic::RejectTemplateCache,
    lifecycle::CenterPeerRegistration,
    template::{ValidationEvidence, ValidationReceiver, ValidationRequest},
};
use sandproof_protocol::{
    internal::NodeIdentity,
    lifecycle::{
        AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ConfirmedRenewal,
        ExpectedSession, LifecycleAccessIdentity, LifecycleOperation, LifecycleRequest,
        OperationBinding, OperationKey, RenewalConfirmation, RetentionBinding,
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};

#[path = "support/tls.rs"]
mod tls;

fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: "renewal-deployment".try_into().unwrap(),
        node_id: "host-a".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    }
}
fn center() -> CenterIdentity {
    CenterIdentity {
        deployment: "renewal-deployment".try_into().unwrap(),
        center: "primary".try_into().unwrap(),
    }
}
fn result() -> RenewalConfirmation {
    RenewalConfirmation {
        request: AcceptedLifecycleIntent {
            center: center(),
            retention: RetentionBinding {
                slot: 1,
                window: 7.try_into().unwrap(),
            },
            accepted_version: 2,
            request: LifecycleRequest {
                binding: OperationBinding {
                    protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                    node: node(),
                    session: "session-a".try_into().unwrap(),
                    key: OperationKey {
                        deployment: node().deployment_id,
                        owner: "owner-a".try_into().unwrap(),
                        window: 2.try_into().unwrap(),
                        sequence: 19.try_into().unwrap(),
                    },
                    expected: ExpectedSession::Version(1),
                },
                operation: LifecycleOperation::Renew {
                    incarnation: 11.try_into().unwrap(),
                    end_at_ms: 250,
                    extend_only: false,
                },
            },
        },
        effective_end_at_ms: 250,
    }
}

type Received = Arc<Mutex<Vec<(RenewalConfirmation, AuthenticatedCenterContext)>>>;
struct Receiver {
    received: Received,
    change_ack: Arc<AtomicBool>,
}
impl LifecycleReceiver for Receiver {
    async fn observe_resume(
        &self,
        _: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
    async fn confirm_renewal(
        &self,
        result: RenewalConfirmation,
        context: AuthenticatedCenterContext,
    ) -> Result<ConfirmedRenewal, ReceiveError> {
        self.received
            .lock()
            .unwrap()
            .push((result.clone(), context));
        let mut ack = ConfirmedRenewal { result };
        if self.change_ack.load(Ordering::SeqCst) {
            ack.result.request.request.binding.key.owner = "other-owner".try_into().unwrap();
        }
        Ok(ack)
    }
}
impl DataReceiver for Receiver {
    async fn receive_data(
        &self,
        identity: LifecycleAccessIdentity,
        context: AuthenticatedCenterContext,
        request: Request<Body>,
    ) -> Response<Body> {
        RejectLifecycle
            .receive_data(identity, context, request)
            .await
    }
}
impl ValidationReceiver for Receiver {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
        context: AuthenticatedCenterContext,
    ) -> Result<ValidationEvidence, ReceiveError> {
        RejectLifecycle.validation_evidence(request, context).await
    }
}

#[tokio::test]
async fn renewal_confirmation_preserves_exact_result_and_independent_peer_context() {
    let certificates = tls::certificates();
    let received = Received::default();
    let change_ack = Arc::new(AtomicBool::new(false));
    let limits = Limits::new(2, Duration::from_secs(5), Duration::from_secs(10)).unwrap();
    let server = InternalServer::new(
        ServerTls::new(
            certificates.host.material(),
            vec![certificates.ca.der().clone()],
            vec![certificates.center.pin()],
        )
        .unwrap(),
        node(),
        limits,
        RejectTemplateCache::new(Receiver {
            received: Arc::clone(&received),
            change_ack: Arc::clone(&change_ack),
        }),
    )
    .with_center_peer(
        CenterPeerRegistration::new(center(), node(), vec![certificates.center.pin()]).unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let client = InternalClient::new(
        ClientTls::new(
            certificates.center.material(),
            vec![certificates.ca.der().clone()],
            vec![certificates.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(address, "host.test".into(), node()).unwrap(),
        limits,
    );
    let original = result();
    assert_eq!(
        client.confirm_lifecycle_renewal(&original).await.unwrap(),
        ConfirmedRenewal {
            result: original.clone()
        }
    );
    assert_eq!(
        *received.lock().unwrap(),
        vec![(
            original.clone(),
            AuthenticatedCenterContext::from_verified_peer_registration(center(), node())
        )]
    );
    assert_rejections(&client, &original, &received, &change_ack).await;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
}

async fn assert_rejections(
    client: &InternalClient,
    original: &RenewalConfirmation,
    received: &Received,
    change_ack: &AtomicBool,
) {
    let mut foreign = original.clone();
    foreign.request.center.center = "unregistered-center".try_into().unwrap();
    assert_eq!(
        client.confirm_lifecycle_renewal(&foreign).await,
        Err(TransportError::Rejected)
    );
    let mut public = original.clone();
    public.request.request.binding.key.window = 3.try_into().unwrap();
    assert_eq!(
        client.confirm_lifecycle_renewal(&public).await,
        Err(TransportError::Rejected)
    );
    let mut changed_end = original.clone();
    changed_end.effective_end_at_ms += 1;
    assert_eq!(
        client.confirm_lifecycle_renewal(&changed_end).await,
        Err(TransportError::Rejected)
    );
    assert_eq!(
        received.lock().unwrap().len(),
        1,
        "rejected input must not reach the provider"
    );
    change_ack.store(true, Ordering::SeqCst);
    assert_eq!(
        client.confirm_lifecycle_renewal(original).await,
        Err(TransportError::Rejected)
    );
    assert_eq!(
        received.lock().unwrap().len(),
        2,
        "a changed reply is not an absent call"
    );
}
