//! Original windows and actual Redis, not a second private-VM controller.
use super::*;
use sandbox_controller_core::maintenance::windows::WindowLimits;
use sandproof_api::{BuildPort, BuildSettlementRequest, BuildVmRequest};
use sandproof_center_service::business::{BuildParameters, PrivateBuildConfig};
use sandproof_center_service::{CenterBuildPort, TemplateProvider};
use sandproof_internal::cache::{CacheAvailability, CacheRequest};
use sandproof_protocol::internal::{lifecycle::Intent, window::WindowCloseAck};
use std::sync::Arc;

fn provision(fixture: &Engine, proxy: &reply_loss::ReplyLoss) -> SharedBusinessAuthority {
    provision_with_policy(
        fixture,
        proxy,
        sandproof_center_service::LifecyclePolicy {
            max_timeout_secs: 5,
            max_lifetime_secs: 5,
            startup_timeout_secs: 1,
            public_ipv4_enabled: false,
        },
    )
}
fn provision_with_policy(
    fixture: &Engine,
    proxy: &reply_loss::ReplyLoss,
    policy: sandproof_center_service::LifecyclePolicy,
) -> SharedBusinessAuthority {
    let mut original = serde_json::to_value(envelope()).unwrap();
    let builder = wire::node("builder-a");
    let binding = descriptor("builder-base", None);
    let config = PrivateBuildConfig {
        windows: WindowLimits::new(2, 10_000).unwrap(),
        window_duration_ms: 10_000,
        max_anchors: 2,
        builder_revision: "builder-r1".try_into().unwrap(),
        builder_digest: binding.canonical_digest().unwrap(),
        policy,
    };
    original["metadata"]["private_builds"] = serde_json::json!({"config":config,"windows":{"limits":config.windows,"issued_through":null,"time_floor_ms":0,"open":[]},"anchors":[]});
    original["metadata"]["catalog"]["templates"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(&binding).unwrap());
    original["metadata"]["catalog"]["aliases"]["private-builder"] =
        serde_json::json!("builder-base");
    original["metadata"]["registrations"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"revision":"builder-r1","template_id":"builder-base"}));
    original["center"]["bounds"]["max_nodes"] = serde_json::json!(2);
    let mut registered = original["center"]["nodes"][0].clone();
    registered["identity"] = serde_json::to_value(&builder).unwrap();
    registered["role"] = serde_json::json!("builder");
    original["center"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(registered);
    let lanes = original["center"]["retention_lanes"]
        .as_array()
        .unwrap()
        .clone();
    for mut lane in lanes {
        lane["node"] = serde_json::to_value(&builder).unwrap();
        original["center"]["retention_lanes"]
            .as_array_mut()
            .unwrap()
            .push(lane);
    }
    let original: BusinessEnvelope<RegistryMetadata> = serde_json::from_value(original).unwrap();
    assert!(original.is_valid());
    let (ledger, _) = RedisLedger::initialize(
        engine::endpoint(proxy.address),
        Engine::config(original.required_block_bytes()),
        fixture.lease(),
        &original,
    )
    .unwrap();
    let business = SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap());
    let request = CacheRequest {
        node: builder,
        revision: config.builder_revision,
        digest: config.builder_digest,
        descriptor: binding,
    };
    business
        .register_cache(
            &request,
            &CacheAvailability {
                node: request.node.clone(),
                revision: request.revision.clone(),
                digest: request.digest,
            },
        )
        .unwrap();
    business
}
fn create_binding() -> CreateBinding {
    CreateBinding {
        template_revision: "builder-r1".try_into().unwrap(),
        template_digest: descriptor("builder-base", None).canonical_digest().unwrap(),
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
fn parameters() -> BuildParameters {
    BuildParameters {
        timeout_secs: 5,
        allow_internet_access: false,
    }
}

fn confirm_build_cleanup(
    business: &SharedBusinessAuthority,
    lifecycle: &mut SharedBusinessAuthority,
    create: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
) -> sandproof_protocol::internal::lifecycle::LifecycleResponse {
    let destroy = AuthorityPort::destroy(
        lifecycle,
        &create.identity.tenant_id,
        &create.identity.sandbox_id,
    )
    .unwrap();
    let cleaned = wire::response(&destroy, 1, LifecycleFact::Cleaned);
    AuthorityPort::apply_response(lifecycle, &cleaned).unwrap();
    business.confirm_terminal(&cleaned).unwrap();
    cleaned
}

#[test]
fn actual_private_admit_unknown_is_one_hidden_global_builder_create_and_closed_window_fences_replay()
 {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = provision(&fixture, &proxy);
    let tenant = "8".try_into().unwrap();
    let window = business.issue_build_window(&tenant, 0).unwrap();
    // AdvanceTime fresh+commit, then actual admission fresh+commit. Lose only
    // the final coupled commit; never invent a second sandbox after recovery.
    proxy.lose_after(3);
    assert_eq!(
        business.accept_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            &parameters(),
            create_binding(),
            0
        ),
        Err(PortError::OutcomeUnknown)
    );
    assert!(business.snapshot().is_err());
    let mut lifecycle = business.clone();
    assert!(AuthorityPort::snapshot(&mut lifecycle).is_err());
    AuthorityPort::recover(&mut lifecycle).unwrap();
    let original = business
        .build_anchor(&tenant, &window.request_window, &candidate().candidate_id)
        .unwrap()
        .unwrap()
        .original_create;
    assert_eq!(original.identity.node, wire::node("builder-a"));
    assert_eq!(original.identity.sandbox_id.as_str(), "sandbox-1");
    let view = AuthorityPort::public_snapshot(&mut lifecycle).unwrap();
    assert!(!view.visible(&view.state.sandboxes()[0]));
    let repeated = business
        .accept_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            &parameters(),
            create_binding(),
            1,
        )
        .unwrap();
    assert_eq!(repeated, original);
    let changed = BuildParameters {
        timeout_secs: 6,
        ..parameters()
    };
    assert_eq!(
        business.accept_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            &changed,
            create_binding(),
            1
        ),
        Err(PortError::Conflict)
    );
    assert_eq!(business.snapshot().unwrap().center.sandboxes().len(), 1);
    business
        .close_build_window(&tenant, &window.request_window, 1)
        .unwrap();
    let unknown = format!("{:032x}", 2);
    assert_eq!(
        business.accept_build(
            &tenant,
            &window.request_window,
            &unknown,
            &parameters(),
            create_binding(),
            2
        ),
        Err(PortError::Conflict)
    );
    assert_eq!(
        business.snapshot().unwrap().center.identity_frontiers(),
        (1, 1)
    );
}

