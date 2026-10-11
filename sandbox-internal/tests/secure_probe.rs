use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode, body::Bytes, client::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer,
    KeyPair, KeyUsagePurpose,
};
use rustls::{ClientConfig, RootCertStore, ServerConfig, pki_types::PrivatePkcs8KeyDer};
use sandproof_internal::{
    CertificatePin, ClientTls, Limits, ProbeClient, ProbeServer, RegisteredEndpoint, ServerTls,
    TlsIdentity, TransportError,
};
use sandproof_protocol::{
    DeploymentId, NodeId, OperationId,
    internal::{
        INTERNAL_PROTOCOL_VERSION, NodeIdentity, PositiveCounter, ProbeRequest, ProbeResponse,
    },
};
use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, TlsConnector, client::TlsStream};

struct Identity {
    certificate: Certificate,
    key: KeyPair,
}

impl Identity {
    fn material(&self) -> TlsIdentity {
        TlsIdentity {
            chain: vec![self.certificate.der().clone()],
            private_key: PrivatePkcs8KeyDer::from(self.key.serialize_der()).into(),
        }
    }

    fn pin(&self) -> CertificatePin {
        CertificatePin::from_certificate(self.certificate.der())
    }
}

struct Certificates {
    ca: Certificate,
    center: Identity,
    host: Identity,
    other: Identity,
}

fn certificates() -> Certificates {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = KeyPair::generate().unwrap();
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(params, ca_key);
    Certificates {
        ca,
        center: leaf(&issuer, "center.test", ExtendedKeyUsagePurpose::ClientAuth),
        host: leaf(&issuer, "host.test", ExtendedKeyUsagePurpose::ServerAuth),
        other: leaf(&issuer, "other.test", ExtendedKeyUsagePurpose::ClientAuth),
    }
}

fn leaf(issuer: &Issuer<'_, KeyPair>, name: &str, purpose: ExtendedKeyUsagePurpose) -> Identity {
    let mut params = CertificateParams::new(vec![name.to_owned()]).unwrap();
    params.extended_key_usages = vec![purpose];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let key = KeyPair::generate().unwrap();
    Identity {
        certificate: params.signed_by(&key, issuer).unwrap(),
        key,
    }
}

fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: DeploymentId::try_from("deployment-a").unwrap(),
        node_id: NodeId::try_from("node-a").unwrap(),
        node_generation: PositiveCounter::try_from(1).unwrap(),
    }
}

fn request() -> ProbeRequest {
    ProbeRequest {
        protocol_version: INTERNAL_PROTOCOL_VERSION,
        node: node(),
        request_id: OperationId::try_from("probe-a").unwrap(),
    }
}

fn limits() -> Limits {
    Limits::new(4, Duration::from_secs(5), Duration::from_secs(10)).unwrap()
}

struct Running {
    server: ProbeServer,
    address: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), TransportError>>,
}

impl Running {
    async fn stop(self) {
        self.shutdown.send(()).unwrap();
        self.task.await.unwrap().unwrap();
        assert_eq!(self.server.active_connections(), 0);
    }
}

async fn start(certs: &Certificates, limits: Limits) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    start_listener(certs, limits, listener)
}

fn start_listener(certs: &Certificates, limits: Limits, listener: TcpListener) -> Running {
    let tls = ServerTls::new(
        certs.host.material(),
        vec![certs.ca.der().clone()],
        vec![certs.center.pin()],
    )
    .unwrap();
    let server = ProbeServer::new(tls, node(), limits);
    let address = listener.local_addr().unwrap();
    let (shutdown, receiver) = oneshot::channel();
    let run_server = server.clone();
    let task = tokio::spawn(async move { run_server.serve(listener, receiver).await });
    Running {
        server,
        address,
        shutdown,
        task,
    }
}

fn client(certs: &Certificates, address: SocketAddr) -> ProbeClient {
    let tls = ClientTls::new(
        certs.center.material(),
        vec![certs.ca.der().clone()],
        vec![certs.host.pin()],
    )
    .unwrap();
    ProbeClient::new(
        tls,
        RegisteredEndpoint::new(address, "host.test".into(), node()).unwrap(),
        limits(),
    )
}

