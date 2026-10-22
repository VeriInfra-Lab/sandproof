//! Actual fixed Redis and original journal certificates. Physical effects in
//! certificate fixtures are controlled; these tests do NOT claim KVM.
#![cfg(unix)]
#[allow(
    dead_code,
    reason = "shared fixed Redis fixture exposes other storage scenarios"
)]
#[path = "../../sandbox-center-store/tests/support/engine.rs"]
mod engine;
#[path = "business_authority/lifecycle_public.rs"]
mod lifecycle_public;
#[allow(
    dead_code,
    reason = "original worker fixture used to extract actual committed evidence"
)]
#[path = "../../controller-service/tests/support/mod.rs"]
mod local;
#[path = "business_authority/owner_retirement.rs"]
mod owner_retirement;
#[path = "business_authority/private_builds.rs"]
mod private_builds;
#[path = "business_authority/registration.rs"]
mod registration;
#[path = "../../sandbox-center-store/tests/support/reply_loss.rs"]
mod reply_loss;
#[path = "business_authority/workflow.rs"]
mod templates;
#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod wire;

use engine::Engine;
use sandproof_api::PortError;
use sandproof_center_core::{Bounds, CenterState, NodeMode, NodeRegistration, TenantBudget};
use sandproof_center_service::{
    AuthorityPort, PublicTemplate,
    business::{
        LocalValidationBinding, MetadataBudget, RegistryMetadata, SharedBusinessAuthority,
        TemplateRegistration,
    },
};
use sandproof_center_store::{
    ControlState, RedisLedger, StoreError,
    authority::{AuthorityError, AuthorityState, CenterAuthority},
    business::BusinessEnvelope,
};
use sandproof_controller_service::{
    CreateNetworking, CreateOptions,
    template_registry::{
        Publication, PublicationFailure, PublicationState, RegistryConfig, ValidationCertificate,
        evidence,
    },
    templates::{ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding, TemplateCatalog},
};
use sandproof_protocol::{
    ResourceId,
    internal::lifecycle::{CreateBinding, LifecycleFact, Resources},
};
use std::collections::BTreeMap;

