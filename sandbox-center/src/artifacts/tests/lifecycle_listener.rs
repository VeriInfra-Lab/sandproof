//! Real bind + original Redis request-window, not a VM or isolated-router test.
use super::*;
use sandproof_api::{
    ApiCredential,
    admission::{RequestAdmission, RequestAdmissionLimits, RequestClass},
};
use sandproof_center_service::{CenterApplication, CenterConfig, business::BusinessTemplates};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SECRET: &str = "lifecycle-listener-fixture-api-key-0001";

async fn bound(fixture: &engine::Engine) -> (crate::boundaries::Boundaries, Arc<RequestAdmission>) {
    let (store, binding) = store(fixture);
    let authority = authority(fixture, &store, &binding);
    let certificates = tls::certificates_with_dual_roles();
    let identity = node();
    let client = InternalClient::new(
        ClientTls::new(
            certificates.center.material(),
            vec![certificates.ca.der().clone()],
            vec![certificates.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(
            "127.0.0.1:9".parse().unwrap(),
            "host.test".into(),
            identity.clone(),
        )
        .unwrap(),
        Limits::new(2, Duration::from_secs(1), Duration::from_secs(3)).unwrap(),
    );
    let hosts =
        RegisteredHosts::new(vec![RegisteredHost { identity, client }], Handle::current()).unwrap();
    let candidates = ProtectedCandidates::new(authority.clone(), hosts.clone(), store);
    let templates = BusinessTemplates::new(authority.clone(), candidates, hosts.clone()).unwrap();
    let app = Arc::new(
        CenterApplication::new(
            authority,
            hosts,
            templates,
            CenterConfig {
                domain: "example.invalid".into(),
                max_timeout_secs: 300,
                max_lifetime_secs: 300,
                startup_timeout_secs: 30,
                reply_timeout: Duration::from_secs(3),
                poll_interval: Duration::from_millis(10),
                public_ipv4_enabled: false,
            },
            &[42; 32],
        )
        .unwrap(),
    );
    let admission = RequestAdmission::new(RequestAdmissionLimits {
        max_pending_requests: 2,
        control_reserve: 1,
    })
    .unwrap();
    let limits = sandproof_data_transport::TransferLimits {
        file_upload_bytes: 1024,
        file_download_bytes: 1024,
        command_input_bytes: 1024,
        rpc_request_bytes: 1024,
        rpc_response_bytes: 1024,
        max_transfers: 2,
        max_transfers_per_vm: 2,
        control_reserve: 1,
        request_idle_seconds: 1,
        file_idle_seconds: 1,
        write_idle_seconds: 1,
    };
    let boundaries = crate::boundaries::bind(
        app,
        "127.0.0.1:0".parse().unwrap(),
        vec![ApiCredential {
            tenant: "1".into(),
            secret: SECRET.into(),
        }],
        None,
        Arc::clone(&admission),
        limits,
    )
    .await
    .unwrap();
    (boundaries, admission)
}

async fn get(address: std::net::SocketAddr, authenticated: bool) -> (u16, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let auth = if authenticated { format!("X-API-Key: {SECRET}\r\n") } else { String::new() };
        let request = format!("GET /sandproof/lifecycle/request-window HTTP/1.1\r\nHost: localhost\r\n{auth}Connection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        stream.take(8193).read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.len() <= 8192);
        let split = bytes.windows(4).position(|part| part == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&bytes[..split]).unwrap();
        let status = headers.split_whitespace().nth(1).unwrap().parse::<u16>().unwrap();
        (status, bytes[split + 4..].to_vec())
    }).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn formal_public_listener_mounts_original_lifecycle_and_shared_admission() {
    let fixture = engine::Engine::new();
    let (boundaries, admission) = bound(&fixture).await;
    assert!(boundaries.private.is_none());
    let address = boundaries.public.socket.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(async move {
        axum::serve(boundaries.public.socket, boundaries.public.router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    assert_eq!(get(address, false).await.0, 401);
    let (status, body) = get(address, true).await;
    assert_eq!(status, 200);
    // Nine comes from the existing original Redis registration in authority(),
    // not from a handler stub or a guessed new window.
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"window": 9})
    );
    let held = admission.try_acquire(RequestClass::Ordinary).unwrap();
    assert_eq!(get(address, true).await.0, 429);
    assert_eq!(get(address, false).await.0, 401);
    drop(held);
    assert_eq!(get(address, true).await.0, 200);
    stop.send(()).unwrap();
    serving.await.unwrap();
}