// Raw trusted TLS clients are only test inputs for the real receiving boundary.
fn raw_config(certs: &Certificates, identity: Option<&Identity>) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(certs.ca.der().clone()).unwrap();
    let builder =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots);
    let mut config = if let Some(identity) = identity {
        let material = identity.material();
        builder
            .with_client_auth_cert(material.chain, material.private_key)
            .unwrap()
    } else {
        builder.with_no_client_auth()
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

async fn raw_stream(
    address: SocketAddr,
    config: Arc<ClientConfig>,
) -> Result<TlsStream<TcpStream>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(TlsConnector::from(config)
        .connect(
            "host.test".try_into().unwrap(),
            TcpStream::connect(address).await?,
        )
        .await?)
}

async fn raw_request(
    address: SocketAddr,
    config: Arc<ClientConfig>,
    request: Request<Full<Bytes>>,
) -> Result<(StatusCode, Bytes), Box<dyn std::error::Error + Send + Sync>> {
    let stream = raw_stream(address, config).await?;
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await?;
    let driver = tokio::spawn(connection);
    let result = async {
        let response = sender.send_request(request).await?;
        let status = response.status();
        let body = response.into_body().collect().await?.to_bytes();
        Ok((status, body))
    }
    .await;
    driver.abort();
    let _ = driver.await;
    result
}

fn http_request(path: &str, body: Bytes) -> Request<Full<Bytes>> {
    Request::post(path)
        .header("host", "host.test")
        .header("content-type", "application/json")
        .header("connection", "close")
        .body(Full::new(body))
        .unwrap()
}

// A real authenticated peer emitting adversarial HTTP; not a lifecycle model.
fn peer_acceptor(certs: &Certificates) -> TlsAcceptor {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    roots.add(certs.ca.der().clone()).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::clone(&provider),
    )
    .build()
    .unwrap();
    let material = certs.host.material();
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(material.chain, material.private_key)
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsAcceptor::from(Arc::new(config))
}

fn peer_response(body: Bytes) -> Response<Full<Bytes>> {
    Response::builder()
        .header("content-type", "application/json")
        .header("connection", "close")
        .body(Full::new(body))
        .unwrap()
}

async fn start_response_peer(
    certs: &Certificates,
    response: Response<Full<Bytes>>,
) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let acceptor = peer_acceptor(certs);
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let stream = acceptor.accept(socket).await.unwrap();
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let response = response.clone();
            async move {
                request.into_body().collect().await.unwrap();
                Ok::<_, Infallible>(response)
            }
        });
        // The client may reject and close before the response finishes.
        let _ = hyper::server::conn::http1::Builder::new()
            .keep_alive(false)
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
    (address, task)
}

#[tokio::test]
async fn real_mutual_tls_probe_is_repeatable_without_claiming_lifecycle_success() {
    let certs = certificates();
    let running = start(&certs, limits()).await;
    let client = client(&certs, running.address);
    for _ in 0..8 {
        let response = client.probe(&request()).await.unwrap();
        response.validate_for(&request()).unwrap();
        assert_eq!(client.active_calls(), 0);
    }
    running.stop().await;
}

#[tokio::test]
async fn absent_foreign_and_wrong_role_client_certificates_cannot_reach_handler() {
    let certs = certificates();
    let foreign = certificates();
    let running = start(&certs, limits()).await;
    for identity in [
        None,
        Some(&certs.other),
        Some(&foreign.center),
        Some(&certs.host),
    ] {
        let encoded = Bytes::from(serde_json::to_vec(&request()).unwrap());
        assert!(
            raw_request(
                running.address,
                raw_config(&certs, identity),
                http_request("/internal/v1/probe", encoded),
            )
            .await
            .is_err()
        );
    }
    running.stop().await;
}