fn descriptor(id: &str, owner: Option<u64>) -> TemplateBinding {
    TemplateBinding {
        guest: None,
        template_id: id.into(),
        revision: "r1".into(),
        owner,
        kernel: TemplateArtifact {
            path: "/center/kernel".into(),
            size_bytes: 4096,
            sha256: "1".repeat(64),
        },
        rootfs: TemplateArtifact {
            path: "/center/disk".into(),
            size_bytes: 1024 * 1024,
            sha256: "2".repeat(64),
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            host_memory_bytes: 256 * 1024 * 1024,
            disk_bytes: 1024 * 1024,
        },
        envd_version: "0.9.0".into(),
        startup: None,
    }
}
fn candidate() -> Publication {
    let id = format!("{:032x}", 1);
    let mut binding = descriptor(&format!("tpl-{id}"), Some(8));
    binding.guest = Some(
        serde_json::from_str(include_str!(
            "../../tests/compat/guest-template-identity.json"
        ))
        .unwrap(),
    );
    Publication {
        candidate_id: id.clone(),
        binding,
        image_digest: format!("sha256:{}", "3".repeat(64)),
        free_bytes: 4096,
        min_free_bytes: 4096,
        state: PublicationState::Checking,
        checked: false,
        validation_sandbox: None,
        validation_certificate: None,
        failure: None,
    }
}
fn revision() -> ResourceId {
    "candidate-1-r1".try_into().unwrap()
}
fn resources(instances: u64) -> Resources {
    Resources {
        instances,
        cpu: instances,
        memory_bytes: instances * 256 * 1024 * 1024,
        disk_bytes: instances * 1024 * 1024,
    }
}
fn binding() -> CreateBinding {
    let public = PublicTemplate::from_binding(candidate().binding, revision()).unwrap();
    CreateBinding {
        template_revision: revision(),
        template_digest: public.digest(),
        resources: resources(1),
        options: serde_json::to_vec(&CreateOptions {
            networking: Some(CreateNetworking {
                allow_internet_access: Some(false),
                network: None,
            }),
            ..CreateOptions::default()
        })
        .unwrap(),
        startup_deadline_ms: 1000,
        timeout_ms: 5000,
        max_lifetime_ms: 5000,
    }
}
fn envelope() -> BusinessEnvelope<RegistryMetadata> {
    let metadata = RegistryMetadata::new(
        TemplateCatalog {
            templates: vec![descriptor("base", None)],
            aliases: BTreeMap::new(),
            compute_limits: ComputeLimits {
                max_vcpus_global: 2,
                max_vcpus_per_tenant: 2,
                max_memory_bytes_global: 512 * 1024 * 1024,
                max_memory_bytes_per_tenant: 512 * 1024 * 1024,
            },
        },
        vec![TemplateRegistration {
            revision: "base-r1".try_into().unwrap(),
            template_id: "base".into(),
        }],
        RegistryConfig {
            root: "/center/registry".into(),
            max_templates_global: 2,
            max_templates_per_tenant: 1,
            max_artifact_bytes_global: 8 * 1024 * 1024,
            max_artifact_bytes_per_tenant: 4 * 1024 * 1024,
            validation_timeout_secs: 5,
        },
        MetadataBudget {
            max_certificate_bytes: 128 * 1024,
        },
        None,
    )
    .unwrap();
    BusinessEnvelope {
        center: CenterState::new(
            engine::deployment(),
            Bounds {
                max_nodes: 1,
                max_templates_per_node: 2,
                max_sandboxes: 2,
                operations_per_sandbox: 4,
                max_options_bytes: 256,
                platform: resources(2),
            },
            vec![NodeRegistration {
                retention_slots: 2,
                role: sandproof_center_core::NodeRole::Runtime,
                identity: wire::node("host-a"),
                mode: NodeMode::Accepting,
                limit: resources(2),
                free: resources(2),
                healthy_until_ms: 2000,
                protocol_version: 2,
                templates: vec![],
            }],
            vec![TenantBudget {
                tenant: "8".try_into().unwrap(),
                limit: resources(2),
            }],
        )
        .unwrap(),
        metadata,
    }
}
fn authority(fixture: &Engine, proxy: &reply_loss::ReplyLoss) -> SharedBusinessAuthority {
    let state = envelope();
    let (ledger, _) = RedisLedger::initialize(
        engine::endpoint(proxy.address),
        Engine::config(state.required_block_bytes()),
        fixture.lease(),
        &state,
    )
    .unwrap();
    SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap())
}
fn cache_candidate(business: &SharedBusinessAuthority) {
    let request = sandproof_internal::cache::CacheRequest {
        node: wire::node("host-a"),
        revision: revision(),
        digest: binding().template_digest,
        descriptor: candidate().binding,
    };
    business
        .register_cache(
            &request,
            &sandproof_internal::cache::CacheAvailability {
                node: request.node.clone(),
                revision: request.revision.clone(),
                digest: request.digest,
            },
        )
        .unwrap();
}

