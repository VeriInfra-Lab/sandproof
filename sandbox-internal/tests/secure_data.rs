//! Actual TLS/HTTP stream boundary, not VM or local authorization evidence.
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
};
use http_body_util::BodyExt;
use hyper::body::{Bytes, Frame};
use sandproof_internal::{
    CertificatePin, ClientTls, DataReceiver, InternalClient, InternalServer, LifecycleReceiver,
    Limits, ProtectedHttp, ProtectedServer, ReceiveError, RegisteredEndpoint, ServerTls,
    TransportError,
};
use sandproof_protocol::branching::{
    BranchAccessIdentity, BranchAccessSelector, BranchBatchIdentity,
};
use sandproof_protocol::internal::{
    BRANCH_DATA_SELECTOR_HEADER, DATA_PATH_PREFIX, NodeIdentity, ProbeRequest,
};
use sandproof_protocol::lifecycle::{
    AuthenticatedCenterContext, CenterIdentity, LifecycleAccessIdentity, OperationKey,
};
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
};
#[path = "support/tls.rs"]
mod tls;
use tls::Certificates;
#[cfg(unix)]
#[path = "support/python_relay.rs"]
mod python_relay;
#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod wire;

struct Frames(mpsc::Receiver<Result<Frame<Bytes>, io::Error>>);
impl hyper::body::Body for Frames {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.0.poll_recv(cx)
    }
}
type Sender = mpsc::Sender<Result<Frame<Bytes>, io::Error>>;
struct Streams(Arc<Mutex<Vec<Sender>>>);
impl sandproof_internal::ValidationReceiver for Streams {
    async fn validation_evidence(
        &self,
        _: sandproof_internal::template::ValidationRequest,
        _: AuthenticatedCenterContext,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl LifecycleReceiver for Streams {
    async fn observe_resume(
        &self,
        _: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl DataReceiver for Streams {
    async fn receive_data(
        &self,
        identity: LifecycleAccessIdentity,
        context: AuthenticatedCenterContext,
        request: Request<Body>,
    ) -> Response<Body> {
        assert_eq!(identity, self::identity());
        assert_eq!(context.center(), &center());
        assert_eq!(context.expected_node(), &wire::node("host-a"));
        assert_eq!(request.uri(), "/process.Process/Start?fixture=1");
        for secret in [
            "x-api-key",
            "x-access-token",
            "x-sandproof-control-identity",
            BRANCH_DATA_SELECTOR_HEADER,
        ] {
            assert!(!request.headers().contains_key(secret));
        }
        let (sender, receiver) = mpsc::channel(1);
        self.0.lock().unwrap().push(sender);
        Response::new(Body::new(Frames(receiver)))
    }

    async fn receive_branch_data(
        &self,
        access: BranchAccessIdentity,
        context: AuthenticatedCenterContext,
        request: Request<Body>,
    ) -> Response<Body> {
        assert_eq!(access, branch_identity());
        assert!(access.valid_for(&context));
        assert_eq!(request.uri(), "/process.Process/Start?fixture=1");
        for name in [
            "x-api-key",
            "x-access-token",
            "x-sandproof-control-identity",
            BRANCH_DATA_SELECTOR_HEADER,
        ] {
            assert!(!request.headers().contains_key(name));
        }
        Response::new(Body::from("exact branch route"))
    }
}

struct Fixture {
    client: InternalClient,
    senders: Arc<Mutex<Vec<Sender>>>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), TransportError>>,
    certificates: tls::Certificates,
    address: std::net::SocketAddr,
}
impl Fixture {
    async fn new() -> Self {
        let certs = tls::certificates();
        let limits = Limits::new(3, Duration::from_secs(1), Duration::from_secs(2)).unwrap();
        let senders = Arc::new(Mutex::new(Vec::new()));
        let server = InternalServer::new(
            ServerTls::new(
                certs.host.material(),
                vec![certs.ca.der().clone()],
                vec![certs.center.pin()],
            )
            .unwrap(),
            wire::node("host-a"),
            limits,
            sandproof_internal::diagnostic::RejectTemplateCache::new(Streams(Arc::clone(&senders))),
        )
        .with_center_peer(
            sandproof_internal::lifecycle::CenterPeerRegistration::new(
                center(),
                wire::node("host-a"),
                vec![certs.center.pin()],
            )
            .unwrap(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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
                wire::node("host-a"),
            )
            .unwrap(),
            limits,
        );
        let address = listener.local_addr().unwrap();
        let (stop, shutdown) = oneshot::channel();
        let task = tokio::spawn(async move { server.serve(listener, shutdown).await });
        Self {
            client,
            senders,
            stop,
            task,
            certificates: certs,
            address,
        }
    }
    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap().unwrap();
    }
}
fn data_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/process.Process/Start?fixture=1")
        .header("x-api-key", "never-forward-public-key")
        .header("x-access-token", "never-forward-center-token")
        .header(BRANCH_DATA_SELECTOR_HEADER, "never-forward-caller-selector")
        .header(
            BRANCH_DATA_SELECTOR_HEADER,
            "never-forward-duplicate-selector",
        )
        .header(
            "x-sandproof-control-identity",
            "never-forward-caller-identity",
        )
        .body(Body::empty())
        .unwrap()
}
fn center() -> CenterIdentity {
    CenterIdentity {
        deployment: wire::node("host-a").deployment_id,
        center: "center".try_into().unwrap(),
    }
}
fn identity() -> LifecycleAccessIdentity {
    let node = wire::node("host-a");
    LifecycleAccessIdentity {
        protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
        center: center(),
        operation: OperationKey {
            deployment: node.deployment_id.clone(),
            owner: "tenant-a".try_into().unwrap(),
            window: 1.try_into().unwrap(),
            sequence: 1.try_into().unwrap(),
        },
        node,
        session: "sandbox-1".try_into().unwrap(),
        accepted_version: 1,
        incarnation: 1.try_into().unwrap(),
    }
}
fn branch_identity() -> BranchAccessIdentity {
    let parent = identity();
    let mut operation = parent.operation.clone();
    operation.sequence = 2.try_into().unwrap();
    BranchAccessIdentity {
        selector: BranchAccessSelector {
            batch: BranchBatchIdentity {
                parent: parent.session.clone(),
                incarnation: parent.incarnation,
                checkpoint: "checkpoint".try_into().unwrap(),
                batch: "batch".try_into().unwrap(),
            },
            operation,
            branch: "branch".try_into().unwrap(),
        },
        parent,
    }
}

#[tokio::test]
async fn branch_stream_only_forwards_protected_typed_selector() {
    let f = Fixture::new().await;
    let exact = branch_identity();
    let mut foreign_owner = exact.clone();
    foreign_owner.selector.operation.owner = "tenant-b".try_into().unwrap();
    let mut stale_incarnation = exact.clone();
    stale_incarnation.selector.batch.incarnation = 2.try_into().unwrap();
    for mismatched in [foreign_owner, stale_incarnation] {
        assert!(matches!(
            f.client
                .stream_branch_data(&mismatched, data_request(), Arc::new(()), max_deadline())
                .await,
            Err(TransportError::Rejected)
        ));
    }
    let response = f
        .client
        .stream_branch_data(&exact, data_request(), Arc::new(()), max_deadline())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        Bytes::from_static(b"exact branch route")
    );
    assert!(f.senders.lock().unwrap().is_empty());
    idle_client(&f.client).await;
    f.stop().await;
}

