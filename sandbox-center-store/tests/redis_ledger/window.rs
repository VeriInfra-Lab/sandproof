//! Real Redis commit/recovery checks; supplied host facts are controlled
//! component observations, not claims that this fixture launched a real VM.
use super::*;
fn terminal(center: &mut CenterAuthority, sequence: u64) -> TerminalLifecycleObservation {
    let create = accept_create(center, lifecycle_fixture::binding(), sequence);
    let destroy = accept_destroy(center, &create, sequence + 1);
    let cleaned = TerminalLifecycleObservation {
        request: destroy,
        status: TerminalStatus::Cleaned,
    };
    center
        .accept_lifecycle_terminal_observation(&cleaned, 100)
        .unwrap();
    cleaned
}
fn confirm(center: &mut CenterAuthority, result: &TerminalLifecycleObservation) {
    // Controlled host acknowledgement for the original Redis save contract.
    center
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
}

#[test]
fn bounded_lanes_retire_only_after_exact_durable_confirmation() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let node = lifecycle_fixture::node("host-a");
    for round in 1..=8 {
        let cleaned = terminal(&mut center, 2 * round - 1);
        assert_eq!(cleaned.request.retention.window.get(), round);
        assert!(center.begin_close(&cleaned, 200).is_err());
        confirm(&mut center, &cleaned);
        let close = center.begin_close(&cleaned, 200).unwrap();
        assert_eq!(center.close_batch(200).unwrap(), vec![close.clone()]);
        let mut wrong = close.clone();
        wrong.terminal.request.request.binding.node.node_generation = 2.try_into().unwrap();
        assert!(
            center
                .apply_close_ack(
                    &ClosedLifecycleWindow {
                        retired_through: wrong.terminal.request.retention.window,
                        terminal: wrong.terminal
                    },
                    200
                )
                .is_err()
        );
        center
            .apply_close_ack(
                &ClosedLifecycleWindow {
                    retired_through: close.terminal.request.retention.window,
                    terminal: close.terminal.clone(),
                },
                200,
            )
            .unwrap();
        assert!(center.snapshot().unwrap().sandboxes().is_empty());
        assert_eq!(
            center
                .state()
                .unwrap()
                .lane(&node, cleaned.request.retention.slot)
                .unwrap()
                .retired_through(),
            round
        );
        assert!(
            center
                .confirm_terminal(&ConfirmedLifecycleResult {
                    result: cleaned.clone()
                })
                .is_err()
        );
        // An exact already-retired ack does not erase/advance the successor.
        center
            .apply_close_ack(
                &ClosedLifecycleWindow {
                    retired_through: close.terminal.request.retention.window,
                    terminal: close.terminal,
                },
                200,
            )
            .unwrap();
        assert_eq!(
            center
                .state()
                .unwrap()
                .lane(&node, cleaned.request.retention.slot)
                .unwrap()
                .current()
                .unwrap()
                .get(),
            round + 1
        );
    }
    let create = accept_create(&mut center, lifecycle_fixture::binding(), 17);
    assert_eq!(
        create.request.binding.session.as_protocol_id().as_str(),
        "sandbox-9"
    );
    assert_eq!(center.state().unwrap().identity_frontiers(), (9, 17));
}

#[test]
fn lost_close_and_retirement_commits_recover_original_frontier() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let mut center = center_authority(&fixture, proxy.address);
    let node = lifecycle_fixture::node("host-a");
    let cleaned = terminal(&mut center, 1);
    confirm(&mut center, &cleaned);
    // Reaffirm precedes this pure decision. Lose the NEXT actual close CAS,
    // not the prerequisite read/confirmation, to exercise durable unknown.
    proxy.lose_after(1);
    assert_eq!(
        center.begin_close(&cleaned, 200),
        Err(AuthorityError::Store(StoreError::OutcomeUnknown))
    );
    assert!(matches!(
        center.close_batch(200),
        Err(AuthorityError::RecoveryRequired)
    ));
    center.recover().unwrap();
    let pending = center.close_batch(200).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].terminal.request.retention.window,
        cleaned.request.retention.window
    );
    let ack = ClosedLifecycleWindow {
        retired_through: pending[0].terminal.request.retention.window,
        terminal: pending[0].terminal.clone(),
    };
    proxy.lose_after(1);
    assert_eq!(
        center.apply_close_ack(&ack, 200),
        Err(AuthorityError::Store(StoreError::OutcomeUnknown))
    );
    assert!(center.state().is_err());
    center.recover().unwrap();
    assert!(center.close_batch(200).unwrap().is_empty());
    assert!(center.state().unwrap().sandboxes().is_empty());
    assert_eq!(
        center
            .state()
            .unwrap()
            .lane(&node, cleaned.request.retention.slot)
            .unwrap()
            .retired_through(),
        1
    );
    center.apply_close_ack(&ack, 200).unwrap();
    let create = accept_create(&mut center, lifecycle_fixture::binding(), 3);
    assert_eq!(
        create.request.binding.session.as_protocol_id().as_str(),
        "sandbox-2"
    );
    assert_eq!(create.retention.window.get(), 2);
}

#[test]
fn live_survivor_does_not_pin_its_peers_history_or_retention_capacity() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let survivor = accept_create(&mut center, lifecycle_fixture::binding(), 1);
    center
        .accept_lifecycle_create_observation(&lifecycle::observed(&survivor))
        .unwrap();
    let original = center.snapshot().unwrap().sandboxes()[0].clone();
    let mut previous_ack = None;
    for round in 1..=12 {
        let cleaned = terminal(&mut center, 2 * round);
        assert_ne!(cleaned.request.retention.slot, survivor.retention.slot);
        assert_eq!(cleaned.request.retention.window.get(), round);
        confirm(&mut center, &cleaned);
        let close = center.begin_close(&cleaned, 200).unwrap();
        let acknowledgement = ClosedLifecycleWindow {
            retired_through: close.terminal.request.retention.window,
            terminal: close.terminal,
        };
        // A different slot with the SAME generation must not authorize this one.
        let mut foreign = acknowledgement.clone();
        foreign.terminal.request.retention.slot = survivor.retention.slot;
        assert!(center.apply_close_ack(&foreign, 200).is_err());
        center.apply_close_ack(&acknowledgement, 200).unwrap();
        if let Some(stale) = &previous_ack {
            assert!(center.apply_close_ack(stale, 200).is_err());
        }
        previous_ack = Some(acknowledgement);
        let snapshot = center.snapshot().unwrap();
        assert_eq!(snapshot.sandboxes(), std::slice::from_ref(&original));
        assert_eq!(
            snapshot.reserved(None, None),
            Some(lifecycle_fixture::resources(1))
        );
        let survivor_lane = snapshot
            .lane(&survivor.request.binding.node, survivor.retention.slot)
            .unwrap();
        assert_eq!(survivor_lane.retired_through(), 0);
        assert!(survivor_lane.last_retired().is_none());
        assert!(
            serde_json::to_vec(&snapshot).unwrap().len() < snapshot.bounds().required_block_bytes()
        );
    }
}