#[test]
fn actual_unknown_metadata_and_coupled_create_fence_both_adapters_then_reaffirm_same_identity() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = authority(&fixture, &proxy);
    let mut lifecycle = business.clone();
    // Lose decision commit, not the preceding fresh-view reaffirmation.
    proxy.lose_after(1);
    assert_eq!(
        business.begin_publication(candidate(), revision()),
        Err(PortError::OutcomeUnknown)
    );
    assert!(business.snapshot().is_err());
    assert!(AuthorityPort::snapshot(&mut lifecycle).is_err());
    AuthorityPort::recover(&mut lifecycle).unwrap();
    assert_eq!(
        business
            .snapshot()
            .unwrap()
            .metadata
            .registry()
            .records()
            .len(),
        1
    );
    assert_eq!(
        business.begin_publication(candidate(), revision()),
        Err(PortError::Conflict)
    );
    cache_candidate(&business);
    proxy.lose_after(1);
    assert_eq!(
        business.create_validation(&candidate().candidate_id, binding(), 0),
        Err(PortError::OutcomeUnknown)
    );
    assert!(business.snapshot().is_err());
    assert!(AuthorityPort::snapshot(&mut lifecycle).is_err());
    assert!(matches!(
        AuthorityPort::create(&mut lifecycle, &"8".try_into().unwrap(), binding(), 0),
        Err(AuthorityError::RecoveryRequired)
    ));
    AuthorityPort::recover(&mut lifecycle).unwrap();
    let recovered = business.snapshot().unwrap();
    assert!(recovered.is_valid());
    assert_eq!(recovered.center.pending().len(), 1);
    let original = &recovered.center.pending()[0].identity;
    let record = &recovered.metadata.registry().records()[0];
    assert_eq!(record.value().binding.guest, candidate().binding.guest);
    assert_eq!(&record.validation().unwrap().create, original);
    assert_eq!(original.sandbox_id.as_str(), "sandbox-1");
    assert_eq!(original.operation_id.as_str(), "operation-1");
    assert_eq!(recovered.center.reserved(None, None), Some(resources(1)));
    assert_validation_projection_enforced(&recovered);
    assert_eq!(
        business.reject_unattached(&candidate().candidate_id, PublicationFailure::ArtifactCheck),
        Err(PortError::Conflict)
    );
    assert_eq!(
        business.create_validation(&candidate().candidate_id, binding(), 0),
        Err(PortError::Conflict)
    );
}

fn assert_validation_projection_enforced(state: &BusinessEnvelope<RegistryMetadata>) {
    for case in 0..4 {
        let mut altered = serde_json::to_value(state).unwrap();
        let create = &mut altered["center"]["sandboxes"][0]["operations"][0]["request"]["create"];
        match case {
            0 => create["resources"]["cpu"] = serde_json::json!(2),
            1 => create["timeout_ms"] = serde_json::json!(4000),
            _ => {
                let mut options: CreateOptions =
                    serde_json::from_slice(&binding().options).unwrap();
                if case == 2 {
                    options
                        .env_vars
                        .insert("KEY".into(), "not-a-validation-option".into());
                } else {
                    options.networking.as_mut().unwrap().allow_internet_access = Some(true);
                }
                create["options"] =
                    serde_json::to_value(serde_json::to_vec(&options).unwrap()).unwrap();
            }
        }
        let altered: BusinessEnvelope<RegistryMetadata> = serde_json::from_value(altered).unwrap();
        // These are valid ordinary lifecycle inputs, but no longer describe
        // this exact protected validation and may not survive restoration.
        assert!(altered.center.is_valid());
        assert!(!altered.is_valid());
    }
}

fn local_certificate(activated: bool) -> (LocalValidationBinding, ValidationCertificate) {
    let root = tempfile::tempdir().unwrap();
    let mut journal =
        local::open_complete(root.path().join("native.db"), local::NAMESPACE).unwrap();
    local::apply(&mut journal, local::create());
    if activated {
        sandproof_controller_service::finish_admitted_create_v1(
            &mut journal,
            local::SANDBOX,
            local::CREATE_OP,
            &mut local::worker,
        )
        .unwrap();
        local::apply(&mut journal, local::kill(1));
        sandproof_controller_service::finish_accepted_kill_v1(
            &mut journal,
            local::SANDBOX,
            sandbox_controller_core::OperationId(101),
            &mut local::worker,
        )
        .unwrap();
    } else {
        assert_eq!(
            journal
                .abort_due_startups(300_000, &mut sandproof_controller_service::NeverCrash)
                .unwrap(),
            1
        );
        sandproof_controller_service::finish_compensating_create_v1(
            &mut journal,
            local::SANDBOX,
            local::CREATE_OP,
            &mut local::worker,
        )
        .unwrap();
    }
    let mut publication = candidate();
    publication.binding.owner = Some(1);
    publication.binding.kernel.path = "/host/cache/kernel".into();
    publication.binding.rootfs.path = "/host/cache/rootfs".into();
    publication.checked = activated;
    publication.state = if activated {
        PublicationState::Ready
    } else {
        PublicationState::Failed
    };
    publication.failure = (!activated).then_some(PublicationFailure::Interrupted);
    let certificate =
        evidence::make_certificate(&journal.world().unwrap(), &publication, local::SANDBOX)
            .unwrap();
    publication.validation_certificate = Some(certificate.clone());
    evidence::validate_publication(&publication).unwrap();
    (
        LocalValidationBinding {
            owner: 1,
            sandbox: local::SANDBOX,
            create_operation: local::CREATE_OP,
            descriptor: publication.binding,
        },
        certificate,
    )
}

