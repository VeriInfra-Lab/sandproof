//! Original transaction/crash/frontier regressions on the current window API.
use super::*;
use crate::NeverCrash;
use crate::controller::{lifecycle_inbox::tests as fixture, transaction};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, RetentionWindowId};

struct Stop(CrashPoint);
impl CrashInjector for Stop {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}
fn context() -> AuthenticatedCenterContext {
    let create = fixture::create("alpha", "session-a");
    AuthenticatedCenterContext::from_verified_peer_registration(
        create.center,
        create.request.binding.node,
    )
}
fn record_terminal(journal: &mut ControllerJournal) -> LifecycleWindowClose {
    let create = fixture::create("alpha", "session-a");
    journal
        .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
        .unwrap();
    let destroy = fixture::destroy(&create);
    journal
        .record_lifecycle_intent(&destroy, &context(), &mut NeverCrash)
        .unwrap();
    assert!(
        journal
            .lifecycle_destroy_cleaned(&destroy, &context())
            .unwrap()
    );
    LifecycleWindowClose {
        terminal: TerminalLifecycleObservation {
            request: destroy,
            status: TerminalStatus::Cleaned,
        },
    }
}
fn inbox(journal: &ControllerJournal) -> super::super::model::Inbox {
    codec::load(&journal.journal.connection).unwrap().unwrap()
}

#[test]
fn terminal_confirmation_known_crashes_preserve_exact_stamp_or_refuse_close() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let close = record_terminal(&mut journal);
        assert!(matches!(
            journal.confirm_lifecycle_terminal(&close.terminal, &context(), &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        let confirmed = inbox(&journal).sessions[0].confirmed_terminal.clone();
        assert_eq!(confirmed.is_some(), point == CrashPoint::AfterCommit);
        if point == CrashPoint::BeforeCommit {
            assert!(matches!(
                journal.close_lifecycle_window(&close, &context(), &mut NeverCrash),
                Err(JournalError::UnresolvedAction)
            ));
        } else {
            assert_eq!(confirmed, Some(close.terminal.clone()));
        }
        journal
            .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .close_lifecycle_window(&close, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&close)
        );
    }
}

#[test]
fn unknown_confirmation_commit_reopens_both_outcomes_under_the_same_full_key() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let close = record_terminal(&mut journal);
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        assert_eq!(
            inbox(&journal).sessions[0].confirmed_terminal.is_some(),
            durable
        );
        journal
            .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .close_lifecycle_window(&close, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&close)
        );
    }
}

#[test]
fn close_known_crashes_keep_original_confirmation_or_replay_the_exact_retired_stamp() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let close = record_terminal(&mut journal);
        journal
            .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
            .unwrap();
        assert!(matches!(
            journal.close_lifecycle_window(&close, &context(), &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        let saved = inbox(&journal);
        assert_eq!(saved.sessions.is_empty(), point == CrashPoint::AfterCommit);
        assert_eq!(
            saved.lanes[0].retired_through,
            u64::from(point == CrashPoint::AfterCommit)
        );
        if point == CrashPoint::BeforeCommit {
            assert_eq!(
                saved.sessions[0].confirmed_terminal,
                Some(close.terminal.clone())
            );
        }
        assert!(
            journal
                .close_lifecycle_window(&close, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&close)
        );
        assert!(
            journal
                .close_lifecycle_window(&close, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&close)
        );
    }
}

#[test]
fn unknown_close_commit_does_not_mint_a_new_window_or_acknowledge_another_request() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let close = record_terminal(&mut journal);
        journal
            .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
            .unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.close_lifecycle_window(&close, &context(), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        assert_eq!(inbox(&journal).sessions.is_empty(), durable);
        let mut wrong = close.clone();
        wrong.terminal.request.request.binding.key.sequence = 9.try_into().unwrap();
        assert!(
            journal
                .close_lifecycle_window(&wrong, &context(), &mut NeverCrash)
                .is_err()
        );
        assert!(
            journal
                .close_lifecycle_window(&close, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&close)
        );
        assert_eq!(
            inbox(&journal).lanes[0].current_window,
            Some(2.try_into().unwrap())
        );
    }
}

