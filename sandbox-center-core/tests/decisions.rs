#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use fixture::*;
use sandproof_center_core::*;
use sandproof_protocol::internal::lifecycle::LifecycleFact;

fn registration(name: &str) -> NodeRegistration {
    NodeRegistration {
        role: sandproof_center_core::NodeRole::Runtime,
        identity: node(name),
        limit: resources(2),
        mode: NodeMode::Accepting,
        healthy_until_ms: 2000,
        free: resources(2),
        templates: vec![sandproof_center_core::AvailableTemplate {
            revision: binding().template_revision,
            digest: binding().template_digest,
        }],
        protocol_version: 1,
    }
}
fn state() -> CenterState {
    CenterState::new(
        node("host-a").deployment_id,
        Bounds {
            max_nodes: 2,
            max_templates_per_node: 16,
            max_sandboxes: 8,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            platform: resources(4),
        },
        vec![registration("host-b"), registration("host-a")],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources(4),
        }],
    )
    .unwrap()
}
fn ready(
    state: &CenterState,
    request: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
) -> CenterState {
    state
        .apply_response(&response(
            request,
            1,
            LifecycleFact::Ready {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600,
            },
        ))
        .unwrap()
}

#[test]
fn deterministic_balanced_fixed_binding_and_capacity() {
    let s = state();
    let tenant = "1".try_into().unwrap();
    let (s, first) = s.create(&tenant, binding(), 0).unwrap();
    assert_eq!(first.identity.node, node("host-a"));
    let (s, second) = s.create(&tenant, binding(), 0).unwrap();
    assert_eq!(second.identity.node, node("host-b"));
    let (s, _) = s.create(&tenant, binding(), 0).unwrap();
    let (s, _) = s.create(&tenant, binding(), 0).unwrap();
    assert_eq!(
        s.create(&tenant, binding(), 0),
        Err(DecisionError::Capacity)
    );
    assert_eq!(s.pending()[0], first);
    assert_eq!(s.reserved(None, None), Some(resources(4)));
    assert!(s.is_valid());
}
#[test]
fn stale_health_physical_free_templates_and_draining_are_vetoes() {
    let s = state();
    let tenant = "1".try_into().unwrap();
    let mut a = registration("host-a");
    a.mode = NodeMode::Draining;
    let s = s.observe_node(a).unwrap();
    let mut b = registration("host-b");
    b.free.disk_bytes = 0;
    let s = s.observe_node(b.clone()).unwrap();
    assert_eq!(
        s.create(&tenant, binding(), 0),
        Err(DecisionError::Capacity)
    );
    b.free = resources(2);
    b.healthy_until_ms = 0;
    let s = s.observe_node(b.clone()).unwrap();
    assert_eq!(
        s.create(&tenant, binding(), 0),
        Err(DecisionError::Unavailable)
    );
    b.healthy_until_ms = 2000;
    b.templates.clear();
    let s = s.observe_node(b).unwrap();
    assert_eq!(
        s.create(&tenant, binding(), 0),
        Err(DecisionError::Unavailable)
    );
}
#[test]
fn unknown_holds_quota_destroy_supersedes_late_ready_and_stopped_is_not_cleaned() {
    let tenant = "1".try_into().unwrap();
    let (s, create) = state().create(&tenant, binding(), 0).unwrap();
    let (s, destroy) = s.destroy(&tenant, &create.identity.sandbox_id).unwrap();
    assert!(matches!(
        s.apply_response(&response(
            &create,
            1,
            LifecycleFact::Ready {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600
            }
        )),
        Err(DecisionError::Stale)
    ));
    assert_eq!(s.pending(), vec![destroy.clone()]);
    let s = s
        .apply_response(&response(&destroy, 2, LifecycleFact::Stopped))
        .unwrap();
    assert_eq!(s.reserved(None, None), Some(resources(1)));
    let s = s
        .apply_response(&response(&destroy, 3, LifecycleFact::Cleaned))
        .unwrap();
    assert_eq!(s.reserved(None, None), Some(resources(0)));
    assert!(s.pending().is_empty());
    assert_eq!(
        s.destroy(&tenant, &create.identity.sandbox_id).unwrap().1,
        destroy
    );
}
#[test]
fn lifetime_is_original_and_renewal_cannot_consume_destroy_reserve() {
    let tenant = "1".try_into().unwrap();
    let (s, create) = state().create(&tenant, binding(), 0).unwrap();
    let s = ready(&s, &create);
    assert_eq!(
        s.renew(&tenant, &create.identity.sandbox_id, 601, 150),
        Err(DecisionError::Conflict)
    );
    let (s, renew) = s
        .renew(&tenant, &create.identity.sandbox_id, 400, 150)
        .unwrap();
    assert_eq!(s.sandboxes()[0].end_at_ms, Some(200));
    let fact = response(&renew, 2, LifecycleFact::Renewed { end_at_ms: 400 });
    let s = s.apply_response(&fact).unwrap();
    assert_eq!(s.apply_response(&fact).unwrap(), s);
    assert_eq!(
        s.renew(&tenant, &create.identity.sandbox_id, 500, 160),
        Err(DecisionError::Capacity)
    );
    assert!(s.destroy(&tenant, &create.identity.sandbox_id).is_ok());
    assert_eq!(s.sandboxes()[0].started_at_ms, Some(100));
}
#[test]
fn wrong_owner_intent_generation_and_observation_cannot_publish() {
    let tenant = "1".try_into().unwrap();
    let (s, create) = state().create(&tenant, binding(), 0).unwrap();
    assert_eq!(
        s.destroy(&"2".try_into().unwrap(), &create.identity.sandbox_id),
        Err(DecisionError::NotFound)
    );
    let recorded = response(&create, 2, LifecycleFact::Recorded);
    let s = s.apply_response(&recorded).unwrap();
    assert_eq!(
        s.apply_response(&response(&create, 1, LifecycleFact::NotAccepted)),
        Err(DecisionError::Stale)
    );
    let mut bad = response(&create, 3, LifecycleFact::NotAccepted);
    bad.identity.node.node_generation = 2.try_into().unwrap();
    assert_eq!(s.apply_response(&bad), Err(DecisionError::Stale));
    let s = s
        .apply_response(&response(&create, 3, LifecycleFact::NotAccepted))
        .unwrap();
    assert_eq!(s.reserved(None, None), Some(resources(0)));
    assert!(s.is_valid());
}
#[test]
fn u64_max_ratios_are_exact_without_overflow() {
    let mut a = registration("host-a");
    let mut b = registration("host-b");
    a.limit.cpu = u64::MAX;
    b.limit.cpu = u64::MAX;
    a.free.cpu = u64::MAX;
    b.free.cpu = u64::MAX;
    let s = CenterState::new(
        node("host-a").deployment_id,
        Bounds {
            max_nodes: 2,
            max_templates_per_node: 16,
            max_sandboxes: 8,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            platform: resources(4),
        },
        vec![a, b],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources(4),
        }],
    )
    .unwrap();
    let (s, first) = s.create(&"1".try_into().unwrap(), binding(), 0).unwrap();
    let (_, second) = s.create(&"1".try_into().unwrap(), binding(), 0).unwrap();
    assert_eq!(first.identity.node, node("host-a"));
    assert_eq!(second.identity.node, node("host-b"));
}