struct ParentOnlyPaths;
impl ProtectedHttp for ParentOnlyPaths {
    async fn respond(
        &self,
        _: CertificatePin,
        request: Request<hyper::body::Incoming>,
        _: tokio::sync::watch::Sender<bool>,
    ) -> Result<Response<Body>, std::convert::Infallible> {
        // Preserve the deployed parent path recognizer from before branching.
        let response = if request.uri().path().starts_with(DATA_PATH_PREFIX) {
            Response::new(Body::from("parent"))
        } else {
            sandproof_data_transport::response(StatusCode::NOT_FOUND)
        };
        Ok(response)
    }
}

#[tokio::test]
async fn legacy_host_with_matching_tls_cannot_treat_branch_stream_as_parent_data() {
    let certs = tls::certificates();
    let limits = Limits::new(3, Duration::from_secs(1), Duration::from_secs(2)).unwrap();
    let server = ProtectedServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        limits,
        ParentOnlyPaths,
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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
            wire::node("host-a"),
        )
        .unwrap(),
        limits,
    );
    let (stop, shutdown) = oneshot::channel();
    let task = tokio::spawn(async move { server.serve(listener, shutdown).await });
    let parent = client
        .stream_data(&identity(), data_request(), Arc::new(()), max_deadline())
        .await
        .unwrap();
    assert_eq!(parent.status(), StatusCode::OK);
    assert_eq!(
        parent.into_body().collect().await.unwrap().to_bytes(),
        "parent"
    );
    let branch = client
        .stream_branch_data(
            &branch_identity(),
            data_request(),
            Arc::new(()),
            max_deadline(),
        )
        .await
        .unwrap();
    assert_eq!(branch.status(), StatusCode::NOT_FOUND);
    assert!(
        branch
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
    idle_client(&client).await;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}