#[test]
fn original_local_certificate_exact_global_mapping_and_cleaned_fact_are_all_required() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = authority(&fixture, &proxy);
    let mut lifecycle = business.clone();
    business.begin_publication(candidate(), revision()).unwrap();
    cache_candidate(&business);
    let create = business
        .create_validation(&candidate().candidate_id, binding(), 0)
        .unwrap();
    let (mapping, certificate) = local_certificate(true);
    bind_exact_mapping(&business, &create.identity, &mapping);
    AuthorityPort::apply_response(
        &mut lifecycle,
        &wire::response(
            &create,
            1,
            LifecycleFact::Ready {
                started_at_ms: 10,
                end_at_ms: 5010,
                max_end_at_ms: 5010,
            },
        ),
    )
    .unwrap();
    let destroy = AuthorityPort::destroy(
        &mut lifecycle,
        &"8".try_into().unwrap(),
        &create.identity.sandbox_id,
    )
    .unwrap();
    assert!(
        business
            .settle_validation(
                &candidate().candidate_id,
                &destroy.identity,
                certificate.clone()
            )
            .is_err()
    );
    AuthorityPort::apply_response(
        &mut lifecycle,
        &wire::response(&destroy, 2, LifecycleFact::Cleaned),
    )
    .unwrap();
    let mut foreign = destroy.identity.clone();
    foreign.node.node_id = "host-b".try_into().unwrap();
    assert!(
        business
            .settle_validation(&candidate().candidate_id, &foreign, certificate.clone())
            .is_err()
    );
    reject_changed_certificates(&business, &destroy.identity, &certificate);
    settle_then_retire(&business, &proxy, &destroy, certificate);
}

#[test]
fn original_failed_create_certificate_remains_bound_after_same_envelope_retirement() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = authority(&fixture, &proxy);
    let mut lifecycle = business.clone();
    business.begin_publication(candidate(), revision()).unwrap();
    cache_candidate(&business);
    let create = business
        .create_validation(&candidate().candidate_id, binding(), 0)
        .unwrap();
    let (mapping, certificate) = local_certificate(false);
    assert!(certificate.activation.is_none());
    assert_eq!(
        certificate.terminal_operation.operation.id,
        mapping.create_operation
    );
    business
        .bind_local_validation(&candidate().candidate_id, &create.identity, &mapping)
        .unwrap();
    let destroy = AuthorityPort::destroy(
        &mut lifecycle,
        &"8".try_into().unwrap(),
        &create.identity.sandbox_id,
    )
    .unwrap();
    let cleaned = wire::response(&destroy, 1, LifecycleFact::Cleaned);
    AuthorityPort::apply_response(&mut lifecycle, &cleaned).unwrap();
    assert_unsettled_close_veto(&business, &cleaned);
    business
        .settle_validation(&candidate().candidate_id, &destroy.identity, certificate)
        .unwrap();
    let close = business.begin_close(&cleaned).unwrap();
    business
        .apply_close_ack(&sandproof_protocol::internal::window::WindowCloseAck {
            terminal: close.terminal,
        })
        .unwrap();
    let retired = business.snapshot().unwrap();
    assert!(retired.is_valid());
    assert!(retired.center.sandboxes().is_empty());
    let record = &retired.metadata.registry().records()[0];
    assert_eq!(record.value().state, PublicationState::Failed);
    assert!(record.validation().unwrap().settlement.is_some());
    let mut changed = serde_json::to_value(&retired).unwrap();
    changed["metadata"]["registry"]["records"][0]["validation"]["local"]["create_operation"] =
        serde_json::json!(999);
    let changed: BusinessEnvelope<RegistryMetadata> = serde_json::from_value(changed).unwrap();
    assert!(!changed.is_valid());
}

