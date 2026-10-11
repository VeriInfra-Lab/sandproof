//! Real pinned HTTPS and Redis; artifact images are controlled format fixtures,
//! not an assertion that these tiny bytes can boot a KVM guest.
use super::*;
#[allow(
    dead_code,
    reason = "shared fixed Redis fixture exposes additional failure cases"
)]
#[path = "../../../sandbox-center-store/tests/support/engine.rs"]
mod engine;
#[path = "../../../sandbox-internal/tests/support/tls.rs"]
mod tls;
use sandproof_center_core::{
    Bounds, CenterState, NodeMode, NodeRegistration, NodeRole, TenantBudget,
};
use sandproof_center_service::RegisteredHost;
use sandproof_center_service::business::{MetadataBudget, RegistryMetadata, TemplateRegistration};
use sandproof_center_store::{
    ControlState, RedisLedger, authority::AuthorityState, authority::CenterAuthority,
    business::BusinessEnvelope,
};
use sandproof_controller_service::{
    template_registry::RegistryConfig,
    templates::{ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding, TemplateCatalog},
};
use sandproof_internal::{
    ArtifactClient, CacheReceiver, ClientTls, DataReceiver, InternalClient, InternalServer,
    LifecycleReceiver, Limits, ProtectedServer, ReceiveError, RegisteredEndpoint, RejectLifecycle,
    ServerTls, TransportError, ValidationReceiver,
    cache::{ArtifactPart, ArtifactRequest, CacheAvailability, CacheRequest},
    status::{HostStatus, StatusReceiver, StatusRequest},
    template::{ValidationEvidence, ValidationRequest},
};
use sandproof_protocol::internal::{
    ControlIdentity, NodeIdentity,
    lifecycle::{LifecycleRequest, LifecycleResponse, Resources},
    window::{WindowCloseAck, WindowCloseRequest},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    sync::Mutex,
    time::Duration,
};

