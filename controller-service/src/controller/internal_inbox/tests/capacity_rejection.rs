//! Original durable pre-admission rejection, never fabricated worker cleanup.
use super::*;

#[test]
fn capacity_rejection_crashes_and_unknown_commits_preserve_one_exact_fact() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let create = local_request();
        let original = journal
            .record_internal_intent(&create, &mut NeverCrash)
            .unwrap();
        assert!(matches!(
            journal.reject_internal_capacity(&create, &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(journal);
        let mut journal = open(&path);
        let saved = journal.internal_work_state(&create).unwrap();
        assert!(saved.local.is_none());
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
        assert!(journal.world().unwrap().sandbox_records().is_empty());
        if point == CrashPoint::BeforeCommit {
            assert_eq!(saved.response, original);
        } else {
            assert_eq!(saved.response.fact, LifecycleFact::NotAccepted);
        }
        let result = journal
            .reject_internal_capacity(&create, &mut NeverCrash)
            .unwrap();
        assert_eq!(result.fact, LifecycleFact::NotAccepted);
        assert_eq!(
            journal
                .reject_internal_capacity(&create, &mut NeverCrash)
                .unwrap(),
            result
        );
        assert_eq!(
            journal
                .record_internal_intent(&create, &mut NeverCrash)
                .unwrap(),
            result
        );
        assert!(journal.internal_pending(&config().node).unwrap().is_empty());
    }
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let create = local_request();
        journal
            .record_internal_intent(&create, &mut NeverCrash)
            .unwrap();
        crate::controller::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.reject_internal_capacity(&create, &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        // The original memo invalidates and reconstructs durable authority on
        // the next read. Neither ambiguous outcome can allocate a native VM.
        assert!(journal.world().unwrap().sandbox_records().is_empty());
        drop(journal);
        let mut journal = open(&path);
        let saved = journal.internal_work_state(&create).unwrap();
        assert_eq!(
            saved.response.fact,
            if durable {
                LifecycleFact::NotAccepted
            } else {
                LifecycleFact::Recorded
            }
        );
        assert!(saved.local.is_none());
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
        assert_eq!(
            journal
                .reject_internal_capacity(&create, &mut NeverCrash)
                .unwrap()
                .fact,
            LifecycleFact::NotAccepted
        );
    }
}

#[test]
fn native_admission_or_newer_terminal_never_becomes_no_admission_rejection() {
    for admitted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = open(&dir.path().join("db"));
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let create = local_request();
        journal
            .record_internal_intent(&create, &mut NeverCrash)
            .unwrap();
        if admitted {
            journal
                .admit_internal_create(&create, local_admission(), &mut NeverCrash)
                .unwrap();
        } else {
            let mut destroy = create.clone();
            destroy.intent = Intent::Destroy;
            destroy.identity.intent_version = 2.try_into().unwrap();
            destroy.identity.operation_id = "destroy".try_into().unwrap();
            journal
                .record_internal_intent(&destroy, &mut NeverCrash)
                .unwrap();
        }
        let before = journal.internal_audit().unwrap();
        assert!(matches!(
            journal.reject_internal_capacity(&create, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
        assert_eq!(journal.internal_audit().unwrap(), before);
    }
}