fn max_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_mins(1)
}
async fn idle_client(client: &InternalClient) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while client.active_calls() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

/// The same real stream body stays open after its first frame. The relay must
/// forward that frame before EOF, then release the actual Host connection when
/// the caller cancels. No command/VM or local data authorization is simulated.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_stream_crosses_python_relay_before_eof_and_cancels_backend() {
    let f = Fixture::new().await;
    let mut relay = python_relay::PythonRelay::start_streaming(
        &f.certificates,
        f.address,
        &wire::node("host-a"),
    );
    let client = InternalClient::new(
        ClientTls::new(
            f.certificates.center.material(),
            vec![f.certificates.ca.der().clone()],
            vec![f.certificates.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(relay.address, "host.test".into(), wire::node("host-a")).unwrap(),
        Limits::new(3, Duration::from_secs(1), Duration::from_secs(2)).unwrap(),
    );
    let mut response = client
        .stream_data(&identity(), data_request(), Arc::new(()), max_deadline())
        .await
        .unwrap();
    let sender = f.senders.lock().unwrap()[0].clone();
    sender
        .send(Ok(Frame::data(Bytes::from_static(b"started without EOF"))))
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), response.body_mut().frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    assert_eq!(frame, "started without EOF");
    assert!(!sender.is_closed());
    assert_eq!(client.active_calls(), 1);
    drop(response);
    idle_client(&client).await;
    tokio::time::timeout(Duration::from_secs(2), sender.closed())
        .await
        .unwrap();
    relay.stop();
    f.stop().await;
}

