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
fn bounded_windows_retire_only_after_exact_durable_confirmation() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let node = lifecycle_fixture::node("host-a");
    for round in 1..=8 {
        let cleaned = terminal(&mut center);
        assert_eq!(cleaned.identity.request_window.get(), round);
        center.begin_drain(&node).unwrap();
        assert!(center.begin_close(&node).is_err());
        assert!(
            center
                .create(&"1".try_into().unwrap(), lifecycle_fixture::binding(), 0)
                .is_err()
        );
        center.confirm_terminal(&cleaned).unwrap();
        let close = center.begin_close(&node).unwrap();
        assert_eq!(center.close_batch().unwrap(), vec![close.clone()]);
        let mut wrong = close.clone();
        wrong.identity.node.node_generation = 2.try_into().unwrap();
        assert!(
            center
                .apply_close_ack(&WindowCloseAck {
                    identity: wrong.identity
                })
                .is_err()
        );
        center
            .apply_close_ack(&WindowCloseAck {
                identity: close.identity.clone(),
            })
            .unwrap();
        assert!(center.snapshot().unwrap().sandboxes().is_empty());
        assert_eq!(
            center
                .state()
                .unwrap()
                .window(&node)
                .unwrap()
                .retired_through(),
            round
        );
        assert!(center.confirm_terminal(&cleaned).is_err());
        // An exact already-retired ack does not erase/advance the successor.
        center
            .apply_close_ack(&WindowCloseAck {
                identity: close.identity,
            })
            .unwrap();
        assert_eq!(
            center
                .state()
                .unwrap()
                .window(&node)
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
    center.begin_drain(&node).unwrap();
    proxy.lose_after(0);
    assert_eq!(
        center.begin_close(&node),
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
        pending[0].identity.request_window,
        cleaned.identity.request_window
    );
    let ack = WindowCloseAck {
        identity: pending[0].identity.clone(),
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
            .window(&node)
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