#[test]
fn restored_confirmation_and_frontier_must_match_the_complete_original_result() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let close = record_terminal(&mut journal);
    journal
        .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
        .unwrap();
    let original = inbox(&journal);
    let mut wrong = original.clone();
    wrong.lanes[0].retired_through = 1;
    assert!(matches!(wrong.validate(), Err(JournalError::CorruptStore)));
    let mut wrong = original.clone();
    wrong.sessions[0]
        .confirmed_terminal
        .as_mut()
        .unwrap()
        .status = TerminalStatus::Pending;
    assert!(matches!(wrong.validate(), Err(JournalError::CorruptStore)));
    let mut wrong = original.clone();
    wrong.sessions[0]
        .confirmed_terminal
        .as_mut()
        .unwrap()
        .request
        .request
        .binding
        .key
        .sequence = 99.try_into().unwrap();
    assert!(matches!(wrong.validate(), Err(JournalError::CorruptStore)));
    let mut missing = serde_json::to_value(&original).unwrap();
    missing.as_object_mut().unwrap().remove("lanes");
    assert!(serde_json::from_value::<super::super::model::Inbox>(missing).is_err());
    let mut missing = serde_json::to_value(&original).unwrap();
    missing["sessions"][0]
        .as_object_mut()
        .unwrap()
        .remove("confirmed_terminal");
    let unconfirmed: super::super::model::Inbox = serde_json::from_value(missing).unwrap();
    assert!(unconfirmed.sessions[0].confirmed_terminal.is_none());
    codec::save(&journal.journal.connection, &unconfirmed).unwrap();
    assert!(matches!(
        journal.close_lifecycle_window(&close, &context(), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
}

#[test]
fn foreign_or_changed_close_cannot_mutate_the_original_confirmed_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let close = record_terminal(&mut journal);
    journal
        .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
        .unwrap();
    let before = inbox(&journal);
    for index in 0..5 {
        let mut wrong = close.clone();
        match index {
            0 => wrong.terminal.request.request.binding.key.owner = "foreign".try_into().unwrap(),
            1 => wrong.terminal.request.retention.window = 2.try_into().unwrap(),
            2 => wrong.terminal.request.request.binding.session = "foreign".try_into().unwrap(),
            3 => wrong.terminal.request.accepted_version = 99,
            _ => wrong.terminal.status = TerminalStatus::Pending,
        }
        assert!(
            journal
                .close_lifecycle_window(&wrong, &context(), &mut NeverCrash)
                .is_err()
        );
        assert_eq!(inbox(&journal), before);
    }
}

#[test]
fn exhausted_retention_lanes_never_wrap_and_advertise_no_fresh_logical_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let close = record_terminal(&mut journal);
    journal
        .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
        .unwrap();
    journal
        .close_lifecycle_window(&close, &context(), &mut NeverCrash)
        .unwrap();
    let mut boundary = inbox(&journal);
    for lane in &mut boundary.lanes {
        let mut stamp = close.terminal.clone();
        stamp.request.retention.slot = lane.slot;
        if lane.slot == 2 {
            stamp.request.request.binding.key.owner = "beta".try_into().unwrap();
            stamp.request.request.binding.session = "past-b".try_into().unwrap();
        }
        stamp.request.request.binding.key.window = (u64::MAX - 1).try_into().unwrap();
        stamp.request.retention.window = (u64::MAX - 1).try_into().unwrap();
        lane.retired_through = u64::MAX - 1;
        lane.current_window = Some(u64::MAX.try_into().unwrap());
        lane.last_retired = Some(stamp);
    }
    // Finite overflow-boundary fixture; not a claim to have executed 2^64 uses.
    codec::save(&journal.journal.connection, &boundary).unwrap();
    for slot in 1..=boundary.config.max_sessions {
        let mut request = if slot == 1 {
            fixture::create("alpha", "session-a")
        } else {
            fixture::create("beta", "session-b")
        };
        request.retention.slot = slot;
        request.retention.window = RetentionWindowId::try_from(u64::MAX).unwrap();
        request.request.binding.key.window = u64::MAX.try_into().unwrap();
        journal
            .record_lifecycle_intent(&request, &context(), &mut NeverCrash)
            .unwrap();
        let request = fixture::destroy(&request);
        journal
            .record_lifecycle_intent(&request, &context(), &mut NeverCrash)
            .unwrap();
        let terminal = TerminalLifecycleObservation {
            request,
            status: TerminalStatus::Cleaned,
        };
        journal
            .confirm_lifecycle_terminal(&terminal, &context(), &mut NeverCrash)
            .unwrap();
        journal
            .close_lifecycle_window(
                &LifecycleWindowClose { terminal },
                &context(),
                &mut NeverCrash,
            )
            .unwrap();
    }
    assert!(
        inbox(&journal)
            .lanes
            .iter()
            .all(|lane| lane.current_window.is_none() && lane.retired_through == u64::MAX)
    );
    assert_eq!(
        journal
            .lifecycle_available_instances(context().expected_node())
            .unwrap(),
        0
    );
    let fresh: AcceptedLifecycleIntent = fixture::create("alpha", "session-a");
    assert!(
        journal
            .record_lifecycle_intent(&fresh, &context(), &mut NeverCrash)
            .is_err()
    );
}