/// Normal completion must survive the relay's TLS close, not just first-frame
/// forwarding and caller cancellation. The original client must observe EOF.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_stream_completes_through_python_relay_without_truncated_http() {
    let f = Fixture::new().await;
    let mut relay = python_relay::PythonRelay::start_streaming(
        &f.certificates,
        f.address,
        &wire::node("host-a"),
    );
    let client = InternalClient::new(
        ClientTls::new(
            f.certificates.center.material(),
            vec![f.certificates.ca.der().clone()],
            vec![f.certificates.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(relay.address, "host.test".into(), wire::node("host-a")).unwrap(),
        Limits::new(3, Duration::from_secs(1), Duration::from_secs(2)).unwrap(),
    );
    let response = client
        .stream_data(&identity(), data_request(), Arc::new(()), max_deadline())
        .await
        .unwrap();
    let sender = f.senders.lock().unwrap().pop().unwrap();
    sender
        .send(Ok(Frame::data(Bytes::from_static(b"started"))))
        .await
        .unwrap();
    sender
        .send(Ok(Frame::data(Bytes::from_static(b"finished"))))
        .await
        .unwrap();
    drop(sender);
    let collected = tokio::time::timeout(Duration::from_secs(2), response.into_body().collect())
        .await
        .expect("original body deadline")
        .expect("complete response must not become an HTTP truncation");
    assert_eq!(collected.to_bytes(), "startedfinished");
    idle_client(&client).await;
    relay.stop();
    f.stop().await;
}

#[tokio::test]
async fn command_stream_survives_management_deadline_and_keeps_bounded_socket_leases() {
    let f = Fixture::new().await;
    let lease = Arc::new(());
    let mut response = f
        .client
        .stream_data(&identity(), data_request(), lease.clone(), max_deadline())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(f.client.active_calls(), 1);
    assert_eq!(Arc::strong_count(&lease), 3); // external, response and actual socket
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::time::resume();
    let sender = f.senders.lock().unwrap()[0].clone();
    sender
        .send(Ok(Frame::data(Bytes::from_static(b"late guest output"))))
        .await
        .unwrap();
    assert_eq!(
        response
            .body_mut()
            .frame()
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap(),
        "late guest output"
    );
    // A long stream does not make management unavailable when a slot is reserved.
    let probe = ProbeRequest {
        protocol_version: sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION,
        node: wire::node("host-a"),
        request_id: "probe-while-streaming".try_into().unwrap(),
    };
    f.client.probe(&probe).await.unwrap();
    let response2 = f
        .client
        .stream_data(&identity(), data_request(), Arc::new(()), max_deadline())
        .await
        .unwrap();
    let response3 = f
        .client
        .stream_data(&identity(), data_request(), Arc::new(()), max_deadline())
        .await
        .unwrap();
    assert!(matches!(
        f.client
            .stream_data(&identity(), data_request(), Arc::new(()), max_deadline())
            .await,
        Err(TransportError::Overloaded)
    ));
    assert_eq!(f.senders.lock().unwrap().len(), 3);
    drop((response, response2, response3));
    idle_client(&f.client).await;
    assert_eq!(Arc::strong_count(&lease), 1);
    f.stop().await;
}

#[tokio::test]
async fn foreign_binding_and_absolute_url_refuse_before_any_socket_or_receiver() {
    let f = Fixture::new().await;
    let mut identity = identity();
    identity.node.node_generation = 2.try_into().unwrap();
    assert!(matches!(
        f.client
            .stream_data(&identity, data_request(), Arc::new(()), max_deadline())
            .await,
        Err(TransportError::Rejected)
    ));
    let mut request = data_request();
    *request.uri_mut() = "http://attacker.invalid/process.Process/Start"
        .parse()
        .unwrap();
    assert!(matches!(
        f.client
            .stream_data(&self::identity(), request, Arc::new(()), max_deadline())
            .await,
        Err(TransportError::Rejected)
    ));
    assert_eq!(f.client.active_calls(), 0);
    assert!(f.senders.lock().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn valid_center_certificate_cannot_select_another_registered_node() {
    let f = Fixture::new().await;
    // Deliberately misconfigured trusted client passes its local node check.
    // The ACTUAL server must reject the full foreign binding after mTLS.
    let client = InternalClient::new(
        ClientTls::new(
            f.certificates.center.material(),
            vec![f.certificates.ca.der().clone()],
            vec![f.certificates.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(f.address, "host.test".into(), wire::node("other-host")).unwrap(),
        Limits::new(2, Duration::from_secs(1), Duration::from_secs(2)).unwrap(),
    );
    let mut identity = identity();
    identity.node = wire::node("other-host");
    let response = client
        .stream_data(&identity, data_request(), Arc::new(()), max_deadline())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    drop(response);
    idle_client(&client).await;
    assert!(f.senders.lock().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn verified_sender_cannot_replace_the_protected_center_or_accepted_version() {
    let f = Fixture::new().await;
    let mut foreign = identity();
    foreign.center.center = "other-center".try_into().unwrap();
    let mut zero = identity();
    zero.accepted_version = 0;
    for identity in [foreign, zero] {
        let response = f
            .client
            .stream_data(&identity, data_request(), Arc::new(()), max_deadline())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        drop(response);
        idle_client(&f.client).await;
        assert!(f.senders.lock().unwrap().is_empty());
    }
    f.stop().await;
}

#[tokio::test]
async fn silent_return_black_hole_fails_at_original_maximum_lifetime_without_eof_success() {
    let f = Fixture::new().await;
    let lease = Arc::new(());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut response = f
        .client
        .stream_data(&identity(), data_request(), lease.clone(), deadline)
        .await
        .unwrap();
    // The remote stream intentionally never sends a frame, EOF or cancellation.
    // No writes are pending, so write-idle cannot detect this return black hole.
    assert_eq!(f.senders.lock().unwrap().len(), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    let frame = response.body_mut().frame().await;
    assert!(
        frame.is_some_and(|frame| frame.is_err()),
        "expiry must be an error, not successful EOF"
    );
    assert!(response.body_mut().frame().await.is_none());
    tokio::time::resume();
    drop(response);
    idle_client(&f.client).await;
    assert_eq!(Arc::strong_count(&lease), 1);
    f.stop().await;
}
