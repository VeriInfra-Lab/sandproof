use super::*;
use sandproof_internal::status::{
    HostStatus, STATUS_PATH, STATUS_REQUEST_BYTES, StatusReceiver, StatusRequest,
};
use sandproof_internal::{
    CacheReceiver, DataReceiver, LifecycleReceiver, ReceiveError, ValidationReceiver,
};
use sandproof_protocol::internal::lifecycle::Resources;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Observed(Arc<AtomicUsize>);

#[test]
fn status_requires_explicit_bounded_protected_retention_capacity() {
    let request = StatusRequest { node: node() };
    let valid = HostStatus {
        node: node(),
        retention_slots: 2,
        observed_at_ms: 1234,
        free: Resources::ZERO,
    };
    assert!(valid.validate_for(&request).is_ok());
    for slots in [0, 4097] {
        let mut invalid = valid.clone();
        invalid.retention_slots = slots;
        assert!(invalid.validate_for(&request).is_err());
    }
    let mut encoded = serde_json::to_value(valid).unwrap();
    encoded.as_object_mut().unwrap().remove("retention_slots");
    assert!(serde_json::from_value::<HostStatus>(encoded).is_err());
}
impl StatusReceiver for Observed {
    async fn status(&self, request: StatusRequest) -> Result<HostStatus, ReceiveError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(HostStatus {
            node: request.node,
            retention_slots: 2,
            observed_at_ms: 1234,
            free: Resources::ZERO,
        })
    }
}
impl LifecycleReceiver for Observed {}
impl DataReceiver for Observed {
    async fn receive_data(
        &self,
        identity: sandproof_protocol::lifecycle::LifecycleAccessIdentity,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
        request: Request<axum::body::Body>,
    ) -> Response<axum::body::Body> {
        RejectLifecycle.receive_data(identity, context, request).await
    }
}
impl ValidationReceiver for Observed {
    async fn validation_evidence(
        &self,
        request: sandproof_internal::template::ValidationRequest,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        RejectLifecycle.validation_evidence(request).await
    }
}
impl CacheReceiver for Observed {
    async fn cache_template(
        &self,
        request: sandproof_internal::cache::CacheRequest,
    ) -> Result<sandproof_internal::cache::CacheAvailability, ReceiveError> {
        RejectLifecycle.cache_template(request).await
    }
}

#[tokio::test]
async fn status_uses_original_mtls_budget_and_rejects_foreign_or_oversized_input() {
    let certs = certificates();
    let count = Arc::new(AtomicUsize::new(0));
    let server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        node(),
        limits(),
        Observed(Arc::clone(&count)),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let caller = client(&certs, address);
    let status = caller.status().await.unwrap();
    assert_eq!(status.node, node());
    assert_eq!(status.observed_at_ms, 1234);
    assert_eq!(status.retention_slots, 2);
    assert_eq!(caller.active_calls(), 0);
    let mut wrong = StatusRequest { node: node() };
    wrong.node.node_generation = 2.try_into().unwrap();
    let (code, _) = raw_request(
        address,
        raw_config(&certs, Some(&certs.center)),
        http_request(
            STATUS_PATH,
            Bytes::from(serde_json::to_vec(&wrong).unwrap()),
        ),
    )
    .await
    .unwrap();
    assert_eq!(code, StatusCode::CONFLICT);
    let (code, _) = raw_request(
        address,
        raw_config(&certs, Some(&certs.center)),
        http_request(
            STATUS_PATH,
            Bytes::from(vec![b' '; STATUS_REQUEST_BYTES + 1]),
        ),
    )
    .await
    .unwrap();
    assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
}

/// The real Rust status transport crosses both original Python relay TLS legs.
/// This is a local authenticated component test, not a KVM/SDK acceptance claim.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_status_crosses_python_double_mtls_with_strict_chain_and_alpn() {
    let certs = certificates_with_dual_roles();
    let count = Arc::new(AtomicUsize::new(0));
    let server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        node(),
        limits(),
        Observed(Arc::clone(&count)),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let mut relay = super::python_relay::PythonRelay::start(&certs, backend, &node());
    let caller = client(&certs, relay.address);
    let status = caller.status().await.unwrap();
    assert_eq!(status.node, node());
    assert_eq!(status.retention_slots, 2);
    assert_eq!(status.observed_at_ms, 1234);
    assert_eq!(caller.active_calls(), 0);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // A CA-valid but unregistered caller must not cross the first leaf pin.
    let foreign = raw_request(
        relay.address,
        raw_config(&certs, Some(&certs.other)),
        http_request(
            STATUS_PATH,
            Bytes::from(serde_json::to_vec(&StatusRequest { node: node() }).unwrap()),
        ),
    )
    .await;
    assert!(foreign.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    relay.stop();
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
}
