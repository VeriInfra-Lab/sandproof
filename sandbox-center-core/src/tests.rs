#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use crate::*;
use fixture::*;
use sandproof_protocol::internal::lifecycle::{LifecycleFact, Resources};
#[path = "window_tests.rs"]
mod windows;
fn state() -> CenterState {
    CenterState::new(
        node("host-a").deployment_id,
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 16,
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            platform: resources(2),
        },
        vec![NodeRegistration {
            identity: node("host-a"),
            retention_slots: 2,
            role: NodeRole::Runtime,
            limit: resources(2),
            mode: NodeMode::Accepting,
            healthy_until_ms: 2000,
            free: resources(2),
            templates: vec![crate::AvailableTemplate {
                revision: binding().template_revision,
                digest: binding().template_digest,
            }],
            protocol_version: sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION,
        }],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources(2),
        }],
    )
    .unwrap()
}
#[test]
fn registration_checks_static_subset_without_resetting_dynamic_facts() {
    let configured = state();
    let mut current = configured.clone();
    current.nodes[0].templates.push(AvailableTemplate {
        revision: "other-revision".try_into().unwrap(),
        digest: [8; 32],
    });
    current.nodes[0].healthy_until_ms = 0;
    current.nodes[0].free = Resources::ZERO;
    assert!(current.registration_matches(&configured));
    current.nodes[0].templates.remove(0);
    assert!(!current.registration_matches(&configured));
    let mut changed = configured.clone();
    changed.nodes[0].identity.node_generation = 2.try_into().unwrap();
    assert!(!current.registration_matches(&changed));
}

#[test]
fn physical_refresh_cannot_forget_unknown_responsibility_or_change_policy() {
    let (original, request) = state()
        .create(&"1".try_into().unwrap(), binding(), 0)
        .unwrap();
    let refreshed = original
        .refresh_node(&node("host-a"), 0, Resources::ZERO)
        .unwrap();
    assert!(refreshed.is_valid());
    assert_eq!(refreshed.pending(), vec![request]);
    assert_eq!(
        refreshed.reserved(None, None),
        original.reserved(None, None)
    );
    assert_eq!(refreshed.retention_lanes, original.retention_lanes);
    assert_eq!(refreshed.nodes[0].templates, original.nodes[0].templates);
    assert_eq!(refreshed.nodes[0].mode, original.nodes[0].mode);
    assert_eq!(
        refreshed.create(&"1".try_into().unwrap(), binding(), 0),
        Err(DecisionError::Unavailable)
    );
    assert_eq!(
        refreshed.identity_frontiers(),
        original.identity_frontiers()
    );
    assert_eq!(
        original.refresh_node(&node("host-a"), 3000, resources(3)),
        Err(DecisionError::Invalid)
    );
    assert_eq!(
        original.refresh_node(&node("host-b"), 3000, Resources::ZERO),
        Err(DecisionError::NotFound)
    );
}
#[test]
fn stale_or_vetoed_health_blocks_new_renewals_but_not_original_destroy() {
    let tenant = "1".try_into().unwrap();
    let (created, request) = state().create(&tenant, binding(), 0).unwrap();
    let running = created
        .apply_response(&response(
            &request,
            1,
            LifecycleFact::Ready {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600,
            },
        ))
        .unwrap();
    for healthy_until in [0, 149, 150] {
        let stale = running
            .refresh_node(&node("host-a"), healthy_until, Resources::ZERO)
            .unwrap();
        assert_eq!(
            stale.renew(&tenant, &request.identity.sandbox_id, 400, 150),
            Err(DecisionError::Unavailable)
        );
        assert_eq!(stale.identity_frontiers(), running.identity_frontiers());
        assert_eq!(stale.pending(), running.pending());
        let (terminating, destroy) = stale
            .destroy(&tenant, &request.identity.sandbox_id)
            .unwrap();
        assert_eq!(
            destroy.intent,
            sandproof_protocol::internal::lifecycle::Intent::Destroy
        );
        assert!(terminating.sandboxes()[0].terminating);
    }
    let fresh = running
        .refresh_node(&node("host-a"), 151, Resources::ZERO)
        .unwrap()
        .begin_drain(&node("host-a"))
        .unwrap();
    assert!(
        fresh
            .renew(&tenant, &request.identity.sandbox_id, 400, 150)
            .is_ok()
    );
}

