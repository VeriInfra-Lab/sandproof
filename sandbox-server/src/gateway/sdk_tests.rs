//! Pinned SDK -> real gateway body handling / HTTP -> synthetic envd.
//! No management authorization, relay subprocess, namespace or VM is tested.

use super::*;
use axum::Router;
use std::{net::SocketAddr, path::Path, process::Stdio, sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

#[tokio::test]
async fn pinned_sdk_through_gateway() {
    timeout(Duration::from_mins(2), run_contract())
        .await
        .expect("SDK/gateway contract timed out");
}

async fn run_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sdk = Command::new("uv")
        .current_dir(root)
        .args([
            "run",
            "--locked",
            "--project",
            "tests/compat",
            "python",
            "tests/compat/gateway_sdk.py",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("check-compat.sh requires uv and the locked SDK");
    let mut startup = BufReader::new(sdk.stdout.take().unwrap());
    let mut address = String::new();
    startup.read_line(&mut address).await.unwrap();
    let envd: SocketAddr = address.trim().parse().expect("synthetic envd address");
    assert!(envd.ip().is_loopback());

    let drivers = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::clone(&drivers);
    let app = Router::new().fallback(move |request: Request<Body>| {
        let drivers = Arc::clone(&requests);
        async move {
            // Deliberately below handle's authorization/namespace boundary.
            // The production classification, buffering and forwarding run here.
            let Some((_, file_request)) = operation(request.method(), request.uri().path()) else {
                return response(StatusCode::NOT_FOUND);
            };
            let request = match prepare_body(request, file_request).await {
                Ok(request) => request,
                Err(rejection) => return rejection,
            };
            let stream = TcpStream::connect(envd).await.unwrap();
            let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await.unwrap();
            drivers.lock().unwrap().push(tokio::spawn(connection));
            forward_request(&mut sender, request).await
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}\n", listener.local_addr().unwrap());
    let (shutdown, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let mut input = sdk.stdin.take().unwrap();
    input.write_all(url.as_bytes()).await.unwrap();
    drop(input);
    let status = sdk.wait().await.unwrap();
    let _ = shutdown.send(());
    timeout(Duration::from_secs(5), server)
        .await
        .expect("gateway connections did not close")
        .unwrap()
        .unwrap();
    let drivers = std::mem::take(&mut *drivers.lock().unwrap());
    for driver in drivers {
        // Truncated streams may error; panics and leaked drivers must fail.
        let _ = timeout(Duration::from_secs(5), driver)
            .await
            .expect("upstream connection did not close")
            .unwrap();
    }
    assert!(
        status.success(),
        "pinned SDK through gateway failed: {status}"
    );
}
