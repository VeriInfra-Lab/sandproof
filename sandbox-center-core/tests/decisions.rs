//! Current pure center decisions with controlled host observations, not KVM.
#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use fixture::{binding, node, resources};
use sandproof_center_core::{
    Bounds, CenterState, CreateParameters, DecisionError, LifecycleSubmission, LifecycleWindow,
    NodeMode, NodeRegistration, NodeRole, TenantBudget,
};
use sandproof_protocol::{
    TenantId,
    internal::lifecycle::CreateBinding,
    lifecycle::{
        AcceptedLifecycleIntent, CenterIdentity, ConfirmedLifecycleResult, CreateObservation,
        CreateRefusalReason, CurrentCreateReadiness, CurrentReadyFacts, LifecycleOperation,
        OperationKey, TerminalLifecycleObservation, TerminalStatus,
    },
};

fn registration(name: &str) -> NodeRegistration {
    NodeRegistration {
        role: NodeRole::Runtime,
        identity: node(name),
        retention_slots: 8,
        limit: resources(2),
        mode: NodeMode::Accepting,
        healthy_until_ms: 2000,
        free: resources(2),
        templates: vec![sandproof_center_core::AvailableTemplate {
            revision: binding().template_revision,
            digest: binding().template_digest,
        }],
        protocol_version: sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION,
    }
}

fn configured(nodes: Vec<NodeRegistration>, tenants: Vec<TenantBudget>) -> CenterState {
    let windows = tenants
        .iter()
        .map(|budget| LifecycleWindow {
            owner: budget.tenant.clone(),
            window: 9.try_into().unwrap(),
        })
        .collect();
    CenterState::new(
        node("host-a").deployment_id,
        Bounds {
            max_nodes: 2,
            max_templates_per_node: 16,
            max_sandboxes: 8,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            result_retention_ms: 100,
            platform: resources(4),
        },
        nodes,
        tenants,
        CenterIdentity {
            deployment: node("host-a").deployment_id,
            center: "center".try_into().unwrap(),
        },
        windows,
    )
    .unwrap()
}

fn state() -> CenterState {
    configured(
        vec![registration("host-b"), registration("host-a")],
        tenants(),
    )
}

fn tenants() -> Vec<TenantBudget> {
    vec![TenantBudget {
        tenant: "1".try_into().unwrap(),
        limit: resources(4),
    }]
}

fn key(owner: &TenantId, sequence: u64) -> OperationKey {
    OperationKey {
        deployment: node("host-a").deployment_id,
        owner: owner.clone(),
        window: 9.try_into().unwrap(),
        sequence: sequence.try_into().unwrap(),
    }
}

fn create_input(owner: &TenantId, shape: CreateBinding, sequence: u64) -> LifecycleSubmission {
    LifecycleSubmission::Create {
        key: key(owner, sequence),
        parameters: CreateParameters {
            template_revision: shape.template_revision,
            template_digest: shape.template_digest,
            resources: shape.resources,
            options: shape.options,
            // These fixtures admit Create at time zero, so the original
            // absolute startup deadline is the same duration at this input.
            startup_timeout_ms: shape.startup_deadline_ms,
            timeout_ms: shape.timeout_ms,
            max_lifetime_ms: shape.max_lifetime_ms,
        },
    }
}

fn create(
    image: &CenterState,
    owner: &TenantId,
    shape: CreateBinding,
    sequence: u64,
) -> Result<(CenterState, AcceptedLifecycleIntent), DecisionError> {
    image.accept_lifecycle(owner, &create_input(owner, shape, sequence), 0)
}

fn destroy(
    image: &CenterState,
    original: &AcceptedLifecycleIntent,
    sequence: u64,
) -> Result<(CenterState, AcceptedLifecycleIntent), DecisionError> {
    let owner = &original.request.binding.key.owner;
    image.accept_lifecycle(
        owner,
        &LifecycleSubmission::Destroy {
            key: key(owner, sequence),
            session: original.request.binding.session.clone(),
            expected_version: original.accepted_version,
        },
        0,
    )
}

fn renew(
    image: &CenterState,
    current: &AcceptedLifecycleIntent,
    sequence: u64,
    timeout_secs: u64,
    now_ms: u64,
) -> Result<(CenterState, AcceptedLifecycleIntent), DecisionError> {
    let owner = &current.request.binding.key.owner;
    image.accept_lifecycle(
        owner,
        &LifecycleSubmission::Renew {
            key: key(owner, sequence),
            session: current.request.binding.session.clone(),
            expected_version: current.accepted_version,
            timeout_secs,
            extend_only: false,
        },
        now_ms,
    )
}

fn observed(
    original: &AcceptedLifecycleIntent,
    current: &AcceptedLifecycleIntent,
    end_at_ms: u64,
    observed_at_ms: u64,
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
                observed_at_ms,
            },
        },
    }
}

