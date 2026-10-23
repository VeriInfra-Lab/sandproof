//! Original renewal assertions on the current full-request transaction. The
//! native workflow is real; physical host acknowledgements are controlled.
use super::*;
use crate::controller::{lifecycle_inbox::tests as fixture, transaction};
use crate::{InternalLocalBinding, NeverCrash};
use sandproof_protocol::lifecycle::ExpectedSession;

fn context() -> AuthenticatedCenterContext {
    let request = fixture::create("alpha", "session-a");
    AuthenticatedCenterContext::from_verified_peer_registration(
        request.center,
        request.request.binding.node,
    )
}
fn running(journal: &mut ControllerJournal) -> (AcceptedLifecycleIntent, InternalLocalBinding) {
    let create = fixture::create("alpha", "session-a");
    journal
        .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
        .unwrap();
    let local = journal
        .admit_lifecycle_create(&create, &context(), fixture::admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    crate::finish_admitted_create_timed(
        journal,
        local.sandbox,
        local.operation,
        &mut crate::controller::tests_support::worker,
        &mut || Ok(20),
    )
    .unwrap();
    (create, local)
}
fn renewal(
    create: &AcceptedLifecycleIntent,
    version: u64,
    end_at_ms: i64,
) -> AcceptedLifecycleIntent {
    let LifecycleOperation::Create {
        target_incarnation: incarnation,
        ..
    } = create.request.operation
    else {
        panic!()
    };
    let mut request = create.clone();
    request.accepted_version = version;
    request.request.binding.expected = ExpectedSession::Version(version - 1);
    request.request.binding.key.sequence = version.try_into().unwrap();
    request.request.operation = LifecycleOperation::Renew {
        incarnation,
        end_at_ms,
        extend_only: false,
    };
    request
}
fn deadline(journal: &ControllerJournal) -> i64 {
    journal
        .journal
        .connection
        .query_row("SELECT deadline_ms FROM sandbox_lifetimes", [], |r| {
            r.get(0)
        })
        .unwrap()
}
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

#[test]
fn fixed_deadline_and_exact_application_commit_together_across_original_crash_points() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let (create, local) = running(&mut journal);
        let request = renewal(&create, 2, 90_020);
        journal
            .record_lifecycle_intent(&request, &context(), &mut NeverCrash)
            .unwrap();
        assert!(
            matches!(journal.apply_lifecycle_renew(&request, &context(), 100, &mut Stop(point)),
            Err(AdmissionError::Journal(JournalError::InjectedCrash(hit))) if hit==point)
        );
        drop(journal);
        let mut journal = fixture::open(&path);
        assert_eq!(
            deadline(&journal),
            if point == CrashPoint::AfterCommit {
                90_020
            } else {
                60_020
            }
        );
        assert_eq!(
            journal
                .lifecycle_pending_renewals(&request.request.binding.node)
                .unwrap()
                .contains(&request),
            point == CrashPoint::BeforeCommit
        );
        let (state, end) = journal
            .apply_lifecycle_renew(&request, &context(), 100, &mut NeverCrash)
            .unwrap();
        assert_eq!(state.local, Some(local));
        assert_eq!(end, 90_020);
        assert_eq!(
            journal
                .apply_lifecycle_renew(&request, &context(), 100_000, &mut NeverCrash)
                .unwrap()
                .1,
            end
        );
        let mut changed = request.clone();
        let LifecycleOperation::Renew { end_at_ms, .. } = &mut changed.request.operation else {
            panic!()
        };
        *end_at_ms += 1;
        assert!(matches!(
            journal.apply_lifecycle_renew(&changed, &context(), 100, &mut NeverCrash),
            Err(AdmissionError::Journal(JournalError::Conflict))
        ));
        assert_eq!(deadline(&journal), 90_020);
        assert_eq!(
            journal.next_sandbox_id().unwrap(),
            sandbox_controller_core::SandboxId(2)
        );
        assert!(
            journal
                .lifecycle_pending_renewals(&request.request.binding.node)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn unknown_renewal_commit_keeps_native_deadline_and_saved_application_atomic() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let (create, _) = running(&mut journal);
        let request = renewal(&create, 2, 90_020);
        journal
            .record_lifecycle_intent(&request, &context(), &mut NeverCrash)
            .unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.apply_lifecycle_renew(&request, &context(), 100, &mut NeverCrash),
            Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        assert_eq!(deadline(&journal), if durable { 90_020 } else { 60_020 });
        assert_eq!(
            journal
                .lifecycle_pending_renewals(&request.request.binding.node)
                .unwrap()
                .contains(&request),
            !durable
        );
        assert_eq!(
            journal
                .apply_lifecycle_renew(&request, &context(), 100, &mut NeverCrash)
                .unwrap()
                .1,
            90_020
        );
        assert_eq!(deadline(&journal), 90_020);
    }
}

