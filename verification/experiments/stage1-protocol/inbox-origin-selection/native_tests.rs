//! One last-origin contract for original, candidate, wrong and restored Rust.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, Session, proof_execution_origin};
use sandbox_controller_core::OperationId;
use sandproof_protocol::lifecycle::ContentDigest;

fn entry(operation: LifecycleOperation) -> Entry {
    let mut intent = create("owner", "session");
    if !matches!(operation, LifecycleOperation::Create { .. }) {
        intent.request.binding.expected = ExpectedSession::Version(1);
        intent.request.binding.key.sequence = 2.try_into().unwrap();
        intent.accepted_version = 2;
    }
    intent.request.operation = operation;
    assert!(intent.validate(&context()).is_ok());
    Entry {
        intent,
        local_operation: Some(OperationId(30)),
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    }
}

fn created() -> Entry {
    entry(create("owner", "session").request.operation)
}

fn resumed() -> Entry {
    entry(LifecycleOperation::Resume {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        target_incarnation: 9.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([7; 32]),
    })
}

fn session(entries: Vec<Entry>) -> Session {
    let intent = create("owner", "session");
    Session {
        id: intent.request.binding.session.clone(),
        retention: intent.retention,
        owner: intent.request.binding.key.owner.clone(),
        current_version: 2,
        terminal: false,
        refusal: None,
        incarnation: Some(1.try_into().unwrap()),
        local: None,
        entries,
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: None,
    }
}

fn selected(value: &Session, index: usize) {
    let found = proof_execution_origin(value).expect("last matching entry exists");
    assert!(std::ptr::eq(
        std::ptr::from_ref(found),
        std::ptr::from_ref(&value.entries[index]),
    ));
}

#[test]
fn empty_returns_none() {
    assert!(proof_execution_origin(&session(vec![])).is_none());
}

#[test]
fn non_origins_return_none() {
    let entries = vec![
        entry(LifecycleOperation::Unload {
            source_incarnation: 1.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
        }),
        entry(LifecycleOperation::Renew {
            incarnation: 1.try_into().unwrap(),
            end_at_ms: 100,
            extend_only: false,
        }),
        entry(LifecycleOperation::Destroy {}),
    ];
    assert!(proof_execution_origin(&session(entries)).is_none());
}

#[test]
fn single_create_selected() {
    selected(&session(vec![created()]), 0);
}

#[test]
fn single_resume_selected() {
    selected(&session(vec![resumed()]), 0);
}

#[test]
fn newest_resume_selected() {
    selected(&session(vec![created(), resumed()]), 1);
}

#[test]
fn newest_create_selected() {
    selected(&session(vec![resumed(), created()]), 1);
}

#[test]
fn ignores_trailing_non_origins() {
    selected(
        &session(vec![
            created(),
            entry(LifecycleOperation::Renew {
                incarnation: 1.try_into().unwrap(),
                end_at_ms: 100,
                extend_only: true,
            }),
            entry(LifecycleOperation::Destroy {}),
        ]),
        0,
    );
}

#[test]
fn unattached_resume_selected() {
    let mut last = resumed();
    last.local_operation = None;
    selected(&session(vec![created(), last]), 1);
}

#[test]
fn full_entry_identity_preserved() {
    let first = created();
    let mut last = first.clone();
    last.local_operation = Some(OperationId(31));
    last.applied_deadline = Some(700);
    last.previous_effective_end_at_ms = Some(600);
    selected(&session(vec![first, last]), 1);
}

#[test]
fn unrelated_session_metadata_not_strengthened() {
    let mut original = created();
    original.local_operation = None;
    let mut value = session(vec![original]);
    value.current_version = 0;
    value.incarnation = None;
    value.terminal = true;
    selected(&value, 0);
}