fn directory(path: &std::path::Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: engine::deployment(),
        node_id: "artifact-host".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    }
}
fn resources() -> Resources {
    Resources {
        instances: 2,
        cpu: 2,
        memory_bytes: 512 * 1024 * 1024,
        disk_bytes: 2 * 1024 * 1024,
    }
}
fn store(fixture: &engine::Engine) -> (Arc<CandidateStore>, TemplateBinding) {
    let root = fixture.root.path().canonicalize().unwrap().join("registry");
    directory(&root);
    directory(&root.join("inbox"));
    directory(&root.join("artifacts"));
    let bytes = b"fixed kernel bytes for artifact protocol fixture";
    let kernel = root.join("kernel");
    fs::write(&kernel, bytes).unwrap();
    fs::set_permissions(&kernel, fs::Permissions::from_mode(0o400)).unwrap();
    let rootfs = root.join("rootfs.ext4");
    let mut image = vec![0; 1024 * 1024];
    image[1028..1032].copy_from_slice(&1024_u32.to_le_bytes());
    image[1036..1040].copy_from_slice(&1000_u32.to_le_bytes());
    image[1080..1082].copy_from_slice(&[0x53, 0xef]);
    fs::write(&rootfs, &image).unwrap();
    fs::set_permissions(&rootfs, fs::Permissions::from_mode(0o400)).unwrap();
    let binding = TemplateBinding {
        template_id: "base".into(),
        revision: "r1".into(),
        owner: None,
        kernel: TemplateArtifact {
            path: kernel,
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        },
        rootfs: TemplateArtifact {
            path: rootfs,
            size_bytes: image.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&image)),
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            host_memory_bytes: 256 * 1024 * 1024,
            disk_bytes: 1024 * 1024,
        },
        envd_version: "0.9.0".into(),
        startup: None,
    };
    let config = RegistryConfig {
        root,
        max_templates_global: 2,
        max_templates_per_tenant: 2,
        max_artifact_bytes_global: 4 * 1024 * 1024,
        max_artifact_bytes_per_tenant: 4 * 1024 * 1024,
        validation_timeout_secs: 5,
    };
    (
        Arc::new(
            CandidateStore::new(config, fs::metadata(fixture.root.path()).unwrap().uid()).unwrap(),
        ),
        binding,
    )
}
fn authority(
    fixture: &engine::Engine,
    store: &CandidateStore,
    binding: &TemplateBinding,
) -> SharedBusinessAuthority {
    authority_with_catalog(fixture, store, std::slice::from_ref(binding))
}
fn authority_with_catalog(
    fixture: &engine::Engine,
    store: &CandidateStore,
    bindings: &[TemplateBinding],
) -> SharedBusinessAuthority {
    let metadata = RegistryMetadata::new(
        TemplateCatalog {
            templates: bindings.to_vec(),
            aliases: BTreeMap::new(),
            compute_limits: ComputeLimits {
                max_vcpus_global: 2,
                max_vcpus_per_tenant: 2,
                max_memory_bytes_global: resources().memory_bytes,
                max_memory_bytes_per_tenant: resources().memory_bytes,
            },
        },
        bindings
            .iter()
            .map(|binding| TemplateRegistration {
                revision: format!("{}-{}", binding.template_id, binding.revision)
                    .try_into()
                    .unwrap(),
                template_id: binding.template_id.clone(),
            })
            .collect(),
        store.config().clone(),
        MetadataBudget {
            max_certificate_bytes: 65_536,
        },
        None,
    )
    .unwrap();
    let center = CenterState::new(
        engine::deployment(),
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 2,
            max_sandboxes: 2,
            operations_per_sandbox: 4,
            max_options_bytes: 4096,
            platform: resources(),
        },
        vec![NodeRegistration {
            retention_slots: 2,
            identity: node(),
            role: NodeRole::Runtime,
            limit: resources(),
            free: Resources::ZERO,
            healthy_until_ms: 0,
            mode: NodeMode::Accepting,
            protocol_version: 2,
            templates: vec![],
        }],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources(),
        }],
    )
    .unwrap();
    let envelope = BusinessEnvelope { center, metadata };
    assert!(envelope.is_valid());
    let (ledger, _) = RedisLedger::initialize(
        engine::endpoint(fixture.address),
        engine::Engine::config(envelope.required_block_bytes()),
        fixture.lease(),
        &envelope,
    )
    .unwrap();
    SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_artifact_https_binds_registered_node_revision_hash_and_immutable_file() {
    let fixture = engine::Engine::new();
    let (store, binding) = store(&fixture);
    store.verify_binding(&binding).unwrap();
    let authority = authority(&fixture, &store, &binding);
    let certificates = tls::certificates_with_dual_roles();
    let limits = Limits::new(4, Duration::from_secs(1), Duration::from_secs(3)).unwrap();
    let tls = ServerTls::for_registered_hosts(
        certificates.center.material(),
        vec![certificates.ca.der().clone()],
        vec![certificates.host.pin()],
    )
    .unwrap();
    let peer = sandproof_center_service::deployment::ArtifactPeer {
        node: node(),
        pins: vec![certificates.host.pin()],
    };
    let owner = fs::metadata(fixture.root.path()).unwrap().uid();
    let handler = http::ArtifactHttp::new(
        authority,
        store,
        vec![peer],
        owner,
        Duration::from_secs(5),
        4,
        Handle::current(),
    );
    let server = ProtectedServer::new(tls, limits, handler);
    let bound = ArtifactService {
        bind: "127.0.0.1:0".parse().unwrap(),
        server,
    }
    .listen()
    .await
    .unwrap();
    let address = bound.listener.local_addr().unwrap();
    let (stop, signal) = watch::channel(false);
    let serving = tokio::spawn(bound.serve(signal));
    let material = ClientTls::new(
        certificates.host.material(),
        vec![certificates.ca.der().clone()],
        vec![certificates.center.pin()],
    )
    .unwrap();
    let client = artifact_client(material.clone(), address, node(), limits);
    let request = ArtifactRequest {
        node: node(),
        revision: "base-r1".try_into().unwrap(),
        digest: binding.canonical_digest().unwrap(),
        part: ArtifactPart::Kernel,
    };
    let mut output = tempfile::tempfile().unwrap();
    client
        .download_to(&request, &binding.kernel, &mut output)
        .await
        .unwrap();
    assert_eq!(output.metadata().unwrap().len(), binding.kernel.size_bytes);
    let mut wrong_hash = binding.kernel.clone();
    wrong_hash.sha256 = "0".repeat(64);
    assert_eq!(
        client
            .download_to(&request, &wrong_hash, &mut tempfile::tempfile().unwrap())
            .await,
        Err(TransportError::InvalidResponse)
    );
    let mut wrong_revision = request.clone();
    wrong_revision.revision = "missing-r1".try_into().unwrap();
    assert_eq!(
        client
            .download_to(
                &wrong_revision,
                &binding.kernel,
                &mut tempfile::tempfile().unwrap()
            )
            .await,
        Err(TransportError::InvalidResponse)
    );
    let mut wrong_node = node();
    wrong_node.node_generation = 2.try_into().unwrap();
    let other = artifact_client(material, address, wrong_node.clone(), limits);
    let mut crossed = request;
    crossed.node = wrong_node;
    assert_eq!(
        other
            .download_to(
                &crossed,
                &binding.kernel,
                &mut tempfile::tempfile().unwrap()
            )
            .await,
        Err(TransportError::InvalidResponse)
    );
    let _ = stop.send(true);
    serving.await.unwrap().unwrap();
}

