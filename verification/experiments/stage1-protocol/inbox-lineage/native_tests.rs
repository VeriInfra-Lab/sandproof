//! Decoded-value checks only: these fixtures make no durability or VM claim.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, Session, proof_execution_lineage};

fn initial() -> AcceptedLifecycleIntent {
    let mut intent = create("owner", "session");
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = &mut intent.request.operation
    else {
        panic!("expected original Create fixture");
    };
    *target_incarnation = 1.try_into().unwrap();
    intent
}

fn follow(version: u64, operation: LifecycleOperation) -> AcceptedLifecycleIntent {
    let mut intent = initial();
    intent.accepted_version = version;
    intent.request.binding.expected = ExpectedSession::Version(version - 1);
    intent.request.binding.key.sequence = version.try_into().unwrap();
    intent.request.operation = operation;
    intent
}

fn entry(intent: AcceptedLifecycleIntent, operation: Option<u64>) -> Entry {
    assert!(intent.validate(&context()).is_ok());
    Entry {
        intent,
        local_operation: operation.map(core::OperationId),
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    }
}

fn unload(version: u64, source: u64, capture: u64) -> Entry {
    entry(
        follow(
            version,
            LifecycleOperation::Unload {
                source_incarnation: source.try_into().unwrap(),
                capture: capture.try_into().unwrap(),
            },
        ),
        None,
    )
}

fn resume(target: u64, operation: Option<u64>) -> Entry {
    entry(
        follow(
            3,
            LifecycleOperation::Resume {
                source_incarnation: 1.try_into().unwrap(),
                capture: 1.try_into().unwrap(),
                target_incarnation: target.try_into().unwrap(),
                manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32]),
            },
        ),
        operation,
    )
}

fn session(incarnation: Option<u64>, entries: Vec<Entry>) -> Session {
    let first = initial();
    Session {
        id: first.request.binding.session,
        retention: first.retention,
        owner: first.request.binding.key.owner,
        current_version: entries
            .last()
            .map_or(0, |entry| entry.intent.accepted_version),
        terminal: entries.last().is_some_and(|entry| {
            matches!(
                entry.intent.request.operation,
                LifecycleOperation::Destroy {}
            )
        }),
        refusal: None,
        incarnation: incarnation.map(|n| n.try_into().unwrap()),
        local: Some(InternalLocalBinding {
            sandbox: core::SandboxId(10),
            owner: core::TenantId(20),
            operation: core::OperationId(30),
        }),
        entries,
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: None,
    }
}

fn check(value: &Session, accepted: bool) {
    let before = value.clone();
    match (proof_execution_lineage(value), accepted) {
        (Ok(()), true) | (Err(JournalError::CorruptStore), false) => {}
        (result, _) => {
            panic!("unexpected lineage result: {result:?}; expected accepted={accepted}")
        }
    }
    assert_eq!(*value, before);
}

#[test]
fn normal_create_renew_destroy() {
    check(&session(Some(1), vec![entry(initial(), Some(30))]), true);
    check(
        &session(
            Some(1),
            vec![
                entry(initial(), Some(30)),
                entry(
                    follow(
                        2,
                        LifecycleOperation::Renew {
                            incarnation: 1.try_into().unwrap(),
                            end_at_ms: 100,
                            extend_only: false,
                        },
                    ),
                    None,
                ),
                entry(follow(3, LifecycleOperation::Destroy {}), None),
            ],
        ),
        true,
    );
}

#[test]
fn pending_resume_preserves_source() {
    check(
        &session(
            Some(1),
            vec![entry(initial(), Some(30)), unload(2, 1, 1), resume(2, None)],
        ),
        true,
    );
}

#[test]
fn capture_frontier_survives_attached_resume() {
    check(
        &session(
            Some(2),
            vec![
                entry(initial(), Some(30)),
                unload(2, 1, 1),
                resume(2, Some(31)),
                unload(4, 2, 2),
            ],
        ),
        true,
    );
    check(
        &session(
            Some(2),
            vec![
                entry(initial(), Some(30)),
                unload(2, 1, 1),
                resume(2, Some(31)),
                unload(4, 2, 1),
            ],
        ),
        false,
    );
}

#[test]
fn stored_incarnation_matches_computed() {
    check(&session(None, vec![]), true);
    check(&session(Some(1), vec![]), false);
    check(&session(Some(2), vec![entry(initial(), Some(30))]), false);
}

#[test]
fn resume_requires_successor_and_exact_previous_unload() {
    // The DTO is valid; the private inbox lineage check is stricter.
    check(
        &session(
            Some(1),
            vec![entry(initial(), Some(30)), unload(2, 1, 1), resume(9, None)],
        ),
        false,
    );
    check(
        &session(
            Some(1),
            vec![entry(initial(), Some(30)), unload(2, 1, 2), resume(2, None)],
        ),
        false,
    );
    check(
        &session(Some(1), vec![entry(initial(), Some(30)), resume(2, None)]),
        false,
    );
}

#[test]
fn noninitial_create_and_foreign_renew_are_rejected() {
    check(
        &session(
            Some(1),
            vec![entry(initial(), Some(30)), entry(initial(), None)],
        ),
        false,
    );
    check(
        &session(
            Some(1),
            vec![
                entry(initial(), Some(30)),
                entry(
                    follow(
                        2,
                        LifecycleOperation::Renew {
                            incarnation: 2.try_into().unwrap(),
                            end_at_ms: 100,
                            extend_only: false,
                        },
                    ),
                    None,
                ),
            ],
        ),
        false,
    );
}