#[test]
fn identity_exhaustion_reserves_the_final_destroy_id() {
    let mut s = state();
    s.operation_highwater = u64::MAX - 1;
    assert!(s.is_valid());
    assert_eq!(
        s.create(&"1".try_into().unwrap(), binding(), 0),
        Err(DecisionError::Exhausted)
    );
    s.operation_highwater = u64::MAX - 2;
    let (s, create) = s.create(&"1".try_into().unwrap(), binding(), 0).unwrap();
    let s = s
        .apply_response(&response(
            &create,
            1,
            LifecycleFact::Ready {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600,
            },
        ))
        .unwrap();
    assert_eq!(
        s.renew(
            &"1".try_into().unwrap(),
            &create.identity.sandbox_id,
            400,
            150
        ),
        Err(DecisionError::Exhausted)
    );
    let (s, destroy) = s
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    assert_eq!(
        destroy.identity.operation_id.as_str(),
        format!("operation-{}", u64::MAX)
    );
    assert!(s.is_valid());
}
#[test]
fn recovery_rejects_corrupt_projection_binding_and_capacity() {
    let (s, create) = state()
        .create(&"1".try_into().unwrap(), binding(), 0)
        .unwrap();
    let mut bad = s.clone();
    bad.sandboxes[0].released = true;
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    bad.sandboxes[0].operations[0]
        .request
        .identity
        .node
        .node_generation = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    bad.sandboxes[0].end_at_ms = Some(200);
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    bad.sandboxes[0].operations[0].request.identity.operation_id =
        "operation-00".try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    bad.operation_highwater = u64::MAX;
    assert!(!bad.is_valid());
    assert!(s.pending().contains(&create));
}

#[test]
fn recovery_keeps_operation_ids_unique_across_live_sandbox_histories() {
    let tenant = "1".try_into().unwrap();
    let (first, first_request) = state().create(&tenant, binding(), 0).unwrap();
    let (valid, second_request) = first.create(&tenant, binding(), 0).unwrap();
    assert_ne!(
        first_request.identity.retention_slot,
        second_request.identity.retention_slot
    );
    assert!(valid.is_valid());

    let mut duplicate = valid.clone();
    duplicate.sandboxes[1].operations[0]
        .request
        .identity
        .operation_id = first_request.identity.operation_id;
    let before = duplicate.clone();
    assert!(!duplicate.is_valid());
    assert_eq!(duplicate, before);
    assert_eq!(
        duplicate.create(&tenant, binding(), 0),
        Err(DecisionError::Invalid)
    );
    assert_eq!(duplicate, before);
}

#[test]
fn create_keeps_first_rejection_when_multiple_checks_would_fail() {
    let tenant = "1".try_into().unwrap();
    let missing_tenant = "missing".try_into().unwrap();
    let (one, _) = state().create(&tenant, binding(), 0).unwrap();
    let (full, _) = one.create(&tenant, binding(), 0).unwrap();
    let mut invalid_full = full.clone();
    invalid_full.sandboxes[0].operations.clear();

    let mut unavailable = state();
    unavailable.nodes[0].mode = NodeMode::Draining;
    unavailable.sandbox_highwater = u64::MAX;
    let mut too_large = binding();
    too_large.resources.cpu = 3;
    assert!(too_large.is_valid());

    let mut exhausted = state();
    exhausted.sandbox_highwater = u64::MAX;
    for (image, owner, create, expected) in [
        (
            invalid_full,
            &missing_tenant,
            binding(),
            DecisionError::Invalid,
        ),
        (full, &missing_tenant, binding(), DecisionError::Capacity),
        (
            unavailable.clone(),
            &missing_tenant,
            too_large.clone(),
            DecisionError::NotFound,
        ),
        (
            unavailable.clone(),
            &tenant,
            too_large,
            DecisionError::Capacity,
        ),
        (unavailable, &tenant, binding(), DecisionError::Unavailable),
        (exhausted, &tenant, binding(), DecisionError::Exhausted),
    ] {
        let before = image.clone();
        assert_eq!(image.create(owner, create, 0), Err(expected));
        assert_eq!(image, before);
    }
}