fn ready(image: &CenterState, original: &AcceptedLifecycleIntent) -> CenterState {
    image
        .accept_lifecycle_create_observation(&observed(original, original, 200, 100))
        .unwrap()
}

#[test]
fn deterministic_balanced_fixed_binding_and_capacity() {
    let tenant = "1".try_into().unwrap();
    let (s, first) = create(&state(), &tenant, binding(), 1).unwrap();
    assert_eq!(first.request.binding.node, node("host-a"));
    let (s, second) = create(&s, &tenant, binding(), 2).unwrap();
    assert_eq!(second.request.binding.node, node("host-b"));
    let (s, _) = create(&s, &tenant, binding(), 3).unwrap();
    let (s, _) = create(&s, &tenant, binding(), 4).unwrap();
    assert_eq!(
        create(&s, &tenant, binding(), 5),
        Err(DecisionError::Capacity)
    );
    assert_eq!(s.pending_lifecycle()[0], first);
    assert_eq!(s.reserved(None, None), Some(resources(4)));
    assert!(s.is_valid());
}

#[test]
fn stale_health_physical_free_templates_and_draining_are_vetoes() {
    let tenant = "1".try_into().unwrap();
    let mut a = registration("host-a");
    a.mode = NodeMode::Draining;
    let s = state().observe_node(a).unwrap();
    let mut b = registration("host-b");
    b.free.disk_bytes = 0;
    let s = s.observe_node(b.clone()).unwrap();
    assert_eq!(
        create(&s, &tenant, binding(), 1),
        Err(DecisionError::Capacity)
    );
    b.free = resources(2);
    b.healthy_until_ms = 0;
    let s = s.observe_node(b.clone()).unwrap();
    assert_eq!(
        create(&s, &tenant, binding(), 1),
        Err(DecisionError::Unavailable)
    );
    b.healthy_until_ms = 2000;
    b.templates.clear();
    let s = s.observe_node(b).unwrap();
    assert_eq!(
        create(&s, &tenant, binding(), 1),
        Err(DecisionError::Unavailable)
    );
}

#[test]
fn unknown_holds_quota_destroy_supersedes_late_ready_and_pending_is_not_cleaned() {
    let tenant = "1".try_into().unwrap();
    let (s, create) = create(&state(), &tenant, binding(), 1).unwrap();
    let (s, destroy_request) = destroy(&s, &create, 2).unwrap();
    assert_eq!(
        s.accept_lifecycle_create_observation(&observed(&create, &create, 200, 100)),
        Err(DecisionError::Stale)
    );
    assert_eq!(s.pending_lifecycle(), vec![destroy_request.clone()]);
    let pending = TerminalLifecycleObservation {
        request: destroy_request.clone(),
        status: TerminalStatus::Pending,
    };
    let s = s
        .accept_lifecycle_terminal_observation(&pending, 150)
        .unwrap();
    assert_eq!(s.reserved(None, None), Some(resources(1)));
    let cleaned = TerminalLifecycleObservation {
        request: destroy_request.clone(),
        status: TerminalStatus::Cleaned,
    };
    let s = s
        .accept_lifecycle_terminal_observation(&cleaned, 150)
        .unwrap();
    assert_eq!(s.reserved(None, None), Some(resources(0)));
    // Saving Cleaned does not discharge the pending confirmation responsibility.
    assert_eq!(s.pending_lifecycle(), vec![destroy_request.clone()]);
    let s = s
        .confirm_terminal(&ConfirmedLifecycleResult { result: cleaned })
        .unwrap();
    assert!(s.pending_lifecycle().is_empty());
    assert_eq!(destroy(&s, &create, 2).unwrap().1, destroy_request);
}

#[test]
fn lifetime_is_original_normalization_is_bounded_and_destroy_keeps_its_reserve() {
    let tenant = "1".try_into().unwrap();
    let (s, create) = create(&state(), &tenant, binding(), 1).unwrap();
    let s = ready(&s, &create);
    // The current public input is relative seconds. Even an enormous duration
    // is normalized to the original total limit, not a renewed execution.
    let (_, clamped) = renew(&s, &create, 2, u64::MAX, 150).unwrap();
    assert!(matches!(
        clamped.request.operation,
        LifecycleOperation::Renew { end_at_ms: 600, .. }
    ));
    let (s, renewal) = renew(&s, &create, 2, 1, 150).unwrap();
    assert_eq!(s.sandboxes()[0].last_running().unwrap().end_at_ms, 200);
    assert_eq!(
        s.accept_lifecycle_create_observation(&observed(&create, &renewal, 601, 150)),
        Err(DecisionError::Invalid)
    );
    let fact = observed(&create, &renewal, 600, 150);
    let s = s.accept_lifecycle_create_observation(&fact).unwrap();
    assert_eq!(s.accept_lifecycle_create_observation(&fact).unwrap(), s);
    assert_eq!(renew(&s, &renewal, 3, 1, 160), Err(DecisionError::Capacity));
    assert!(destroy(&s, &renewal, 4).is_ok());
    assert_eq!(s.sandboxes()[0].last_running().unwrap().started_at_ms, 100);
}