#[test]
fn actual_private_settlement_precedes_close_and_only_exact_confirmed_retirement_removes_anchor() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = provision(&fixture, &proxy);
    let tenant = "8".try_into().unwrap();
    let window = business.issue_build_window(&tenant, 0).unwrap();
    let original = business
        .accept_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            &parameters(),
            create_binding(),
            0,
        )
        .unwrap();
    let mut lifecycle = business.clone();
    business
        .close_build_window(&tenant, &window.request_window, 1)
        .unwrap();
    assert_eq!(
        business.settle_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            Some(&original.identity.sandbox_id)
        ),
        Err(PortError::Conflict)
    );
    let destroy =
        AuthorityPort::destroy(&mut lifecycle, &tenant, &original.identity.sandbox_id).unwrap();
    assert_eq!(destroy.intent, Intent::Destroy);
    let cleaned = wire::response(&destroy, 1, LifecycleFact::Cleaned);
    AuthorityPort::apply_response(&mut lifecycle, &cleaned).unwrap();
    business.confirm_terminal(&cleaned).unwrap();
    // No close request can escape before the private build has acknowledged
    // settlement, even though physical cleanup is already durably confirmed.
    assert_eq!(business.begin_close(&cleaned), Err(PortError::Conflict));
    assert!(business.snapshot().is_ok());
    proxy.lose_after(1);
    assert_eq!(
        business.settle_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            Some(&original.identity.sandbox_id)
        ),
        Err(PortError::OutcomeUnknown)
    );
    AuthorityPort::recover(&mut lifecycle).unwrap();
    assert!(
        business
            .build_anchor(&tenant, &window.request_window, &candidate().candidate_id)
            .unwrap()
            .unwrap()
            .settlement
            .is_some()
    );
    let close = business.begin_close(&cleaned).unwrap();
    let acknowledgement = WindowCloseAck {
        terminal: close.terminal.clone(),
    };
    // Original apply_close_ack consumes its already confirmed view; unlike
    // settlement it commits directly, without a preceding fresh-view write.
    proxy.lose_after(0);
    assert_eq!(
        business.apply_close_ack(&acknowledgement),
        Err(PortError::OutcomeUnknown)
    );
    AuthorityPort::recover(&mut lifecycle).unwrap();
    let final_view = business.snapshot().unwrap();
    assert!(final_view.center.sandboxes().is_empty());
    assert!(final_view.metadata.private_sandboxes().is_empty());
    assert_eq!(
        final_view.center.reserved(None, None),
        Some(Resources::ZERO)
    );
    assert!(
        business
            .build_anchor(&tenant, &window.request_window, &candidate().candidate_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        business.accept_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            &parameters(),
            create_binding(),
            2
        ),
        Err(PortError::Conflict)
    );
}