#[test]
fn position_queries_keep_first_match_and_original_available_slot_order() {
    let original = state();
    let host = node("host-a");
    assert!(std::ptr::eq(
        original.lane(&host, 1).unwrap(),
        std::ptr::from_ref(&original.retention_lanes[0])
    ));
    assert!(original.lane(&node("host-b"), 1).is_none());
    let mut wrong_generation = host.clone();
    wrong_generation.node_generation = 2.try_into().unwrap();
    assert!(original.lane(&wrong_generation, 1).is_none());
    assert!(original.lane(&host, 0).is_none());
    assert!(original.lane(&host, 3).is_none());
    assert!(std::ptr::eq(
        original.available_lane(&host).unwrap(),
        std::ptr::from_ref(&original.retention_lanes[0])
    ));

    let (occupied, _) = original
        .create(&"1".try_into().unwrap(), binding(), 0)
        .unwrap();
    assert!(std::ptr::eq(
        occupied.available_lane(&host).unwrap(),
        std::ptr::from_ref(&occupied.retention_lanes[1])
    ));

    // Queries also keep their first-match behavior before recovery validation
    // rejects a duplicate registration; the duplicate must not become a match
    // ahead of the earlier row or make this malformed image valid.
    let mut duplicate = original.clone();
    let mut later = duplicate.retention_lanes[0].clone();
    later.retired_through = 7;
    duplicate.retention_lanes.push(later);
    let before = duplicate.clone();
    assert!(std::ptr::eq(
        duplicate.lane(&host, 1).unwrap(),
        std::ptr::from_ref(&duplicate.retention_lanes[0])
    ));
    assert!(std::ptr::eq(
        duplicate.available_lane(&host).unwrap(),
        std::ptr::from_ref(&duplicate.retention_lanes[0])
    ));
    assert!(!duplicate.is_valid());
    assert_eq!(duplicate, before);
}