#[test]
fn invalid_clock_expiry_signed_range_and_original_maximum_leave_saved_renewal_unapplied() {
    for (now, end) in [
        (-1, 90_020),
        (19, 90_020),
        (60_020, 90_020),
        (100, i64::MAX),
        (100, 600_021),
        (100, 19),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = fixture::initialize(&dir.path().join("db"));
        let (create, _) = running(&mut journal);
        let request = renewal(&create, 2, end);
        let original = journal
            .record_lifecycle_intent(&request, &context(), &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .apply_lifecycle_renew(&request, &context(), now, &mut NeverCrash)
                .is_err()
        );
        assert_eq!(deadline(&journal), 60_020);
        assert_eq!(
            journal.lifecycle_work_state(&request, &context()).unwrap(),
            original
        );
        assert_eq!(
            journal
                .lifecycle_pending_renewals(&request.request.binding.node)
                .unwrap(),
            vec![request]
        );
    }
}

#[test]
fn newer_renewal_and_terminal_request_fence_an_unapplied_old_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let (create, _) = running(&mut journal);
    let old = renewal(&create, 2, 90_020);
    journal
        .record_lifecycle_intent(&old, &context(), &mut NeverCrash)
        .unwrap();
    let newer = renewal(&create, 3, 40_020);
    journal
        .record_lifecycle_intent(&newer, &context(), &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        journal.apply_lifecycle_renew(&old, &context(), 100, &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::Conflict))
    ));
    assert_eq!(deadline(&journal), 60_020);
    journal
        .apply_lifecycle_renew(&newer, &context(), 100, &mut NeverCrash)
        .unwrap();
    assert_eq!(deadline(&journal), 40_020);
    let mut destroy = fixture::destroy(&create);
    destroy.accepted_version = 4;
    destroy.request.binding.expected = ExpectedSession::Version(3);
    destroy.request.binding.key.sequence = 4.try_into().unwrap();
    journal
        .record_lifecycle_intent(&destroy, &context(), &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        journal.apply_lifecycle_renew(&old, &context(), 101, &mut NeverCrash),
        Err(AdmissionError::Journal(JournalError::Conflict))
    ));
    assert_eq!(deadline(&journal), 40_020);
    assert!(
        journal
            .lifecycle_pending_renewals(&destroy.request.binding.node)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn historical_applied_renewal_replays_its_exact_end_without_overwriting_a_newer_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = fixture::initialize(&path);
    let (create, local) = running(&mut journal);
    let old = renewal(&create, 2, 90_020);
    journal
        .record_lifecycle_intent(&old, &context(), &mut NeverCrash)
        .unwrap();
    let old_end = journal
        .apply_lifecycle_renew(&old, &context(), 100, &mut NeverCrash)
        .unwrap()
        .1;
    let current = renewal(&create, 3, 40_020);
    journal
        .record_lifecycle_intent(&current, &context(), &mut NeverCrash)
        .unwrap();
    let current_end = journal
        .apply_lifecycle_renew(&current, &context(), 200, &mut NeverCrash)
        .unwrap()
        .1;
    drop(journal);
    let mut journal = fixture::open(&path);
    assert_eq!(deadline(&journal), 40_020);
    let (history, end) = journal
        .apply_lifecycle_renew(&old, &context(), 100_000, &mut NeverCrash)
        .unwrap();
    assert!(!history.current);
    assert_eq!(history.local, Some(local));
    assert_eq!(end, old_end);
    assert_eq!(
        journal
            .apply_lifecycle_renew(&current, &context(), 100_000, &mut NeverCrash)
            .unwrap()
            .1,
        current_end
    );
    assert_eq!(deadline(&journal), 40_020);
    assert!(
        journal
            .lifecycle_pending_renewals(&current.request.binding.node)
            .unwrap()
            .is_empty()
    );
}
