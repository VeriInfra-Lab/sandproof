//! Real original SQLite/native workflow; only physical host acknowledgements
//! use the existing controlled worker. No confirmation is synthesized by a stub.
use super::*;
use crate::controller::{lifecycle_inbox::tests as fixture, transaction};
use crate::{InternalLocalBinding, NeverCrash};
use sandproof_protocol::lifecycle::OperationKey;

fn context() -> AuthenticatedCenterContext {
    let create = fixture::create("alpha", "session-a");
    AuthenticatedCenterContext::from_verified_peer_registration(
        create.center,
        create.request.binding.node,
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
    window: u64,
) -> AcceptedLifecycleIntent {
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = create.request.operation
    else {
        panic!()
    };
    let mut request = create.clone();
    request.accepted_version = version;
    request.request.binding.expected = ExpectedSession::Version(version - 1);
    request.request.binding.key.window = window.try_into().unwrap();
    request.request.binding.key.sequence = version.try_into().unwrap();
    request.request.operation = LifecycleOperation::Renew {
        incarnation: target_incarnation,
        end_at_ms,
        extend_only: false,
    };
    request
}

fn apply(
    journal: &mut ControllerJournal,
    request: &AcceptedLifecycleIntent,
) -> RenewalConfirmation {
    journal
        .record_lifecycle_intent(request, &context(), &mut NeverCrash)
        .unwrap();
    let end = journal
        .apply_lifecycle_renew(request, &context(), 100, &mut NeverCrash)
        .unwrap()
        .1;
    RenewalConfirmation {
        request: request.clone(),
        effective_end_at_ms: end,
    }
}

fn saved(journal: &ControllerJournal) -> super::super::model::Inbox {
    codec::load(&journal.journal.connection).unwrap().unwrap()
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

#[test]
fn confirmed_sdk_renewals_reuse_bounded_history_through_version_twenty_four_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = fixture::initialize(&path);
    let (create, local) = running(&mut journal);
    assert_eq!(saved(&journal).config.max_operations_per_session, 8);
    for version in 2..=24 {
        let result = apply(
            &mut journal,
            &renewal(
                &create,
                version,
                60_020 + i64::try_from(version).unwrap() * 1000,
                2,
            ),
        );
        assert!(
            saved(&journal).sessions[0].entries.len() <= 3,
            "actual peak must fit the original four-row property"
        );
        assert!(
            journal
                .confirm_lifecycle_renewal(&result, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&result)
        );
        let inbox = saved(&journal);
        let session = &inbox.sessions[0];
        assert_eq!(session.entries.len(), 2);
        assert_eq!(session.entries[0].intent, create);
        assert_eq!(session.entries[1].intent, result.request);
        assert_eq!(session.confirmed_renewal.as_ref(), Some(&result));
        assert_eq!(
            session.compacted_sdk_through,
            if version == 2 {
                None
            } else {
                Some(version - 1)
            }
        );
        assert_eq!(session.local, Some(local));
        assert!(serde_json::to_vec(&inbox).unwrap().len() as u64 <= inbox.config.required_bytes());
        drop(journal);
        journal = fixture::open(&path);
        assert_eq!(deadline(&journal), result.effective_end_at_ms);
        assert_eq!(
            journal
                .confirm_lifecycle_renewal(&result, &context(), &mut NeverCrash)
                .unwrap()
                .result,
            result
        );
        assert_eq!(
            journal.next_sandbox_id().unwrap(),
            sandbox_controller_core::SandboxId(2)
        );
    }
    let mut destroy = renewal(&create, 25, 90_020, 11);
    destroy.request.operation = LifecycleOperation::Destroy {};
    assert!(
        journal
            .record_lifecycle_intent(&destroy, &context(), &mut NeverCrash)
            .unwrap()
            .terminal
    );
}

#[test]
fn confirmed_sdk_compaction_preserves_public_replay_and_current_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = fixture::initialize(&path);
    let (create, _) = running(&mut journal);
    let old = apply(&mut journal, &renewal(&create, 2, 90_020, 2));
    journal
        .confirm_lifecycle_renewal(&old, &context(), &mut NeverCrash)
        .unwrap();
    let public = renewal(&create, 3, 40_020, 11);
    let public_result = apply(&mut journal, &public);
    let current = apply(&mut journal, &renewal(&create, 4, 80_020, 2));
    journal
        .confirm_lifecycle_renewal(&current, &context(), &mut NeverCrash)
        .unwrap();
    let inbox = saved(&journal);
    assert!(
        inbox.sessions[0]
            .entries
            .iter()
            .any(|entry| entry.intent == public),
        "public renewal replay must survive SDK compaction"
    );
    assert!(
        !inbox.sessions[0]
            .entries
            .iter()
            .any(|entry| entry.intent == old.request)
    );
    assert_eq!(
        inbox.sessions[0].compacted_sdk_through,
        Some(2),
        "only an actually deleted SDK version is recorded"
    );
    drop(journal);
    let mut journal = fixture::open(&path);
    let (state, end) = journal
        .apply_lifecycle_renew(&public, &context(), 100_000, &mut NeverCrash)
        .unwrap();
    assert!(!state.current);
    assert_eq!(end, public_result.effective_end_at_ms);
    assert_eq!(deadline(&journal), current.effective_end_at_ms);
    assert!(matches!(
        journal.confirm_lifecycle_renewal(&old, &context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert_eq!(
        saved(&journal).sessions[0].confirmed_renewal.as_ref(),
        Some(&current)
    );
}

#[test]
fn outstanding_sdk_hidden_by_public_row_blocks_next_sdk_but_never_destroy() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let (create, _) = running(&mut journal);
    let sdk = renewal(&create, 2, 90_020, 2);
    journal
        .record_lifecycle_intent(&sdk, &context(), &mut NeverCrash)
        .unwrap();
    let public = renewal(&create, 3, 50_020, 11);
    journal
        .record_lifecycle_intent(&public, &context(), &mut NeverCrash)
        .unwrap();
    let next = renewal(&create, 4, 100_020, 2);
    assert!(matches!(
        journal.record_lifecycle_intent(&next, &context(), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(deadline(&journal), 60_020);
    assert_eq!(saved(&journal).sessions[0].current_version, 3);
    let mut destroy = next;
    destroy.request.operation = LifecycleOperation::Destroy {};
    assert!(
        journal
            .record_lifecycle_intent(&destroy, &context(), &mut NeverCrash)
            .unwrap()
            .terminal
    );
    assert!(
        saved(&journal).sessions[0]
            .entries
            .iter()
            .any(|entry| entry.intent == sdk)
    );
}

#[test]
fn confirmation_requires_actual_application_and_exact_request_end_and_peer() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let (create, _) = running(&mut journal);
    let sdk = renewal(&create, 2, 90_020, 2);
    journal
        .record_lifecycle_intent(&sdk, &context(), &mut NeverCrash)
        .unwrap();
    let result = RenewalConfirmation {
        request: sdk.clone(),
        effective_end_at_ms: 90_020,
    };
    assert!(matches!(
        journal.confirm_lifecycle_renewal(&result, &context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    journal
        .apply_lifecycle_renew(&sdk, &context(), 100, &mut NeverCrash)
        .unwrap();
    let mut foreign_center = context().center().clone();
    foreign_center.center = "other".try_into().unwrap();
    let foreign = AuthenticatedCenterContext::from_verified_peer_registration(
        foreign_center,
        context().expected_node().clone(),
    );
    assert!(matches!(
        journal.confirm_lifecycle_renewal(&result, &foreign, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let mut changed = result.clone();
    changed.effective_end_at_ms += 1;
    assert!(matches!(
        journal.confirm_lifecycle_renewal(&changed, &context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    changed = result.clone();
    changed.request.request.binding.key.owner = "other-owner".try_into().unwrap();
    assert!(matches!(
        journal.confirm_lifecycle_renewal(&changed, &context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    changed = result.clone();
    changed.request.request.binding.key.window = 11.try_into().unwrap();
    assert!(matches!(
        journal.confirm_lifecycle_renewal(&changed, &context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert!(saved(&journal).sessions[0].confirmed_renewal.is_none());
    assert_eq!(deadline(&journal), 90_020);
    assert!(
        journal
            .confirm_lifecycle_renewal(&result, &context(), &mut NeverCrash)
            .unwrap()
            .matches(&result)
    );
}

#[test]
fn unknown_confirmation_commit_keeps_checkpoint_and_actual_pruning_atomic() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let (create, _) = running(&mut journal);
        let first = apply(&mut journal, &renewal(&create, 2, 90_020, 2));
        journal
            .confirm_lifecycle_renewal(&first, &context(), &mut NeverCrash)
            .unwrap();
        let second = apply(&mut journal, &renewal(&create, 3, 100_020, 2));
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.confirm_lifecycle_renewal(&second, &context(), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        let inbox = saved(&journal);
        assert_eq!(inbox.sessions[0].entries.len(), if durable { 2 } else { 3 });
        assert_eq!(
            inbox.sessions[0].compacted_sdk_through,
            if durable { Some(2) } else { None }
        );
        assert_eq!(
            inbox.sessions[0].confirmed_renewal.as_ref(),
            Some(if durable { &second } else { &first })
        );
        assert_eq!(deadline(&journal), 100_020);
        assert!(
            journal
                .confirm_lifecycle_renewal(&second, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&second)
        );
        assert_eq!(saved(&journal).sessions[0].entries.len(), 2);
    }
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
fn original_before_and_after_confirmation_commit_crashes_preserve_recoverable_result() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let (create, _) = running(&mut journal);
        let result = apply(&mut journal, &renewal(&create, 2, 90_020, 2));
        assert!(
            matches!(journal.confirm_lifecycle_renewal(&result, &context(), &mut Stop(point)),
            Err(JournalError::InjectedCrash(hit)) if hit == point)
        );
        drop(journal);
        let mut journal = fixture::open(&path);
        assert_eq!(
            saved(&journal).sessions[0].confirmed_renewal.is_some(),
            point == CrashPoint::AfterCommit
        );
        assert_eq!(deadline(&journal), 90_020);
        assert!(
            journal
                .confirm_lifecycle_renewal(&result, &context(), &mut NeverCrash)
                .unwrap()
                .matches(&result)
        );
    }
}

#[test]
fn compacted_extend_only_keeps_the_actual_shorter_predecessor_not_initial_create_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = fixture::initialize(&path);
    let (create, _) = running(&mut journal);
    let first = apply(&mut journal, &renewal(&create, 2, 10_020, 2));
    journal
        .confirm_lifecycle_renewal(&first, &context(), &mut NeverCrash)
        .unwrap();
    let mut request = renewal(&create, 3, 5_020, 2);
    let LifecycleOperation::Renew { extend_only, .. } = &mut request.request.operation else {
        panic!()
    };
    *extend_only = true;
    let second = apply(&mut journal, &request);
    assert_eq!(second.effective_end_at_ms, 10_020);
    journal
        .confirm_lifecycle_renewal(&second, &context(), &mut NeverCrash)
        .unwrap();
    drop(journal);
    let journal = fixture::open(&path);
    let inbox = saved(&journal);
    let entry = inbox.sessions[0].entries.last().unwrap();
    assert_eq!(entry.previous_effective_end_at_ms, Some(10_020));
    assert_eq!(deadline(&journal), 10_020);
    assert_eq!(inbox.sessions[0].compacted_sdk_through, Some(2));
    let mut corrupt = inbox;
    corrupt.sessions[0]
        .entries
        .last_mut()
        .unwrap()
        .previous_effective_end_at_ms = Some(60_020);
    assert!(matches!(
        corrupt.validate(),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn exact_prune_predicate_retains_public_current_unapplied_and_pinned_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let (create, _) = running(&mut journal);
    let result = apply(&mut journal, &renewal(&create, 2, 90_020, 2));
    let inbox = saved(&journal);
    let original = inbox.sessions[0].entries[1].clone();
    assert!(discard_previous(&original, &result, 3, false));
    assert!(!discard_previous(&original, &result, 2, false));
    assert!(!discard_previous(&original, &result, 3, true));
    let mut entry = original.clone();
    entry.applied_deadline = None;
    entry.previous_effective_end_at_ms = None;
    assert!(!discard_previous(&entry, &result, 3, false));
    entry = original.clone();
    entry.local_operation = Some(sandbox_controller_core::OperationId(1));
    assert!(!discard_previous(&entry, &result, 3, false));
    entry = original;
    entry.intent.request.binding.key = OperationKey {
        window: 11.try_into().unwrap(),
        ..entry.intent.request.binding.key
    };
    assert!(!discard_previous(&entry, &result, 3, false));
}

#[test]
fn current_renewal_fields_require_explicit_null_instead_of_missing_old_format_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    running(&mut journal);
    let original = serde_json::to_value(saved(&journal)).unwrap();
    for name in ["confirmed_renewal", "compacted_sdk_through"] {
        let mut missing = original.clone();
        assert!(
            missing["sessions"][0]
                .as_object_mut()
                .unwrap()
                .remove(name)
                .unwrap()
                .is_null()
        );
        assert!(serde_json::from_value::<super::super::model::Inbox>(missing).is_err());
    }
    let mut missing = original.clone();
    assert!(
        missing["sessions"][0]["entries"][0]
            .as_object_mut()
            .unwrap()
            .remove("previous_effective_end_at_ms")
            .unwrap()
            .is_null()
    );
    assert!(serde_json::from_value::<super::super::model::Inbox>(missing).is_err());
    assert!(serde_json::from_value::<super::super::model::Inbox>(original).is_ok());
}

#[test]
fn current_renewal_store_refuses_prior_formats_nineteen_and_twenty_without_mutation() {
    for prior in [19, 20] {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let journal = fixture::initialize(&path);
    assert_eq!(
        crate::LIFECYCLE_STORE_FORMAT_VERSION,
        21,
        "run only the complete lifecycle format 21 candidate"
    );
    journal
        .journal
        .connection
        .execute("UPDATE controller_meta SET version=?1 WHERE id=1", [prior])
        .unwrap();
    drop(journal);
    let before = std::fs::read(&path).unwrap();
    let host = crate::controller::tests_support::host(4);
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
}
