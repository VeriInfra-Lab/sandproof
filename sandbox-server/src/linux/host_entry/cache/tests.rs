//! Same artifact transport and original `SQLite` application, not KVM evidence.
use super::*;
#[allow(
    dead_code,
    reason = "original fixture includes lifecycle/recovery test capabilities"
)]
#[path = "../../../../../controller-service/tests/support/internal_application.rs"]
mod local;
#[path = "../../../../../sandbox-internal/tests/support/tls.rs"]
mod tls;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Request, Response, body::Incoming};
use sandproof_internal::{
    CertificatePin, ClientTls, Limits, ProtectedHttp, ProtectedServer, ServerTls,
};
use std::{
    convert::Infallible,
    sync::atomic::{AtomicI64, AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Notify, oneshot, watch},
};

struct Assets {
    kernel: Vec<u8>,
    rootfs: Vec<u8>,
    requests: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    block: bool,
}
impl ProtectedHttp for Assets {
    async fn respond(
        &self,
        _: CertificatePin,
        request: Request<Incoming>,
        _: watch::Sender<bool>,
    ) -> Result<Response<Body>, Infallible> {
        let payload = request.into_body().collect().await.unwrap().to_bytes();
        let value: ArtifactRequest = serde_json::from_slice(&payload).unwrap();
        let index = self.requests.fetch_add(1, Ordering::SeqCst);
        if self.block && index == 0 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        let body = match value.part {
            ArtifactPart::Kernel => self.kernel.clone(),
            ArtifactPart::Rootfs => self.rootfs.clone(),
        };
        Ok(Response::builder()
            .header("content-type", "application/octet-stream")
            .header("content-length", body.len())
            .body(Body::from(body))
            .unwrap())
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    cache: Arc<TemplateCache<local::TestHost>>,
    request: CacheRequest,
    host: Arc<Mutex<local::State>>,
    requests: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    shutdown: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<Result<(), sandproof_internal::TransportError>>,
}
fn assets() -> (Vec<u8>, Vec<u8>, CacheRequest) {
    let kernel = b"trusted-kernel".to_vec();
    let mut rootfs = vec![0; 4096];
    rootfs[1028..1032].copy_from_slice(&4u32.to_le_bytes());
    rootfs[1036..1040].copy_from_slice(&2u32.to_le_bytes());
    rootfs[1080..1082].copy_from_slice(&[0x53, 0xef]);
    let mut descriptor = local::template().registered;
    descriptor.envd_version = "0.9.0".into();
    descriptor.kernel.path = "/center-global-only/kernel".into();
    descriptor.kernel.sha256 = format!("{:x}", Sha256::digest(&kernel));
    descriptor.kernel.size_bytes = kernel.len() as u64;
    descriptor.rootfs.path = "/center-global-only/rootfs".into();
    descriptor.rootfs.sha256 = format!("{:x}", Sha256::digest(&rootfs));
    let request = CacheRequest {
        node: local::wire::node("host-a"),
        revision: "trusted-revision".try_into().unwrap(),
        digest: descriptor.canonical_digest().unwrap(),
        descriptor,
    };
    (kernel, rootfs, request)
}
async fn fixture(block: bool) -> Fixture {
    let directory = tempfile::tempdir_in("/run").unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = directory.path().join("cache");
    for path in [&root, &root.join("inbox"), &root.join("artifacts")] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join(".cache.lock"))
        .unwrap();
    let config = PhysicalTemplateCache {
        namespace: "0123456789abcdef".into(),
        root,
        max_entries: 2,
        max_downloads: 1,
        max_entry_bytes: 8192,
        artifact_bytes: 16384,
        temporary_bytes: 8192,
        metadata_bytes: 4096 + 2 * 2 * (64 * 1024 + 4096),
        inode_budget: 19,
    };
    let host = Arc::new(Mutex::new(local::State::default()));
    let app = local::application_with_cache_config(
        &directory.path().join("db"),
        Arc::clone(&host),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
        sandproof_controller_service::InternalInboxConfig {
            node: local::wire::node("host-a"),
            max_sandboxes: 2,
            operations_per_sandbox: 4,
            max_options_bytes: 1024,
        },
        Some(config.clone()),
    );
    let (kernel, rootfs, request) = assets();
    let certificates = tls::certificates_with_dual_roles();
    let limits = Limits::new(2, Duration::from_secs(2), Duration::from_secs(20)).unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let requests = Arc::new(AtomicUsize::new(0));
    let server = ProtectedServer::new(
        ServerTls::new(
            certificates.center.material(),
            vec![certificates.ca.der().clone()],
            vec![certificates.host.pin()],
        )
        .unwrap(),
        limits,
        Assets {
            kernel,
            rootfs,
            requests: Arc::clone(&requests),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            block,
        },
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ArtifactClient::new(
        ClientTls::new(
            certificates.host.material(),
            vec![certificates.ca.der().clone()],
            vec![certificates.center.pin()],
        )
        .unwrap(),
        listener.local_addr().unwrap(),
        "center.test".into(),
        request.node.clone(),
        limits,
        Duration::from_secs(20),
        Duration::from_secs(20),
    )
    .unwrap();
    let (shutdown, wait) = oneshot::channel();
    let server = tokio::spawn(async move { server.serve(listener, wait).await });
    let cache = Arc::new(
        TemplateCache::open(
            config,
            request.node.clone(),
            client,
            HostLifecycle::new(app),
        )
        .unwrap(),
    );
    Fixture {
        directory,
        cache,
        request,
        host,
        requests,
        entered,
        release,
        shutdown,
        server,
    }
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_cancel_retains_actual_work_budget_then_replay_and_restart_reuse_one_copy() {
    let f = fixture(true).await;
    let cache = Arc::clone(&f.cache);
    let request = f.request.clone();
    let caller = tokio::spawn(async move { cache.receive(request).await });
    f.entered.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(f.cache.downloads.available_permits(), 0);
    assert_eq!(
        Arc::clone(&f.cache).receive(f.request.clone()).await,
        Err(ReceiveError::Capacity)
    );
    f.release.notify_one();
    f.cache.drain().await.unwrap();
    assert!(f.cache.known(&f.request).unwrap().is_some());
    let value = Arc::clone(&f.cache)
        .receive(f.request.clone())
        .await
        .unwrap();
    value.validate_for(&f.request).unwrap();
    assert_eq!(f.requests.load(Ordering::SeqCst), 2);
    let config = f.cache.config.clone();
    let client = f.cache.client.clone();
    let lifecycle = f.cache.lifecycle.clone();
    assert!(
        TemplateCache::open(
            config.clone(),
            f.request.node.clone(),
            client.clone(),
            lifecycle.clone()
        )
        .is_err()
    );
    drop(f.cache);
    let reopened = TemplateCache::open(config, f.request.node.clone(), client, lifecycle).unwrap();
    assert!(reopened.known(&f.request).unwrap().is_some());
    let mut conflict = f.request.clone();
    conflict.descriptor.revision = "r2".into();
    conflict.digest = conflict.descriptor.canonical_digest().unwrap();
    assert_eq!(reopened.known(&conflict), Err(ReceiveError::Conflict));
    assert_eq!(
        fs::read_dir(reopened.config.root.join("artifacts"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        fs::read_dir(reopened.config.root.join("inbox"))
            .unwrap()
            .count(),
        0
    );
    assert!(f.directory.path().join("db").exists());
    f.shutdown.send(()).unwrap();
    f.server.await.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_profile_reserve_is_checked_before_any_download_or_managed_directory() {
    let f = fixture(false).await;
    f.host.lock().unwrap().disk_available = Some(0);
    assert_eq!(
        Arc::clone(&f.cache).receive(f.request.clone()).await,
        Err(ReceiveError::Unavailable)
    );
    assert_eq!(f.requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read_dir(f.cache.config.root.join("inbox"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(f.cache.config.root.join("artifacts"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(f.cache.downloads.available_permits(), 1);
    f.shutdown.send(()).unwrap();
    f.server.await.unwrap().unwrap();
}
