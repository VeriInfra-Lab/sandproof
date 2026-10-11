use super::*;
use sandproof_internal::status::{
    HostStatus, STATUS_PATH, STATUS_REQUEST_BYTES, StatusReceiver, StatusRequest,
};
use sandproof_internal::{
    CacheReceiver, DataReceiver, LifecycleReceiver, ReceiveError, ValidationReceiver,
};
use sandproof_protocol::internal::{
    ControlIdentity,
    lifecycle::{LifecycleRequest, LifecycleResponse, Resources},
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Observed(Arc<AtomicUsize>);
impl StatusReceiver for Observed {
    async fn status(&self, request: StatusRequest) -> Result<HostStatus, ReceiveError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(HostStatus {
            node: request.node,
            observed_at_ms: 1234,
            free: Resources::ZERO,
        })
    }
}
impl LifecycleReceiver for Observed {
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        RejectLifecycle.receive(request).await
    }
    async fn confirm(&self, result: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        RejectLifecycle.confirm(result).await
    }
    async fn close(&self, request: WindowCloseRequest) -> Result<WindowCloseAck, ReceiveError> {
        RejectLifecycle.close(request).await
    }
}
impl DataReceiver for Observed {
    async fn receive_data(
        &self,
        identity: ControlIdentity,
        request: Request<axum::body::Body>,
    ) -> Response<axum::body::Body> {
        RejectLifecycle.receive_data(identity, request).await
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
