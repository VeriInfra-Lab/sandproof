#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use crate::*;
use fixture::*;
use sandproof_protocol::internal::lifecycle::{LifecycleFact, Resources};
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
            limit: resources(2),
            mode: NodeMode::Accepting,
            healthy_until_ms: 2000,
            free: resources(2),
            templates: vec![crate::AvailableTemplate {
                revision: binding().template_revision,
                digest: binding().template_digest,
            }],
            protocol_version: 1,
        }],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources(2),
        }],
    )
    .unwrap()
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
