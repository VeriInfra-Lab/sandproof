//! Actual TLS/HTTP stream boundary, not VM or local authorization evidence.
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
};
use http_body_util::BodyExt;
use hyper::body::{Bytes, Frame};
use sandproof_internal::{
    ClientTls, DataReceiver, InternalClient, InternalServer, LifecycleReceiver, Limits,
    ReceiveError, RegisteredEndpoint, ServerTls, TransportError,
};
use sandproof_protocol::internal::{
    ControlIdentity, ProbeRequest,
    lifecycle::{LifecycleRequest, LifecycleResponse},
};
use std::{
    io,
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
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl LifecycleReceiver for Streams {
    async fn receive(&self, _: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
    async fn confirm(&self, _: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
    async fn close(
        &self,
        _: sandproof_protocol::internal::window::WindowCloseRequest,
    ) -> Result<sandproof_protocol::internal::window::WindowCloseAck, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl DataReceiver for Streams {
    async fn receive_data(
        &self,
        identity: ControlIdentity,
        request: Request<Body>,
    ) -> Response<Body> {
        assert_eq!(
            identity,
            wire::request(
                1,
                1,
                sandproof_protocol::internal::lifecycle::Intent::Create
            )
            .identity
        );
        assert_eq!(request.uri(), "/process.Process/Start?fixture=1");
        for secret in [
            "x-api-key",
            "x-access-token",
            "x-sandproof-control-identity",
        ] {
            assert!(!request.headers().contains_key(secret));
        }
        let (sender, receiver) = mpsc::channel(1);
        self.0.lock().unwrap().push(sender);
        Response::new(Body::new(Frames(receiver)))
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
        .header(
            "x-sandproof-control-identity",
            "never-forward-caller-identity",
        )
        .body(Body::empty())
        .unwrap()
}
fn identity() -> ControlIdentity {
    wire::request(
        1,
        1,
        sandproof_protocol::internal::lifecycle::Intent::Create,
    )
    .identity
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
