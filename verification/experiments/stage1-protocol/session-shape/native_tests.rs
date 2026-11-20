//! Actual `SessionShape` boundary checks; no durable or resource claim.
use super::*;
use crate::controller::lifecycle_inbox::model::{
    Entry, Inbox, LifecycleRetentionLane, Session, proof_inbox_session_shape_original,
};
use sandproof_protocol::lifecycle::{
    CreateRefusalReason, TerminalLifecycleObservation, TerminalStatus,
};

fn entry(intent: AcceptedLifecycleIntent) -> Entry {
    Entry {
        intent,
        local_operation: None,
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    }
}

fn normal() -> (Inbox, Session) {
    let intent = create("owner", "session");
    assert_eq!(intent.validate(&context()), Ok(()));
    let session = Session {
        id: intent.request.binding.session.clone(),
        retention: intent.retention,
        owner: intent.request.binding.key.owner.clone(),
        current_version: 1,
        terminal: false,
        refusal: None,
        incarnation: None,
        local: None,
        entries: vec![entry(intent)],
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: None,
    };
    let inbox = Inbox {
        config: config(),
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![],
        lanes: vec![
            LifecycleRetentionLane::fresh(1),
            LifecycleRetentionLane::fresh(2),
        ],
    };
    (inbox, session)
}

fn local(inbox: &mut Inbox, session: &mut Session) {
    session.local = Some(InternalLocalBinding {
        sandbox: core::SandboxId(10),
        owner: core::TenantId(20),
        operation: core::OperationId(30),
    });
    inbox.host_epoch_observation = Some(crate::capture::HostEpochObservation {
        node: inbox.config.node.clone(),
        registration: inbox.config.host_epoch.clone().unwrap(),
    });
}

fn terminal() -> (Inbox, Session) {
    let (inbox, mut session) = normal();
    let mut destroy = session.entries[0].intent.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2_u64.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    assert_eq!(destroy.validate(&context()), Ok(()));
    session.current_version = 2;
    session.terminal = true;
    session.entries.push(entry(destroy.clone()));
    session.confirmed_terminal = Some(TerminalLifecycleObservation {
        request: destroy,
        status: TerminalStatus::Cleaned,
    });
    (inbox, session)
}

fn check(inbox: &Inbox, index: usize, session: &Session, accepted: bool) {
    let before = (inbox.clone(), session.clone());
    match (
        proof_inbox_session_shape_original(inbox, index, session),
        accepted,
    ) {
        (Ok(()), true) | (Err(JournalError::CorruptStore), false) => {}
        (result, _) => panic!("unexpected SessionShape result: {result:?}; accepted={accepted}"),
    }
    assert_eq!((inbox.clone(), session.clone()), before);
}

#[test]
fn legal_create_shape_accepted_readonly() {
    let (inbox, session) = normal();
    check(&inbox, 0, &session, true);
}

#[test]
fn nonempty_slot_and_operation_capacity_guards() {
    let (inbox, session) = normal();
    let mut empty = session.clone();
    empty.entries.clear();
    check(&inbox, 0, &empty, false);
    for slot in [0, inbox.config.max_sessions + 1] {
        let mut wrong = session.clone();
        wrong.retention.slot = slot;
        check(&inbox, 0, &wrong, false);
    }
    let mut limited = inbox;
    limited.config.max_operations_per_session = 0;
    check(&limited, 0, &session, false);
}

#[test]
fn first_matching_lane_controls_window() {
    let (mut inbox, session) = normal();
    inbox.lanes.clear();
    check(&inbox, 0, &session, false);
    let good = LifecycleRetentionLane::fresh(1);
    let mut bad = good.clone();
    bad.current_window = None;
    inbox.lanes = vec![bad.clone(), good.clone()];
    check(&inbox, 0, &session, false);
    inbox.lanes = vec![good, bad];
    check(&inbox, 0, &session, true);
    inbox.lanes.insert(0, LifecycleRetentionLane::fresh(2));
    check(&inbox, 0, &session, true);
}

#[test]
fn local_requires_observation_presence_only() {
    let (mut inbox, mut session) = normal();
    local(&mut inbox, &mut session);
    check(&inbox, 0, &session, true);
    inbox.host_epoch_observation.as_mut().unwrap().node.node_id = "foreign".try_into().unwrap();
    check(&inbox, 0, &session, true);
    inbox.host_epoch_observation = None;
    check(&inbox, 0, &session, false);
}

