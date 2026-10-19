//! Original production transport bypasses the fixture prebuffer.
use super::*;
use tokio::io::AsyncReadExt;

async fn check_early_rejection(asynchronous: bool, partial: bool, via_gateway: bool) {
    use futures_util::StreamExt;
    let fixture = Arc::new(Fixture::new(&json!({"process":{"cmd":"/bin/true"}})).await);
    let claims = fixture.directory.path().join("pid-claims.bin");
    let before = std::fs::read(&claims).unwrap();
    let control_path = fixture.directory.path().join("sdk-control");
    let control = tokio::net::UnixListener::bind(&control_path).unwrap();
    let (ended, mut endings) = tokio::sync::mpsc::unbounded_channel();
    let message_sent = Arc::new(tokio::sync::Semaphore::new(0));
    let observed = message_sent.clone();
    let gate = tokio::spawn(async move {
        let (mut socket, _) = control.accept().await.unwrap();
        let mut marker = [0];
        socket.read_exact(&mut marker).await.unwrap();
        assert_eq!(&marker, b"m");
        observed.add_permits(1);
        endings.recv().await.expect("response did not reach EOF");
        socket.write_all(b"x").await.unwrap();
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let http = fixture.http.clone();
    let server = tokio::spawn(async move {
        sandproof_guest_service::listener::Listener::new(
            sandproof_guest_service::listener::Limits {
                connections: 2,
                header_count: 32,
                buffer_bytes: LIMIT,
                header_timeout: Duration::from_secs(5),
                connection_timeout: Duration::from_secs(15),
            },
        )
        .unwrap()
        .serve(
            listener,
            move |request| {
                let http = http.clone();
                let ended = ended.clone();
                let message_sent = message_sent.clone();
                async move {
                    // Control only the scheduling: the real handler still owns the
                    // unread request, original admission, authorization and error.
                    message_sent.acquire().await.unwrap().forget();
                    let response = http.handle(request).await.unwrap();
                    assert_eq!(response.status(), 200);
                    let (parts, body) = response.into_parts();
                    let stream = futures_util::stream::unfold(
                        (body.into_data_stream(), ended),
                        |(mut body, ended)| async move {
                            if let Some(chunk) = body.next().await {
                                Some((chunk, (body, ended)))
                            } else {
                                ended.send(()).unwrap();
                                None
                            }
                        },
                    );
                    Ok::<_, std::convert::Infallible>(Response::from_parts(
                        parts,
                        Body::from_stream(stream),
                    ))
                }
            },
            async {
                let _ = stopped.await;
            },
        )
        .await
    });
    let gateway = if via_gateway {
        Some(start_gateway(address).await)
    } else {
        None
    };
    let destination = gateway.as_ref().map_or(address, |(_, address)| *address);
    let status = run_sdk(destination, &control_path, asynchronous, partial).await;
    finish_gateway(gateway).await;
    stop.send(()).unwrap();
    let report = timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(5), gate)
        .await
        .unwrap()
        .unwrap();
    assert_no_effects(&fixture, &before);
    assert_eq!(fixture.lookups.load(Ordering::SeqCst), 0);
    assert_eq!(report.admitted, 1);
    assert_eq!(report.completed, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.expired, 0);
    assert!(
        status.success(),
        "original SDK must decode the original invalid_argument rejection; child status: {status}"
    );
}

async fn finish_gateway(gateway: Option<(tokio::process::Child, std::net::SocketAddr)>) {
    if let Some((mut child, _)) = gateway {
        drop(child.stdin.take());
        assert!(
            timeout(Duration::from_secs(10), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}

fn assert_no_effects(fixture: &Fixture, before: &[u8]) {
    assert_eq!(
        std::fs::read(fixture.directory.path().join("pid-claims.bin")).unwrap(),
        before
    );
    assert!(!fixture.directory.path().join("effect").exists());
    assert_eq!(fixture.registry.available_slots(), 1);
    assert!(fixture.owners.reserve().is_ok());
    assert_eq!(
        std::fs::read_dir(fixture.directory.path().join("gates"))
            .unwrap()
            .count(),
        0
    );
    for pool in [&fixture.requests, &fixture.controls] {
        let slots: Vec<_> = (0..REQUESTS).map(|_| pool.admit().unwrap()).collect();
        drop(slots);
    }
}

async fn run_sdk(
    address: std::net::SocketAddr,
    control_path: &Path,
    asynchronous: bool,
    partial: bool,
) -> std::process::ExitStatus {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sdk = Command::new("uv")
        .current_dir(root)
        .env("PYTHONFAULTHANDLER", "1")
        .args([
            "run",
            "--locked",
            "--project",
            "tests/compat",
            "python",
            "tests/compat/guest_header_rejection_sdk.py",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let settings = json!({"url":format!("http://{address}"),
        "asynchronous":asynchronous,"control":control_path,"partial":partial});
    sdk.stdin
        .take()
        .unwrap()
        .write_all(settings.to_string().as_bytes())
        .await
        .unwrap();
    timeout(Duration::from_secs(15), sdk.wait())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn original_listener_preserves_early_rejection_for_sync_sdk() {
    check_early_rejection(false, false, false).await;
}
#[tokio::test]
async fn original_listener_preserves_early_rejection_for_async_sdk() {
    check_early_rejection(true, false, false).await;
}

async fn start_gateway(
    upstream: std::net::SocketAddr,
) -> (tokio::process::Child, std::net::SocketAddr) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let helper = std::env::var_os("SANDPROOF_GUEST_GATEWAY_HELPER")
        .expect("build the original gateway helper using scripts/check-compat.sh");
    let mut child = Command::new(helper)
        .arg(upstream.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut address = String::new();
    timeout(Duration::from_secs(10), output.read_line(&mut address))
        .await
        .unwrap()
        .unwrap();
    let address: std::net::SocketAddr = address.trim().parse().unwrap();
    assert!(address.ip().is_loopback());
    (child, address)
}

#[tokio::test]
async fn partial_upload_rejection_reaches_sync_sdk() {
    check_early_rejection(false, true, false).await;
}
#[tokio::test]
async fn partial_upload_rejection_reaches_async_sdk() {
    check_early_rejection(true, true, false).await;
}
#[tokio::test]
async fn original_gateway_and_relay_preserve_rejection_for_sync_sdk() {
    check_early_rejection(false, true, true).await;
}
#[tokio::test]
async fn original_gateway_and_relay_preserve_rejection_for_async_sdk() {
    check_early_rejection(true, true, true).await;
}