#[test]
fn capacity_refused_builder_settles_exact_create_after_window_close_and_terminal_confirmation() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = provision(&fixture, &proxy);
    let tenant = "8".try_into().unwrap();
    let window = business.issue_build_window(&tenant, 0).unwrap();
    let original = business
        .accept_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            &parameters(),
            create_binding(),
            0,
        )
        .unwrap();
    let mut lifecycle = business.clone();
    let refused = wire::response(&original, 1, LifecycleFact::NotAccepted);
    AuthorityPort::apply_response(&mut lifecycle, &refused).unwrap();
    let settle = || {
        business.settle_build(
            &tenant,
            &window.request_window,
            &candidate().candidate_id,
            Some(&original.identity.sandbox_id),
        )
    };
    assert_eq!(settle(), Err(PortError::Conflict));
    business
        .close_build_window(&tenant, &window.request_window, 1)
        .unwrap();
    assert_eq!(settle(), Err(PortError::Conflict));
    business.confirm_terminal(&refused).unwrap();
    // The original pre-admission receipt releases resources, but not its
    // business pin. A lost settlement reply fences both views until recovery.
    assert_eq!(business.begin_close(&refused), Err(PortError::Conflict));
    proxy.lose_after(1);
    assert_eq!(settle(), Err(PortError::OutcomeUnknown));
    assert!(business.snapshot().is_err());
    AuthorityPort::recover(&mut lifecycle).unwrap();
    let settled = business.snapshot().unwrap();
    let anchor = &settled.metadata.private_build_anchors()[0];
    assert_eq!(anchor.original_create, original);
    assert_eq!(anchor.settlement.as_ref(), Some(&refused));
    assert_eq!(settled.center.sandboxes()[0].operations.len(), 1);
    assert_eq!(settled.center.reserved(None, None), Some(Resources::ZERO));
    let mut unconfirmed = serde_json::to_value(&settled).unwrap();
    unconfirmed["center"]["sandboxes"][0]["terminal_confirmation"] = serde_json::Value::Null;
    let invalid: BusinessEnvelope<RegistryMetadata> = serde_json::from_value(unconfirmed).unwrap();
    assert!(!invalid.is_valid());
    let close = business.begin_close(&refused).unwrap();
    assert_eq!(close.terminal, refused);
    business
        .apply_close_ack(&WindowCloseAck {
            terminal: close.terminal,
        })
        .unwrap();
    let retired = business.snapshot().unwrap();
    assert!(retired.center.sandboxes().is_empty());
    assert!(retired.metadata.private_build_anchors().is_empty());
    assert!(retired.is_valid());
    assert_eq!(settle(), Err(PortError::Conflict));
}

#[test]
fn one_unsettled_build_does_not_pin_a_different_build_lane_or_global_authority() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = provision(&fixture, &proxy);
    let tenant = "8".try_into().unwrap();
    let first_window = business.issue_build_window(&tenant, 0).unwrap();
    let peer_window = business.issue_build_window(&tenant, 0).unwrap();
    let peer_job = format!("{:032x}", 2);
    let pinned = business
        .accept_build(
            &tenant,
            &first_window.request_window,
            &candidate().candidate_id,
            &parameters(),
            create_binding(),
            0,
        )
        .unwrap();
    let peer = business
        .accept_build(
            &tenant,
            &peer_window.request_window,
            &peer_job,
            &parameters(),
            create_binding(),
            0,
        )
        .unwrap();
    assert_ne!(pinned.identity.retention_slot, peer.identity.retention_slot);
    let mut lifecycle = business.clone();
    business
        .close_build_window(&tenant, &first_window.request_window, 1)
        .unwrap();
    let pinned_cleaned = confirm_build_cleanup(&business, &mut lifecycle, &pinned);
    assert_eq!(
        business.begin_close(&pinned_cleaned),
        Err(PortError::Conflict)
    );
    let original = business
        .build_anchor(
            &tenant,
            &first_window.request_window,
            &candidate().candidate_id,
        )
        .unwrap()
        .unwrap();
    assert!(business.snapshot().is_ok());
    business
        .close_build_window(&tenant, &peer_window.request_window, 1)
        .unwrap();
    let peer_cleaned = confirm_build_cleanup(&business, &mut lifecycle, &peer);
    business
        .settle_build(
            &tenant,
            &peer_window.request_window,
            &peer_job,
            Some(&peer.identity.sandbox_id),
        )
        .unwrap();
    let close = business.begin_close(&peer_cleaned).unwrap();
    business
        .apply_close_ack(&WindowCloseAck {
            terminal: close.terminal,
        })
        .unwrap();
    let retained = business.snapshot().unwrap();
    assert_eq!(retained.center.sandboxes().len(), 1);
    assert_eq!(
        serde_json::to_value(retained.metadata.private_build_anchors()).unwrap(),
        serde_json::to_value(std::slice::from_ref(&original)).unwrap()
    );
    assert_eq!(
        retained
            .center
            .lane(&pinned.identity.node, pinned.identity.retention_slot)
            .unwrap()
            .retired_through(),
        0
    );
    assert!(
        business
            .build_anchor(&tenant, &peer_window.request_window, &peer_job)
            .unwrap()
            .is_none()
    );
    let next_window = business.issue_build_window(&tenant, 2).unwrap();
    let next = business
        .accept_build(
            &tenant,
            &next_window.request_window,
            &format!("{:032x}", 3),
            &parameters(),
            create_binding(),
            2,
        )
        .unwrap();
    assert_eq!(next.identity.retention_slot, peer.identity.retention_slot);
    assert_eq!(next.identity.request_window.get(), 2);
}

