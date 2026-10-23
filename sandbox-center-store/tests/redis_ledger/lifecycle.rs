//! Real original Redis block/CAS, including dropped commit replies and reopen.
use super::*;
use sandproof_center_core::{CreateParameters, LifecycleSubmission, LifecycleWindow, SessionPhase};
use sandproof_protocol::lifecycle::{CenterIdentity, OperationKey};

fn configured() -> CenterState {
    // Two registered owners in the original bounded center registration.
    let mut value = serde_json::to_value(center_state()).unwrap();
    let mut second = value["tenants"][0].clone();
    second["tenant"] = "2".into();
    value["tenants"].as_array_mut().unwrap().push(second);
    let state: CenterState = serde_json::from_value(value).unwrap();
    state
        .enable_lifecycle(
            CenterIdentity {
                deployment: deployment(),
                center: "center".try_into().unwrap(),
            },
            ["1", "2"]
                .into_iter()
                .map(|owner| LifecycleWindow {
                    owner: owner.try_into().unwrap(),
                    window: 9.try_into().unwrap(),
                })
                .collect(),
        )
        .unwrap()
}
fn input(owner: &str) -> LifecycleSubmission {
    let create = lifecycle_fixture::binding();
    LifecycleSubmission::Create {
        key: OperationKey {
            deployment: deployment(),
            owner: owner.try_into().unwrap(),
            window: 9.try_into().unwrap(),
            sequence: 1.try_into().unwrap(),
        },
        parameters: CreateParameters {
            template_revision: create.template_revision,
            template_digest: create.template_digest,
            resources: create.resources,
            options: create.options,
            startup_timeout_ms: 1000,
            timeout_ms: 100,
            max_lifetime_ms: 500,
        },
    }
}
fn initialize(fixture: &Engine, address: SocketAddrV4) -> CenterAuthority {
    let state = configured();
    let (ledger, _) = RedisLedger::initialize(
        endpoint(address),
        Engine::config(state.required_authority_bytes()),
        fixture.lease(),
        &state,
    )
    .unwrap();
    CenterAuthority::attach(ledger).unwrap()
}
fn reopen(fixture: &Engine) -> CenterAuthority {
    let (ledger, _) = RedisLedger::<CenterState>::open(
        endpoint(fixture.address),
        Engine::config(configured().required_authority_bytes()),
        fixture.lease(),
    )
    .unwrap();
    CenterAuthority::attach(ledger).unwrap()
}

#[test]
fn lifecycle_same_cas_reopen_retains_both_owners_original_requests_and_unload() {
    let mut fixture = Engine::new();
    let mut authority = initialize(&fixture, fixture.address);
    let a = authority
        .accept_lifecycle(&"1".try_into().unwrap(), &input("1"), 10)
        .unwrap();
    let b = authority
        .accept_lifecycle(&"2".try_into().unwrap(), &input("2"), 10)
        .unwrap();
    authority
        .accept_lifecycle_create_observation(&observed(&a))
        .unwrap();
    let mut key = a.request.binding.key.clone();
    key.sequence = 2.try_into().unwrap();
    let unload = LifecycleSubmission::Unload {
        key,
        session: a.request.binding.session.clone(),
        expected_version: 1,
    };
    let accepted = authority
        .accept_lifecycle(&"1".try_into().unwrap(), &unload, 150)
        .unwrap();
    let original_state = authority.snapshot().unwrap();
    assert_eq!(
        original_state.reserved(None, None),
        Some(lifecycle_fixture::resources(2))
    );
    assert!(authority.dispatch_batch().unwrap().is_empty());
    assert_eq!(
        authority.lifecycle_dispatch_batch().unwrap(),
        vec![accepted.clone(), b]
    );
    let count: u64 = redis::cmd("DBSIZE")
        .query(&mut connect(fixture.address))
        .unwrap();
    assert_eq!(count, 1, "no second authority, outbox or cache key");
    drop(authority);
    fixture.stop();
    fixture.start();
    let mut authority = reopen(&fixture);
    assert_eq!(authority.snapshot().unwrap(), original_state);
    let view = authority
        .query_lifecycle(&"1".try_into().unwrap(), &a.request.binding.session)
        .unwrap()
        .unwrap();
    assert_eq!(view.phase(), SessionPhase::Unloading);
    assert_eq!(view.current_version(), Some(2));
    assert_eq!(
        authority
            .accept_lifecycle(&"1".try_into().unwrap(), &unload, u64::MAX)
            .unwrap(),
        accepted
    );
    assert_eq!(authority.state().unwrap().identity_frontiers(), (2, 2));
    assert!(
        authority
            .query_lifecycle(&"2".try_into().unwrap(), &a.request.binding.session)
            .unwrap()
            .is_none()
    );
}

