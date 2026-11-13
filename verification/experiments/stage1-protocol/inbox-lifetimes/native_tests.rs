//! Same decoded-value assertions for original, refactored and mutant Rust.
//! These fixtures do not establish a live store transaction or authorization.
use super::*;
use crate::Lifetime;
use crate::controller::lifecycle_inbox::model::{Entry, Session, proof_renewal_lifetimes};

fn initial() -> AcceptedLifecycleIntent {
    let mut intent = create("owner", "session");
    let LifecycleOperation::Create {
        create,
        target_incarnation,
    } = &mut intent.request.operation
    else {
        unreachable!()
    };
    create.timeout_ms = 100;
    *target_incarnation = 1.try_into().unwrap();
    assert!(intent.validate(&context()).is_ok());
    intent
}

fn renewal(version: u64, end: i64) -> AcceptedLifecycleIntent {
    let mut intent = initial();
    intent.accepted_version = version;
    intent.request.binding.expected = ExpectedSession::Version(version - 1);
    intent.request.binding.key.sequence = version.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms: end,
        extend_only: false,
    };
    assert!(intent.validate(&context()).is_ok());
    intent
}

fn entry(intent: AcceptedLifecycleIntent, previous: Option<i64>, applied: Option<i64>) -> Entry {
    Entry {
        intent,
        local_operation: None,
        previous_effective_end_at_ms: previous,
        applied_deadline: applied,
        unload_resolution: None,
    }
}

fn session(entries: Vec<Entry>) -> Session {
    let first = initial();
    Session {
        id: first.request.binding.session,
        retention: first.retention,
        owner: first.request.binding.key.owner,
        current_version: entries
            .last()
            .map_or(0, |entry| entry.intent.accepted_version),
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

fn lifetime(deadline: i64) -> Lifetime {
    Lifetime {
        started_at_ms: Some(10),
        deadline_ms: deadline,
        max_deadline_ms: 1_000,
        startup_deadline_ms: 20,
        disk_bytes: 4_096,
    }
}

fn applied(previous: i64, end: i64) -> Session {
    session(vec![
        entry(initial(), None, None),
        entry(renewal(2, end), Some(previous), Some(end)),
    ])
}

#[test]
fn accepts_normal_and_endpoint_boundaries() {
    for end in [10, 200, 1_000] {
        assert!(proof_renewal_lifetimes(&applied(110, end), lifetime(end)).is_ok());
    }
}

#[test]
fn predecessor_field_cannot_be_omitted() {
    assert!(matches!(
        proof_renewal_lifetimes(&applied(111, 200), lifetime(200)),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn version_gap_resets_only_the_predecessor() {
    let mut gap = applied(300, 400);
    gap.entries[1].intent = renewal(4, 400);
    assert!(proof_renewal_lifetimes(&gap, lifetime(400)).is_ok());
    gap.entries[1].previous_effective_end_at_ms = Some(1_001);
    assert!(matches!(
        proof_renewal_lifetimes(&gap, lifetime(400)),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn final_deadline_cannot_be_omitted() {
    assert!(matches!(
        proof_renewal_lifetimes(&applied(110, 200), lifetime(201)),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn applied_entries_require_previous_and_bounded_endpoints() {
    let mut missing = applied(110, 200);
    missing.entries[1].previous_effective_end_at_ms = None;
    assert!(matches!(
        proof_renewal_lifetimes(&missing, lifetime(200)),
        Err(JournalError::CorruptStore)
    ));
    for (previous, end) in [(9, 200), (1_001, 200), (110, 9), (110, 1_001)] {
        assert!(matches!(
            proof_renewal_lifetimes(&applied(previous, end), lifetime(end)),
            Err(JournalError::CorruptStore)
        ));
    }
}

#[test]
fn unapplied_records_update_version_without_adding_metadata_requirements() {
    let records = session(vec![
        entry(initial(), None, None),
        entry(renewal(2, 500), Some(-100), None),
        entry(renewal(3, 200), Some(110), Some(200)),
    ]);
    assert!(proof_renewal_lifetimes(&records, lifetime(200)).is_ok());
    let no_applied = session(vec![entry(initial(), None, None)]);
    assert!(proof_renewal_lifetimes(&no_applied, lifetime(-1)).is_ok());
}

#[test]
fn rejects_absent_start_noncreate_overflow_and_initial_bound() {
    let created = session(vec![entry(initial(), None, None)]);
    let mut missing = lifetime(200);
    missing.started_at_ms = None;
    assert!(matches!(
        proof_renewal_lifetimes(&created, missing),
        Err(JournalError::CorruptStore)
    ));
    for records in [vec![], vec![entry(renewal(2, 200), None, None)]] {
        assert!(matches!(
            proof_renewal_lifetimes(&session(records), lifetime(200)),
            Err(JournalError::CorruptStore)
        ));
    }
    let mut overflowing = initial();
    let LifecycleOperation::Create { create, .. } = &mut overflowing.request.operation else {
        unreachable!()
    };
    create.timeout_ms = u64::MAX;
    assert!(matches!(
        proof_renewal_lifetimes(
            &session(vec![entry(overflowing, None, None)]),
            lifetime(200)
        ),
        Err(JournalError::CorruptStore)
    ));
    let mut overflow_start = lifetime(200);
    overflow_start.started_at_ms = Some(i64::MAX);
    assert!(matches!(
        proof_renewal_lifetimes(&created, overflow_start),
        Err(JournalError::CorruptStore)
    ));
    let mut low_max = lifetime(200);
    low_max.max_deadline_ms = 109;
    assert!(matches!(
        proof_renewal_lifetimes(&created, low_max),
        Err(JournalError::CorruptStore)
    ));
}
