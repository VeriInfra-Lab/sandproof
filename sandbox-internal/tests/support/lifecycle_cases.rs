use super::*;
use sandproof_internal::{LifecycleReceiver, ReceiveError};
use sandproof_protocol::internal::lifecycle::{
    Intent, LIFECYCLE_BODY_BYTES, LIFECYCLE_PATH, LifecycleFact, LifecycleRequest,
    LifecycleResponse,
};
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;

struct Counted(Arc<AtomicUsize>);
impl LifecycleReceiver for Counted {
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(fixture::response(&request, 2, LifecycleFact::Recorded))
    }
}

#[tokio::test]
async fn lifecycle_rejects_untrusted_binding_and_bounded_wire_before_receiver() {
    let certs = certificates();
    let count = Arc::new(AtomicUsize::new(0));
    let server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        fixture::node("host-a"),
        limits(),
        Counted(Arc::clone(&count)),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let good = fixture::request(1, 1, Intent::Create);
    let mut variants = vec![good.clone(); 3];
    variants[0].identity.node.node_generation = 2.try_into().unwrap();
    variants[1].identity.protocol_version += 1;
    variants[2].identity.node.node_id = "other-host".try_into().unwrap();
    for request in variants {
        let (status, body) = raw_request(
            address,
            raw_config(&certs, Some(&certs.center)),
            http_request(
                LIFECYCLE_PATH,
                Bytes::from(serde_json::to_vec(&request).unwrap()),
            ),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body.is_empty());
    }
    let mut unknown = serde_json::to_value(&good).unwrap();
    unknown["host_command"] = "never executed".into();
    let (status, _) = raw_request(
        address,
        raw_config(&certs, Some(&certs.center)),
        http_request(
            LIFECYCLE_PATH,
            Bytes::from(serde_json::to_vec(&unknown).unwrap()),
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = raw_request(
        address,
        raw_config(&certs, Some(&certs.center)),
        http_request(
            LIFECYCLE_PATH,
            Bytes::from(vec![b' '; LIFECYCLE_BODY_BYTES + 1]),
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        raw_request(
            address,
            raw_config(&certs, Some(&certs.other)),
            http_request(
                LIFECYCLE_PATH,
                Bytes::from(serde_json::to_vec(&good).unwrap())
            )
        )
        .await
        .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let tls = ClientTls::new(
        certs.center.material(),
        vec![certs.ca.der().clone()],
        vec![certs.host.pin()],
    )
    .unwrap();
    let client = InternalClient::new(
        tls,
        RegisteredEndpoint::new(address, "host.test".into(), fixture::node("host-a")).unwrap(),
        limits(),
    );
    assert_eq!(
        client.lifecycle(&good).await.unwrap().fact,
        LifecycleFact::Recorded
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
}