#[test]
fn actual_private_driver_reuses_global_lifecycle_and_original_token_but_never_public_visibility() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = provision(&fixture, &proxy);
    let (templates, app, _) = super::templates::application(&business, true);
    let tenant = "8".try_into().unwrap();
    for name in ["builder-base", "private-builder"] {
        assert!(matches!(
            templates.resolve(&tenant, name),
            Err(PortError::NotFound)
        ));
        assert!(matches!(templates.get("8", name), Err(PortError::NotFound)));
    }
    assert!(templates.list("8", 10, None).unwrap().items.is_empty());
    assert!(templates.resolve(&tenant, "base").is_ok());
    let app = Arc::new(app);
    let port = CenterBuildPort::new(app.clone());
    let window = port.issue_build_window("8").unwrap();
    let request = BuildVmRequest {
        request_window: Some(window.request_window.clone()),
        timeout: 5,
        allow_internet_access: false,
    };
    let vm = port
        .build_vm("8", &candidate().candidate_id, Some(request.clone()))
        .unwrap();
    assert_eq!(vm.state, "Running");
    let session = vm.session.unwrap();
    assert!(matches!(
        app.get("8", &vm.sandbox_id),
        Err(PortError::NotFound)
    ));
    assert!(matches!(
        app.resolve_data_route(&session.envd_access_token),
        Err(PortError::NotFound)
    ));
    assert!(matches!(
        app.resolve_private_data_route("9", &session.envd_access_token),
        Err(PortError::NotFound)
    ));
    let routed = app
        .resolve_private_data_route("8", &session.envd_access_token)
        .unwrap();
    assert_eq!(routed.request.identity.node, wire::node("builder-a"));
    assert_eq!(routed.request.identity.sandbox_id.as_str(), vm.sandbox_id);
    assert!(matches!(
        port.kill_build_vm("8", "sandbox-999"),
        Err(PortError::NotFound)
    ));
    port.close_build_window("8", &window.request_window)
        .unwrap();
    port.kill_build_vm("8", &vm.sandbox_id).unwrap();
    assert!(matches!(
        app.resolve_private_data_route("8", &session.envd_access_token),
        Err(PortError::NotFound)
    ));
    let tombstone = port
        .build_vm("8", &candidate().candidate_id, Some(request))
        .unwrap();
    assert_eq!(tombstone.sandbox_id, vm.sandbox_id);
    assert!(tombstone.cleanup_complete && tombstone.session.is_none());
    port.settle_build_vm(
        "8",
        &candidate().candidate_id,
        BuildSettlementRequest {
            request_window: window.request_window,
            sandbox_id: Some(vm.sandbox_id),
        },
    )
    .unwrap();
    assert_eq!(
        business.snapshot().unwrap().center.identity_frontiers().0,
        1
    );
}

