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
    ArtifactClient, ClientTls, Limits, ProtectedServer, ServerTls, TransportError,
    cache::{ArtifactPart, ArtifactRequest},
};
use sandproof_protocol::internal::{NodeIdentity, lifecycle::Resources};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
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
    let metadata = RegistryMetadata::new(
        TemplateCatalog {
            templates: vec![binding.clone()],
            aliases: BTreeMap::new(),
            compute_limits: ComputeLimits {
                max_vcpus_global: 2,
                max_vcpus_per_tenant: 2,
                max_memory_bytes_global: resources().memory_bytes,
                max_memory_bytes_per_tenant: resources().memory_bytes,
            },
        },
        vec![TemplateRegistration {
            revision: "base-r1".try_into().unwrap(),
            template_id: "base".into(),
        }],
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
