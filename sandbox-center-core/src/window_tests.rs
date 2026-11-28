//! Pure current-contract regressions, not network, durable stores or KVM retirement.
use super::*;
use sandproof_protocol::internal::{
    lifecycle::RESULT_CONFIRMATION_BODY_BYTES, window::RETENTION_LANE_BYTES,
};
use sandproof_protocol::lifecycle::{
    ClosedLifecycleWindow, LifecycleWindowClose, RENEWAL_CONFIRMATION_BODY_BYTES,
    TERMINAL_RESULT_BODY_BYTES,
};
fn make_create(image: &CenterState) -> (CenterState, AcceptedLifecycleIntent) {
    let owner = "1".try_into().unwrap();
    let key = image.next_sdk_operation_key(&owner).unwrap();
    image
        .accept_lifecycle(&owner, &create_submission(key, binding()), 0)
        .unwrap()
}
fn ack(close: &LifecycleWindowClose) -> ClosedLifecycleWindow {
    ClosedLifecycleWindow {
        terminal: close.terminal.clone(),
        retired_through: close.terminal.request.retention.window,
    }
}
fn retire(
    image: &CenterState,
    result: &TerminalLifecycleObservation,
) -> (CenterState, ClosedLifecycleWindow) {
    let (pending, close) = image
        .confirm_terminal(&confirmed(result))
        .unwrap()
        .begin_close(result, 250)
        .unwrap();
    let answer = ack(&close);
    (pending.apply_close_ack(&answer, 250).unwrap(), answer)
}
#[test]
fn future_envelope_explicitly_reserves_confirmations_and_every_retention_lane() {
    let image = state();
    let b = &image.bounds;
    let prior = usize::from(b.max_sandboxes)
        * (usize::from(b.operations_per_sandbox) * (b.max_options_bytes as usize * 4 + 8192)
            + 8192)
        + usize::from(b.max_nodes) * (4096 + usize::from(b.max_templates_per_node) * 512)
        + 8192;
    assert_eq!(
        b.required_block_bytes(),
        prior
            + usize::from(b.max_sandboxes) * usize::from(b.operations_per_sandbox) * 128
            + usize::from(b.max_sandboxes)
                * (RESULT_CONFIRMATION_BODY_BYTES
                    + RENEWAL_CONFIRMATION_BODY_BYTES
                    + TERMINAL_RESULT_BODY_BYTES
                    + 256)
            + usize::from(b.max_nodes) * usize::from(b.max_sandboxes) * RETENTION_LANE_BYTES
    );
    for slots in [0, b.max_sandboxes + 1] {
        let mut nodes = image.nodes.clone();
        nodes[0].retention_slots = slots;
        assert_eq!(
            CenterState::new(
                image.deployment.clone(),
                b.clone(),
                nodes,
                image.tenants.clone(),
                center(),
                request_windows()
            ),
            Err(DecisionError::Invalid)
        );
    }
}
#[test]
fn retirement_requires_nonempty_exactly_confirmed_terminal_history() {
    let initial = state();
    assert!(initial.retirement_candidates(250).is_empty());
    let (unknown, create) = make_create(&initial);
    assert!(unknown.retirement_candidates(250).is_empty());
    let (terminal, result) = refused_and_cleaned(&unknown, &create);
    assert!(terminal.retirement_candidates(250).is_empty());
    assert_eq!(
        terminal.begin_close(&result, 250),
        Err(DecisionError::Conflict)
    );
    let saved = terminal.confirm_terminal(&confirmed(&result)).unwrap();
    assert!(saved.retirement_candidates(249).is_empty());
    assert_eq!(
        saved.begin_close(&result, 249),
        Err(DecisionError::Unavailable)
    );
    assert_eq!(saved.retirement_candidates(250), vec![result.clone()]);
    let (pending, close) = saved.begin_close(&result, 250).unwrap();
    assert!(pending.retirement_candidates(250).is_empty());
    assert_eq!(
        pending.apply_close_ack(&ack(&close), 249),
        Err(DecisionError::Unavailable)
    );
    let successor = pending.apply_close_ack(&ack(&close), 250).unwrap();
    assert!(successor.retirement_candidates(250).is_empty());
    assert_eq!(
        successor
            .lane(&create.request.binding.node, 1)
            .unwrap()
            .current()
            .unwrap()
            .get(),
        2
    );
    assert_eq!(
        successor
            .lane(&create.request.binding.node, 2)
            .unwrap()
            .current()
            .unwrap()
            .get(),
        1
    );
}
#[test]
fn operational_drain_stops_creates_but_not_existing_renewal_or_terminal_retirement() {
    let host = node("host-a");
    let (initial, create) = make_create(&state());
    let running = save_ready(&initial, &create).begin_drain(&host).unwrap();
    let owner = create.request.binding.key.owner.clone();
    assert_eq!(running.nodes[0].mode, NodeMode::Draining);
    assert_eq!(running.lane(&host, 1).unwrap().phase(), LanePhase::Open);
    assert_eq!(running.begin_drain(&host).unwrap(), running);
    let key = running.next_sdk_operation_key(&owner).unwrap();
    assert_eq!(
        running.accept_lifecycle(&owner, &create_submission(key.clone(), binding()), 0),
        Err(DecisionError::Unavailable)
    );
    assert_eq!(running.identity_frontiers(), (1, 1));
    let (renewing, renewal) = admit_renew(&running, &create, key).unwrap();
    assert_eq!(renewal.retention, create.retention);
    let renewed = renewing
        .accept_lifecycle_create_observation(&observed(&create, &renewal, 600))
        .unwrap();
    let key = renewed.next_sdk_operation_key(&owner).unwrap();
    let (destroying, destroy) = admit_destroy(&renewed, &renewal, key).unwrap();
    assert_eq!(destroy.retention, create.retention);
    assert!(destroying.retirement_candidates(250).is_empty());
}
#[test]
fn unknown_ready_pending_and_unconfirmed_cleaned_cannot_close() {
    let (unknown, create) = make_create(&state());
    let running = save_ready(&unknown, &create);
    let key = running
        .next_sdk_operation_key(&create.request.binding.key.owner)
        .unwrap();
    let (destroying, destroy) = admit_destroy(&running, &create, key).unwrap();
    // Current Pending carries no completed physical stop or cleanup fact.
    let pending = destroying
        .accept_lifecycle_terminal_observation(
            &TerminalLifecycleObservation {
                request: destroy.clone(),
                status: TerminalStatus::Pending,
            },
            150,
        )
        .unwrap();
    assert_eq!(pending, destroying);
    let (cleaned, result) = save_cleaned(&pending, &destroy);
    for image in [unknown, running, destroying, pending, cleaned.clone()] {
        assert!(image.begin_close(&result, 250).is_err());
        assert!(image.retirement_candidates(250).is_empty());
    }
    let saved = cleaned.confirm_terminal(&confirmed(&result)).unwrap();
    assert_eq!(saved.confirm_terminal(&confirmed(&result)).unwrap(), saved);
    assert!(saved.begin_close(&result, 250).is_ok());
}
#[test]
fn terminal_confirmation_matches_all_fields_and_cannot_certify_running() {
    let (initial, create) = make_create(&state());
    let running = save_ready(&initial, &create);
    let fake = TerminalLifecycleObservation {
        request: create.clone(),
        status: TerminalStatus::Cleaned,
    };
    assert_eq!(
        running.confirm_terminal(&confirmed(&fake)),
        Err(DecisionError::Stale)
    );
    let (terminal, result) = refused_and_cleaned(&initial, &create);
    let mut variants = vec![result.clone(); 10];
    variants[0].request.accepted_version += 1;
    variants[1].request.request.binding.key.sequence = 99.try_into().unwrap();
    variants[2].request.retention.window = 2.try_into().unwrap();
    variants[3].request.request.binding.node.node_generation = 2.try_into().unwrap();
    variants[4].request.request.binding.key.owner = "2".try_into().unwrap();
    variants[5].request.request.binding.session = "sandbox-2".try_into().unwrap();
    variants[6].request.request.binding.protocol_version += 1;
    variants[7].status = TerminalStatus::Pending;
    variants[8].request.retention.slot = 2;
    variants[9].request.center.center = "other-center".try_into().unwrap();
    for altered in variants {
        assert!(terminal.confirm_terminal(&confirmed(&altered)).is_err());
        assert!(terminal.sandboxes[0].terminal_confirmation.is_none());
    }
    assert_eq!(terminal.reserved(None, None), Some(Resources::ZERO));
}
#[test]
fn lost_close_ack_keeps_exact_outbox_and_occupied_lane_without_blocking_a_peer() {
    let (initial, create) = make_create(&state());
    let (terminal, result) = refused_and_cleaned(&initial, &create);
    let (pending, close) = terminal
        .confirm_terminal(&confirmed(&result))
        .unwrap()
        .begin_close(&result, 250)
        .unwrap();
    assert_eq!(pending.pending_closes(), vec![close.clone()]);
    assert_eq!(pending.sandboxes.len(), 1);
    assert_eq!(
        pending
            .lane(&create.request.binding.node, 1)
            .unwrap()
            .retired_through(),
        0
    );
    assert_eq!(
        pending.begin_close(&result, 250).unwrap(),
        (pending.clone(), close.clone())
    );
    let (peer, request) = make_create(&pending);
    assert_eq!(request.retention.slot, 2);
    assert_eq!(peer.pending_closes(), vec![close]);
}
#[test]
fn survivor_does_not_pin_long_churn_or_global_ids_or_native_position_generations() {
    let (initial, survivor) = make_create(&state());
    let mut image = save_ready(&initial, &survivor);
    let mut old_results = vec![];
    let mut old_acks: Vec<ClosedLifecycleWindow> = vec![];
    for round in 1..=32 {
        let (next, victim) = make_create(&image);
        assert_eq!(victim.retention.slot, 2);
        assert_eq!(victim.retention.window.get(), round);
        assert_eq!(
            victim.request.binding.session.as_protocol_id().as_str(),
            format!("sandbox-{}", round + 1)
        );
        let (terminal, result) = refused_and_cleaned(&next, &victim);
        let (next, answer) = retire(&terminal, &result);
        image = next;
        assert!(image.is_valid());
        assert_eq!(image.sandboxes.len(), 1);
        assert_eq!(image.sandboxes[0].operations[0].intent, survivor);
        assert_eq!(image.reserved(None, None), Some(resources(1)));
        assert_eq!(
            image
                .lane(&survivor.request.binding.node, 1)
                .unwrap()
                .current()
                .unwrap()
                .get(),
            1
        );
        assert_eq!(
            image
                .lane(&survivor.request.binding.node, 2)
                .unwrap()
                .retired_through(),
            round
        );
        assert_eq!(image.apply_close_ack(&answer, 250).unwrap(), image);
        for stale in &old_acks {
            assert_eq!(image.apply_close_ack(stale, 250), Err(DecisionError::Stale));
        }
        old_results.push(result);
        old_acks.push(answer);
        for stale in &old_results {
            assert_eq!(
                image.accept_lifecycle_terminal_observation(stale, 250),
                Err(DecisionError::NotFound)
            );
            assert_eq!(
                image.confirm_terminal(&confirmed(stale)),
                Err(DecisionError::NotFound)
            );
        }
        image = serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
        assert!(image.is_valid());
    }
    let (successor, _) = make_create(&image);
    assert_eq!(
        successor
            .apply_close_ack(old_acks.last().unwrap(), 250)
            .unwrap(),
        successor
    );
    let mut changed = old_acks.last().unwrap().clone();
    changed.terminal.request.request.binding.key.sequence = changed
        .terminal
        .request
        .request
        .binding
        .key
        .sequence
        .checked_next()
        .unwrap();
    assert_eq!(
        successor.apply_close_ack(&changed, 250),
        Err(DecisionError::Stale)
    );
    // Each refused Create retains a separate original Destroy until Cleaned.
    assert_eq!(image.identity_frontiers(), (33, 65));
}
#[test]
fn wrong_or_premature_close_ack_never_advances_a_lane() {
    let (initial, create) = make_create(&state());
    let fake = TerminalLifecycleObservation {
        request: create.clone(),
        status: TerminalStatus::Cleaned,
    };
    assert!(
        initial
            .apply_close_ack(&ack(&LifecycleWindowClose { terminal: fake }), 250)
            .is_err()
    );
    let (terminal, result) = refused_and_cleaned(&initial, &create);
    let (pending, request) = terminal
        .confirm_terminal(&confirmed(&result))
        .unwrap()
        .begin_close(&result, 250)
        .unwrap();
    let mut variants = vec![ack(&request); 9];
    variants[0]
        .terminal
        .request
        .request
        .binding
        .protocol_version += 1;
    variants[1]
        .terminal
        .request
        .request
        .binding
        .node
        .deployment_id = "other-deployment".try_into().unwrap();
    variants[2].terminal.request.request.binding.node.node_id = "other-node".try_into().unwrap();
    variants[3]
        .terminal
        .request
        .request
        .binding
        .node
        .node_generation = 2.try_into().unwrap();
    variants[4].terminal.request.retention.window = 2.try_into().unwrap();
    variants[5].terminal.request.retention.slot = 2;
    variants[6].terminal.request.accepted_version += 1;
    variants[7].terminal.status = TerminalStatus::Pending;
    variants[8].retired_through = 2.try_into().unwrap();
    for altered in variants {
        assert!(pending.apply_close_ack(&altered, 250).is_err());
        assert_eq!(
            pending
                .lane(&create.request.binding.node, 1)
                .unwrap()
                .retired_through(),
            0
        );
    }
}
#[test]
fn health_and_catalog_refresh_never_reset_lanes_or_change_protected_slot_count() {
    let host = node("host-a");
    let initial = state().begin_drain(&host).unwrap();
    let mut observation = initial.nodes[0].clone();
    observation.healthy_until_ms = 10_000;
    let updated = initial.observe_node(observation.clone()).unwrap();
    assert_eq!(updated.retention_lanes, initial.retention_lanes);
    observation.retention_slots = 1;
    assert_eq!(
        updated.observe_node(observation.clone()),
        Err(DecisionError::Conflict)
    );
    observation = initial.nodes[0].clone();
    observation.identity = node("host-b");
    assert_eq!(
        updated.observe_node(observation.clone()),
        Err(DecisionError::NotFound)
    );
    assert_eq!(
        updated.begin_drain(&observation.identity),
        Err(DecisionError::NotFound)
    );
}
#[test]
fn retirement_of_another_node_preserves_active_responsibility() {
    let mut initial = state();
    initial.bounds.max_nodes = 2;
    let mut second = initial.nodes[0].clone();
    second.identity = node("host-b");
    initial.nodes.push(second);
    let initial = CenterState::new(
        initial.deployment,
        initial.bounds,
        initial.nodes,
        initial.tenants,
        center(),
        request_windows(),
    )
    .unwrap();
    let (unknown, first) = make_create(&initial);
    let draining = unknown.begin_drain(&node("host-a")).unwrap();
    let (next, second) = make_create(&draining);
    assert_eq!(second.request.binding.node, node("host-b"));
    let (terminal, result) = refused_and_cleaned(&next, &second);
    let (closed, _) = retire(&terminal, &result);
    assert_eq!(closed.sandboxes.len(), 1);
    assert_eq!(closed.sandboxes[0].operations[0].intent, first);
    assert_eq!(
        closed.lane(&node("host-a"), 1).unwrap().retired_through(),
        0
    );
    assert_eq!(
        closed.lane(&node("host-b"), 1).unwrap().retired_through(),
        1
    );
}
#[test]
fn maximum_incarnation_exhausts_only_its_lane_without_wrap() {
    let (next, create) = make_create(&state());
    let (terminal, result) = refused_and_cleaned(&next, &create);
    let (mut prior, _) = retire(&terminal, &result);
    prior.retention_lanes[0].retired_through = u64::MAX - 1;
    prior.retention_lanes[0]
        .last_retired
        .as_mut()
        .unwrap()
        .request
        .retention
        .window = (u64::MAX - 1).try_into().unwrap();
    assert!(prior.is_valid());
    let (next, create) = make_create(&prior);
    assert_eq!(create.retention.window.get(), u64::MAX);
    let (terminal, result) = refused_and_cleaned(&next, &create);
    let (exhausted, answer) = retire(&terminal, &result);
    assert_eq!(
        exhausted
            .lane(&create.request.binding.node, 1)
            .unwrap()
            .current(),
        None
    );
    assert_eq!(
        exhausted
            .lane(&create.request.binding.node, 1)
            .unwrap()
            .phase(),
        LanePhase::Exhausted
    );
    assert_eq!(exhausted.apply_close_ack(&answer, 250).unwrap(), exhausted);
    let (_, peer) = make_create(&exhausted);
    assert_eq!(peer.retention.slot, 2);
    assert_eq!(peer.retention.window.get(), 1);
}
#[test]
fn restore_rejects_lane_gaps_duplicates_wrong_nodes_and_allocator_regressions() {
    let initial = state();
    let mut bad = initial.clone();
    bad.retention_lanes[0].retired_through = 1;
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.retention_lanes[0].phase = LanePhase::Exhausted;
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.retention_lanes[0].node.node_generation = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.retention_lanes[0].retention_slot = 0;
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.retention_lanes.push(bad.retention_lanes[0].clone());
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.retention_lanes.clear();
    assert!(!bad.is_valid());
    let (unknown, create) = make_create(&initial);
    let mut bad = unknown.clone();
    bad.retention_lanes[0].phase = LanePhase::ClosePending;
    assert!(!bad.is_valid());
    let (terminal, result) = refused_and_cleaned(&unknown, &create);
    let (retired, _) = retire(&terminal, &result);
    let mut bad = retired.clone();
    bad.sandbox_highwater = 0;
    assert!(!bad.is_valid());
    let mut bad = retired.clone();
    bad.operation_highwater = 0;
    assert!(!bad.is_valid());
    let mut bad = retired.clone();
    bad.retention_lanes[0]
        .last_retired
        .as_mut()
        .unwrap()
        .request
        .retention
        .slot = 3;
    assert!(!bad.is_valid());
    let mut bad = terminal.confirm_terminal(&confirmed(&result)).unwrap();
    bad.sandboxes[0]
        .terminal_confirmation
        .as_mut()
        .unwrap()
        .request
        .retention
        .window = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = unknown.clone();
    bad.sandboxes[0].operations[0].intent.retention.slot = 3;
    assert!(!bad.is_valid());
    let mut bad = save_ready(&unknown, &create);
    bad.sandboxes[0].terminal_confirmation = Some(TerminalLifecycleObservation {
        request: create,
        status: TerminalStatus::Cleaned,
    });
    assert!(!bad.is_valid());
}
#[test]
fn recent_stamp_global_ids_are_unique_across_retired_and_live_lanes() {
    let (first_state, first) = make_create(&state());
    let (both, second) = make_create(&first_state);
    let (terminal, result1) = refused_and_cleaned(&both, &first);
    let (terminal, result2) = refused_and_cleaned(&terminal, &second);
    let (one, _) = retire(&terminal, &result1);
    let (closed, _) = retire(&one, &result2);
    let mut bad = closed.clone();
    let mut duplicate = bad.retention_lanes[0].last_retired.clone().unwrap();
    duplicate.request.retention.slot = 2;
    bad.retention_lanes[1].last_retired = Some(duplicate);
    assert!(!bad.is_valid());
    let (live, current) = make_create(&closed);
    let mut bad = live.clone();
    bad.retention_lanes[1]
        .last_retired
        .as_mut()
        .unwrap()
        .request
        .request
        .binding
        .session = current.request.binding.session;
    assert!(!bad.is_valid());
    let mut bad = live;
    bad.retention_lanes[1]
        .last_retired
        .as_mut()
        .unwrap()
        .request
        .request
        .binding
        .key = current.request.binding.key;
    assert!(!bad.is_valid());
}
