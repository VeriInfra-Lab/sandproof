//! Decoded retained-order metadata. No committed removal or deadline claim.
use super::*;
use crate::controller::lifecycle_inbox::model::{Session, proof_retained_history};
use sandproof_protocol::lifecycle::RenewalConfirmation;

fn renewal(version: u64) -> AcceptedLifecycleIntent {
    let mut intent = create("owner", "session");
    intent.accepted_version = version;
    intent.request.binding.expected = ExpectedSession::Version(version - 1);
    intent.request.binding.key.sequence = version.try_into().unwrap();
    intent.request.binding.key.window = 2.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms: 100,
        extend_only: false,
    };
    assert!(intent.validate(&context()).is_ok());
    intent
}

fn session(through: Option<u64>, confirmed: Option<u64>) -> Session {
    let first = create("owner", "session");
    Session {
        id: first.request.binding.session,
        retention: first.retention,
        owner: first.request.binding.key.owner,
        current_version: 0,
        terminal: false,
        refusal: None,
        incarnation: None,
        local: None,
        entries: Vec::new(),
        confirmed_terminal: None,
        confirmed_renewal: confirmed.map(|version| RenewalConfirmation {
            request: renewal(version),
            effective_end_at_ms: 100,
        }),
        compacted_sdk_through: through,
        native_retirement: None,
    }
}

#[test]
fn accepts_initial_adjacent_and_compacted_boundary() {
    let initial = create("owner", "session");
    assert!(initial.validate(&context()).is_ok());
    assert!(proof_retained_history(&session(None, None), 0, &initial));
    assert!(proof_retained_history(&session(None, None), 1, &renewal(2)));
    assert!(proof_retained_history(
        &session(Some(3), Some(4)),
        1,
        &renewal(4)
    ));
}

#[test]
fn compaction_bound_is_not_reversed() {
    assert!(!proof_retained_history(
        &session(Some(3), Some(5)),
        1,
        &renewal(5)
    ));
}

#[test]
fn confirmation_bound_is_not_reversed() {
    assert!(!proof_retained_history(
        &session(Some(4), Some(4)),
        1,
        &renewal(5)
    ));
}

#[test]
fn version_zero_is_not_adjacent() {
    assert!(!proof_retained_history(
        &session(None, None),
        0,
        &renewal(1)
    ));
}

#[test]
fn gap_requires_both_saved_metadata_fields() {
    assert!(!proof_retained_history(
        &session(None, Some(4)),
        1,
        &renewal(4)
    ));
    assert!(!proof_retained_history(
        &session(Some(3), None),
        1,
        &renewal(4)
    ));
    assert!(!proof_retained_history(
        &session(None, None),
        1,
        &renewal(4)
    ));
}

#[test]
fn rejects_absent_after_history_and_backward_expected() {
    assert!(!proof_retained_history(
        &session(None, None),
        1,
        &create("owner", "session")
    ));
    assert!(!proof_retained_history(
        &session(Some(3), Some(4)),
        3,
        &renewal(3)
    ));
}
