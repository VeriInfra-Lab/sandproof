//! Same assertions on the original, candidate, wrong and restored Rust values.
use super::*;
use crate::InternalLocalBinding;
use crate::controller::lifecycle_inbox::model::{Entry, Session, proof_renewal_session};
use sandproof_protocol::lifecycle::RenewalConfirmation;

fn renewal(version: u64) -> AcceptedLifecycleIntent {
    let mut intent = create("owner", "session");
    intent.accepted_version = version;
    intent.request.binding.expected = ExpectedSession::Version(version - 1);
    intent.request.binding.key.window = 2.try_into().unwrap();
    intent.request.binding.key.sequence = version.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms: 200,
        extend_only: false,
    };
    assert!(intent.validate(&context()).is_ok());
    intent
}

fn entry(intent: AcceptedLifecycleIntent, applied: Option<i64>) -> Entry {
    Entry {
        intent,
        local_operation: None,
        previous_effective_end_at_ms: Some(100),
        applied_deadline: applied,
        unload_resolution: None,
    }
}

pub(super) fn legal(version: u64) -> Session {
    let intent = renewal(version);
    let confirmed = RenewalConfirmation {
        request: intent.clone(),
        effective_end_at_ms: 200,
    };
    assert!(confirmed.is_sdk_renewal());
    Session {
        id: intent.request.binding.session.clone(),
        retention: intent.retention,
        owner: intent.request.binding.key.owner.clone(),
        current_version: version,
        terminal: false,
        refusal: None,
        incarnation: Some(1.try_into().unwrap()),
        local: Some(InternalLocalBinding {
            sandbox: core::SandboxId::new(10),
            owner: core::TenantId::new(20),
            operation: core::OperationId::new(30),
        }),
        entries: vec![
            entry(create("owner", "session"), None),
            entry(intent, Some(200)),
        ],
        confirmed_terminal: None,
        confirmed_renewal: Some(confirmed),
        compacted_sdk_through: None,
        native_retirement: None,
    }
}

fn corrupt(session: &Session) {
    assert!(matches!(
        proof_renewal_session(session),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn normal_legal_and_gap_accepted() {
    assert!(proof_renewal_session(&legal(2)).is_ok());
    let mut gap = legal(4);
    gap.compacted_sdk_through = Some(2);
    assert!(proof_renewal_session(&gap).is_ok());
    // This function checks presence, not the predecessor's lifetime bounds.
    gap.entries[1].previous_effective_end_at_ms = Some(-100);
    assert!(proof_renewal_session(&gap).is_ok());
}

#[test]
fn absent_confirmation_requires_no_frontier() {
    let mut session = legal(2);
    session.confirmed_renewal = None;
    session.local = None;
    session.entries.clear();
    assert!(proof_renewal_session(&session).is_ok());
    session.compacted_sdk_through = Some(2);
    corrupt(&session);
}

#[test]
fn checkpoint_requires_full_intent() {
    let mut session = legal(2);
    session.entries[1].intent.retention.window = 2.try_into().unwrap();
    assert!(session.entries[1].intent.validate(&context()).is_ok());
    assert_eq!(
        session.entries[1].intent.request.binding.key,
        session
            .confirmed_renewal
            .as_ref()
            .unwrap()
            .request
            .request
            .binding
            .key
    );
    corrupt(&session);
}

#[test]
fn first_full_match_controls_metadata() {
    let mut session = legal(2);
    let good = session.entries[1].clone();
    session.entries[1].previous_effective_end_at_ms = None;
    session.entries.push(good);
    corrupt(&session);
    session.entries.swap(1, 2);
    assert!(proof_renewal_session(&session).is_ok());
}

#[test]
fn local_and_checkpoint_metadata_required() {
    let mut session = legal(2);
    session.local = None;
    corrupt(&session);
    let mut session = legal(2);
    session.entries[1].applied_deadline = Some(201);
    corrupt(&session);
    let mut session = legal(2);
    session.entries[1].previous_effective_end_at_ms = None;
    corrupt(&session);
    let mut session = legal(2);
    session.entries.clear();
    corrupt(&session);
}

#[test]
fn frontier_must_not_be_retained() {
    let mut session = legal(4);
    session.compacted_sdk_through = Some(2);
    session.entries.insert(1, entry(renewal(2), Some(200)));
    corrupt(&session);
}

#[test]
fn frontier_limits_and_absent_entries() {
    for through in [0, 1, 4, u64::MAX] {
        let mut session = legal(4);
        session.compacted_sdk_through = Some(through);
        corrupt(&session);
    }
    let mut session = legal(4);
    session.compacted_sdk_through = Some(3);
    assert!(proof_renewal_session(&session).is_ok());
}

#[test]
fn unapplied_sdk_cannot_be_omitted() {
    let mut session = legal(3);
    session.entries.insert(1, entry(renewal(2), None));
    corrupt(&session);
    // The entry classifier does not additionally require the confirmation shape.
    session.entries[1].intent.accepted_version = 0;
    corrupt(&session);
    session.entries[1].intent.request.binding.key.window = 3.try_into().unwrap();
    assert!(proof_renewal_session(&session).is_ok());
}

#[test]
fn invalid_confirmation_shape_rejected() {
    let mut session = legal(2);
    session
        .confirmed_renewal
        .as_mut()
        .unwrap()
        .effective_end_at_ms = 199;
    corrupt(&session);
}
