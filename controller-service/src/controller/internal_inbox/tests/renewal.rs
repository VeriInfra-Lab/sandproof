use super::*;

fn running(journal: &mut ControllerJournal) -> (LifecycleRequest, InternalLocalBinding) {
    let create = local_request();
    journal
        .record_internal_intent(&create, &mut NeverCrash)
        .unwrap();
    let local = journal
        .admit_internal_create(&create, local_admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    crate::finish_admitted_create_v1_timed(
        journal,
        local.sandbox,
        local.operation,
        &mut workflow_fixture::worker,
        &mut || Ok(20),
    )
    .unwrap();
    journal
        .capture_internal_progress(&create, &mut NeverCrash)
        .unwrap();
    (create, local)
}

fn renewal(create: &LifecycleRequest, operation: u64, end: u64) -> LifecycleRequest {
    LifecycleRequest {
        intent: Intent::Renew { end_at_ms: end },
        identity: request(operation, operation, Intent::Renew { end_at_ms: end }).identity,
        ..create.clone()
    }
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
fn fixed_renewal_and_response_commit_together_and_replay_never_retime() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (create, _) = running(&mut journal);
        let renew = renewal(&create, 2, 90_020);
        journal
            .record_internal_intent(&renew, &mut NeverCrash)
            .unwrap();
        assert!(matches!(
            journal.apply_internal_renew(&renew, 100, &mut Stop(point)),
            Err(AdmissionError::Journal(JournalError::InjectedCrash(_)))
        ));
        drop(journal);
        let mut journal = open(&path);
        let committed = point == CrashPoint::AfterCommit;
        assert_eq!(deadline(&journal), if committed { 90_020 } else { 60_020 });
        let result = journal
            .apply_internal_renew(&renew, 100, &mut NeverCrash)
            .unwrap();
        assert_eq!(result.fact, LifecycleFact::Renewed { end_at_ms: 90_020 });
        // The old successful result is history, not a new execution permit.
        assert_eq!(
            journal
                .apply_internal_renew(&renew, 100_000, &mut NeverCrash)
                .unwrap(),
            result
        );
        assert_eq!(deadline(&journal), 90_020);
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
    }
}

#[test]
fn unknown_renew_commit_keeps_exact_identity_deadline_and_fact_atomic() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (create, _) = running(&mut journal);
        let renew = renewal(&create, 2, 90_020);
        journal
            .record_internal_intent(&renew, &mut NeverCrash)
            .unwrap();
        crate::controller::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.apply_internal_renew(&renew, 100, &mut NeverCrash),
            Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
        ));
        drop(journal);
        let mut journal = open(&path);
        assert_eq!(deadline(&journal), if durable { 90_020 } else { 60_020 });
        assert_eq!(
            journal
                .apply_internal_renew(&renew, 100, &mut NeverCrash)
                .unwrap()
                .fact,
            LifecycleFact::Renewed { end_at_ms: 90_020 }
        );
    }
}

#[test]
fn invalid_clock_expiry_overflow_and_original_maximum_do_not_mutate() {
    for (now, end) in [
        (-1, 90_020),
        (19, 90_020),
        (60_020, 90_020),
        (100, u64::MAX),
        (100, 600_021),
        (100, 19),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = open(&dir.path().join("db"));
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (create, _) = running(&mut journal);
        let renew = renewal(&create, 2, end);
        let original = journal
            .record_internal_intent(&renew, &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .apply_internal_renew(&renew, now, &mut NeverCrash)
                .is_err()
        );
        assert_eq!(deadline(&journal), 60_020);
        assert_eq!(
            journal
                .record_internal_intent(&renew, &mut NeverCrash)
                .unwrap(),
            original
        );
    }
}

#[test]
fn newer_renewal_and_terminal_intent_never_allow_stale_renewal_to_change_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    let mut cfg = config();
    cfg.operations_per_sandbox = 5;
    journal
        .initialize_internal_inbox(cfg, &mut NeverCrash)
        .unwrap();
    let (create, _) = running(&mut journal);
    let old = renewal(&create, 2, 90_020);
    let original = journal
        .record_internal_intent(&old, &mut NeverCrash)
        .unwrap();
    let newer = renewal(&create, 3, 40_020);
    journal
        .record_internal_intent(&newer, &mut NeverCrash)
        .unwrap();
    assert_eq!(
        journal
            .apply_internal_renew(&old, 100, &mut NeverCrash)
            .unwrap(),
        original
    );
    journal
        .apply_internal_renew(&newer, 100, &mut NeverCrash)
        .unwrap();
    assert_eq!(deadline(&journal), 40_020);
    let stop = LifecycleRequest {
        intent: Intent::Destroy,
        identity: request(4, 4, Intent::Destroy).identity,
        ..create
    };
    journal
        .record_internal_intent(&stop, &mut NeverCrash)
        .unwrap();
    assert_eq!(
        journal
            .apply_internal_renew(&old, 101, &mut NeverCrash)
            .unwrap(),
        original
    );
    assert_eq!(deadline(&journal), 40_020);
}

