use super::engine;
use sandproof_center_core::{
    Bounds, CenterState, LifecycleWindow, NodeMode, NodeRegistration, NodeRole, TenantBudget,
};
use sandproof_center_service::{
    BranchBatchLimits, PublicTemplate,
    business::{MetadataBudget, RegistryMetadata, SharedBusinessAuthority, TemplateRegistration},
};
use sandproof_center_store::{
    RedisLedger,
    authority::{AuthorityState, CenterAuthority},
    business::BusinessEnvelope,
};
use sandproof_controller_service::{
    CreateNetworking, CreateOptions,
    template_registry::RegistryConfig,
    templates::{ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding, TemplateCatalog},
};
use sandproof_internal::cache::{CacheAvailability, CacheRequest};
use sandproof_protocol::{
    ResourceId,
    internal::{NodeIdentity, lifecycle::Resources},
    lifecycle::{CenterIdentity, CreateParameters, PublicCreateRequest},
};
use std::collections::BTreeMap;

const MIB: u64 = 1024 * 1024;
const RESOURCES: Resources = Resources {
    instances: 1,
    cpu: 1,
    memory_bytes: 256 * MIB,
    disk_bytes: MIB,
};

pub(super) fn authority(
    fixture: &engine::Engine,
) -> (SharedBusinessAuthority, PublicCreateRequest) {
    let node = NodeIdentity {
        deployment_id: engine::deployment(),
        node_id: "host-a".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    };
    let owner = "8".try_into().unwrap();
    let revision: ResourceId = "base-r1".try_into().unwrap();
    let descriptor = template_binding();
    let template = PublicTemplate::from_binding(descriptor.clone(), revision.clone()).unwrap();
    let state = BusinessEnvelope {
        center: CenterState::new(
            engine::deployment(),
            Bounds {
                max_nodes: 1,
                max_templates_per_node: 2,
                max_sandboxes: 1,
                operations_per_sandbox: 4,
                max_options_bytes: 256,
                result_retention_ms: 100,
                platform: RESOURCES,
            },
            vec![NodeRegistration {
                retention_slots: 1,
                role: NodeRole::Runtime,
                identity: node.clone(),
                mode: NodeMode::Accepting,
                limit: RESOURCES,
                free: RESOURCES,
                healthy_until_ms: 2000,
                protocol_version: 2,
                templates: vec![],
            }],
            vec![TenantBudget {
                tenant: owner,
                limit: RESOURCES,
            }],
            CenterIdentity {
                deployment: engine::deployment(),
                center: "center".try_into().unwrap(),
            },
            vec![LifecycleWindow {
                owner: "8".try_into().unwrap(),
                window: 1.try_into().unwrap(),
            }],
        )
        .unwrap(),
        metadata: branch_registry(descriptor.clone(), revision.clone()),
    };
    let (ledger, _) = RedisLedger::initialize(
        engine::endpoint(fixture.address),
        engine::Engine::config(state.required_block_bytes()),
        fixture.lease(),
        &state,
    )
    .unwrap();
    let authority = SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap());
    let digest = template.digest();
    authority
        .register_cache(
            &CacheRequest {
                node: node.clone(),
                revision: revision.clone(),
                digest,
                descriptor,
            },
            &CacheAvailability {
                node,
                revision: revision.clone(),
                digest,
            },
        )
        .unwrap();
    let options = CreateOptions {
        networking: Some(CreateNetworking {
            allow_internet_access: Some(false),
            network: None,
        }),
        ..CreateOptions::default()
    };
    let request = PublicCreateRequest {
        window: 1.try_into().unwrap(),
        sequence: 1.try_into().unwrap(),
        parameters: CreateParameters {
            template_revision: revision,
            template_digest: digest,
            resources: RESOURCES,
            options: serde_json::to_vec(&options).unwrap(),
            startup_timeout_ms: 1000,
            timeout_ms: 5000,
            max_lifetime_ms: 5000,
        },
    };
    (authority, request)
}

fn template_binding() -> TemplateBinding {
    TemplateBinding {
        guest: None,
        template_id: "base".into(),
        revision: "r1".into(),
        owner: None,
        kernel: TemplateArtifact {
            path: "/center/kernel".into(),
            size_bytes: 4096,
            sha256: "1".repeat(64),
        },
        rootfs: TemplateArtifact {
            path: "/center/disk".into(),
            size_bytes: MIB,
            sha256: "2".repeat(64),
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            host_memory_bytes: 256 * MIB,
            disk_bytes: MIB,
        },
        envd_version: "0.9.0".into(),
        startup: None,
    }
}

fn branch_registry(descriptor: TemplateBinding, revision: ResourceId) -> RegistryMetadata {
    RegistryMetadata::new(
        TemplateCatalog {
            templates: vec![descriptor],
            aliases: BTreeMap::new(),
            compute_limits: ComputeLimits {
                max_vcpus_global: 2,
                max_vcpus_per_tenant: 2,
                max_memory_bytes_global: 512 * MIB,
                max_memory_bytes_per_tenant: 512 * MIB,
            },
        },
        vec![TemplateRegistration {
            revision: revision.clone(),
            template_id: "base".into(),
        }],
        RegistryConfig {
            root: "/center/registry".into(),
            max_templates_global: 2,
            max_templates_per_tenant: 1,
            max_artifact_bytes_global: 8 * MIB,
            max_artifact_bytes_per_tenant: 4 * MIB,
            validation_timeout_secs: 5,
        },
        MetadataBudget {
            max_certificate_bytes: 128 * 1024,
        },
        None,
    )
    .unwrap()
    .with_branch_batches(
        BranchBatchLimits {
            max_batches: 4,
            max_branches_per_vm: 2,
            first_port: 31_000,
        },
        vec![revision],
    )
    .unwrap()
}