#[test]
fn actual_builder_policy_does_not_widen_runtime_network_or_lifetime_limits() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let policy = sandproof_center_service::LifecyclePolicy {
        max_timeout_secs: 600,
        max_lifetime_secs: 900,
        startup_timeout_secs: 60,
        public_ipv4_enabled: true,
    };
    let business = provision_with_policy(&fixture, &proxy, policy);
    let (_, app, _) = super::templates::application(&business, true);
    let app = Arc::new(app);
    let port = CenterBuildPort::new(app.clone());
    let window = port.issue_build_window("8").unwrap();
    let vm = port
        .build_vm(
            "8",
            &candidate().candidate_id,
            Some(BuildVmRequest {
                request_window: Some(window.request_window.clone()),
                timeout: 600,
                allow_internet_access: true,
            }),
        )
        .unwrap();
    assert_eq!(vm.state, "Running");
    let snapshot = business.snapshot().unwrap();
    let original = &snapshot.metadata.private_build_anchors()[0].original_create;
    assert_eq!(original.identity.node, wire::node("builder-a"));
    assert_eq!(original.create.timeout_ms, 600_000);
    assert_eq!(original.create.max_lifetime_ms, 900_000);
    assert_eq!(original.create.startup_deadline_ms, 60_000);
    let options: CreateOptions = serde_json::from_slice(&original.create.options).unwrap();
    assert_eq!(
        options.networking.unwrap().allow_internet_access,
        Some(true)
    );
    let public = sandproof_controller_service::application::CreateCommand {
        template_id: "base".into(),
        timeout: Some(300),
        env_vars: None,
        metadata: None,
        allow_internet_access: Some(true),
        network: None,
    };
    assert!(matches!(
        app.create("8", public.clone()),
        Err(PortError::Invalid)
    ));
    assert!(matches!(
        app.create(
            "8",
            sandproof_controller_service::application::CreateCommand {
                timeout: Some(600),
                allow_internet_access: Some(false),
                ..public
            }
        ),
        Err(PortError::Invalid)
    ));
    assert_eq!(
        business.snapshot().unwrap().center.identity_frontiers().0,
        1
    );
    let mut lifecycle = business.clone();
    AuthorityPort::recover(&mut lifecycle).unwrap();
    let recovered = business.snapshot().unwrap();
    assert_eq!(
        recovered.metadata.private_build_config().unwrap().policy,
        policy
    );
    assert!(recovered.metadata.registration_matches(&snapshot.metadata));
    let mut changed = serde_json::to_value(&snapshot.metadata).unwrap();
    changed["private_builds"]["config"]["policy"]["max_lifetime_secs"] = serde_json::json!(901);
    let changed: RegistryMetadata = serde_json::from_value(changed).unwrap();
    assert!(!recovered.metadata.registration_matches(&changed));
}

#[test]
fn actual_disabled_builder_network_rejects_before_global_admission() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = provision(&fixture, &proxy);
    let (_, app, _) = super::templates::application(&business, true);
    let port = CenterBuildPort::new(Arc::new(app));
    let window = port.issue_build_window("8").unwrap();
    assert!(matches!(
        port.build_vm(
            "8",
            &candidate().candidate_id,
            Some(BuildVmRequest {
                request_window: Some(window.request_window),
                timeout: 5,
                allow_internet_access: true,
            })
        ),
        Err(PortError::Invalid)
    ));
    assert_eq!(
        business.snapshot().unwrap().center.identity_frontiers(),
        (0, 0)
    );
}

#[test]
fn actual_static_runtime_and_builder_catalog_do_not_consume_dynamic_publication_quota() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = provision(&fixture, &proxy);
    assert_eq!(
        business
            .snapshot()
            .unwrap()
            .metadata
            .registry()
            .config()
            .max_templates_global,
        2
    );
    for (number, owner) in [(1_u64, 8_u64), (2, 9)] {
        let mut value = candidate();
        value.candidate_id = format!("{number:032x}");
        value.binding = descriptor(&format!("tpl-{}", value.candidate_id), Some(owner));
        business
            .begin_publication(value, format!("candidate-{number}-r1").try_into().unwrap())
            .unwrap();
    }
    assert_eq!(
        business
            .snapshot()
            .unwrap()
            .metadata
            .registry()
            .records()
            .len(),
        2
    );
    let mut third = candidate();
    third.candidate_id = format!("{:032x}", 3);
    third.binding = descriptor(&format!("tpl-{}", third.candidate_id), Some(10));
    assert_eq!(
        business.begin_publication(third, "candidate-3-r1".try_into().unwrap()),
        Err(PortError::QuotaExceeded)
    );
    assert_eq!(
        business
            .snapshot()
            .unwrap()
            .metadata
            .registry()
            .records()
            .len(),
        2
    );
}