#[tokio::test]
async fn client_requires_server_root_name_and_registered_pin() {
    let certs = certificates();
    let foreign = certificates();
    let running = start(&certs, limits()).await;
    for (root, pin, name) in [
        (foreign.ca.der().clone(), certs.host.pin(), "host.test"),
        (certs.ca.der().clone(), certs.other.pin(), "host.test"),
        (certs.ca.der().clone(), certs.host.pin(), "other.test"),
    ] {
        let tls = ClientTls::new(certs.center.material(), vec![root], vec![pin]).unwrap();
        let client = ProbeClient::new(
            tls,
            RegisteredEndpoint::new(running.address, name.into(), node()).unwrap(),
            limits(),
        );
        assert!(client.probe(&request()).await.is_err());
        assert_eq!(client.active_calls(), 0);
    }
    running.stop().await;
}

#[tokio::test]
async fn receiving_boundary_rejects_old_generation_wrong_binding_and_version() {
    let certs = certificates();
    let running = start(&certs, limits()).await;
    let mut variants = vec![request(); 4];
    variants[0].node.node_generation = PositiveCounter::try_from(2).unwrap();
    variants[1].node.node_id = NodeId::try_from("node-b").unwrap();
    variants[2].node.deployment_id = DeploymentId::try_from("deployment-b").unwrap();
    variants[3].protocol_version += 1;
    for request in variants {
        let (status, body) = raw_request(
            running.address,
            raw_config(&certs, Some(&certs.center)),
            http_request(
                "/internal/v1/probe",
                Bytes::from(serde_json::to_vec(&request).unwrap()),
            ),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body.is_empty());
    }
    assert!(
        client(&certs, running.address)
            .probe(&request())
            .await
            .is_ok()
    );
    running.stop().await;
}

