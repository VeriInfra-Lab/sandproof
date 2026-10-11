//! Controlled observations around the SAME original pure authority. The host
//! result does not update health; an explicit later observation does. These
//! deterministic cut points are not Redis, mTLS, or KVM acceptance evidence.
use super::*;

fn observations(f: &Fixture, steps: impl IntoIterator<Item = ProjectionStep>) {
    f.app.authority.lock().unwrap().projection_steps = steps.into_iter().collect();
}

fn reads(f: &Fixture) -> u64 {
    f.app.authority.lock().unwrap().projection_reads
}

fn original_ready(f: &Fixture) -> LifecycleRequest {
    let authority = f.app.authority.lock().unwrap();
    assert_eq!(authority.state.sandboxes().len(), 1);
    let record = &authority.state.sandboxes()[0];
    assert_eq!(record.operations.len(), 1);
    let operation = &record.operations[0];
    assert!(matches!(
        operation.response.as_ref().unwrap().fact,
        LifecycleFact::Ready { .. }
    ));
    assert_eq!(operation.request.identity.sandbox_id.as_str(), "sandbox-1");
    assert_eq!(
        operation.request.identity.operation_id.as_str(),
        "operation-1"
    );
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 1);
    operation.request.clone()
}

#[test]
fn ready_create_waits_for_original_node_observation_without_another_create() {
    let f = fixture();
    // The synthetic epoch stays at 1000. Ready is received while the previous
    // authenticated observation expires exactly at that epoch; the next view
    // includes a separately committed observation through the original reducer.
    observations(
        &f,
        [ProjectionStep::Health(1000), ProjectionStep::Health(1001)],
    );
    let session = f.app.create("1", command()).unwrap();
    let request = original_ready(&f);
    assert_eq!(session.sandbox_id, request.identity.sandbox_id.as_str());
    assert_eq!(reads(&f), 2);
    assert_eq!(f.host.last.lock().unwrap().as_ref(), Some(&request));
    assert!(f.app.resolve_data_route(&session.envd_access_token).is_ok());
}

#[test]
fn renewed_session_uses_same_observation_wait_without_another_renewal() {
    let f = fixture();
    let original = f.app.create("1", command()).unwrap();
    let before = reads(&f);
    observations(
        &f,
        [
            ProjectionStep::Health(u64::MAX), // original connect acceptance
            ProjectionStep::Health(1000),
            ProjectionStep::Health(1001),
        ],
    );
    let connected = f
        .app
        .connect("1", &original.sandbox_id, ConnectCommand::default())
        .unwrap();
    assert_eq!(connected, original);
    assert_eq!(reads(&f) - before, 3);
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 2);
    let state = f.app.snapshot().unwrap();
    assert_eq!(state.sandboxes().len(), 1);
    let operations = &state.sandboxes()[0].operations;
    assert_eq!(operations.len(), 2);
    assert!(matches!(operations[1].request.intent, Intent::Renew { .. }));
    assert_eq!(
        operations[1].request.identity.operation_id.as_str(),
        "operation-2"
    );
}

#[test]
fn ready_without_fresh_health_expires_unknown_and_keeps_original_responsibility() {
    let f = fixture();
    observations(&f, [ProjectionStep::Health(1000)]);
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    original_ready(&f);
    assert_eq!(f.app.get("1", "sandbox-1"), Err(PortError::Busy));
    assert_eq!(
        f.app.snapshot().unwrap().reserved(None, None),
        Some(resources(1))
    );
}

#[test]
fn shared_fault_during_health_wait_is_immediate_not_a_retryable_observation() {
    let f = fixture();
    observations(
        &f,
        [ProjectionStep::Health(1000), ProjectionStep::SharedFault],
    );
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    original_ready(&f);
    assert_eq!(reads(&f), 2);
    assert!(f.unreadable.load(Ordering::SeqCst));
    assert_eq!(f.app.snapshot(), Err(PortError::OutcomeUnknown));
}

#[test]
fn ready_at_original_reply_deadline_cannot_start_a_new_session_budget() {
    let f = fixture();
    // The controlled host consumes exactly the budget supplied by the original
    // wait. No random sleep or second synthetic timer decides the outcome.
    f.host.consume_reply_budget.store(true, Ordering::SeqCst);
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    original_ready(&f);
    assert_eq!(reads(&f), 0);
}

#[test]
fn otherwise_valid_session_projection_cannot_complete_after_reply_deadline() {
    let f = fixture();
    observations(
        &f,
        [ProjectionStep::CompleteAfter(f.app.config.reply_timeout)],
    );
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    assert_eq!(reads(&f), 1);
    original_ready(&f);
    // Ordinary queries preserve their own original semantics, independently
    // of the expired Create reply budget.
    assert!(f.app.get("1", "sandbox-1").is_ok());
}

#[test]
fn renewed_at_original_reply_deadline_cannot_issue_another_session() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    let before = reads(&f);
    f.host.consume_reply_budget.store(true, Ordering::SeqCst);
    assert_eq!(
        f.app
            .connect("1", &session.sandbox_id, ConnectCommand::default()),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(
        reads(&f) - before,
        1,
        "only the original connect acceptance view"
    );
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 2);
    let state = f.app.snapshot().unwrap();
    assert_eq!(state.sandboxes().len(), 1);
    assert_eq!(state.sandboxes()[0].operations.len(), 2);
    assert!(matches!(
        state.sandboxes()[0].operations[1]
            .response
            .as_ref()
            .unwrap()
            .fact,
        LifecycleFact::Renewed { .. }
    ));
}
