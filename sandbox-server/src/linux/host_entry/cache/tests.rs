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
use sandproof_host_service::DriverConfig;
use sandproof_internal::{
    CertificatePin, ClientTls, LifecycleReceiver, Limits, ProtectedHttp, ProtectedServer, ServerTls,
};
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact, LifecycleRequest};
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
    clock: Arc<local::TestClock>,
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
fn cache_config(root: std::path::PathBuf) -> PhysicalTemplateCache {
    PhysicalTemplateCache {
        namespace: "0123456789abcdef".into(),
        root,
        max_entries: 2,
        max_downloads: 1,
        max_entry_bytes: 8192,
        artifact_bytes: 16384,
        temporary_bytes: 8192,
        metadata_bytes: 4096 + 2 * 2 * (64 * 1024 + 4096),
        inode_budget: 19,
    }
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
    let config = cache_config(root);
    let host = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application_with_cache_config(
        &directory.path().join("db"),
        Arc::clone(&host),
        Arc::clone(&clock),
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
        clock,
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

fn partial_file(directory: &Path, name: &str, contents: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(name))
        .unwrap();
    file.write_all(contents).unwrap();
    file.sync_all().unwrap();
    File::open(directory).unwrap().sync_all().unwrap();
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_restart_reclaims_durable_empty_and_partial_downloads_without_changing_identity() {
    for (kernel_length, rootfs_length) in [
        (None, None),
        (Some(0), None),
        (None, Some(0)),
        (Some(4), None),
        (None, Some(128)),
        (Some(13), Some(128)),
    ] {
        let mut f = fixture(false).await;
        let (kernel, rootfs, _) = assets();
        partial_file(f.directory.path(), "base-kernel", &kernel);
        partial_file(f.directory.path(), "base-rootfs", &rootfs);
        let original_kernel = f.directory.path().join("base-kernel");
        let original_rootfs = f.directory.path().join("base-rootfs");
        for path in [&original_kernel, &original_rootfs] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
        }
        f.request.descriptor.kernel.path = original_kernel.clone();
        f.request.descriptor.rootfs.path = original_rootfs.clone();
        f.request.digest = f.request.descriptor.canonical_digest().unwrap();
        let manifest = f.cache.prepare(f.request.clone()).unwrap();
        let staging = f.cache.config.root.join("inbox").join(key(&f.request));
        let manifest_bytes = fs::read(staging.join("manifest.json")).unwrap();
        if let Some(length) = kernel_length {
            partial_file(&staging, "kernel", &kernel[..length]);
        }
        if let Some(length) = rootfs_length {
            partial_file(&staging, "rootfs.ext4", &rootfs[..length]);
        }
        let config = f.cache.config.clone();
        let client = f.cache.client.clone();
        let lifecycle = f.cache.lifecycle.clone();
        // Exact fault point: original manifest and any partial file are durable,
        // but neither download completion nor publish ran. Dropping this sole
        // owner releases only the process lease, not the original disk record.
        drop(f.cache);
        let reopened = Arc::new(
            TemplateCache::open(config, f.request.node.clone(), client, lifecycle).unwrap(),
        );
        assert!(reopened.known(&f.request).unwrap().is_none());
        let metadata = fs::symlink_metadata(&staging).unwrap();
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            (manifest.device, manifest.inode)
        );
        assert_eq!(
            fs::read(staging.join("manifest.json")).unwrap(),
            manifest_bytes
        );
        assert!(!staging.join("kernel").exists());
        assert!(!staging.join("rootfs.ext4").exists());
        assert!(
            !reopened.state.lock().unwrap().ready().unwrap().pending[&key(&f.request)].downloading
        );
        let availability = Arc::clone(&reopened)
            .receive(f.request.clone())
            .await
            .unwrap();
        availability.validate_for(&f.request).unwrap();
        assert_eq!(f.requests.load(Ordering::SeqCst), 2);
        assert_eq!(fs::read(&original_kernel).unwrap(), kernel);
        assert_eq!(fs::read(&original_rootfs).unwrap(), rootfs);
        assert_eq!(fs::metadata(original_rootfs).unwrap().mode() & 0o777, 0o400);
        assert!(
            reopened
                .state
                .lock()
                .unwrap()
                .ready()
                .unwrap()
                .pending
                .is_empty()
        );
        assert_eq!(reopened.downloads.available_permits(), 1);
        f.shutdown.send(()).unwrap();
        f.server.await.unwrap().unwrap();
    }
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_idle_recovery_holds_full_entry_reserve_without_consuming_download_capacity() {
    let f = fixture(false).await;
    f.cache.prepare(f.request.clone()).unwrap();
    let config = f.cache.config.clone();
    let client = f.cache.client.clone();
    let lifecycle = f.cache.lifecycle.clone();
    drop(f.cache);
    let reopened =
        Arc::new(TemplateCache::open(config, f.request.node.clone(), client, lifecycle).unwrap());
    let mut next = f.request.clone();
    next.revision = "other-revision".try_into().unwrap();
    next.descriptor.revision = "r2".into();
    next.digest = next.descriptor.canonical_digest().unwrap();
    let availability = Arc::clone(&reopened).receive(next).await.unwrap();
    assert_eq!(availability.revision.as_str(), "other-revision");
    assert_eq!(reopened.downloads.available_permits(), 1);
    assert_eq!(
        reopened
            .state
            .lock()
            .unwrap()
            .ready()
            .unwrap()
            .pending
            .len(),
        1
    );
    let mut third = f.request.clone();
    third.revision = "third-revision".try_into().unwrap();
    third.descriptor.revision = "r3".into();
    third.digest = third.descriptor.canonical_digest().unwrap();
    assert_eq!(
        Arc::clone(&reopened).receive(third).await,
        Err(ReceiveError::Capacity)
    );
    assert_eq!(f.requests.load(Ordering::SeqCst), 2);
    let original = Arc::clone(&reopened)
        .receive(f.request.clone())
        .await
        .unwrap();
    original.validate_for(&f.request).unwrap();
    assert_eq!(
        reopened.state.lock().unwrap().ready().unwrap().ready.len(),
        2
    );
    assert!(
        reopened
            .state
            .lock()
            .unwrap()
            .ready()
            .unwrap()
            .pending
            .is_empty()
    );
    f.shutdown.send(()).unwrap();
    f.server.await.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_recovery_rejects_changed_identity_bad_manifest_and_symlink_without_cleanup() {
    for failure in ["identity", "manifest", "symlink", "owner"] {
        let f = fixture(false).await;
        let mut manifest = f.cache.prepare(f.request.clone()).unwrap();
        let staging = f.cache.config.root.join("inbox").join(key(&f.request));
        partial_file(&staging, "rootfs.ext4", b"original-partial");
        match failure {
            "identity" => {
                manifest.inode += 1;
                fs::set_permissions(
                    staging.join("manifest.json"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                fs::write(
                    staging.join("manifest.json"),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
                fs::set_permissions(
                    staging.join("manifest.json"),
                    fs::Permissions::from_mode(0o400),
                )
                .unwrap();
            }
            "manifest" => {
                fs::set_permissions(
                    staging.join("manifest.json"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                fs::write(staging.join("manifest.json"), b"{partial").unwrap();
                fs::set_permissions(
                    staging.join("manifest.json"),
                    fs::Permissions::from_mode(0o400),
                )
                .unwrap();
            }
            "symlink" => {
                std::os::unix::fs::symlink(
                    f.directory.path().join("not-owned"),
                    staging.join("kernel"),
                )
                .unwrap();
            }
            "owner" => std::os::unix::fs::chown(&staging, Some(64_001), None).unwrap(),
            _ => unreachable!(),
        }
        let config = f.cache.config.clone();
        let client = f.cache.client.clone();
        let lifecycle = f.cache.lifecycle.clone();
        drop(f.cache);
        let reopened = Arc::new(
            TemplateCache::open(config, f.request.node.clone(), client, lifecycle).unwrap(),
        );
        assert!(matches!(
            &*reopened.state.lock().unwrap(),
            CacheState::Unavailable
        ));
        assert_eq!(
            Arc::clone(&reopened).receive(f.request.clone()).await,
            Err(ReceiveError::Unavailable)
        );
        assert!(matches!(
            reopened.bound(&local::request().create),
            Err(ApplicationError::Busy)
        ));
        assert_eq!(f.requests.load(Ordering::SeqCst), 0);
        assert_eq!(
            fs::read(staging.join("rootfs.ext4")).unwrap(),
            b"original-partial"
        );
        assert!(staging.join("manifest.json").exists());
        if failure == "owner" {
            std::os::unix::fs::chown(&staging, Some(0), None).unwrap();
        }
        f.shutdown.send(()).unwrap();
        f.server.await.unwrap().unwrap();
    }
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_failed_exact_cleanup_does_not_release_unknown_temporary_reservation() {
    let f = fixture(true).await;
    let cache = Arc::clone(&f.cache);
    let request = f.request.clone();
    let caller = tokio::spawn(async move { cache.receive(request).await });
    f.entered.notified().await;
    let staging = f.cache.config.root.join("inbox").join(key(&f.request));
    partial_file(
        &staging,
        "unknown-file",
        b"not part of the original manifest",
    );
    f.release.notify_one();
    assert_eq!(caller.await.unwrap(), Err(ReceiveError::Unavailable));
    assert_eq!(f.cache.downloads.available_permits(), 1);
    assert!(f.cache.state.lock().unwrap().ready().unwrap().pending[&key(&f.request)].downloading);
    let mut next = f.request.clone();
    next.revision = "other-revision".try_into().unwrap();
    next.descriptor.revision = "r2".into();
    next.digest = next.descriptor.canonical_digest().unwrap();
    assert_eq!(
        Arc::clone(&f.cache).receive(next).await,
        Err(ReceiveError::Capacity)
    );
    assert_eq!(f.requests.load(Ordering::SeqCst), 2);
    assert!(staging.join("kernel").exists());
    assert!(staging.join("rootfs.ext4").exists());
    assert!(staging.join("unknown-file").exists());
    f.shutdown.send(()).unwrap();
    f.server.await.unwrap().unwrap();
}

fn driver_config() -> DriverConfig {
    DriverConfig {
        node: local::wire::node("host-a"),
        compute: local::compute(),
        interval: Duration::from_millis(100),
    }
}

async fn finish_original_destroy(
    lifecycle: &HostLifecycle<local::TestHost>,
    templates: Arc<TemplateCache<local::TestHost>>,
    mut destroy: LifecycleRequest,
) {
    destroy.intent = Intent::Destroy;
    destroy.identity.operation_id = "operation-2".try_into().unwrap();
    destroy.identity.intent_version = 2.try_into().unwrap();
    lifecycle.receive(destroy.clone()).await.unwrap();
    lifecycle
        .drive_once(driver_config(), templates)
        .await
        .unwrap();
    let cleaned = lifecycle.receive(destroy).await.unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    lifecycle.confirm(cleaned.clone()).await.unwrap();
    lifecycle
        .close(sandproof_protocol::internal::window::WindowCloseRequest::new(cleaned))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_missing_or_partial_manifest_fences_material_but_preserves_original_vm_cleanup() {
    for (partial_manifest, expire_first) in [(false, false), (true, true)] {
        let f = fixture(false).await;
        Arc::clone(&f.cache)
            .receive(f.request.clone())
            .await
            .unwrap();
        let lifecycle = f.cache.lifecycle.clone();
        let mut create = local::request();
        create.create.template_revision = f.request.revision.clone();
        create.create.template_digest = f.request.digest;
        lifecycle.receive(create.clone()).await.unwrap();
        lifecycle
            .drive_once(driver_config(), Arc::clone(&f.cache))
            .await
            .unwrap();
        assert!(matches!(
            lifecycle.receive(create.clone()).await.unwrap().fact,
            LifecycleFact::Ready { .. }
        ));
        let mut interrupted = f.request.clone();
        interrupted.revision = "interrupted-revision".try_into().unwrap();
        interrupted.descriptor.revision = "r2".into();
        interrupted.digest = interrupted.descriptor.canonical_digest().unwrap();
        let staging = f.cache.config.root.join("inbox").join(key(&interrupted));
        fs::DirBuilder::new().mode(0o700).create(&staging).unwrap();
        if partial_manifest {
            partial_file(&staging, "manifest.json", b"{partial");
            fs::set_permissions(
                staging.join("manifest.json"),
                fs::Permissions::from_mode(0o400),
            )
            .unwrap();
        }
        File::open(f.cache.config.root.join("inbox"))
            .unwrap()
            .sync_all()
            .unwrap();
        let identity = fs::symlink_metadata(&staging).unwrap();
        let config = f.cache.config.clone();
        let client = f.cache.client.clone();
        drop(f.cache);
        let fenced = Arc::new(
            TemplateCache::open(config, f.request.node.clone(), client, lifecycle.clone()).unwrap(),
        );
        assert!(matches!(
            &*fenced.state.lock().unwrap(),
            CacheState::Unavailable
        ));
        assert_eq!(
            Arc::clone(&fenced).receive(interrupted).await,
            Err(ReceiveError::Unavailable)
        );
        assert!(matches!(
            fenced.bound(&create.create),
            Err(ApplicationError::Busy)
        ));
        assert_eq!(f.requests.load(Ordering::SeqCst), 2);
        if expire_first {
            f.clock.0.store(60_010, Ordering::SeqCst);
            lifecycle
                .drive_once(driver_config(), Arc::clone(&fenced))
                .await
                .unwrap();
            assert!(f.host.lock().unwrap().actions.iter().any(|action| matches!(
                action.command.command(),
                sandbox_controller_core::Command::StopVm { .. }
            )));
        }
        finish_original_destroy(&lifecycle, Arc::clone(&fenced), create).await;
        let current = fs::symlink_metadata(&staging).unwrap();
        assert_eq!(
            (current.dev(), current.ino()),
            (identity.dev(), identity.ino())
        );
        if partial_manifest {
            assert_eq!(
                fs::read(staging.join("manifest.json")).unwrap(),
                b"{partial"
            );
        } else {
            assert!(fs::read_dir(&staging).unwrap().next().is_none());
        }
        assert_eq!(fenced.downloads.available_permits(), 1);
        assert!(matches!(
            &*fenced.state.lock().unwrap(),
            CacheState::Unavailable
        ));
        f.shutdown.send(()).unwrap();
        f.server.await.unwrap().unwrap();
    }
}

#[tokio::test]
#[ignore = "root-owned protected cache and real Linux TLS component"]
async fn cache_configuration_private_roots_and_writer_lease_remain_shared_startup_failures() {
    for failure in ["configuration", "root", "lease"] {
        let f = fixture(false).await;
        let mut config = f.cache.config.clone();
        let client = f.cache.client.clone();
        let lifecycle = f.cache.lifecycle.clone();
        drop(f.cache);
        match failure {
            "configuration" => config.max_downloads = 0,
            "root" => {
                fs::set_permissions(config.root.join("inbox"), fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
            "lease" => fs::set_permissions(
                config.root.join(".cache.lock"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap(),
            _ => unreachable!(),
        }
        assert!(TemplateCache::open(config, f.request.node.clone(), client, lifecycle).is_err());
        assert_eq!(f.requests.load(Ordering::SeqCst), 0);
        f.shutdown.send(()).unwrap();
        f.server.await.unwrap().unwrap();
    }
}