#[test]
fn sequential_reservations_match_original_fold_for_scopes_and_overflow() {
    // Frozen pre-refactor query as a test oracle, not a second production spec.
    fn original_reserved(
        image: &CenterState,
        tenant: Option<&sandproof_protocol::TenantId>,
        node: Option<&sandproof_protocol::internal::NodeIdentity>,
    ) -> Option<Resources> {
        image
            .sandboxes
            .iter()
            .filter(|s| !s.released)
            .filter_map(|s| s.operations.first())
            .filter(|op| {
                tenant.is_none_or(|t| &op.request.identity.tenant_id == t)
                    && node.is_none_or(|n| &op.request.identity.node == n)
            })
            .try_fold(Resources::ZERO, |sum, op| {
                sum.checked_add(op.request.create.resources)
            })
    }

    let tenant = "1".try_into().unwrap();
    let other_tenant = "other".try_into().unwrap();
    let host = node("host-a");
    let other_host = node("host-b");
    let (one, _) = state().create(&tenant, binding(), 0).unwrap();
    let (two, _) = one.create(&tenant, binding(), 0).unwrap();
    let mut released = two.clone();
    released.sandboxes[0].released = true;
    let mut empty_history = two.clone();
    empty_history.sandboxes[0].operations.clear();
    let mut images = vec![state(), one, two.clone(), released, empty_history];
    for component in 0..4 {
        let mut overflow = two.clone();
        let shape = &mut overflow.sandboxes[0].operations[0].request.create.resources;
        match component {
            0 => shape.instances = u64::MAX,
            1 => shape.cpu = u64::MAX,
            2 => shape.memory_bytes = u64::MAX,
            _ => shape.disk_bytes = u64::MAX,
        }
        assert_eq!(overflow.reserved(None, None), None);
        images.push(overflow);
    }
    for image in images {
        let before = image.clone();
        for tenant_scope in [None, Some(&tenant), Some(&other_tenant)] {
            for node_scope in [None, Some(&host), Some(&other_host)] {
                assert_eq!(
                    image.reserved(tenant_scope, node_scope),
                    original_reserved(&image, tenant_scope, node_scope),
                );
            }
        }
        assert_eq!(image, before);
    }
}
#[test]
fn retained_history_is_bounded_without_forgetting_old_identities() {
    let tenant = "1".try_into().unwrap();
    let mut s = state();
    for _ in 0..2 {
        let (next, create) = s.create(&tenant, binding(), 0).unwrap();
        s = next
            .apply_response(&response(&create, 1, LifecycleFact::NotAccepted))
            .unwrap();
    }
    assert_eq!(s.reserved(None, None), Some(resources(0)));
    assert_eq!(
        s.create(&tenant, binding(), 0),
        Err(DecisionError::Capacity)
    );
    assert!(s.is_valid());
}

#[test]
fn confirmed_compaction_bounds_renewal_history_without_reusing_intent_or_destroy_slot() {
    let tenant = "1".try_into().unwrap();
    let mut s = state();
    s.bounds.operations_per_sandbox = 4;
    let (next, create) = s.create(&tenant, binding(), 0).unwrap();
    s = next
        .apply_response(&response(
            &create,
            1,
            LifecycleFact::Ready {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600,
            },
        ))
        .unwrap();
    let mut old = None;
    for version in 2..40 {
        let (next, request) = s
            .renew(&tenant, &create.identity.sandbox_id, 500, 150)
            .unwrap();
        assert_eq!(request.identity.intent_version.get(), version);
        s = next;
        if let Some(previous) = &old {
            assert_eq!(s.compact_confirmed(previous), Err(DecisionError::Stale));
        }
        let result = response(&request, version, LifecycleFact::Renewed { end_at_ms: 500 });
        s = s.apply_response(&result).unwrap();
        s = s.compact_confirmed(&result).unwrap();
        assert_eq!(s.sandboxes[0].operations.len(), 2);
        assert_eq!(s.sandboxes[0].operations[0].request, create);
        assert_eq!(s.sandboxes[0].end_at_ms, Some(500));
        assert!(s.is_valid());
        old = Some(result);
    }
    // One missed confirmation leaves an extra historical renewal. Destruction
    // may compact it only while retaining the final confirmed lifetime anchor.
    let (next, renewal) = s
        .renew(&tenant, &create.identity.sandbox_id, 500, 150)
        .unwrap();
    s = next
        .apply_response(&response(
            &renewal,
            40,
            LifecycleFact::Renewed { end_at_ms: 500 },
        ))
        .unwrap();
    let (s, destroy) = s.destroy(&tenant, &create.identity.sandbox_id).unwrap();
    assert_eq!(destroy.identity.intent_version.get(), 41);
    assert_eq!(s.reserved(None, None), Some(resources(1)));
    let result = response(&destroy, 41, LifecycleFact::Cleaned);
    let s = s
        .apply_response(&result)
        .unwrap()
        .compact_confirmed(&result)
        .unwrap();
    assert!(s.is_valid());
    assert_eq!(s.sandboxes[0].operations.len(), 3);
    assert_eq!(s.sandboxes[0].operations[1].request, renewal);
    assert_eq!(s.sandboxes[0].end_at_ms, Some(500));
    assert_eq!(s.reserved(None, None), Some(Resources::ZERO));
}