#[test]
fn unsettled_validation_pins_only_its_exact_lane_without_fencing_healthy_siblings() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = authority(&fixture, &proxy);
    let mut lifecycle = business.clone();
    let tenant = "8".try_into().unwrap();
    business.begin_publication(candidate(), revision()).unwrap();
    cache_candidate(&business);
    let validation = business
        .create_validation(&candidate().candidate_id, binding(), 0)
        .unwrap();
    let peer = AuthorityPort::create(&mut lifecycle, &tenant, binding(), 0).unwrap();
    assert_ne!(
        validation.identity.retention_slot,
        peer.identity.retention_slot
    );
    let destroy =
        AuthorityPort::destroy(&mut lifecycle, &tenant, &validation.identity.sandbox_id).unwrap();
    let pinned = wire::response(&destroy, 1, LifecycleFact::Cleaned);
    AuthorityPort::apply_response(&mut lifecycle, &pinned).unwrap();
    business.confirm_terminal(&pinned).unwrap();
    assert_eq!(business.begin_close(&pinned), Err(PortError::Conflict));
    // A pure, known business veto must not turn into an authority fault.
    assert!(business.snapshot().is_ok());
    let original = AuthorityPort::snapshot(&mut lifecycle).unwrap().sandboxes()[0].clone();
    let destroy_peer =
        AuthorityPort::destroy(&mut lifecycle, &tenant, &peer.identity.sandbox_id).unwrap();
    let cleaned_peer = wire::response(&destroy_peer, 1, LifecycleFact::Cleaned);
    AuthorityPort::apply_response(&mut lifecycle, &cleaned_peer).unwrap();
    business.confirm_terminal(&cleaned_peer).unwrap();
    let close = business.begin_close(&cleaned_peer).unwrap();
    business
        .apply_close_ack(&sandproof_protocol::internal::window::WindowCloseAck {
            terminal: close.terminal,
        })
        .unwrap();
    let retained = business.snapshot().unwrap();
    assert_eq!(retained.center.sandboxes(), std::slice::from_ref(&original));
    assert!(retained.center.pending_closes().is_empty());
    assert_eq!(
        retained
            .center
            .lane(
                &validation.identity.node,
                validation.identity.retention_slot
            )
            .unwrap()
            .retired_through(),
        0
    );
    let next = AuthorityPort::create(&mut lifecycle, &tenant, binding(), 0).unwrap();
    assert_eq!(next.identity.retention_slot, peer.identity.retention_slot);
    assert_eq!(next.identity.request_window.get(), 2);
    let public = AuthorityPort::public_snapshot(&mut lifecycle).unwrap();
    assert_eq!(
        public
            .state
            .sandboxes()
            .iter()
            .filter(|record| public.visible(record))
            .count(),
        1
    );
}

