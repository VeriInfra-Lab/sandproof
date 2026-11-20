use super::*;
use crate::NeverCrash;
use crate::controller::{lifecycle_inbox::tests as fixture, transaction};
use sandproof_protocol::lifecycle::{
    LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
};

fn context() -> AuthenticatedCenterContext {
    let create = fixture::create("alpha", "session-a");
    AuthenticatedCenterContext::from_verified_peer_registration(
        create.center,
        create.request.binding.node,
    )
}

fn occupied(journal: &mut ControllerJournal) -> AcceptedLifecycleIntent {
    let first = fixture::create("alpha", "session-a");
    journal
        .record_lifecycle_intent(&first, &context(), &mut NeverCrash)
        .unwrap();
    journal
        .admit_lifecycle_create(&first, &context(), fixture::admission(), &mut NeverCrash)
        .unwrap();
    let second = fixture::create("beta", "session-b");
    journal
        .record_lifecycle_intent(&second, &context(), &mut NeverCrash)
        .unwrap();
    second
}

fn limited() -> crate::InternalAdmission {
    let mut admission = fixture::admission();
    admission.limits.max_active_global = 1;
    admission
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
fn refusal_crashes_before_and_after_commit_preserve_original_responsibility() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let request = occupied(&mut journal);
        let before = journal.world().unwrap();
        assert!(
            matches!(journal.admit_lifecycle_create(&request, &context(), limited(), &mut Stop(point)),
            Err(crate::AdmissionError::Journal(JournalError::InjectedCrash(hit))) if hit == point)
        );
        drop(journal);
        let journal = fixture::open(&path);
        let state = journal.lifecycle_work_state(&request, &context()).unwrap();
        assert_eq!(
            state.refusal,
            (point == CrashPoint::AfterCommit).then_some(CreateRefusalReason::Capacity)
        );
        assert!(state.local.is_none());
        assert_eq!(journal.world().unwrap(), before);
    }
}

#[test]
fn current_refusal_store_rejects_prior_format_in_both_original_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let journal = fixture::initialize(&path);
    assert_eq!(
        journal
            .journal
            .connection
            .query_row("SELECT version FROM controller_meta WHERE id=1", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        crate::LIFECYCLE_STORE_FORMAT_VERSION
    );
    journal
        .journal
        .connection
        .execute("UPDATE controller_meta SET version=18 WHERE id=1", [])
        .unwrap();
    drop(journal);
    let before = std::fs::read(&path).unwrap();
    let host = fixture::host(&path);
    let maintenance = crate::controller::tests_support::lifecycle_application::maintenance();
    assert!(matches!(
        ControllerJournal::preflight_complete(&path, "0123456789abcdef", &host, &maintenance),
        Err(JournalError::UnsupportedStore)
    ));
    assert!(matches!(
        ControllerJournal::open_complete(&path, "0123456789abcdef", host, maintenance, 0),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn native_quota_refusal_keeps_exact_request_and_never_retries_when_capacity_returns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = fixture::initialize(&path);
    let request = occupied(&mut journal);
    let before = journal.world().unwrap();
    let next = journal.next_sandbox_id().unwrap();
    let refused = journal
        .admit_lifecycle_create(&request, &context(), limited(), &mut NeverCrash)
        .unwrap();
    assert_eq!(refused.refusal, Some(CreateRefusalReason::Capacity));
    assert!(refused.current && !refused.terminal && refused.local.is_none());
    assert_eq!(refused.intent, request);
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.next_sandbox_id().unwrap(), next);
    assert!(
        journal
            .lifecycle_pending_creates(&request.request.binding.node)
            .unwrap()
            .iter()
            .all(|pending| pending != &request)
    );
    let mut changed = request.clone();
    let LifecycleOperation::Create { create, .. } = &mut changed.request.operation else {
        panic!()
    };
    create.timeout_ms += 1000;
    assert!(matches!(
        journal.record_lifecycle_intent(&changed, &context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    drop(journal);
    let mut journal = fixture::open(&path);
    // Saved refusal is independent of a later good quota, changed cache/time,
    // or absence of fresh startup eligibility after manager restart.
    let mut admission = fixture::admission();
    admission.now_ms = i64::MAX;
    admission.template.digest = [9; 32];
    assert_eq!(
        journal
            .admit_lifecycle_create(&request, &context(), admission, &mut NeverCrash)
            .unwrap(),
        refused
    );
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.next_sandbox_id().unwrap(), next);
    let observed = journal
        .lifecycle_ready_candidate::<crate::controller::tests_support::TestHostEvidence>(
            &request,
            &context(),
            20,
        )
        .unwrap()
        .view;
    assert_eq!(observed.request, request);
    assert_eq!(observed.current_request, request);
    assert_eq!(
        observed.readiness,
        super::super::CreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity
        }
    );
    assert!(
        observed.local.is_none()
            && observed.historical_result.is_none()
            && observed.native.is_none()
    );
}

#[test]
fn uncertain_refusal_commit_requires_original_query_for_both_durable_outcomes() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let request = occupied(&mut journal);
        let before = journal.world().unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.admit_lifecycle_create(&request, &context(), limited(), &mut NeverCrash),
            Err(crate::AdmissionError::Journal(
                JournalError::CommitUncertain(_)
            ))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        let state = journal.lifecycle_work_state(&request, &context()).unwrap();
        assert_eq!(
            state.refusal,
            durable.then_some(CreateRefusalReason::Capacity)
        );
        assert!(state.local.is_none());
        assert_eq!(journal.world().unwrap(), before);
        assert_eq!(
            journal
                .lifecycle_pending_creates(&request.request.binding.node)
                .unwrap()
                .contains(&request),
            !durable
        );
        fixture::refresh_epoch(&mut journal);
        if durable {
            assert_eq!(
                journal
                    .admit_lifecycle_create(
                        &request,
                        &context(),
                        fixture::admission(),
                        &mut NeverCrash
                    )
                    .unwrap(),
                state
            );
        } else {
            // No durable refusal and the failed quota transaction produced no
            // effects; a newly sufficient original admission may still succeed.
            assert!(
                journal
                    .admit_lifecycle_create(
                        &request,
                        &context(),
                        fixture::admission(),
                        &mut NeverCrash
                    )
                    .unwrap()
                    .local
                    .is_some()
            );
        }
    }
}

