//! Pure component regressions, NOT network/Redis/SQLite or KVM retirement.
use super::*;
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleRequest, LifecycleResponse, RESULT_CONFIRMATION_BODY_BYTES,
};
use sandproof_protocol::internal::window::{
    RETENTION_LANE_BYTES, WindowCloseAck, WindowCloseRequest,
};
fn make_create(state: &CenterState) -> (CenterState, LifecycleRequest) {
    state
        .create(&"1".try_into().unwrap(), binding(), 0)
        .unwrap()
}
fn ready(state: &CenterState, request: &LifecycleRequest) -> CenterState {
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
fn ack(request: &WindowCloseRequest) -> WindowCloseAck {
    WindowCloseAck {
        terminal: request.terminal.clone(),
    }
}
fn not_accepted(
    state: &CenterState,
    request: &LifecycleRequest,
) -> (CenterState, LifecycleResponse) {
    let result = response(request, 1, LifecycleFact::NotAccepted);
    (state.apply_response(&result).unwrap(), result)
}
fn retire(state: &CenterState, result: &LifecycleResponse) -> (CenterState, WindowCloseAck) {
    let (pending, close) = state
        .confirm_terminal(result)
        .unwrap()
        .begin_close(result)
        .unwrap();
    let answer = ack(&close);
    (pending.apply_close_ack(&answer).unwrap(), answer)
}
#[test]
fn future_envelope_explicitly_reserves_confirmations_and_every_retention_lane() {
    let state = state();
    let b = &state.bounds;
    let prior = usize::from(b.max_sandboxes)
        * (usize::from(b.operations_per_sandbox) * (b.max_options_bytes as usize * 4 + 8192)
            + 8192)
        + usize::from(b.max_nodes) * (4096 + usize::from(b.max_templates_per_node) * 512)
        + 8192;
    assert_eq!(
        b.required_block_bytes(),
        prior
            + usize::from(b.max_sandboxes) * RESULT_CONFIRMATION_BODY_BYTES
            + usize::from(b.max_nodes) * usize::from(b.max_sandboxes) * RETENTION_LANE_BYTES
    );
    let mut nodes = state.nodes.clone();
    nodes[0].retention_slots = 0;
    assert_eq!(
        CenterState::new(
            state.deployment.clone(),
            b.clone(),
            nodes,
            state.tenants.clone()
        ),
        Err(DecisionError::Invalid)
    );
    let mut nodes = state.nodes.clone();
    nodes[0].retention_slots = b.max_sandboxes + 1;
    assert_eq!(
        CenterState::new(
            state.deployment.clone(),
            b.clone(),
            nodes,
            state.tenants.clone()
        ),
        Err(DecisionError::Invalid)
    );
}
#[test]
fn retirement_requires_nonempty_exactly_confirmed_terminal_history() {
    let initial = state();
    assert!(initial.retirement_candidates().is_empty());
    let (unknown, create) = make_create(&initial);
    assert!(unknown.retirement_candidates().is_empty());
    let (terminal, result) = not_accepted(&unknown, &create);
    assert!(terminal.retirement_candidates().is_empty());
    assert_eq!(terminal.begin_close(&result), Err(DecisionError::Conflict));
    let confirmed = terminal.confirm_terminal(&result).unwrap();
    assert_eq!(confirmed.retirement_candidates(), vec![result.clone()]);
    let (pending, close) = confirmed.begin_close(&result).unwrap();
    assert!(pending.retirement_candidates().is_empty());
    let successor = pending.apply_close_ack(&ack(&close)).unwrap();
    assert!(successor.retirement_candidates().is_empty());
    assert_eq!(
        successor
            .lane(&create.identity.node, 1)
            .unwrap()
            .current()
            .unwrap()
            .get(),
        2
    );
    assert_eq!(
        successor
            .lane(&create.identity.node, 2)
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
    let running = ready(&initial, &create).begin_drain(&host).unwrap();
    assert_eq!(running.nodes[0].mode, NodeMode::Draining);
    assert_eq!(running.lane(&host, 1).unwrap().phase(), LanePhase::Open);
    assert_eq!(running.begin_drain(&host).unwrap(), running);
    assert_eq!(
        running.create(&"1".try_into().unwrap(), binding(), 0),
        Err(DecisionError::Unavailable)
    );
    assert_eq!(
        (running.sandbox_highwater, running.operation_highwater),
        (1, 1)
    );
    let (renewing, renewal) = running
        .renew(
            &"1".try_into().unwrap(),
            &create.identity.sandbox_id,
            500,
            150,
        )
        .unwrap();
    assert_eq!(
        renewal.identity.retention_slot,
        create.identity.retention_slot
    );
    assert_eq!(
        renewal.identity.request_window,
        create.identity.request_window
    );
    let renewed = renewing
        .apply_response(&response(
            &renewal,
            2,
            LifecycleFact::Renewed { end_at_ms: 500 },
        ))
        .unwrap();
    let (destroying, destroy) = renewed
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    assert_eq!(
        destroy.identity.retention_slot,
        create.identity.retention_slot
    );
    assert_eq!(
        destroy.identity.request_window,
        create.identity.request_window
    );
    assert!(destroying.retirement_candidates().is_empty());
}
#[test]
fn unknown_ready_stopped_and_unconfirmed_cleaned_cannot_close() {
    let (unknown, create) = make_create(&state());
    let running = ready(&unknown, &create);
    let (destroying, destroy) = running
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    let stopped = destroying
        .apply_response(&response(&destroy, 2, LifecycleFact::Stopped))
        .unwrap();
    let result = response(&destroy, 3, LifecycleFact::Cleaned);
    let cleaned = stopped.apply_response(&result).unwrap();
    for state in [unknown, running, destroying, stopped, cleaned.clone()] {
        assert!(state.begin_close(&result).is_err());
        assert!(state.retirement_candidates().is_empty());
    }
    assert!(!cleaned.sandboxes[0].has_compaction_work());
    let confirmed = cleaned.confirm_terminal(&result).unwrap();
    assert_eq!(confirmed.confirm_terminal(&result).unwrap(), confirmed);
    assert!(confirmed.begin_close(&result).is_ok());
}
#[test]
fn terminal_confirmation_matches_all_fields_and_cannot_certify_running() {
    let (initial, create) = make_create(&state());
    let running = response(
        &create,
        1,
        LifecycleFact::Ready {
            started_at_ms: 100,
            end_at_ms: 200,
            max_end_at_ms: 600,
        },
    );
    assert_eq!(
        initial
            .apply_response(&running)
            .unwrap()
            .confirm_terminal(&running),
        Err(DecisionError::Stale)
    );
    let (terminal, result) = not_accepted(&initial, &create);
    let mut variants = vec![result.clone(); 9];
    variants[0].observation_version = 2.try_into().unwrap();
    variants[1].identity.operation_id = "other-operation".try_into().unwrap();
    variants[2].identity.request_window = 2.try_into().unwrap();
    variants[3].identity.node.node_generation = 2.try_into().unwrap();
    variants[4].identity.tenant_id = "2".try_into().unwrap();
    variants[5].identity.sandbox_id = "other-sandbox".try_into().unwrap();
    variants[6].identity.protocol_version = 3;
    variants[7].fact = LifecycleFact::Cleaned;
    variants[8].identity.retention_slot = 2;
    for altered in variants {
        assert!(terminal.confirm_terminal(&altered).is_err());
        assert!(terminal.sandboxes[0].terminal_confirmation.is_none());
    }
    assert_eq!(terminal.reserved(None, None), Some(Resources::ZERO));
}
#[test]
fn lost_close_ack_keeps_exact_outbox_and_occupied_lane_without_blocking_a_peer() {
    let (initial, create) = make_create(&state());
    let (terminal, result) = not_accepted(&initial, &create);
    let confirmed = terminal.confirm_terminal(&result).unwrap();
    let (pending, close) = confirmed.begin_close(&result).unwrap();
    assert_eq!(pending.pending_closes(), vec![close.clone()]);
    assert_eq!(pending.sandboxes.len(), 1);
    assert_eq!(
        pending
            .lane(&create.identity.node, 1)
            .unwrap()
            .retired_through(),
        0
    );
    assert_eq!(
        pending.begin_close(&result).unwrap(),
        (pending.clone(), close.clone())
    );
    let (peer, request) = make_create(&pending);
    assert_eq!(request.identity.retention_slot, 2);
    assert_eq!(peer.pending_closes(), vec![close]);
}
#[test]
fn survivor_does_not_pin_long_churn_or_global_ids_or_native_position_generations() {
    let (initial, survivor) = make_create(&state());
    let mut state = ready(&initial, &survivor);
    let mut old_results = vec![];
    let mut old_acks: Vec<WindowCloseAck> = vec![];
    for round in 1..=32 {
        let (next, victim) = make_create(&state);
        assert_eq!(victim.identity.retention_slot, 2);
        assert_eq!(victim.identity.request_window.get(), round);
        assert_eq!(
            victim.identity.sandbox_id.as_str(),
            format!("sandbox-{}", round + 1)
        );
        let (terminal, result) = not_accepted(&next, &victim);
        let (next, answer) = retire(&terminal, &result);
        state = next;
        assert!(state.is_valid());
        assert_eq!(state.sandboxes.len(), 1);
        assert_eq!(state.sandboxes[0].operations[0].request, survivor);
        assert_eq!(state.reserved(None, None), Some(resources(1)));
        assert_eq!(
            state
                .lane(&survivor.identity.node, 1)
                .unwrap()
                .current()
                .unwrap()
                .get(),
            1
        );
        assert_eq!(
            state
                .lane(&survivor.identity.node, 2)
                .unwrap()
                .retired_through(),
            round
        );
        assert_eq!(state.apply_close_ack(&answer).unwrap(), state);
        for stale in &old_acks {
            assert_eq!(state.apply_close_ack(stale), Err(DecisionError::Stale));
        }
        old_results.push(result);
        old_acks.push(answer);
        for stale in &old_results {
            assert_eq!(state.apply_response(stale), Err(DecisionError::NotFound));
            assert_eq!(state.confirm_terminal(stale), Err(DecisionError::NotFound));
        }
        // Restart semantics use the exact serialized bounded authority, not a rebuilt catalog.
        state = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(state.is_valid());
    }
    let (successor, _) = make_create(&state);
    assert_eq!(
        successor.apply_close_ack(old_acks.last().unwrap()).unwrap(),
        successor
    );
    let mut changed = old_acks.last().unwrap().clone();
    changed.terminal.observation_version =
        changed.terminal.observation_version.checked_next().unwrap();
    assert_eq!(
        successor.apply_close_ack(&changed),
        Err(DecisionError::Stale)
    );
    assert_eq!(
        (state.sandbox_highwater, state.operation_highwater),
        (33, 33)
    );
}
#[test]
fn wrong_or_premature_close_ack_never_advances_a_lane() {
    let initial = state();
    let fake = response(
        &fixture::request(1, 1, Intent::Create),
        1,
        LifecycleFact::NotAccepted,
    );
    assert!(
        initial
            .apply_close_ack(&ack(&WindowCloseRequest::new(fake)))
            .is_err()
    );
    let (next, create) = make_create(&initial);
    let (terminal, result) = not_accepted(&next, &create);
    let (pending, request) = terminal
        .confirm_terminal(&result)
        .unwrap()
        .begin_close(&result)
        .unwrap();
    let mut variants = vec![ack(&request); 8];
    variants[0].terminal.identity.protocol_version = 3;
    variants[1].terminal.identity.node.deployment_id = "other-deployment".try_into().unwrap();
    variants[2].terminal.identity.node.node_id = "other-node".try_into().unwrap();
    variants[3].terminal.identity.node.node_generation = 2.try_into().unwrap();
    variants[4].terminal.identity.request_window = 2.try_into().unwrap();
    variants[5].terminal.identity.retention_slot = 2;
    variants[6].terminal.observation_version = 2.try_into().unwrap();
    variants[7].terminal.fact = LifecycleFact::Cleaned;
    for altered in variants {
        assert!(pending.apply_close_ack(&altered).is_err());
        assert_eq!(
            pending
                .lane(&create.identity.node, 1)
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
    )
    .unwrap();
    let (unknown, first) = make_create(&initial);
    let draining = unknown.begin_drain(&node("host-a")).unwrap();
    let (next, second) = make_create(&draining);
    assert_eq!(second.identity.node, node("host-b"));
    let (terminal, result) = not_accepted(&next, &second);
    let (closed, _) = retire(&terminal, &result);
    assert_eq!(closed.sandboxes.len(), 1);
    assert_eq!(closed.sandboxes[0].operations[0].request, first);
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
    let initial = state();
    let (next, create) = make_create(&initial);
    let (terminal, result) = not_accepted(&next, &create);
    let (mut prior, _) = retire(&terminal, &result);
    prior.retention_lanes[0].retired_through = u64::MAX - 1;
    prior.retention_lanes[0]
        .last_retired
        .as_mut()
        .unwrap()
        .identity
        .request_window = (u64::MAX - 1).try_into().unwrap();
    assert!(prior.is_valid());
    let (next, create) = make_create(&prior);
    assert_eq!(create.identity.request_window.get(), u64::MAX);
    let (terminal, result) = not_accepted(&next, &create);
    let (exhausted, answer) = retire(&terminal, &result);
    assert_eq!(
        exhausted.lane(&create.identity.node, 1).unwrap().current(),
        None
    );
    assert_eq!(
        exhausted.lane(&create.identity.node, 1).unwrap().phase(),
        LanePhase::Exhausted
    );
    assert_eq!(exhausted.apply_close_ack(&answer).unwrap(), exhausted);
    let (_, peer) = make_create(&exhausted);
    assert_eq!(peer.identity.retention_slot, 2);
    assert_eq!(peer.identity.request_window.get(), 1);
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
    let (terminal, result) = not_accepted(&unknown, &create);
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
        .identity
        .retention_slot = 3;
    assert!(!bad.is_valid());
    let mut bad = terminal.confirm_terminal(&result).unwrap();
    bad.sandboxes[0]
        .terminal_confirmation
        .as_mut()
        .unwrap()
        .identity
        .request_window = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = unknown.clone();
    bad.sandboxes[0].operations[0]
        .request
        .identity
        .retention_slot = 3;
    assert!(!bad.is_valid());
    let mut bad = ready(&unknown, &create);
    bad.sandboxes[0].terminal_confirmation = bad.sandboxes[0].operations[0].response.clone();
    assert!(!bad.is_valid());
}

#[test]
fn recent_stamp_global_ids_are_unique_across_retired_and_live_lanes() {
    let (first_state, first) = make_create(&state());
    let (both, second) = make_create(&first_state);
    let (terminal, result1) = not_accepted(&both, &first);
    let (terminal, result2) = not_accepted(&terminal, &second);
    let (one, _) = retire(&terminal, &result1);
    let (closed, _) = retire(&one, &result2);
    let mut bad = closed.clone();
    let mut duplicate = bad.retention_lanes[0].last_retired.clone().unwrap();
    duplicate.identity.retention_slot = 2;
    bad.retention_lanes[1].last_retired = Some(duplicate);
    assert!(!bad.is_valid());
    let (live, current) = make_create(&closed);
    let mut bad = live.clone();
    bad.retention_lanes[1]
        .last_retired
        .as_mut()
        .unwrap()
        .identity
        .sandbox_id = current.identity.sandbox_id;
    assert!(!bad.is_valid());
    let mut bad = live;
    bad.retention_lanes[1]
        .last_retired
        .as_mut()
        .unwrap()
        .identity
        .operation_id = current.identity.operation_id;
    assert!(!bad.is_valid());
}
