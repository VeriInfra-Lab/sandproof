//! Retired validation history must not pin a reclaimed host tenant mapping.
use super::*;

fn two_publication_authority(
    fixture: &Engine,
    proxy: &reply_loss::ReplyLoss,
) -> SharedBusinessAuthority {
    let mut value = serde_json::to_value(envelope()).unwrap();
    value["metadata"]["registry"]["config"]["max_templates_per_tenant"] = 2.into();
    value["center"]["tenants"].as_array_mut().unwrap().push(
        serde_json::to_value(TenantBudget {
            tenant: "9".try_into().unwrap(),
            limit: resources(2),
        })
        .unwrap(),
    );
    let registered = value["center"]["lifecycle"]["windows"].as_array().unwrap().clone();
    for mut window in registered {
        window["registration"]["owner"] = serde_json::json!("9");
        value["center"]["lifecycle"]["windows"].as_array_mut().unwrap().push(window);
    }
    let state: BusinessEnvelope<RegistryMetadata> = serde_json::from_value(value).unwrap();
    assert!(state.is_valid());
    let (ledger, _) = RedisLedger::initialize(
        engine::endpoint(proxy.address),
        Engine::config(state.required_block_bytes()),
        fixture.lease(),
        &state,
    )
    .unwrap();
    SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap())
}

fn settle_first(
    business: &SharedBusinessAuthority,
) -> TerminalLifecycleObservation {
    let mut lifecycle = business.clone();
    business.begin_publication(candidate(), revision()).unwrap();
    cache_candidate(business);
    let create = business
        .create_validation(&candidate().candidate_id, &lifecycle_parameters(), 0)
        .unwrap();
    let (mapping, certificate) = local_certificate(true);
    business
        .bind_local_validation(&candidate().candidate_id, &create, &mapping)
        .unwrap();
    AuthorityPort::accept_lifecycle_create_observation(&mut lifecycle, &observed_ready(&create))
    .unwrap();
    let destroy = AuthorityPort::destroy_bound_create(&mut lifecycle, &create)
    .unwrap();
    let cleaned = cleaned_observation(&destroy);
    AuthorityPort::accept_lifecycle_terminal_observation(&mut lifecycle, &cleaned, 100).unwrap();
    business
        .settle_validation(&candidate().candidate_id, &cleaned, certificate)
        .unwrap();
    cleaned
}

#[test]
fn later_publication_can_bind_a_new_local_owner_after_exact_original_retirement() {
    later_mapping(true, 8, 2, true);
}

#[test]
fn live_mapping_cannot_change_before_original_window_retirement() {
    later_mapping(false, 8, 2, false);
    later_mapping(false, 8, 1, true);
}

#[test]
fn retirement_never_allows_another_global_tenant_to_reuse_the_local_owner() {
    later_mapping(true, 9, 1, false);
    later_mapping(true, 9, 2, true);
}

fn later_mapping(retire: bool, global_owner: u64, local_owner: u64, accepted: bool) {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = two_publication_authority(&fixture, &proxy);
    let cleaned = settle_first(&business);
    business.confirm_terminal(&ConfirmedLifecycleResult { result: cleaned.clone() }).unwrap();
    if retire {
        let close = business.begin_close(&cleaned, 200).unwrap();
        business
            .apply_close_ack(&ClosedLifecycleWindow {
                retired_through: close.terminal.request.retention.window,
                terminal: close.terminal,
            }, 200)
            .unwrap();
    }
    let before = business.snapshot().unwrap();
    assert_eq!(before.center.sandboxes().is_empty(), retire);
    let original = before.metadata.registry().records()[0].clone();

    let mut next = candidate();
    next.binding.owner = Some(global_owner);
    next.candidate_id = format!("{:032x}", 2);
    next.binding.template_id = format!("tpl-{}", next.candidate_id);
    let revision: ResourceId = "candidate-2-r1".try_into().unwrap();
    let digest = next.binding.canonical_digest().unwrap();
    business
        .begin_publication(next.clone(), revision.clone())
        .unwrap();
    let request = sandproof_internal::cache::CacheRequest {
        node: wire::node("host-a"),
        revision: revision.clone(),
        digest,
        descriptor: next.binding.clone(),
    };
    business
        .register_cache(
            &request,
            &sandproof_internal::cache::CacheAvailability {
                node: request.node.clone(),
                revision: revision.clone(),
                digest,
            },
        )
        .unwrap();
    let mut parameters = lifecycle_parameters();
    parameters.template_revision = revision;
    parameters.template_digest = digest;
    let create = business
        .create_validation(&next.candidate_id, &parameters, 20)
        .unwrap();
    let mut local = local_certificate(true).0;
    local.owner = local_owner;
    local.sandbox = sandbox_controller_core::SandboxId(2);
    local.descriptor = next.binding.clone();
    local.descriptor.owner = Some(local.owner);
    local.descriptor.kernel.path = "/host/cache/kernel".into();
    local.descriptor.rootfs.path = "/host/cache/rootfs".into();
    assert!(local.is_projection_of(&next.binding));
    let result = business.bind_local_validation(&next.candidate_id, &create, &local);
    if accepted {
        result.unwrap();
    } else {
        assert!(result.is_err());
        AuthorityPort::recover(&mut business.clone()).unwrap();
    }
    let after = business.snapshot().unwrap();
    assert!(after.is_valid());
    assert_eq!(after.metadata.registry().records()[0], original);
    assert_eq!(
        after.metadata.registry().records()[1]
            .validation()
            .unwrap()
            .local
            .as_ref(),
        accepted.then_some(&local)
    );
}