fn bind_exact_mapping(
    business: &SharedBusinessAuthority,
    identity: &sandproof_protocol::internal::ControlIdentity,
    mapping: &LocalValidationBinding,
) {
    let mut foreign = identity.clone();
    foreign.tenant_id = "9".try_into().unwrap();
    assert_eq!(
        business.bind_local_validation(&candidate().candidate_id, &foreign, mapping),
        Err(PortError::Conflict)
    );
    let mut oversized = mapping.clone();
    oversized.descriptor.rootfs.path =
        format!("/host/{}", "x".repeat(CreateOptions::MAX_ENCODED_BYTES)).into();
    assert!(
        business
            .bind_local_validation(&candidate().candidate_id, identity, &oversized)
            .is_err()
    );
    assert!(
        business.snapshot().unwrap().metadata.registry().records()[0]
            .validation()
            .unwrap()
            .local
            .is_none()
    );
    business
        .bind_local_validation(&candidate().candidate_id, identity, mapping)
        .unwrap();
    let mut changed = mapping.clone();
    changed.owner = 2;
    assert_eq!(
        business.bind_local_validation(&candidate().candidate_id, identity, &changed),
        Err(PortError::Conflict)
    );
}

fn settle_then_retire(
    business: &SharedBusinessAuthority,
    proxy: &reply_loss::ReplyLoss,
    destroy: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
    certificate: ValidationCertificate,
) {
    let mut lifecycle = business.clone();
    assert_unsettled_close_veto(
        business,
        &wire::response(destroy, 2, LifecycleFact::Cleaned),
    );
    proxy.lose_after(1);
    assert_eq!(
        business.settle_validation(&candidate().candidate_id, &destroy.identity, certificate),
        Err(PortError::OutcomeUnknown)
    );
    assert!(business.snapshot().is_err());
    assert!(AuthorityPort::snapshot(&mut lifecycle).is_err());
    let cleaned = wire::response(destroy, 2, LifecycleFact::Cleaned);
    assert!(business.begin_close(&cleaned).is_err());
    AuthorityPort::recover(&mut lifecycle).unwrap();
    let state = business.snapshot().unwrap();
    assert_settled_and_independent(&state);
    let close = business.begin_close(&cleaned).unwrap();
    let close = sandproof_protocol::internal::window::WindowCloseAck {
        terminal: close.terminal,
    };
    business.apply_close_ack(&close).unwrap();
    let retired = business.snapshot().unwrap();
    assert!(retired.is_valid());
    assert!(retired.center.sandboxes().is_empty());
    assert_eq!(
        serde_json::to_value(&retired.metadata).unwrap(),
        serde_json::to_value(&state.metadata).unwrap()
    );
    assert_settlement_tampering_refused(&retired);
}

fn assert_unsettled_close_veto(
    business: &SharedBusinessAuthority,
    cleaned: &sandproof_protocol::internal::lifecycle::LifecycleResponse,
) {
    let state = business.snapshot().unwrap();
    let mut missing = serde_json::to_value(&state).unwrap();
    missing["center"]["sandboxes"] = serde_json::json!([]);
    assert!(
        !serde_json::from_value::<BusinessEnvelope<RegistryMetadata>>(missing)
            .unwrap()
            .is_valid()
    );
    business.confirm_terminal(cleaned).unwrap();
    assert_eq!(business.begin_close(cleaned), Err(PortError::Conflict));
    let retained = business.snapshot().unwrap();
    assert_eq!(retained.center.sandboxes().len(), 1);
    assert!(retained.center.pending_closes().is_empty());
    assert_eq!(
        retained
            .center
            .lane(&cleaned.identity.node, cleaned.identity.retention_slot)
            .unwrap()
            .phase(),
        sandproof_center_core::LanePhase::Open
    );
}

fn assert_settled_and_independent(state: &BusinessEnvelope<RegistryMetadata>) {
    assert!(state.is_valid());
    assert_eq!(
        state.metadata.registry().records()[0].value().state,
        PublicationState::Ready
    );
    assert_eq!(
        state.metadata.registry().records()[0].value().binding.owner,
        Some(8)
    );
    assert_eq!(
        state.metadata.registry().records()[0]
            .value()
            .validation_certificate
            .as_ref()
            .unwrap()
            .owner,
        1
    );
    assert_eq!(state.center.reserved(None, None), Some(Resources::ZERO));
    assert!(serde_json::to_vec(state).unwrap().len() < state.required_block_bytes());
    // The receipt preserves bindings, but absence is NOT retirement evidence:
    // only the matching slot frontier may authorize lifecycle removal.
    let mut retired = serde_json::to_value(state).unwrap();
    retired["center"]["sandboxes"] = serde_json::json!([]);
    let retired: BusinessEnvelope<RegistryMetadata> = serde_json::from_value(retired).unwrap();
    assert!(!retired.is_valid());
}

