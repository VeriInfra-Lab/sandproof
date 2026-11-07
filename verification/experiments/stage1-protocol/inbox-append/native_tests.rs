//! Identical complete state assertions for original, candidate, wrong, and restored Rust.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, Inbox, Session};

fn pending(intent: AcceptedLifecycleIntent) -> Entry {
    Entry {
        intent,
        local_operation: None,
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    }
}

fn next_intent(
    operation: LifecycleOperation,
    window: u64,
    sequence: u64,
    previous: u64,
) -> AcceptedLifecycleIntent {
    let mut intent = create("owner", "session");
    intent.request.operation = operation;
    intent.request.binding.key.window = window.try_into().unwrap();
    intent.request.binding.key.sequence = sequence.try_into().unwrap();
    intent.request.binding.expected = ExpectedSession::Version(previous);
    intent.accepted_version = previous + 1;
    assert!(intent.validate(&context()).is_ok());
    intent
}

fn unload() -> AcceptedLifecycleIntent {
    next_intent(
        LifecycleOperation::Unload {
            source_incarnation: 1.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
        },
        1,
        2,
        1,
    )
}

fn resume(capture: u64, target: u64) -> AcceptedLifecycleIntent {
    next_intent(
        LifecycleOperation::Resume {
            source_incarnation: 1.try_into().unwrap(),
            capture: capture.try_into().unwrap(),
            target_incarnation: target.try_into().unwrap(),
            manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32]),
        },
        1,
        3,
        2,
    )
}

fn renew(sequence: u64, previous: u64) -> AcceptedLifecycleIntent {
    next_intent(
        LifecycleOperation::Renew {
            incarnation: 1.try_into().unwrap(),
            end_at_ms: 200,
            extend_only: true,
        },
        2,
        sequence,
        previous,
    )
}

fn running() -> Session {
    let mut intent = create("owner", "session");
    intent.request.binding.key.window = 1.try_into().unwrap();
    assert!(intent.validate(&context()).is_ok());
    let mut entry = pending(intent.clone());
    entry.local_operation = Some(core::OperationId(30));
    Session {
        id: intent.request.binding.session.clone(),
        retention: intent.retention,
        owner: intent.request.binding.key.owner.clone(),
        current_version: 1,
        terminal: false,
        refusal: None,
        incarnation: Some(1.try_into().unwrap()),
        local: Some(InternalLocalBinding {
            sandbox: core::SandboxId(10),
            owner: core::TenantId(20),
            operation: core::OperationId(30),
        }),
        entries: vec![entry],
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: None,
    }
}

fn after(intent: AcceptedLifecycleIntent) -> Session {
    let mut session = running();
    session.current_version = intent.accepted_version;
    session.entries.push(pending(intent));
    session
}

fn with_session(session: Session) -> Inbox {
    Inbox {
        config: config(),
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![session],
        lanes: vec![],
    }
}

fn assert_append(mut inbox: Inbox, intent: &AcceptedLifecycleIntent) {
    assert!(intent.validate(&context()).is_ok());
    let mut expected = inbox.clone();
    let index = expected.sessions[0].entries.len();
    let session = &mut expected.sessions[0];
    session.current_version = intent.accepted_version;
    session.terminal = matches!(intent.request.operation, LifecycleOperation::Destroy {});
    session.entries.push(pending(intent.clone()));
    assert_eq!(inbox.record(intent).unwrap(), (0, index));
    assert_eq!(inbox, expected);
}

fn assert_reject(mut inbox: Inbox, intent: &AcceptedLifecycleIntent, error: &JournalError) {
    assert!(intent.validate(&context()).is_ok());
    let before = inbox.clone();
    let actual = inbox.record(intent).unwrap_err();
    assert!(
        matches!(
            (&actual, error),
            (JournalError::Conflict, JournalError::Conflict)
                | (
                    &JournalError::UnsupportedInput,
                    &JournalError::UnsupportedInput
                )
                | (
                    &JournalError::UnresolvedAction,
                    &JournalError::UnresolvedAction
                )
        ),
        "wrong first rejection: {actual:?}, expected {error:?}"
    );
    assert_eq!(inbox, before);
}

#[test]
fn normal_unload_keeps_complete_entry_and_frame() {
    assert_append(with_session(running()), &unload());
}

#[test]
fn normal_resume_keeps_complete_entry_and_frame() {
    assert_append(with_session(after(unload())), &resume(1, 2));
}

#[test]
fn normal_sdk_renewal_keeps_complete_entry_and_frame() {
    assert_append(with_session(running()), &renew(1, 1));
}

#[test]
fn different_prior_capture_rejected_without_effect() {
    assert_reject(
        with_session(after(unload())),
        &resume(2, 2),
        &JournalError::UnsupportedInput,
    );
}

#[test]
fn dto_gap_rejected_by_inbox_without_effect() {
    assert_reject(
        with_session(after(unload())),
        &resume(1, 3),
        &JournalError::UnsupportedInput,
    );
}

#[test]
fn unconfirmed_sdk_rejected_without_effect() {
    assert_reject(
        with_session(after(renew(1, 1))),
        &renew(2, 2),
        &JournalError::UnresolvedAction,
    );
}

#[test]
fn reserved_destroy_ignores_unconfirmed_sdk() {
    let mut inbox = with_session(after(renew(1, 1)));
    inbox.config.max_operations_per_session = 3;
    assert_append(inbox, &next_intent(LifecycleOperation::Destroy {}, 1, 3, 2));
}

#[test]
fn unsupported_shape_precedes_sdk_gate() {
    let mut session = after(renew(1, 1));
    session.local = None;
    assert_reject(
        with_session(session),
        &renew(2, 2),
        &JournalError::UnsupportedInput,
    );
}

#[test]
fn owner_conflict_precedes_shape_and_sdk_gate() {
    let mut session = after(renew(1, 1));
    session.owner = "another-owner".try_into().unwrap();
    session.local = None;
    assert_reject(with_session(session), &renew(2, 2), &JournalError::Conflict);
}

#[test]
fn retained_sdk_replay_precedes_current_guards() {
    let intent = renew(1, 1);
    let mut inbox = with_session(after(intent.clone()));
    inbox.config.max_operations_per_session = 0;
    inbox.sessions[0].terminal = true;
    let before = inbox.clone();
    assert_eq!(inbox.record(&intent).unwrap(), (0, 1));
    assert_eq!(inbox, before);
}
