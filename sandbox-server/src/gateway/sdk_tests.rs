//! Pinned SDK -> real gateway body handling / HTTP -> synthetic envd.
//! No management authorization, relay subprocess, namespace or VM is tested.

use super::*;
use axum::Router;
use std::{net::SocketAddr, path::Path, process::Stdio, sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

#[tokio::test]
async fn pinned_sdk_through_gateway() {
    timeout(Duration::from_mins(2), run_contract(None))
        .await
        .expect("SDK/gateway contract timed out");
}

#[tokio::test]
async fn pinned_sdk_through_streaming_gateway() {
    timeout(
        Duration::from_mins(2),
        run_contract(Some(TransferLimits::fixture())),
    )
    .await
    .expect("streaming SDK/gateway contract timed out");
}

async fn run_contract(limits: Option<TransferLimits>) {
    let before_rss = high_water_rss_kib();
    let streaming = limits.is_some();
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
    let settings = serde_json::json!({
        "streaming": limits.is_some(),
        "upload_bytes": limits.as_ref().map_or(FILE_REQUEST_LIMIT, |limits| limits.file_upload_bytes),
        "download_bytes": limits.as_ref().map_or(RESPONSE_LIMIT, |limits| limits.file_download_bytes),
    });
    let write_idle = limits
        .as_ref()
        .map(|limits| Duration::from_secs(limits.write_idle_seconds));
    let gateway = limits
        .clone()
        .map(Gateway::new)
        .transpose()
        .unwrap()
        .map(Arc::new);
    let app = Router::new().fallback(move |request: Request<Body>| {
        forward_fixture(
            request,
            envd,
            gateway.clone(),
            Arc::clone(&requests),
            write_idle,
        )
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut settings = settings;
    settings["gateway_url"] = format!("http://{}", listener.local_addr().unwrap()).into();
    let settings = format!("{settings}\n");
    let (shutdown, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(GatewayListener::new(listener, write_idle), app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let mut input = sdk.stdin.take().unwrap();
    input.write_all(settings.as_bytes()).await.unwrap();
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
    eprintln!(
        "gateway RSS high-water (KiB): profile={}, before={before_rss}, after={}",
        if streaming { "streaming" } else { "legacy" },
        high_water_rss_kib()
    );
    assert!(
        status.success(),
        "pinned SDK through gateway failed: {status}"
    );
}

type Drivers = Arc<Mutex<Vec<tokio::task::JoinHandle<Result<(), hyper::Error>>>>>;

async fn forward_fixture(
    request: Request<Body>,
    envd: SocketAddr,
    gateway: Option<Arc<Gateway>>,
    drivers: Drivers,
    write_idle: Option<Duration>,
) -> Response<Body> {
    // Only the authorization/namespace boundary is replaced by fixture identity.
    let Some((_, file_request)) = operation(request.method(), request.uri().path()) else {
        return response(StatusCode::NOT_FOUND);
    };
    let lease = if let Some(gateway) = &gateway {
        match gateway.acquire((1, 1), control_request(&request)) {
            Ok(lease) => Some(lease),
            Err(status) => return response(status),
        }
    } else {
        None
    };
    let limits = gateway.as_ref().map(|gateway| &gateway.limits);
    let request = match prepare_body(request, file_request, limits).await {
        Ok(request) => request,
        Err(rejection) => return rejection,
    };
    let stream = TcpStream::connect(envd).await.unwrap();
    let mut builder = http1::Builder::new();
    if limits.is_some() {
        builder.max_buf_size(64 * 1024);
    }
    let (mut sender, connection) = builder
        .handshake(TokioIo::new(WriteIdle::new(stream, write_idle)))
        .await
        .unwrap();
    let connection_lease = lease.clone();
    drivers.lock().unwrap().push(tokio::spawn(async move {
        let result = connection.await;
        drop(connection_lease);
        result
    }));
    let response = forward_request(&mut sender, request, limits).await;
    match lease {
        Some(lease) => LeasedBody::wrap(response, lease),
        None => response,
    }
}

fn high_water_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}