#[test]
fn successive_confirmed_renewals_preserve_history_and_recover_current_exact_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    let mut cfg = config();
    cfg.operations_per_sandbox = 5;
    journal
        .initialize_internal_inbox(cfg, &mut NeverCrash)
        .unwrap();
    let (create, _) = running(&mut journal);
    let first = renewal(&create, 2, 90_020);
    journal
        .record_internal_intent(&first, &mut NeverCrash)
        .unwrap();
    let old_result = journal
        .apply_internal_renew(&first, 100, &mut NeverCrash)
        .unwrap();
    let second = renewal(&create, 3, 40_020);
    journal
        .record_internal_intent(&second, &mut NeverCrash)
        .unwrap();
    let current_result = journal
        .apply_internal_renew(&second, 200, &mut NeverCrash)
        .unwrap();
    drop(journal);
    let mut journal = open(&path);
    assert_eq!(deadline(&journal), 40_020);
    assert!(journal.internal_pending(&config().node).unwrap().is_empty());
    assert_eq!(
        journal
            .apply_internal_renew(&first, 300, &mut NeverCrash)
            .unwrap(),
        old_result
    );
    assert_eq!(
        journal
            .apply_internal_renew(&second, 300, &mut NeverCrash)
            .unwrap(),
        current_result
    );
    assert_eq!(deadline(&journal), 40_020);
}

#[test]
fn authenticated_global_private_owner_is_projected_not_compared_to_local_allocation_order() {
    for (tenant, private_owner) in [("2", 2), ("1", 2), ("01", 1)] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = open(&dir.path().join("db"));
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let mut create = local_request();
        create.identity.tenant_id = tenant.try_into().unwrap();
        let mut admission = local_admission();
        admission.template.local.owner = Some(private_owner);
        admission.template.registered = admission.template.local.clone();
        admission.template.digest = admission.template.registered.canonical_digest().unwrap();
        create.create.template_digest = admission.template.digest;
        journal
            .record_internal_intent(&create, &mut NeverCrash)
            .unwrap();
        let result = journal.admit_internal_create(&create, admission, &mut NeverCrash);
        if tenant == "2" {
            let local = result.unwrap().local.unwrap();
            assert_eq!(local.owner, core::TenantId(1));
            let options = journal.create_options(local.sandbox).unwrap();
            assert_eq!(options.template.unwrap().owner, Some(1));
        } else {
            assert!(matches!(result, Err(AdmissionError::Unauthorized)));
            assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
            assert!(journal.world().unwrap().sandbox_records().is_empty());
        }
    }
}

#[test]
fn confirmed_renewals_compact_both_identity_history_and_recover_native_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    let mut cfg = config();
    cfg.operations_per_sandbox = 4;
    journal
        .initialize_internal_inbox(cfg, &mut NeverCrash)
        .unwrap();
    let (create, local) = running(&mut journal);
    let mut old = None;
    for version in 2..25 {
        let renew = renewal(&create, version, 40_020);
        journal
            .record_internal_intent(&renew, &mut NeverCrash)
            .unwrap();
        let result = journal
            .apply_internal_renew(&renew, 100, &mut NeverCrash)
            .unwrap();
        journal
            .confirm_internal_result(&result, &mut NeverCrash)
            .unwrap();
        let inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
        assert_eq!(inbox.entries.len(), 2);
        assert_eq!(
            journal.internal_work_state(&renew).unwrap().local,
            Some(local)
        );
        if let Some(prior) = &old {
            assert!(matches!(
                journal.record_internal_intent(prior, &mut NeverCrash),
                Err(JournalError::Conflict)
            ));
        }
        old = Some(renew);
    }
    drop(journal);
    let journal = open(&path);
    assert_eq!(deadline(&journal), 40_020);
    assert_eq!(
        journal.internal_work_state(&create).unwrap().local,
        Some(local)
    );
    assert!(journal.retirement_candidates().unwrap().is_empty());
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
}

#[test]
fn result_confirmation_crashes_and_late_ack_never_drop_a_newer_pending_intent() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        let mut cfg = config();
        cfg.operations_per_sandbox = 6;
        journal
            .initialize_internal_inbox(cfg, &mut NeverCrash)
            .unwrap();
        let (create, _) = running(&mut journal);
        let first = renewal(&create, 2, 90_020);
        journal
            .record_internal_intent(&first, &mut NeverCrash)
            .unwrap();
        journal
            .apply_internal_renew(&first, 100, &mut NeverCrash)
            .unwrap();
        let second = renewal(&create, 3, 40_020);
        journal
            .record_internal_intent(&second, &mut NeverCrash)
            .unwrap();
        let result = journal
            .apply_internal_renew(&second, 100, &mut NeverCrash)
            .unwrap();
        let newer = renewal(&create, 4, 50_020);
        journal
            .record_internal_intent(&newer, &mut NeverCrash)
            .unwrap();
        let mut wrong = result.clone();
        wrong.observation_version = wrong.observation_version.checked_next().unwrap();
        assert!(matches!(
            journal.confirm_internal_result(&wrong, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
        assert!(matches!(
            journal.confirm_internal_result(&result, &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(journal);
        let mut journal = open(&path);
        assert_eq!(
            journal.internal_pending(&config().node).unwrap(),
            vec![newer.clone()]
        );
        let expected = if point == CrashPoint::BeforeCommit {
            4
        } else {
            3
        };
        assert_eq!(
            codec::load(&journal.journal.connection)
                .unwrap()
                .unwrap()
                .entries
                .len(),
            expected
        );
        assert_eq!(
            journal
                .confirm_internal_result(&result, &mut NeverCrash)
                .unwrap(),
            result
        );
        journal
            .apply_internal_renew(&newer, 100, &mut NeverCrash)
            .unwrap();
        assert_eq!(deadline(&journal), 50_020);
    }
}