fn artifact_client(
    material: ClientTls,
    address: std::net::SocketAddr,
    node: NodeIdentity,
    limits: Limits,
) -> ArtifactClient {
    ArtifactClient::new(
        material,
        address,
        "center.test".into(),
        node,
        limits,
        Duration::from_secs(5),
        Duration::from_secs(2),
    )
    .unwrap()
}

/// Controlled cache effects, not a claim that these format fixtures boot a VM.
/// Descriptor, file hashes and ext4 geometry use the original material checker;
/// the original HTTPS receiver/client and Redis CAS handle the cache fact.
struct VerifiedCache {
    store: Arc<CandidateStore>,
    requests: Arc<Mutex<Vec<CacheRequest>>>,
    corrupt_authority: Option<std::net::SocketAddrV4>,
}
impl CacheReceiver for VerifiedCache {
    async fn cache_template(
        &self,
        request: CacheRequest,
    ) -> Result<CacheAvailability, ReceiveError> {
        request
            .validate(&node())
            .map_err(|_| ReceiveError::Conflict)?;
        self.store
            .verify_binding(&request.descriptor)
            .map_err(|_| ReceiveError::Conflict)?;
        self.requests.lock().unwrap().push(request.clone());
        if let Some(address) = self.corrupt_authority {
            let _: usize = redis::cmd("HSET")
                .arg(engine::KEY)
                .arg("payload")
                .arg(b"corrupted".as_slice())
                .query(&mut engine::connect(address))
                .unwrap();
        }
        Ok(CacheAvailability {
            node: request.node,
            revision: request.revision,
            digest: request.digest,
        })
    }
}
impl LifecycleReceiver for VerifiedCache {
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        RejectLifecycle.receive(request).await
    }
    async fn confirm(&self, result: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        RejectLifecycle.confirm(result).await
    }
    async fn close(&self, request: WindowCloseRequest) -> Result<WindowCloseAck, ReceiveError> {
        RejectLifecycle.close(request).await
    }
}
impl DataReceiver for VerifiedCache {
    async fn receive_data(
        &self,
        identity: ControlIdentity,
        request: axum::http::Request<axum::body::Body>,
    ) -> axum::http::Response<axum::body::Body> {
        RejectLifecycle.receive_data(identity, request).await
    }
}
impl ValidationReceiver for VerifiedCache {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
    ) -> Result<ValidationEvidence, ReceiveError> {
        RejectLifecycle.validation_evidence(request).await
    }
}
impl StatusReceiver for VerifiedCache {
    async fn status(&self, request: StatusRequest) -> Result<HostStatus, ReceiveError> {
        RejectLifecycle.status(request).await
    }
}

fn second_binding(binding: &TemplateBinding) -> TemplateBinding {
    let mut other = binding.clone();
    other.template_id = "other".into();
    other.kernel.path.set_file_name("other-kernel");
    other.rootfs.path.set_file_name("other-rootfs.ext4");
    fs::copy(&binding.kernel.path, &other.kernel.path).unwrap();
    fs::copy(&binding.rootfs.path, &other.rootfs.path).unwrap();
    other
}

