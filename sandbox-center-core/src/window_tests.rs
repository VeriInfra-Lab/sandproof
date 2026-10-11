//! Pure component regressions, NOT network/Redis/SQLite or KVM retirement.
use super::*;
use sandproof_protocol::internal::lifecycle::{
    LifecycleRequest, LifecycleResponse, RESULT_CONFIRMATION_BODY_BYTES,
};
use sandproof_protocol::internal::window::{
    WINDOW_CLOSE_BODY_BYTES, WindowCloseAck, WindowCloseRequest,
};

fn create(state: &CenterState) -> (CenterState, LifecycleRequest) {
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
        identity: request.identity.clone(),
    }
}
fn not_accepted(
    state: &CenterState,
    request: &LifecycleRequest,
) -> (CenterState, LifecycleResponse) {
    let result = response(request, 1, LifecycleFact::NotAccepted);
    (state.apply_response(&result).unwrap(), result)
}

#[test]
fn future_envelope_explicitly_reserves_confirmations_and_node_windows() {
    let state = state();
    let bounds = &state.bounds;
    let prior = usize::from(bounds.max_sandboxes)
        * (usize::from(bounds.operations_per_sandbox)
            * (bounds.max_options_bytes as usize * 4 + 8192)
            + 8192)
        + usize::from(bounds.max_nodes) * (4096 + usize::from(bounds.max_templates_per_node) * 512)
        + 8192;
    assert_eq!(
        bounds.required_block_bytes(),
        prior
            + usize::from(bounds.max_sandboxes) * RESULT_CONFIRMATION_BODY_BYTES
            + usize::from(bounds.max_nodes) * WINDOW_CLOSE_BODY_BYTES
    );
}

#[test]
fn idle_retirement_requires_nonempty_exactly_confirmed_terminal_history() {
    let initial = state();
    assert!(initial.idle_window_candidates().is_empty());
    let (pending, create) = create(&initial);
    assert!(pending.idle_window_candidates().is_empty());
    let (terminal, result) = not_accepted(&pending, &create);
    assert!(terminal.idle_window_candidates().is_empty());
    let confirmed = terminal.confirm_terminal(&result).unwrap();
    assert_eq!(
        confirmed.idle_window_candidates(),
        vec![create.identity.node.clone()]
    );
    let draining = confirmed.begin_drain(&create.identity.node).unwrap();
    assert_eq!(
        draining.idle_window_candidates(),
        vec![create.identity.node.clone()]
    );
    let (closing, close) = draining.begin_close(&create.identity.node).unwrap();
    assert!(closing.idle_window_candidates().is_empty());
    let successor = closing.apply_close_ack(&ack(&close)).unwrap();
    assert!(successor.idle_window_candidates().is_empty());
    assert_eq!(
        successor
            .window(&create.identity.node)
            .unwrap()
            .current()
            .unwrap()
            .get(),
        2
    );
}

#[test]
fn drain_stops_new_creates_without_consuming_ids_or_closing_live_work() {
    let host = node("host-a");
    let initial = state();
    assert_eq!(initial.begin_close(&host), Err(DecisionError::Conflict));
    let (initial, create) = create(&initial);
    let running = ready(&initial, &create).begin_drain(&host).unwrap();
    assert_eq!(
        running.window(&host).unwrap().phase(),
        WindowPhase::Draining
    );
    assert_eq!(running.begin_drain(&host).unwrap(), running);
    assert_eq!(
        running.create(&"1".try_into().unwrap(), binding(), 0),
        Err(DecisionError::Capacity)
    );
    assert_eq!(running.sandbox_highwater, 1);
    assert_eq!(running.operation_highwater, 1);
    // Draining is not expiry/revocation: ordinary existing lifecycle operations
    // retain the original window, and a final destroy slot remains available.
    let (renewing, renewal) = running
        .renew(
            &"1".try_into().unwrap(),
            &create.identity.sandbox_id,
            500,
            150,
        )
        .unwrap();
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
        destroy.identity.request_window,
        create.identity.request_window
    );
    assert_eq!(destroying.begin_close(&host), Err(DecisionError::Conflict));
}