#[test]
fn destroy_and_admission_winners_cannot_be_relabelled_as_refused() {
    for admitted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = fixture::initialize(&dir.path().join("db"));
        let create = fixture::create("alpha", "session-a");
        journal
            .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
            .unwrap();
        if admitted {
            journal
                .admit_lifecycle_create(&create, &context(), fixture::admission(), &mut NeverCrash)
                .unwrap();
        } else {
            journal
                .record_lifecycle_intent(&fixture::destroy(&create), &context(), &mut NeverCrash)
                .unwrap();
        }
        let before = journal.lifecycle_work_state(&create, &context()).unwrap();
        // Inject the race at the actual persistence boundary, after a physical
        // negative sample could have returned. Current native responsibility wins.
        let after = journal
            .refuse_lifecycle_create_capacity(&create, &context(), &mut NeverCrash)
            .unwrap();
        assert_eq!(after, before);
        assert_eq!(after.refusal, None);
        assert_eq!(after.local.is_some(), admitted);
        assert_eq!(after.terminal, !admitted);
    }
}

#[test]
fn recovery_rejects_refusal_attached_to_an_original_native_mapping() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = fixture::initialize(&path);
    let create = fixture::create("alpha", "session-a");
    journal
        .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
        .unwrap();
    journal
        .admit_lifecycle_create(&create, &context(), fixture::admission(), &mut NeverCrash)
        .unwrap();
    let mut inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
    inbox.sessions[0].refusal = Some(CreateRefusalReason::Capacity);
    let bytes = serde_json::to_vec(&inbox).unwrap();
    // Canonical bytes and a matching checksum cannot legitimize this impossible
    // relationship. Recovery must inspect the actual native/refusal relation.
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_lifecycle_inbox SET state_json=?1,sha256=?2 WHERE id=1",
            rusqlite::params![bytes, Sha256::digest(&bytes).as_slice()],
        )
        .unwrap();
    drop(journal);
    assert!(matches!(
        ControllerJournal::open_complete(
            &path,
            "0123456789abcdef",
            fixture::host(&path),
            crate::controller::tests_support::lifecycle_application::maintenance(),
            0
        ),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn refused_create_stays_queryable_through_exact_destroy_confirmation_and_close() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = fixture::initialize(&path);
    let create = occupied(&mut journal);
    journal
        .admit_lifecycle_create(&create, &context(), limited(), &mut NeverCrash)
        .unwrap();
    // Refusal alone is no cleanup/refund fact and cannot confirm a Create.
    assert!(matches!(
        journal.lifecycle_destroy_cleaned(&create, &context()),
        Err(JournalError::Conflict)
    ));
    let destroy = fixture::destroy(&create);
    journal
        .record_lifecycle_intent(&destroy, &context(), &mut NeverCrash)
        .unwrap();
    let observed = journal
        .lifecycle_ready_candidate::<crate::controller::tests_support::TestHostEvidence>(
            &create,
            &context(),
            20,
        )
        .unwrap()
        .view;
    assert_eq!(observed.current_request, destroy);
    assert_eq!(
        observed.readiness,
        super::super::CreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity
        }
    );
    let close = LifecycleWindowClose {
        terminal: TerminalLifecycleObservation {
            request: destroy,
            status: TerminalStatus::Cleaned,
        },
    };
    assert!(
        journal
            .lifecycle_destroy_cleaned(&close.terminal.request, &context())
            .unwrap()
    );
    assert!(matches!(
        journal.close_lifecycle_window(&close, &context(), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    journal
        .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
        .unwrap();
    journal
        .close_lifecycle_window(&close, &context(), &mut NeverCrash)
        .unwrap();
    let world = journal.world().unwrap();
    assert_eq!(world.sandbox_records().len(), 1);
    assert_eq!(
        journal.next_sandbox_id().unwrap(),
        sandbox_controller_core::SandboxId(2)
    );
    drop(journal);
    let mut journal = fixture::open(&path);
    assert!(
        journal
            .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
            .is_err()
    );
    assert_eq!(journal.world().unwrap(), world);
    assert_eq!(
        journal
            .close_lifecycle_window(&close, &context(), &mut NeverCrash)
            .unwrap()
            .terminal,
        close.terminal
    );
}