#[test]
fn prior_logical_identity_and_slot_are_distinct_guards() {
    let (mut inbox, session) = normal();
    let mut prior = session.clone();
    prior.id = "other".try_into().unwrap();
    prior.retention.slot = 2;
    inbox.sessions = vec![prior.clone()];
    check(&inbox, 1, &session, true);
    inbox.sessions[0].id = session.id.clone();
    check(&inbox, 1, &session, false);
    inbox.sessions[0] = prior;
    inbox.sessions[0].retention.slot = session.retention.slot;
    check(&inbox, 1, &session, false);
}

#[test]
fn prior_physical_sandbox_conflicts_despite_distinct_owner_and_operation() {
    let (mut inbox, mut session) = normal();
    local(&mut inbox, &mut session);
    let mut prior = session.clone();
    prior.id = "other".try_into().unwrap();
    prior.retention.slot = 2;
    prior.local.as_mut().unwrap().owner = core::TenantId(21);
    prior.local.as_mut().unwrap().operation = core::OperationId(31);
    inbox.sessions.push(prior);
    check(&inbox, 1, &session, false);
    inbox.sessions[0].local.as_mut().unwrap().sandbox = core::SandboxId(11);
    check(&inbox, 1, &session, true);
}

#[test]
fn only_the_prior_prefix_is_observed() {
    let (mut inbox, session) = normal();
    inbox.sessions.push(session.clone());
    check(&inbox, 0, &session, true);
    check(&inbox, 1, &session, false);
}

#[test]
fn refusal_allows_create_and_destroy_without_native_fields() {
    let (inbox, mut session) = terminal();
    session.confirmed_terminal = None;
    session.refusal = Some(CreateRefusalReason::Capacity);
    check(&inbox, 0, &session, true);
}

#[test]
fn refusal_checks_initial_shape_and_every_native_field() {
    let changes: [fn(&mut Session); 6] = [
        |s| s.entries[0].intent.accepted_version = 2,
        |s| s.entries[0].intent.request.operation = LifecycleOperation::Destroy {},
        |s| s.entries[1].local_operation = Some(core::OperationId(1)),
        |s| s.entries[1].applied_deadline = Some(1),
        |s| s.entries[1].previous_effective_end_at_ms = Some(1),
        |s| {
            s.entries[1].intent.request.operation = LifecycleOperation::Unload {
                source_incarnation: 1_u64.try_into().unwrap(),
                capture: 1_u64.try_into().unwrap(),
            }
        },
    ];
    for change in changes {
        let (inbox, mut session) = terminal();
        session.confirmed_terminal = None;
        session.refusal = Some(CreateRefusalReason::Capacity);
        change(&mut session);
        check(&inbox, 0, &session, false);
    }
    let (mut inbox, mut session) = normal();
    session.refusal = Some(CreateRefusalReason::Capacity);
    local(&mut inbox, &mut session);
    check(&inbox, 0, &session, false);
}

#[test]
fn terminal_binds_complete_last_intent_and_cleaned_status() {
    let (inbox, session) = terminal();
    check(&inbox, 0, &session, true);
    let changes: [fn(&mut Session); 4] = [
        |s| s.terminal = false,
        |s| s.confirmed_terminal.as_mut().unwrap().status = TerminalStatus::Pending,
        |s| {
            s.confirmed_terminal
                .as_mut()
                .unwrap()
                .request
                .accepted_version = 0;
        },
        |s| s.entries[1].intent.center.center = "foreign".try_into().unwrap(),
    ];
    for change in changes {
        let mut wrong = session.clone();
        change(&mut wrong);
        check(&inbox, 0, &wrong, false);
    }
}

#[test]
fn accepted_envelope_validation_remains_an_outer_obligation() {
    let (inbox, mut session) = normal();
    session.entries[0].intent.request.binding.protocol_version = 3;
    assert!(session.entries[0].intent.validate(&context()).is_err());
    check(&inbox, 0, &session, true);
}

#[test]
fn slice_bound_panics_only_after_earlier_guards_accept() {
    let (inbox, mut session) = normal();
    let result =
        std::panic::catch_unwind(|| proof_inbox_session_shape_original(&inbox, 1, &session));
    assert!(result.is_err());
    session.entries.clear();
    check(&inbox, 1, &session, false);
}