#[test]
fn unknown_ready_stopped_and_unconfirmed_terminal_windows_cannot_close() {
    let host = node("host-a");
    let (unknown, create) = create(&state());
    let running = ready(&unknown, &create);
    let (destroying, destroy) = running
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    let stopped = destroying
        .apply_response(&response(&destroy, 2, LifecycleFact::Stopped))
        .unwrap();
    let cleaned_result = response(&destroy, 3, LifecycleFact::Cleaned);
    let cleaned = stopped.apply_response(&cleaned_result).unwrap();
    for state in [unknown, running, destroying, stopped, cleaned.clone()] {
        let draining = state.begin_drain(&host).unwrap();
        assert_eq!(draining.begin_close(&host), Err(DecisionError::Conflict));
    }
    // Two-operation history has no ordinary compaction work, but still MUST
    // receive the exact terminal acknowledgement before its window closes.
    assert!(!cleaned.sandboxes[0].has_compaction_work());
    let confirmed = cleaned.confirm_terminal(&cleaned_result).unwrap();
    assert_eq!(
        confirmed.confirm_terminal(&cleaned_result).unwrap(),
        confirmed
    );
    assert!(
        confirmed
            .begin_drain(&host)
            .unwrap()
            .begin_close(&host)
            .is_ok()
    );
}

#[test]
fn terminal_confirmation_matches_every_field_and_cannot_certify_running() {
    let (initial, create) = create(&state());
    let running_result = response(
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
            .apply_response(&running_result)
            .unwrap()
            .confirm_terminal(&running_result),
        Err(DecisionError::Stale)
    );
    let (terminal, result) = not_accepted(&initial, &create);
    let mut variants = vec![result.clone(); 8];
    variants[0].observation_version = 2.try_into().unwrap();
    variants[1].identity.operation_id = "other-operation".try_into().unwrap();
    variants[2].identity.request_window = 2.try_into().unwrap();
    variants[3].identity.node.node_generation = 2.try_into().unwrap();
    variants[4].identity.tenant_id = "2".try_into().unwrap();
    variants[5].identity.sandbox_id = "other-sandbox".try_into().unwrap();
    variants[6].identity.protocol_version = 2;
    variants[7].fact = LifecycleFact::Cleaned;
    for altered in variants {
        assert!(terminal.confirm_terminal(&altered).is_err());
        assert!(terminal.sandboxes[0].terminal_confirmation.is_none());
    }
    assert_eq!(terminal.reserved(None, None), Some(Resources::ZERO));
    assert!(terminal.confirm_terminal(&result).unwrap().is_valid());
}

#[test]
fn lost_close_ack_keeps_exact_outbox_history_and_same_retry_identity() {
    let host = node("host-a");
    let (initial, create) = create(&state());
    let (terminal, result) = not_accepted(&initial, &create);
    let confirmed = terminal
        .confirm_terminal(&result)
        .unwrap()
        .begin_drain(&host)
        .unwrap();
    let (pending, close) = confirmed.begin_close(&host).unwrap();
    assert_eq!(pending.pending_closes(), vec![close.clone()]);
    assert_eq!(pending.sandboxes.len(), 1);
    assert_eq!(pending.window(&host).unwrap().retired_through(), 0);
    assert_eq!(
        pending.begin_close(&host).unwrap(),
        (pending.clone(), close.clone())
    );
    assert_eq!(pending.begin_drain(&host).unwrap(), pending);
    // The lost response has no event to apply. It is NOT an individual Cleaned
    // result and cannot authorize a new window or delete its retained details.
    assert_eq!(
        pending.create(&"1".try_into().unwrap(), binding(), 0),
        Err(DecisionError::Capacity)
    );
}