#[tokio::test]
async fn only_bounded_read_only_json_probe_is_exposed() {
    let certs = certificates();
    let running = start(&certs, limits()).await;
    let body = Bytes::from(serde_json::to_vec(&request()).unwrap());
    let mut cases = vec![
        (
            http_request("/internal/v1/execute", body.clone()),
            StatusCode::NOT_FOUND,
        ),
        (
            http_request("/internal/v1/probe?url=private", body.clone()),
            StatusCode::NOT_FOUND,
        ),
        (
            http_request(
                "/internal/v1/probe",
                Bytes::from_static(b"private-invalid-input"),
            ),
            StatusCode::BAD_REQUEST,
        ),
        (
            http_request("/internal/v1/probe", Bytes::from(vec![b' '; 4097])),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ];
    let mut get = http_request("/internal/v1/probe", body.clone());
    *get.method_mut() = hyper::Method::GET;
    cases.push((get, StatusCode::METHOD_NOT_ALLOWED));
    let mut compressed = http_request("/internal/v1/probe", body);
    compressed
        .headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    cases.push((compressed, StatusCode::UNSUPPORTED_MEDIA_TYPE));
    let mut unknown = serde_json::to_value(request()).unwrap();
    unknown["url"] = "private-untrusted-target".into();
    cases.push((
        http_request(
            "/internal/v1/probe",
            Bytes::from(serde_json::to_vec(&unknown).unwrap()),
        ),
        StatusCode::BAD_REQUEST,
    ));
    for (request, expected) in cases {
        let (status, response) = raw_request(
            running.address,
            raw_config(&certs, Some(&certs.center)),
            request,
        )
        .await
        .unwrap();
        assert_eq!(status, expected);
        assert!(!String::from_utf8_lossy(&response).contains("private"));
    }
    running.stop().await;
}

#[tokio::test]
async fn untrusted_peer_responses_are_rejected_and_release_the_full_call_budget() {
    let certs = certificates();
    let good = ProbeResponse::for_request(&request(), &node()).unwrap();
    let mut old = good.clone();
    old.node.node_generation = PositiveCounter::try_from(2).unwrap();
    let mut foreign = good.clone();
    foreign.request_id = OperationId::try_from("probe-b").unwrap();
    let encoded = Bytes::from(serde_json::to_vec(&good).unwrap());
    let mut oversized_body = encoded.to_vec();
    oversized_body.resize(4097, b' ');
    let mut too_many_headers = peer_response(encoded.clone());
    for index in 0..17 {
        too_many_headers.headers_mut().insert(
            hyper::header::HeaderName::from_bytes(format!("x-test-{index}").as_bytes()).unwrap(),
            "a".parse().unwrap(),
        );
    }
    let mut too_large_header = peer_response(encoded.clone());
    too_large_header
        .headers_mut()
        .insert("x-large", "a".repeat(8193).parse().unwrap());
    let mut wrong_status = peer_response(encoded.clone());
    *wrong_status.status_mut() = StatusCode::CREATED;
    let mut compressed = peer_response(encoded.clone());
    compressed
        .headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    let mut wrong_type = peer_response(encoded);
    wrong_type
        .headers_mut()
        .insert("content-type", "text/plain".parse().unwrap());
    for response in [
        peer_response(Bytes::from(serde_json::to_vec(&old).unwrap())),
        peer_response(Bytes::from(serde_json::to_vec(&foreign).unwrap())),
        peer_response(Bytes::from_static(b"not-json")),
        // Still valid JSON: removing Limited would make this response succeed.
        peer_response(Bytes::from(oversized_body)),
        too_many_headers,
        too_large_header,
        wrong_status,
        compressed,
        wrong_type,
    ] {
        let (address, task) = start_response_peer(&certs, response).await;
        let client = client(&certs, address);
        assert!(client.probe(&request()).await.is_err());
        assert_eq!(client.active_calls(), 0);
        timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn chunked_request_without_length_cannot_bypass_the_body_budget() {
    let certs = certificates();
    let running = start(&certs, limits()).await;
    let mut stream = raw_stream(running.address, raw_config(&certs, Some(&certs.center)))
        .await
        .unwrap();
    let mut oversized_body = serde_json::to_vec(&request()).unwrap();
    oversized_body.resize(4097, b' ');
    stream.write_all(b"POST /internal/v1/probe HTTP/1.1\r\nHost: host.test\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1001\r\n").await.unwrap();
    // Still valid JSON: removing Limited would publish a successful probe.
    stream.write_all(&oversized_body).await.unwrap();
    stream.write_all(b"\r\n0\r\n\r\n").await.unwrap();
    stream.flush().await.unwrap();
    let mut response = Vec::new();
    timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 400"));
    running.stop().await;
}

#[tokio::test]
async fn caller_cancellation_closes_original_tls_connection_before_reusing_slot() {
    let certs = certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let probe_client = client(&certs, address);
    let acceptor = peer_acceptor(&certs);
    let (observed, ready) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut stream = acceptor.accept(socket).await.unwrap();
        // Consume the exact bounded request, then signal without responding.
        let mut input = Vec::new();
        let end = loop {
            let mut chunk = [0; 1024];
            let length = stream.read(&mut chunk).await.unwrap();
            assert!(length > 0);
            input.extend_from_slice(&chunk[..length]);
            assert!(input.len() <= 8192);
            if let Some(index) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let length = std::str::from_utf8(&input[..end])
            .unwrap()
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
            })
            .unwrap();
        assert!(length <= 4096);
        while input.len() < end + length {
            let mut chunk = [0; 1024];
            let length = stream.read(&mut chunk).await.unwrap();
            assert!(length > 0);
            input.extend_from_slice(&chunk[..length]);
        }
        observed.send(()).unwrap();
        let mut chunk = [0; 1];
        match stream.read(&mut chunk).await {
            Ok(0) => {}
            // Cancellation drops, rather than gracefully TLS-shuts down, the
            // original socket. Kernels may report FIN, truncated TLS or RST.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                ) => {}
            result => panic!("cancelled connection was not closed: {result:?}"),
        }
    });
    let caller = probe_client.clone();
    let task = tokio::spawn(async move { caller.probe(&request()).await });
    timeout(Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(probe_client.active_calls(), 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(probe_client.active_calls(), 0);
    timeout(Duration::from_secs(5), peer)
        .await
        .unwrap()
        .unwrap();
    let healthy = start_listener(&certs, limits(), TcpListener::bind(address).await.unwrap());
    assert!(probe_client.probe(&request()).await.is_ok());
    healthy.stop().await;
}

async fn wait_for_active(server: &ProbeServer, count: usize) {
    timeout(Duration::from_secs(5), async {
        while server.active_connections() != count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn connections_are_bounded_before_handshake_and_shutdown_releases_original_slots() {
    let certs = certificates();
    let running = start(
        &certs,
        Limits::new(1, Duration::from_secs(30), Duration::from_mins(1)).unwrap(),
    )
    .await;
    let _first = TcpStream::connect(running.address).await.unwrap();
    wait_for_active(&running.server, 1).await;
    let mut rejected = TcpStream::connect(running.address).await.unwrap();
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_secs(5), rejected.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(running.server.active_connections(), 1);
    running.stop().await;
}

#[tokio::test]
async fn handshake_and_partial_http_deadlines_release_slots_without_sleeps() {
    let certs = certificates();
    let running = start(
        &certs,
        Limits::new(1, Duration::from_secs(30), Duration::from_mins(1)).unwrap(),
    )
    .await;
    let _plain = TcpStream::connect(running.address).await.unwrap();
    wait_for_active(&running.server, 1).await;
    // Allow the accepted task to register its timer before advancing time.
    tokio::task::yield_now().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    wait_for_active(&running.server, 0).await;
    let mut stream = raw_stream(running.address, raw_config(&certs, Some(&certs.center)))
        .await
        .unwrap();
    stream.write_all(b"POST /internal/v1/probe HTTP/1.1\r\nHost: host.test\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{").await.unwrap();
    stream.flush().await.unwrap();
    wait_for_active(&running.server, 1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    wait_for_active(&running.server, 0).await;
    running.stop().await;
}

#[tokio::test]
async fn client_budget_has_no_wait_queue_and_no_detached_driver_on_deadline() {
    let certs = certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tls = ClientTls::new(
        certs.center.material(),
        vec![certs.ca.der().clone()],
        vec![certs.host.pin()],
    )
    .unwrap();
    let client = ProbeClient::new(
        tls,
        RegisteredEndpoint::new(listener.local_addr().unwrap(), "host.test".into(), node())
            .unwrap(),
        Limits::new(1, Duration::from_secs(30), Duration::from_mins(1)).unwrap(),
    );
    let first_client = client.clone();
    let first = tokio::spawn(async move { first_client.probe(&request()).await });
    let (_socket, _) = listener.accept().await.unwrap();
    assert_eq!(client.active_calls(), 1);
    assert_eq!(
        client.probe(&request()).await,
        Err(TransportError::Overloaded)
    );
    tokio::task::yield_now().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    assert_eq!(first.await.unwrap(), Err(TransportError::Deadline));
    tokio::time::resume();
    assert_eq!(client.active_calls(), 0);
}

#[test]
fn invalid_trust_pins_addresses_and_budgets_are_rejected_before_io() {
    let certs = certificates();
    for (count, handshake, request) in
        [(0, 1, 2), (8193, 1, 2), (1, 0, 2), (1, 3, 2), (1, 1, 86401)]
    {
        assert!(
            Limits::new(
                count,
                Duration::from_secs(handshake),
                Duration::from_secs(request)
            )
            .is_err()
        );
    }
    assert!(ServerTls::new(certs.host.material(), vec![], vec![certs.center.pin()]).is_err());
    assert!(
        ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![]
        )
        .is_err()
    );
    assert!(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin(); 2]
        )
        .is_err()
    );
    for (address, name) in [
        ("0.0.0.0:1", "host.test"),
        ("127.0.0.1:0", "host.test"),
        ("224.0.0.1:1", "host.test"),
        ("127.0.0.1:1", "127.0.0.1"),
        ("127.0.0.1:1", "https://host.test"),
    ] {
        assert!(RegisteredEndpoint::new(address.parse().unwrap(), name.into(), node()).is_err());
    }
}
