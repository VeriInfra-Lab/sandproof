//! Actual current result checks; historical rank/observation counters were removed.
#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use fixture::{binding, node, resources};
use sandproof_center_core::{AvailableTemplate, Bounds, CenterState, CreateParameters, DecisionError, LifecycleSubmission, LifecycleWindow, NodeMode, NodeRegistration, NodeRole, TenantBudget};
use sandproof_protocol::{TenantId, internal::lifecycle::CreateBinding, lifecycle::{AcceptedLifecycleIntent, CenterIdentity, CreateObservation, CurrentCreateReadiness, CurrentReadyFacts, LifecycleOperation, OperationKey, TerminalLifecycleObservation, TerminalStatus}};
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
        vec![TenantBudget { tenant: "1".try_into().unwrap(), limit: resources(2) }],
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
    image.accept_lifecycle(owner, &create_submission(public_key(owner, sequence), shape), 0)
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
fn observed(
    original: &AcceptedLifecycleIntent,
    current: &AcceptedLifecycleIntent,
    end_at_ms: u64,
) -> CreateObservation {
    let LifecycleOperation::Create { create, .. } = &original.request.operation else { unreachable!() };
    CreateObservation {
        request: original.clone(),
        current_request: current.clone(),
        historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 100, end_at_ms, max_end_at_ms: 600,
                startup_deadline_ms: create.startup_deadline_ms, observed_at_ms: 150,
            },
        },
    }
}
fn save_ready(image: &CenterState, original: &AcceptedLifecycleIntent) -> CenterState {
    image.accept_lifecycle_create_observation(&observed(original, original, 200)).unwrap()
}

#[test]
fn current_observation_checks_full_binding_before_replay() {
    let owner = "1".try_into().unwrap();
    let (accepted, create) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let exact = observed(&create, &create, 200);
    let running = save_ready(&accepted, &create);
    assert_eq!(running.accept_lifecycle_create_observation(&exact).unwrap(), running);
    // Either origin or current-request substitution must fail, even when the
    // readiness facts exactly equal a result already saved in the original row.
    let mut altered = vec![create.clone(); 7];
    altered[0].accepted_version += 1;
    altered[1].center.center = "other-center".try_into().unwrap();
    altered[2].retention.slot = 2;
    altered[3].retention.window = 2.try_into().unwrap();
    altered[4].request.binding.node.node_generation = 2.try_into().unwrap();
    altered[5].request.binding.key.sequence = 2.try_into().unwrap();
    let LifecycleOperation::Create { create: parameters, .. } = &mut altered[6].request.operation else { unreachable!() };
    parameters.options.push(b' ');
    for wrong in altered {
        for replace_origin in [true, false] {
            let mut candidate = exact.clone();
            if replace_origin { candidate.request = wrong.clone(); }
            else { candidate.current_request = wrong.clone(); }
            assert_eq!(running.accept_lifecycle_create_observation(&candidate), Err(DecisionError::Stale));
        }
    }
    let mut changed = exact.clone();
    let CurrentCreateReadiness::Observed { facts } = &mut changed.readiness else { unreachable!() };
    facts.started_at_ms = 101;
    facts.end_at_ms = 201;
    facts.max_end_at_ms = 601;
    assert_eq!(running.accept_lifecycle_create_observation(&changed), Err(DecisionError::Conflict));
    let (terminating, _) = admit_destroy(&running, &create, public_key(&owner, 2)).unwrap();
    assert_eq!(terminating.accept_lifecycle_create_observation(&exact), Err(DecisionError::Stale));
    assert_eq!(terminating.reserved(None, None), running.reserved(None, None));
}

#[test]
fn exact_terminal_replay_cannot_bypass_current_destroy_or_restart_retention() {
    let owner = "1".try_into().unwrap();
    let (accepted, create) = admit_create(&state(), &owner, binding(), 1).unwrap();
    let running = save_ready(&accepted, &create);
    let fake = TerminalLifecycleObservation { request: create.clone(), status: TerminalStatus::Cleaned };
    assert_eq!(running.accept_lifecycle_terminal_observation(&fake, 150), Err(DecisionError::Stale));
    let (terminating, destroy) = admit_destroy(&running, &create, public_key(&owner, 2)).unwrap();
    let pending = TerminalLifecycleObservation { request: destroy.clone(), status: TerminalStatus::Pending };
    assert_eq!(terminating.accept_lifecycle_terminal_observation(&pending, 150).unwrap(), terminating);
    assert_eq!(terminating.reserved(None, None), Some(resources(1)));
    let exact = TerminalLifecycleObservation { request: destroy, status: TerminalStatus::Cleaned };
    let cleaned = terminating.accept_lifecycle_terminal_observation(&exact, 150).unwrap();
    assert_eq!(cleaned.reserved(None, None), Some(resources(0)));
    for now in [0, 149, 150, 250, u64::MAX] {
        assert_eq!(cleaned.accept_lifecycle_terminal_observation(&exact, now).unwrap(), cleaned);
    }
    let mut wrong = exact.clone();
    wrong.request.accepted_version += 1;
    assert_eq!(cleaned.accept_lifecycle_terminal_observation(&wrong, 150), Err(DecisionError::Stale));
    let mut wrong = exact;
    wrong.request.retention.slot = 2;
    assert_eq!(cleaned.accept_lifecycle_terminal_observation(&wrong, 150), Err(DecisionError::Stale));
    assert_eq!(cleaned.accept_lifecycle_terminal_observation(&pending, 150).unwrap(), cleaned);
    assert!(cleaned.is_valid());
}