#[test]
fn lifecycle_lost_actual_cas_reply_fences_every_projection_and_recovers_same_key() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let mut authority = initialize(&fixture, proxy.address);
    proxy.lose_after(0);
    assert_eq!(
        authority.accept_lifecycle(&"1".try_into().unwrap(), &input("1"), 10),
        Err(AuthorityError::Store(StoreError::OutcomeUnknown))
    );
    assert!(matches!(
        authority.lifecycle_dispatch_batch(),
        Err(AuthorityError::RecoveryRequired)
    ));
    assert!(authority.snapshot().is_err());
    authority.recover().unwrap();
    let batch = authority.lifecycle_dispatch_batch().unwrap();
    assert_eq!(batch.len(), 1);
    let accepted = authority
        .accept_lifecycle(&"1".try_into().unwrap(), &input("1"), u64::MAX)
        .unwrap();
    assert_eq!(accepted, batch[0]);
    assert_eq!(accepted.accepted_version, 1);
    assert_eq!(
        accepted.request.binding.session.as_protocol_id().as_str(),
        "sandbox-1"
    );
    assert_eq!(authority.state().unwrap().identity_frontiers(), (1, 1));
}

#[test]
fn lifecycle_changed_full_input_never_commits_and_corrupt_or_missing_block_never_initializes() {
    let fixture = Engine::new();
    let mut authority = initialize(&fixture, fixture.address);
    authority
        .accept_lifecycle(&"1".try_into().unwrap(), &input("1"), 10)
        .unwrap();
    let revision = fixture.revision();
    let mut wrong = input("1");
    let LifecycleSubmission::Create { parameters, .. } = &mut wrong else {
        unreachable!()
    };
    parameters.timeout_ms += 1;
    assert_eq!(
        authority.accept_lifecycle(&"1".try_into().unwrap(), &wrong, 10),
        Err(AuthorityError::Decision(
            sandproof_center_core::DecisionError::Conflict
        ))
    );
    assert_eq!(fixture.revision(), revision);
    fixture.field("payload", b"corrupted");
    assert!(authority.lifecycle_dispatch_batch().is_err());
    assert!(
        authority
            .accept_lifecycle(&"2".try_into().unwrap(), &input("2"), 10)
            .is_err()
    );
    drop(authority);
    let _: u64 = redis::cmd("DEL")
        .arg(KEY)
        .query(&mut connect(fixture.address))
        .unwrap();
    assert!(matches!(
        RedisLedger::<CenterState>::open(
            endpoint(fixture.address),
            Engine::config(configured().required_authority_bytes()),
            fixture.lease()
        ),
        Err(StoreError::Missing)
    ));
}

// Controlled input to the V3 save contract, not physical-host evidence.
fn observed(intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent) -> sandproof_protocol::lifecycle::CreateObservation {
    use sandproof_protocol::lifecycle::{CreateObservation, CurrentCreateReadiness, CurrentReadyFacts, LifecycleOperation};
    let LifecycleOperation::Create { create, .. } = &intent.request.operation else { unreachable!() };
    CreateObservation {
        request: intent.clone(), current_request: intent.clone(), historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed { facts: CurrentReadyFacts {
            started_at_ms: 100, end_at_ms: 200, max_end_at_ms: 600,
            startup_deadline_ms: create.startup_deadline_ms, observed_at_ms: 100,
        } },
    }
}