fn assert_settlement_tampering_refused(state: &BusinessEnvelope<RegistryMetadata>) {
    for case in 0..14 {
        let mut altered = serde_json::to_value(state).unwrap();
        let link = &mut altered["metadata"]["registry"]["records"][0]["validation"];
        match case {
            0 => {
                link["settlement"]["original_create"]["create"]["resources"]["cpu"] =
                    serde_json::json!(2);
            }
            1 => {
                link["settlement"]["original_create"]["create"]["timeout_ms"] =
                    serde_json::json!(4000);
            }
            2 => {
                link["settlement"]["original_create"]["identity"]["tenant_id"] =
                    serde_json::json!("9");
            }
            3 => {
                link["settlement"]["cleaned"]["identity"]["node"]["node_generation"] =
                    serde_json::json!(2);
            }
            4 => link["settlement"]["cleaned"]["fact"] = serde_json::json!({"kind":"stopped"}),
            5 => {
                link["settlement"]["original_create"]["identity"]["intent_version"] =
                    serde_json::json!(2);
            }
            6 => link["local"]["create_operation"] = serde_json::json!(999),
            7 => link["settlement"]["cleaned"]["identity"]["request_window"] = serde_json::json!(2),
            8 => link["settlement"] = serde_json::Value::Null,
            9 => {
                altered["center"]["nodes"][0]["identity"]["node_generation"] = serde_json::json!(2);
            }
            10 => {
                link["settlement"]["original_create"]["identity"]["retention_slot"] =
                    serde_json::json!(2);
            }
            11 => {
                link["settlement"]["cleaned"]["identity"]["retention_slot"] = serde_json::json!(2);
            }
            12 => {
                link["local"]["descriptor"]["guest"]["configuration"]["sha256"] =
                    serde_json::json!("f".repeat(64));
            }
            _ => link["local"]["descriptor"]["guest"] = serde_json::Value::Null,
        }
        let altered: BusinessEnvelope<RegistryMetadata> = serde_json::from_value(altered).unwrap();
        assert!(!altered.is_valid(), "mutation {case}");
    }
}

fn reject_changed_certificates(
    business: &SharedBusinessAuthority,
    terminal: &sandproof_protocol::internal::ControlIdentity,
    certificate: &ValidationCertificate,
) {
    for case in 0..4 {
        let mut bad = certificate.clone();
        match case {
            0 => bad.owner = 2,
            1 => bad.binding.rootfs.sha256 = "4".repeat(64),
            2 => bad.sandbox = sandbox_controller_core::SandboxId(11),
            _ => bad.candidate_id = format!("{:032x}", 2),
        }
        assert!(
            business
                .settle_validation(&candidate().candidate_id, terminal, bad)
                .is_err()
        );
        assert_eq!(
            business.snapshot().unwrap().metadata.registry().records()[0]
                .value()
                .state,
            PublicationState::Checking
        );
    }
}

#[test]
fn future_metadata_budget_is_required_before_any_publication_is_accepted() {
    let fixture = Engine::new();
    let state = envelope();
    let reserve = state.required_block_bytes();
    let (ledger, _) = RedisLedger::initialize(
        engine::endpoint(fixture.address),
        Engine::config(reserve - 1),
        fixture.lease(),
        &state,
    )
    .unwrap();
    assert!(matches!(
        CenterAuthority::attach(ledger),
        Err(AuthorityError::Store(StoreError::Configuration))
    ));
}