#[test]
fn sequential_retirement_is_bounded_and_never_reuses_global_or_window_ids() {
    let host = node("host-a");
    let mut state = state();
    let mut old_results = vec![];
    let mut old_acks = vec![];
    for round in 1..=32 {
        let (next, create) = create(&state);
        assert_eq!(create.identity.request_window.get(), round);
        assert_eq!(
            create.identity.sandbox_id.as_str(),
            format!("sandbox-{round}")
        );
        assert_eq!(
            create.identity.operation_id.as_str(),
            format!("operation-{round}")
        );
        let (terminal, result) = not_accepted(&next, &create);
        state = terminal
            .confirm_terminal(&result)
            .unwrap()
            .begin_drain(&host)
            .unwrap();
        let (pending, close) = state.begin_close(&host).unwrap();
        let answer = ack(&close);
        state = pending.apply_close_ack(&answer).unwrap();
        assert!(state.is_valid());
        assert!(state.sandboxes.is_empty());
        assert!(state.pending_closes().is_empty());
        assert_eq!(
            state.window(&host).unwrap().current().unwrap().get(),
            round + 1
        );
        assert_eq!(state.window(&host).unwrap().retired_through(), round);
        assert_eq!(
            (state.sandbox_highwater, state.operation_highwater),
            (round, round)
        );
        old_results.push(result);
        old_acks.push(answer);
        for result in &old_results {
            assert_eq!(state.apply_response(result), Err(DecisionError::NotFound));
            assert_eq!(state.confirm_terminal(result), Err(DecisionError::NotFound));
        }
        for duplicate in &old_acks {
            assert_eq!(state.apply_close_ack(duplicate).unwrap(), state);
        }
    }
    let (live, _) = create(&state);
    for old in &old_acks {
        assert_eq!(live.apply_close_ack(old).unwrap(), live);
    }
}

#[test]
fn wrong_or_premature_close_ack_never_advances_a_window() {
    let host = node("host-a");
    let initial = state();
    let request = WindowCloseRequest::new(host.clone(), 1.try_into().unwrap());
    assert_eq!(
        initial.apply_close_ack(&ack(&request)),
        Err(DecisionError::Conflict)
    );
    let (pending, request) = initial
        .begin_drain(&host)
        .unwrap()
        .begin_close(&host)
        .unwrap();
    let mut variants = vec![ack(&request); 5];
    variants[0].identity.protocol_version = 2;
    variants[1].identity.node.deployment_id = "other-deployment".try_into().unwrap();
    variants[2].identity.node.node_id = "other-node".try_into().unwrap();
    variants[3].identity.node.node_generation = 2.try_into().unwrap();
    variants[4].identity.request_window = 2.try_into().unwrap();
    for altered in variants {
        assert!(pending.apply_close_ack(&altered).is_err());
        assert_eq!(pending.window(&host).unwrap().retired_through(), 0);
    }
}

#[test]
fn health_observation_never_resets_durable_window_and_unknown_nodes_are_not_initialized() {
    let host = node("host-a");
    let initial = state().begin_drain(&host).unwrap();
    let mut observation = initial.nodes[0].clone();
    observation.healthy_until_ms = 10_000;
    let updated = initial.observe_node(observation.clone()).unwrap();
    assert_eq!(updated.node_windows, initial.node_windows);
    assert_eq!(
        updated.create(&"1".try_into().unwrap(), binding(), 0),
        Err(DecisionError::Capacity)
    );
    observation.identity = node("host-b");
    assert_eq!(
        updated.observe_node(observation.clone()),
        Err(DecisionError::NotFound)
    );
    assert_eq!(
        updated.begin_drain(&observation.identity),
        Err(DecisionError::NotFound)
    );
    assert_eq!(
        updated.begin_close(&observation.identity),
        Err(DecisionError::NotFound)
    );
    assert_eq!(
        updated.apply_close_ack(&ack(&WindowCloseRequest::new(
            observation.identity,
            1.try_into().unwrap()
        ))),
        Err(DecisionError::NotFound)
    );
}