#[test]
fn wrong_owner_or_full_binding_and_a_refusal_cannot_release_before_cleaned() {
    let tenant = "1".try_into().unwrap();
    let other: TenantId = "2".try_into().unwrap();
    let mut registered = tenants();
    registered.push(TenantBudget {
        tenant: other.clone(),
        limit: resources(4),
    });
    let initial = configured(
        vec![registration("host-a"), registration("host-b")],
        registered,
    );
    let (s, create) = create(&initial, &tenant, binding(), 1).unwrap();
    let foreign = LifecycleSubmission::Destroy {
        key: key(&other, 2),
        session: create.request.binding.session.clone(),
        expected_version: 1,
    };
    assert_eq!(
        s.accept_lifecycle(&other, &foreign, 0),
        Err(DecisionError::NotFound)
    );
    let mut bad = observed(&create, &create, 200, 100);
    bad.request.request.binding.node.node_generation = 2.try_into().unwrap();
    assert_eq!(
        s.accept_lifecycle_create_observation(&bad),
        Err(DecisionError::Stale)
    );
    assert_eq!(s.pending_lifecycle(), vec![create.clone()]);
    let refused = CreateObservation {
        request: create.clone(),
        current_request: create.clone(),
        historical_create_succeeded: false,
        readiness: CurrentCreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity,
        },
    };
    let running = ready(&s, &create);
    assert_eq!(
        running.accept_lifecycle_create_refusal(&refused),
        Err(DecisionError::Conflict)
    );
    assert_eq!(running.reserved(None, None), Some(resources(1)));
    let (s, destroy) = s.accept_lifecycle_create_refusal(&refused).unwrap();
    assert_eq!(s.reserved(None, None), Some(resources(1)));
    assert_eq!(
        s.accept_lifecycle_create_refusal(&refused),
        Err(DecisionError::Stale)
    );
    let mut exact = refused;
    exact.current_request = destroy.clone();
    assert_eq!(
        s.accept_lifecycle_create_refusal(&exact).unwrap(),
        (s.clone(), destroy.clone())
    );
    let cleaned = TerminalLifecycleObservation {
        request: destroy,
        status: TerminalStatus::Cleaned,
    };
    let s = s
        .accept_lifecycle_terminal_observation(&cleaned, 150)
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
    let s = configured(vec![a, b], tenants());
    let tenant = "1".try_into().unwrap();
    let (s, first) = create(&s, &tenant, binding(), 1).unwrap();
    let (_, second) = create(&s, &tenant, binding(), 2).unwrap();
    assert_eq!(first.request.binding.node, node("host-a"));
    assert_eq!(second.request.binding.node, node("host-b"));
}

#[test]
fn tenant_quota_does_not_block_another_tenant_and_template_digest_must_match() {
    let s = configured(
        vec![registration("host-a"), registration("host-b")],
        ["1", "2"]
            .into_iter()
            .map(|id| TenantBudget {
                tenant: id.try_into().unwrap(),
                limit: resources(2),
            })
            .collect(),
    );
    let one = "1".try_into().unwrap();
    let two = "2".try_into().unwrap();
    let (s, _) = create(&s, &one, binding(), 1).unwrap();
    let (s, _) = create(&s, &one, binding(), 2).unwrap();
    assert_eq!(create(&s, &one, binding(), 3), Err(DecisionError::Capacity));
    assert!(create(&s, &two, binding(), 1).is_ok());
    let mut wrong = binding();
    wrong.template_digest = [2; 32];
    assert_eq!(create(&s, &two, wrong, 2), Err(DecisionError::Unavailable));
}

#[test]
fn private_build_role_reuses_admission_without_consuming_runtime_pool() {
    let mut builder = registration("host-a");
    builder.role = NodeRole::Builder;
    assert_eq!(
        state().observe_node(builder.clone()),
        Err(DecisionError::Conflict)
    );
    let initial = configured(vec![builder, registration("host-b")], tenants());
    let tenant = "1".try_into().unwrap();
    let (normal, public) = create(&initial, &tenant, binding(), 1).unwrap();
    assert_eq!(public.request.binding.node, node("host-b"));
    let input = create_input(&tenant, binding(), 2);
    let (both, private) = normal.accept_builder_lifecycle(&tenant, &input, 0).unwrap();
    assert_eq!(private.request.binding.node, node("host-a"));
    assert_ne!(
        public.request.binding.session,
        private.request.binding.session
    );
    assert_ne!(public.request.binding.key, private.request.binding.key);
    assert_eq!(
        both.reserved(None, Some(&node("host-b"))),
        Some(resources(1))
    );
    assert_eq!(
        both.reserved(None, Some(&node("host-a"))),
        Some(resources(1))
    );
    assert_eq!(both.pending_lifecycle(), vec![public, private]);
    assert!(both.is_valid());
}
