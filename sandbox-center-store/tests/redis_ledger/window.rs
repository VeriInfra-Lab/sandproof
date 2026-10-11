//! Real Redis commit/recovery checks; supplied host facts are controlled
//! component observations, not claims that this fixture launched a real VM.
use super::*;
use sandproof_protocol::internal::window::WindowCloseAck;

fn terminal(center: &mut CenterAuthority) -> LifecycleResponse {
    let tenant = "1".try_into().unwrap();
    let create = center
        .create(&tenant, lifecycle_fixture::binding(), 0)
        .unwrap();
    let destroy = center
        .destroy(&tenant, &create.identity.sandbox_id)
        .unwrap();
    let cleaned = lifecycle_fixture::response(&destroy, 2, LifecycleFact::Cleaned);
    center.apply_response(&cleaned).unwrap();
    cleaned
}

#[test]
fn bounded_lanes_retire_only_after_exact_durable_confirmation() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let node = lifecycle_fixture::node("host-a");
    for round in 1..=8 {
        let cleaned = terminal(&mut center);
        assert_eq!(cleaned.identity.request_window.get(), round);
        assert!(center.begin_close(&cleaned).is_err());
        center.confirm_terminal(&cleaned).unwrap();
        let close = center.begin_close(&cleaned).unwrap();
        assert_eq!(center.close_batch().unwrap(), vec![close.clone()]);
        let mut wrong = close.clone();
        wrong.terminal.identity.node.node_generation = 2.try_into().unwrap();
        assert!(
            center
                .apply_close_ack(&WindowCloseAck {
                    terminal: wrong.terminal
                })
                .is_err()
        );
        center
            .apply_close_ack(&WindowCloseAck {
                terminal: close.terminal.clone(),
            })
            .unwrap();
        assert!(center.snapshot().unwrap().sandboxes().is_empty());
        assert_eq!(
            center
                .state()
                .unwrap()
                .lane(&node, cleaned.identity.retention_slot)
                .unwrap()
                .retired_through(),
            round
        );
        assert!(center.confirm_terminal(&cleaned).is_err());
        // An exact already-retired ack does not erase/advance the successor.
        center
            .apply_close_ack(&WindowCloseAck {
                terminal: close.terminal,
            })
            .unwrap();
        assert_eq!(
            center
                .state()
                .unwrap()
                .lane(&node, cleaned.identity.retention_slot)
                .unwrap()
                .current()
                .unwrap()
                .get(),
            round + 1
        );
    }
    let create = center
        .create(&"1".try_into().unwrap(), lifecycle_fixture::binding(), 0)
        .unwrap();
    assert_eq!(create.identity.sandbox_id.as_str(), "sandbox-9");
    assert_eq!(create.identity.operation_id.as_str(), "operation-17");
}

#[test]
fn lost_close_and_retirement_commits_recover_original_frontier() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let mut center = center_authority(&fixture, proxy.address);
    let node = lifecycle_fixture::node("host-a");
    let cleaned = terminal(&mut center);
    center.confirm_terminal(&cleaned).unwrap();
    // Reaffirm precedes this pure decision. Lose the NEXT actual close CAS,
    // not the prerequisite read/confirmation, to exercise durable unknown.
    proxy.lose_after(1);
    assert_eq!(
        center.begin_close(&cleaned),
        Err(AuthorityError::Store(StoreError::OutcomeUnknown))
    );
    assert!(matches!(
        center.close_batch(),
        Err(AuthorityError::RecoveryRequired)
    ));
    center.recover().unwrap();
    let pending = center.close_batch().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].terminal.identity.request_window,
        cleaned.identity.request_window
    );
    let ack = WindowCloseAck {
        terminal: pending[0].terminal.clone(),
    };
    proxy.lose_after(0);
    assert_eq!(
        center.apply_close_ack(&ack),
        Err(AuthorityError::Store(StoreError::OutcomeUnknown))
    );
    assert!(center.state().is_err());
    center.recover().unwrap();
    assert!(center.close_batch().unwrap().is_empty());
    assert!(center.state().unwrap().sandboxes().is_empty());
    assert_eq!(
        center
            .state()
            .unwrap()
            .lane(&node, cleaned.identity.retention_slot)
            .unwrap()
            .retired_through(),
        1
    );
    center.apply_close_ack(&ack).unwrap();
    let create = center
        .create(&"1".try_into().unwrap(), lifecycle_fixture::binding(), 0)
        .unwrap();
    assert_eq!(create.identity.sandbox_id.as_str(), "sandbox-2");
    assert_eq!(create.identity.request_window.get(), 2);
}

#[test]
fn live_survivor_does_not_pin_its_peers_history_or_retention_capacity() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let tenant = "1".try_into().unwrap();
    let survivor = center
        .create(&tenant, lifecycle_fixture::binding(), 0)
        .unwrap();
    let ready = lifecycle_fixture::response(
        &survivor,
        1,
        LifecycleFact::Ready {
            started_at_ms: 10,
            end_at_ms: 110,
            max_end_at_ms: 510,
        },
    );
    center.apply_response(&ready).unwrap();
    let original = center.snapshot().unwrap().sandboxes()[0].clone();
    let mut previous_ack = None;
    for round in 1..=12 {
        let cleaned = terminal(&mut center);
        assert_ne!(
            cleaned.identity.retention_slot,
            survivor.identity.retention_slot
        );
        assert_eq!(cleaned.identity.request_window.get(), round);
        center.confirm_terminal(&cleaned).unwrap();
        let close = center.begin_close(&cleaned).unwrap();
        let acknowledgement = WindowCloseAck {
            terminal: close.terminal,
        };
        // A different slot with the SAME generation must not authorize this one.
        let mut foreign = acknowledgement.clone();
        foreign.terminal.identity.retention_slot = survivor.identity.retention_slot;
        assert!(center.apply_close_ack(&foreign).is_err());
        center.apply_close_ack(&acknowledgement).unwrap();
        if let Some(stale) = &previous_ack {
            assert!(center.apply_close_ack(stale).is_err());
        }
        previous_ack = Some(acknowledgement);
        let snapshot = center.snapshot().unwrap();
        assert_eq!(snapshot.sandboxes(), std::slice::from_ref(&original));
        assert_eq!(
            snapshot.reserved(None, None),
            Some(lifecycle_fixture::resources(1))
        );
        let survivor_lane = snapshot
            .lane(&survivor.identity.node, survivor.identity.retention_slot)
            .unwrap();
        assert_eq!(survivor_lane.retired_through(), 0);
        assert!(survivor_lane.last_retired().is_none());
        assert!(
            serde_json::to_vec(&snapshot).unwrap().len() < snapshot.bounds().required_block_bytes()
        );
    }
}