#[test]
fn blocked_node_does_not_close_or_remove_another_nodes_active_responsibility() {
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
    let (unknown, first) = create(&initial);
    assert_eq!(first.identity.node, node("host-a"));
    let draining = unknown.begin_drain(&node("host-a")).unwrap();
    let (next, second) = create(&draining);
    assert_eq!(second.identity.node, node("host-b"));
    let (terminal, result) = not_accepted(&next, &second);
    let confirmed = terminal.confirm_terminal(&result).unwrap();
    assert_eq!(
        confirmed.begin_close(&node("host-a")),
        Err(DecisionError::Conflict)
    );
    let (pending, close) = confirmed
        .begin_drain(&node("host-b"))
        .unwrap()
        .begin_close(&node("host-b"))
        .unwrap();
    let closed = pending.apply_close_ack(&ack(&close)).unwrap();
    assert_eq!(closed.sandboxes.len(), 1);
    assert_eq!(closed.sandboxes[0].operations[0].request, first);
    assert_eq!(closed.reserved(None, None), Some(resources(1)));
    assert_eq!(closed.window(&node("host-a")).unwrap().retired_through(), 0);
    assert_eq!(closed.window(&node("host-b")).unwrap().retired_through(), 1);
}

#[test]
fn maximum_window_closes_without_wrapping_or_preventing_terminal_retirement() {
    let host = node("host-a");
    let mut initial = state();
    initial.node_windows[0].retired_through = u64::MAX - 1;
    initial.node_windows[0].current = Some(u64::MAX.try_into().unwrap());
    assert!(initial.is_valid());
    let (next, create) = create(&initial);
    assert_eq!(create.identity.request_window.get(), u64::MAX);
    let (terminal, result) = not_accepted(&next, &create);
    let (pending, close) = terminal
        .confirm_terminal(&result)
        .unwrap()
        .begin_drain(&host)
        .unwrap()
        .begin_close(&host)
        .unwrap();
    let exhausted = pending.apply_close_ack(&ack(&close)).unwrap();
    assert!(exhausted.is_valid());
    assert!(exhausted.sandboxes.is_empty());
    assert_eq!(exhausted.window(&host).unwrap().current(), None);
    assert_eq!(exhausted.window(&host).unwrap().retired_through(), u64::MAX);
    assert_eq!(
        exhausted.window(&host).unwrap().phase(),
        WindowPhase::Exhausted
    );
    assert_eq!(exhausted.apply_close_ack(&ack(&close)).unwrap(), exhausted);
    assert_eq!(exhausted.begin_drain(&host), Err(DecisionError::Exhausted));
    assert_eq!(exhausted.begin_close(&host), Err(DecisionError::Exhausted));
    assert_eq!(
        exhausted.create(&"1".try_into().unwrap(), binding(), 0),
        Err(DecisionError::Capacity)
    );
    assert_eq!(
        (exhausted.sandbox_highwater, exhausted.operation_highwater),
        (1, 1)
    );
}

#[test]
fn recovery_rejects_window_gaps_duplicates_foreign_nodes_and_unbound_terminal_stamps() {
    let initial = state();
    let mut bad = initial.clone();
    bad.node_windows[0].current = Some(2.try_into().unwrap());
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.node_windows[0].phase = WindowPhase::Exhausted;
    bad.node_windows[0].current = None;
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.node_windows[0].node.node_generation = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.node_windows.push(bad.node_windows[0].clone());
    assert!(!bad.is_valid());
    let mut bad = initial.clone();
    bad.node_windows.clear();
    assert!(!bad.is_valid());
    let (unknown, create) = create(&initial);
    let mut bad = unknown.clone();
    bad.node_windows[0].phase = WindowPhase::ClosePending;
    assert!(!bad.is_valid());
    let (terminal, result) = not_accepted(&unknown, &create);
    let mut bad = terminal.confirm_terminal(&result).unwrap();
    bad.sandboxes[0]
        .terminal_confirmation
        .as_mut()
        .unwrap()
        .identity
        .request_window = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = terminal;
    bad.sandboxes[0].operations[0]
        .request
        .identity
        .request_window = 2.try_into().unwrap();
    assert!(!bad.is_valid());
    let mut bad = ready(&unknown, &create);
    bad.sandboxes[0].terminal_confirmation = bad.sandboxes[0].operations[0].response.clone();
    assert!(!bad.is_valid());
    assert_eq!(
        bad.begin_drain(&node("host-a")),
        Err(DecisionError::Invalid)
    );
}
