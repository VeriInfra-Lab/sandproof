#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use crate::*;
use fixture::{binding, node, resources};
use sandproof_protocol::{
    TenantId,
    internal::lifecycle::{CreateBinding, Resources},
    lifecycle::{
        AcceptedLifecycleIntent, CenterIdentity, ConfirmedLifecycleResult, ConfirmedRenewal,
        CreateObservation, CreateRefusalReason, CurrentCreateReadiness, CurrentReadyFacts,
        LifecycleOperation, OperationKey, RenewalConfirmation, TerminalLifecycleObservation,
        TerminalStatus,
    },
};
#[path = "window_tests.rs"]
mod windows;

fn center() -> CenterIdentity {
    CenterIdentity {
        deployment: node("host-a").deployment_id,
        center: "center".try_into().unwrap(),
    }
}
fn request_windows() -> Vec<LifecycleWindow> {
    vec![LifecycleWindow {
        owner: "1".try_into().unwrap(),
        window: 9.try_into().unwrap(),
    }]
}
fn state() -> CenterState {
    CenterState::new(
        node("host-a").deployment_id,
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 16,
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            result_retention_ms: 100,
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
            templates: vec![AvailableTemplate {
                revision: binding().template_revision,
                digest: binding().template_digest,
            }],
            protocol_version: sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION,
        }],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources(2),
        }],
        center(),
        request_windows(),
    )
    .unwrap()
}
fn public_key(owner: &TenantId, sequence: u64) -> OperationKey {
    OperationKey {
        deployment: node("host-a").deployment_id,
        owner: owner.clone(),
        window: 9.try_into().unwrap(),
        sequence: sequence.try_into().unwrap(),
    }
}
fn create_submission(key: OperationKey, shape: CreateBinding) -> LifecycleSubmission {
    LifecycleSubmission::Create {
        key,
        parameters: CreateParameters {
            template_revision: shape.template_revision,
            template_digest: shape.template_digest,
            resources: shape.resources,
            options: shape.options,
            startup_timeout_ms: 1000,
            timeout_ms: shape.timeout_ms,
            max_lifetime_ms: shape.max_lifetime_ms,
        },
    }
}
fn admit_create(
    image: &CenterState,
    owner: &TenantId,
    shape: CreateBinding,
    sequence: u64,
) -> Result<(CenterState, AcceptedLifecycleIntent), DecisionError> {
    image.accept_lifecycle(
        owner,
        &create_submission(public_key(owner, sequence), shape),
        0,
    )
}
fn admit_destroy(
    image: &CenterState,
    current: &AcceptedLifecycleIntent,
    key: OperationKey,
) -> Result<(CenterState, AcceptedLifecycleIntent), DecisionError> {
    image.accept_lifecycle(
        &current.request.binding.key.owner,
        &LifecycleSubmission::Destroy {
            key,
            session: current.request.binding.session.clone(),
            expected_version: current.accepted_version,
        },
        0,
    )
}
fn admit_renew(
    image: &CenterState,
    current: &AcceptedLifecycleIntent,
    key: OperationKey,
) -> Result<(CenterState, AcceptedLifecycleIntent), DecisionError> {
    image.accept_lifecycle(
        &current.request.binding.key.owner,
        &LifecycleSubmission::Renew {
            key,
            session: current.request.binding.session.clone(),
            expected_version: current.accepted_version,
            timeout_secs: 1,
            extend_only: false,
        },
        150,
    )
}
// Controlled facts delivered to the actual pure save method, not real VMs.
fn observed(
    original: &AcceptedLifecycleIntent,
    current: &AcceptedLifecycleIntent,
    end_at_ms: u64,
) -> CreateObservation {
    let LifecycleOperation::Create { create, .. } = &original.request.operation else {
        unreachable!()
    };
    CreateObservation {
        request: original.clone(),
        current_request: current.clone(),
        historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 100,
                end_at_ms,
                max_end_at_ms: 600,
                startup_deadline_ms: create.startup_deadline_ms,
                observed_at_ms: 150,
            },
        },
    }
}
fn save_ready(image: &CenterState, original: &AcceptedLifecycleIntent) -> CenterState {
    image
        .accept_lifecycle_create_observation(&observed(original, original, 200))
        .unwrap()
}
fn save_cleaned(
    image: &CenterState,
    destroy: &AcceptedLifecycleIntent,
) -> (CenterState, TerminalLifecycleObservation) {
    let result = TerminalLifecycleObservation {
        request: destroy.clone(),
        status: TerminalStatus::Cleaned,
    };
    let next = image
        .accept_lifecycle_terminal_observation(&result, 150)
        .unwrap();
    (next, result)
}
fn refused_and_cleaned(
    image: &CenterState,
    original: &AcceptedLifecycleIntent,
) -> (CenterState, TerminalLifecycleObservation) {
    let refusal = CreateObservation {
        request: original.clone(),
        current_request: original.clone(),
        historical_create_succeeded: false,
        readiness: CurrentCreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity,
        },
    };
    let (terminating, destroy) = image.accept_lifecycle_create_refusal(&refusal).unwrap();
    assert_eq!(terminating.reserved(None, None), image.reserved(None, None));
    save_cleaned(&terminating, &destroy)
}
fn confirmed(result: &TerminalLifecycleObservation) -> ConfirmedLifecycleResult {
    ConfirmedLifecycleResult {
        result: result.clone(),
    }
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
    let owner = "1".try_into().unwrap();
    let (original, request) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let refreshed = original
        .refresh_node(&node("host-a"), 0, Resources::ZERO)
        .unwrap();
    assert!(refreshed.is_valid());
    assert_eq!(refreshed.pending_lifecycle(), vec![request]);
    assert_eq!(
        refreshed.reserved(None, None),
        original.reserved(None, None)
    );
    assert_eq!(refreshed.retention_lanes, original.retention_lanes);
    assert_eq!(refreshed.nodes[0].templates, original.nodes[0].templates);
    assert_eq!(refreshed.nodes[0].mode, original.nodes[0].mode);
    assert_eq!(
        admit_create(&refreshed, &owner, binding(), 2),
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
    let owner = "1".try_into().unwrap();
    let (created, request) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let running = save_ready(&created, &request);
    for healthy_until in [0, 149, 150] {
        let stale = running
            .refresh_node(&node("host-a"), healthy_until, Resources::ZERO)
            .unwrap();
        assert_eq!(
            admit_renew(&stale, &request, public_key(&owner, 2)),
            Err(DecisionError::Unavailable)
        );
        assert_eq!(stale.identity_frontiers(), running.identity_frontiers());
        assert_eq!(stale.pending_lifecycle(), running.pending_lifecycle());
        let (terminating, destroy) =
            admit_destroy(&stale, &request, public_key(&owner, 3)).unwrap();
        assert!(matches!(
            destroy.request.operation,
            LifecycleOperation::Destroy {}
        ));
        assert!(terminating.sandboxes()[0].terminating());
    }
    let fresh = running
        .refresh_node(&node("host-a"), 151, Resources::ZERO)
        .unwrap()
        .begin_drain(&node("host-a"))
        .unwrap();
    assert!(admit_renew(&fresh, &request, public_key(&owner, 2)).is_ok());
}

#[test]
fn identity_exhaustion_reserves_the_final_destroy_id() {
    let owner = "1".try_into().unwrap();
    let mut s = state();
    s.operation_highwater = u64::MAX - 1;
    assert!(s.is_valid());
    assert_eq!(
        admit_create(&s, &owner, binding(), 1),
        Err(DecisionError::Exhausted)
    );
    s.operation_highwater = u64::MAX - 2;
    let (s, create) = admit_create(&s, &owner, binding(), 1).unwrap();
    let s = save_ready(&s, &create);
    assert_eq!(
        admit_renew(&s, &create, public_key(&owner, 2)),
        Err(DecisionError::Exhausted)
    );
    let (s, destroy) = admit_destroy(&s, &create, public_key(&owner, 3)).unwrap();
    assert_eq!(s.operation_highwater, u64::MAX);
    assert_eq!(
        s.sandboxes[0].operations.last().unwrap().allocation_number,
        u64::MAX
    );
    // A public replay key is not the allocator's global operation number.
    assert_eq!(destroy.request.binding.key.sequence.get(), 3);
    assert!(s.is_valid());
}

#[test]
fn recovery_rejects_corrupt_projection_binding_and_capacity() {
    let owner = "1".try_into().unwrap();
    let (s, create) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let mut bad = s.clone();
    bad.sandboxes[0].operations[0].terminal = Some(crate::lifecycle::RetainedTerminalResult {
        result: TerminalLifecycleObservation {
            request: create.clone(),
            status: TerminalStatus::Cleaned,
        },
        recorded_at_ms: 0,
        retention_started_at_ms: Some(0),
        retain_until_ms: Some(100),
    });
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    bad.sandboxes[0].operations[0]
        .intent
        .request
        .binding
        .node
        .node_generation = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    let CurrentCreateReadiness::Observed { mut facts } = observed(&create, &create, 200).readiness
    else {
        unreachable!()
    };
    facts.max_end_at_ms += 1;
    bad.sandboxes[0].operations[0].running = Some(facts);
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    bad.sandboxes[0].operations[0]
        .intent
        .request
        .binding
        .session = "sandbox-00".try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = s.clone();
    bad.operation_highwater = u64::MAX;
    assert!(!bad.is_valid());
    assert!(s.pending_lifecycle().contains(&create));
}

#[test]
fn recovery_keeps_operation_ids_unique_across_live_sandbox_histories() {
    let owner = "1".try_into().unwrap();
    let (first, first_request) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let (valid, second_request) = admit_create(&first, &owner, binding(), 2).unwrap();
    assert_ne!(first_request.retention.slot, second_request.retention.slot);
    assert!(valid.is_valid());
    let mut duplicate = valid.clone();
    duplicate.sandboxes[1].operations[0].allocation_number =
        duplicate.sandboxes[0].operations[0].allocation_number;
    let before = duplicate.clone();
    assert!(!duplicate.is_valid());
    assert_eq!(duplicate, before);
    assert_eq!(
        admit_create(&duplicate, &owner, binding(), 3),
        Err(DecisionError::Invalid)
    );
    assert_eq!(duplicate, before);
    let mut duplicate = valid.clone();
    let repeated = first_request.request.binding.key;
    duplicate.sandboxes[1].operations[0]
        .intent
        .request
        .binding
        .key = repeated.clone();
    let LifecycleSubmission::Create { key, .. } =
        &mut duplicate.sandboxes[1].operations[0].submission
    else {
        unreachable!()
    };
    *key = repeated;
    assert!(!duplicate.is_valid());
}

#[test]
fn current_acceptance_keeps_first_rejection_when_multiple_checks_would_fail() {
    let owner = "1".try_into().unwrap();
    let missing = "missing".try_into().unwrap();
    let (one, _) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let (full, _) = admit_create(&one, &owner, binding(), 2).unwrap();
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
    // Current public admission checks the registered request window before
    // allocating. All capacity/availability/exhaustion probes use a valid key.
    for (image, tenant, shape, expected) in [
        (invalid_full, &missing, binding(), DecisionError::Invalid),
        (full.clone(), &missing, binding(), DecisionError::Stale),
        (full, &owner, binding(), DecisionError::Capacity),
        (
            unavailable.clone(),
            &missing,
            too_large.clone(),
            DecisionError::Stale,
        ),
        (
            unavailable.clone(),
            &owner,
            too_large,
            DecisionError::Capacity,
        ),
        (unavailable, &owner, binding(), DecisionError::Unavailable),
        (exhausted, &owner, binding(), DecisionError::Exhausted),
    ] {
        let before = image.clone();
        assert_eq!(admit_create(&image, tenant, shape, 3), Err(expected));
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
    let (occupied, _) = admit_create(&original, &"1".try_into().unwrap(), binding(), 1).unwrap();
    assert!(std::ptr::eq(
        occupied.available_lane(&host).unwrap(),
        std::ptr::from_ref(&occupied.retention_lanes[1])
    ));
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
    // Original fold structure over the sole current representation; a test
    // oracle, not another product algorithm or a general equivalence proof.
    fn original_reserved(
        image: &CenterState,
        tenant: Option<&TenantId>,
        node: Option<&sandproof_protocol::internal::NodeIdentity>,
    ) -> Option<Resources> {
        image
            .sandboxes
            .iter()
            .filter(|s| !s.released())
            .filter_map(|s| s.operations.first())
            .filter(|op| {
                tenant.is_none_or(|t| &op.intent.request.binding.key.owner == t)
                    && node.is_none_or(|n| &op.intent.request.binding.node == n)
            })
            .try_fold(Resources::ZERO, |sum, op| {
                let LifecycleOperation::Create { create, .. } = &op.intent.request.operation else {
                    return None;
                };
                sum.checked_add(create.resources)
            })
    }
    let owner = "1".try_into().unwrap();
    let other_owner = "other".try_into().unwrap();
    let host = node("host-a");
    let other_host = node("host-b");
    let (one, first) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let (two, _) = admit_create(&one, &owner, binding(), 2).unwrap();
    let (terminating, destroy) = admit_destroy(&two, &first, public_key(&owner, 3)).unwrap();
    let (released, _) = save_cleaned(&terminating, &destroy);
    let mut empty_history = two.clone();
    empty_history.sandboxes[0].operations.clear();
    let mut images = vec![state(), one, two.clone(), released, empty_history];
    for component in 0..4 {
        let mut overflow = two.clone();
        let LifecycleOperation::Create { create, .. } =
            &mut overflow.sandboxes[0].operations[0].intent.request.operation
        else {
            unreachable!()
        };
        match component {
            0 => create.resources.instances = u64::MAX,
            1 => create.resources.cpu = u64::MAX,
            2 => create.resources.memory_bytes = u64::MAX,
            _ => create.resources.disk_bytes = u64::MAX,
        }
        assert_eq!(overflow.reserved(None, None), None);
        images.push(overflow);
    }
    for image in images {
        let before = image.clone();
        for tenant_scope in [None, Some(&owner), Some(&other_owner)] {
            for node_scope in [None, Some(&host), Some(&other_host)] {
                assert_eq!(
                    image.reserved(tenant_scope, node_scope),
                    original_reserved(&image, tenant_scope, node_scope)
                );
            }
        }
        assert_eq!(image, before);
    }
}

#[test]
fn retained_history_is_bounded_without_forgetting_old_identities() {
    let owner = "1".try_into().unwrap();
    let mut image = state();
    let mut originals = Vec::new();
    for sequence in 1..=2 {
        let (accepted, create) = admit_create(&image, &owner, binding(), sequence).unwrap();
        let (cleaned, _) = refused_and_cleaned(&accepted, &create);
        image = cleaned;
        originals.push(create);
    }
    assert_eq!(image.reserved(None, None), Some(resources(0)));
    assert_eq!(
        admit_create(&image, &owner, binding(), 3),
        Err(DecisionError::Capacity)
    );
    for original in originals {
        assert_eq!(
            image
                .replay_lifecycle(
                    &owner,
                    &create_submission(original.request.binding.key.clone(), binding())
                )
                .unwrap(),
            Some(original)
        );
    }
    assert!(image.is_valid());
}

#[test]
fn confirmed_compaction_bounds_renewal_history_without_reusing_intent_or_destroy_slot() {
    let owner = "1".try_into().unwrap();
    let mut initial = state();
    initial.bounds.operations_per_sandbox = 4;
    let (image, create) = admit_create(&initial, &owner, binding(), 1).unwrap();
    let mut image = save_ready(&image, &create);
    let mut current = create.clone();
    let mut previous: Option<ConfirmedRenewal> = None;
    for version in 2..40 {
        let input_key = image.next_sdk_operation_key(&owner).unwrap();
        let (accepted, renewal) = admit_renew(&image, &current, input_key).unwrap();
        assert_eq!(renewal.accepted_version, version);
        image = accepted;
        if let Some(old) = &previous {
            // Reconfirming the latest saved checkpoint is idempotent while a
            // new result is pending, and cannot remove its lifetime anchor.
            assert_eq!(image.confirm_lifecycle_renewal(old).unwrap(), image);
        }
        image = image
            .accept_lifecycle_create_observation(&observed(&create, &renewal, 600))
            .unwrap();
        let exact = ConfirmedRenewal {
            result: RenewalConfirmation {
                request: renewal.clone(),
                effective_end_at_ms: 600,
            },
        };
        image = image.confirm_lifecycle_renewal(&exact).unwrap();
        if let Some(old) = &previous {
            assert_eq!(
                image.confirm_lifecycle_renewal(old),
                Err(DecisionError::Stale)
            );
        }
        assert_eq!(image.sandboxes[0].operations.len(), 2);
        assert_eq!(image.sandboxes[0].operations[0].intent, create);
        assert_eq!(image.sandboxes[0].last_running().unwrap().end_at_ms, 600);
        assert!(image.is_valid());
        current = renewal;
        previous = Some(exact);
    }
    let input_key = image.next_sdk_operation_key(&owner).unwrap();
    let (next, renewal) = admit_renew(&image, &current, input_key).unwrap();
    image = next
        .accept_lifecycle_create_observation(&observed(&create, &renewal, 600))
        .unwrap();
    let input_key = image.next_sdk_operation_key(&owner).unwrap();
    let (terminating, destroy) = admit_destroy(&image, &renewal, input_key).unwrap();
    assert_eq!(destroy.accepted_version, 41);
    assert_eq!(terminating.reserved(None, None), Some(resources(1)));
    let (cleaned, result) = save_cleaned(&terminating, &destroy);
    let settled = cleaned.confirm_terminal(&confirmed(&result)).unwrap();
    // Terminal confirmation is not an SDK result confirmation.
    assert_eq!(settled.sandboxes[0].operations.len(), 4);
    let exact = ConfirmedRenewal {
        result: RenewalConfirmation {
            request: renewal.clone(),
            effective_end_at_ms: 600,
        },
    };
    let settled = settled.confirm_lifecycle_renewal(&exact).unwrap();
    assert!(settled.is_valid());
    assert_eq!(settled.sandboxes[0].operations.len(), 3);
    assert_eq!(settled.sandboxes[0].operations[1].intent, renewal);
    assert_eq!(settled.sandboxes[0].last_running().unwrap().end_at_ms, 600);
    assert_eq!(settled.reserved(None, None), Some(Resources::ZERO));
}