struct CacheFixture {
    candidates: ProtectedCandidates,
    requests: Arc<Mutex<Vec<CacheRequest>>>,
    stop: tokio::sync::oneshot::Sender<()>,
    serving: tokio::task::JoinHandle<Result<(), TransportError>>,
}
impl CacheFixture {
    async fn start(
        authority: SharedBusinessAuthority,
        store: Arc<CandidateStore>,
        corrupt_authority: Option<std::net::SocketAddrV4>,
    ) -> Self {
        let certificates = tls::certificates_with_dual_roles();
        let limits = Limits::new(4, Duration::from_secs(1), Duration::from_secs(3)).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let receiver = VerifiedCache {
            store: Arc::clone(&store),
            requests: Arc::clone(&requests),
            corrupt_authority,
        };
        let server = InternalServer::new(
            ServerTls::new(
                certificates.host.material(),
                vec![certificates.ca.der().clone()],
                vec![certificates.center.pin()],
            )
            .unwrap(),
            node(),
            limits,
            receiver,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let serving = tokio::spawn(async move { server.serve(listener, shutdown).await });
        let client = InternalClient::new(
            ClientTls::new(
                certificates.center.material(),
                vec![certificates.ca.der().clone()],
                vec![certificates.host.pin()],
            )
            .unwrap(),
            RegisteredEndpoint::new(address, "host.test".into(), node()).unwrap(),
            limits,
        );
        let hosts = RegisteredHosts::new(
            vec![RegisteredHost {
                identity: node(),
                client,
            }],
            Handle::current(),
        )
        .unwrap();
        Self {
            candidates: ProtectedCandidates::new(authority, hosts, store),
            requests,
            stop,
            serving,
        }
    }
    async fn prepare(&self) -> Result<(), sandproof_api::PortError> {
        let candidates = self.candidates.clone();
        tokio::task::spawn_blocking(move || candidates.prepare_static(Duration::from_secs(3)))
            .await
            .unwrap()
    }
    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.serving.await.unwrap().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupt_static_material_does_not_block_a_later_healthy_revision() {
    let fixture = engine::Engine::new();
    let (store, first) = store(&fixture);
    let healthy = second_binding(&first);
    store.verify_binding(&first).unwrap();
    store.verify_binding(&healthy).unwrap();
    let authority = authority_with_catalog(&fixture, &store, &[first.clone(), healthy.clone()]);
    // Corrupt A only AFTER the protected descriptors were durably registered.
    // It retains its readonly shape, so the original actual digest check fails.
    fs::set_permissions(&first.kernel.path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(
        &first.kernel.path,
        vec![0; usize::try_from(first.kernel.size_bytes).unwrap()],
    )
    .unwrap();
    fs::set_permissions(&first.kernel.path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(store.verify_binding(&first).is_err());
    store.verify_binding(&healthy).unwrap();
    let cache = CacheFixture::start(authority.clone(), store, None).await;
    assert_eq!(cache.prepare().await, Ok(()));
    let requests = cache.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].descriptor, healthy);
    let observed = authority.snapshot().unwrap();
    let available = &observed.center.nodes()[0].templates;
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].revision.as_str(), "other-r1");
    assert_eq!(available[0].digest, healthy.canonical_digest().unwrap());
    // Rechecking the bad material does not invent availability or repeat B.
    assert_eq!(cache.prepare().await, Ok(()));
    assert_eq!(cache.requests.lock().unwrap().len(), 1);
    assert_eq!(
        authority.snapshot().unwrap().center.nodes()[0].templates,
        *available
    );
    cache.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_authority_fault_stops_static_scan_without_registering_cache_facts() {
    let fixture = engine::Engine::new();
    let (store, first) = store(&fixture);
    let later = second_binding(&first);
    let authority = authority_with_catalog(&fixture, &store, &[first.clone(), later]);
    let cache = CacheFixture::start(authority.clone(), store, Some(fixture.address)).await;
    // The original receiver returns exact cache evidence, but Redis is damaged
    // before its CAS. This must fence A and prevent any request for healthy B.
    assert_eq!(
        cache.prepare().await,
        Err(sandproof_api::PortError::OutcomeUnknown)
    );
    let requests = cache.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].descriptor, first);
    assert!(authority.snapshot().is_err());
    assert_eq!(
        cache.prepare().await,
        Err(sandproof_api::PortError::OutcomeUnknown)
    );
    assert_eq!(cache.requests.lock().unwrap().len(), 1);
    cache.stop().await;
}