#[test]
fn tenant_quota_does_not_block_another_tenant_and_template_digest_must_match() {
    let s = CenterState::new(
        node("host-a").deployment_id,
        state().bounds().clone(),
        vec![registration("host-a"), registration("host-b")],
        vec![
            TenantBudget {
                tenant: "1".try_into().unwrap(),
                limit: resources(2),
            },
            TenantBudget {
                tenant: "2".try_into().unwrap(),
                limit: resources(2),
            },
        ],
    )
    .unwrap();
    let one = "1".try_into().unwrap();
    let two = "2".try_into().unwrap();
    let (s, _) = s.create(&one, binding(), 0).unwrap();
    let (s, _) = s.create(&one, binding(), 0).unwrap();
    assert_eq!(s.create(&one, binding(), 0), Err(DecisionError::Capacity));
    assert!(s.create(&two, binding(), 0).is_ok());
    let mut wrong = binding();
    wrong.template_digest = [2; 32];
    assert_eq!(s.create(&two, wrong, 0), Err(DecisionError::Unavailable));
}

#[test]
fn private_build_role_reuses_admission_without_consuming_runtime_pool() {
    let mut builder = registration("host-a");
    builder.role = NodeRole::Builder;
    let s = state().observe_node(builder);
    assert_eq!(s, Err(DecisionError::Conflict));
    let mut builder = registration("host-a");
    builder.role = NodeRole::Builder;
    let initial = CenterState::new(
        node("host-a").deployment_id,
        state().bounds().clone(),
        vec![builder, registration("host-b")],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources(4),
        }],
    )
    .unwrap();
    let tenant = "1".try_into().unwrap();
    let (normal, public) = initial.create(&tenant, binding(), 0).unwrap();
    assert_eq!(public.identity.node, node("host-b"));
    let (both, private) = normal.create_builder(&tenant, binding(), 0).unwrap();
    assert_eq!(private.identity.node, node("host-a"));
    assert_ne!(public.identity.sandbox_id, private.identity.sandbox_id);
    assert_ne!(public.identity.operation_id, private.identity.operation_id);
    assert_eq!(
        both.reserved(None, Some(&node("host-b"))),
        Some(resources(1))
    );
    assert_eq!(
        both.reserved(None, Some(&node("host-a"))),
        Some(resources(1))
    );
    assert_eq!(both.pending(), vec![public, private]);
    assert!(both.is_valid());
}
