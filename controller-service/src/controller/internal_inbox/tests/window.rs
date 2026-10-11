//! Original `SQLite` journal integration, with controlled physical acknowledgements.
//! No KVM/network or production window retirement is enabled by these tests.
use super::*;
use sandproof_protocol::internal::{
    PositiveCounter,
    lifecycle::RESULT_CONFIRMATION_BODY_BYTES,
    window::{WINDOW_CLOSE_BODY_BYTES, WindowCloseAck, WindowCloseRequest},
};

fn close(window: u64) -> WindowCloseRequest {
    WindowCloseRequest::new(config().node, window.try_into().unwrap())
}

fn terminal_without_native(
    journal: &mut ControllerJournal,
    window: u64,
) -> (LifecycleRequest, LifecycleResponse) {
    let mut destroy = request(window, 2, Intent::Destroy);
    destroy.identity.request_window = window.try_into().unwrap();
    destroy.identity.sandbox_id = format!("sandbox-{window}").try_into().unwrap();
    journal
        .record_internal_intent(&destroy, &mut NeverCrash)
        .unwrap();
    let result = journal
        .capture_internal_progress(&destroy, &mut NeverCrash)
        .unwrap();
    assert_eq!(result.fact, LifecycleFact::Cleaned);
    (destroy, result)
}

fn running_native(journal: &mut ControllerJournal) -> (LifecycleRequest, InternalLocalBinding) {
    running_native_with_admission(journal, local_admission())
}

fn running_native_with_admission(
    journal: &mut ControllerJournal,
    admission: InternalAdmission,
) -> (LifecycleRequest, InternalLocalBinding) {
    let mut create = local_request();
    create.create.template_digest = admission.template.digest;
    let (_, current) = journal.internal_window_frontier(&config().node).unwrap();
    let current = current.unwrap().get();
    create.identity.request_window = current.try_into().unwrap();
    create.identity.sandbox_id = format!("sandbox-{current}").try_into().unwrap();
    create.identity.operation_id = format!("create-{current}").try_into().unwrap();
    journal
        .record_internal_intent(&create, &mut NeverCrash)
        .unwrap();
    let local = journal
        .admit_internal_create(&create, admission, &mut NeverCrash)
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

fn stop_native(
    journal: &mut ControllerJournal,
    create: &LifecycleRequest,
    local: InternalLocalBinding,
) -> LifecycleRequest {
    let mut destroy = create.clone();
    destroy.intent = Intent::Destroy;
    destroy.identity.intent_version = 2.try_into().unwrap();
    destroy.identity.operation_id = format!("destroy-{}", create.identity.request_window.get())
        .try_into()
        .unwrap();
    journal
        .record_internal_intent(&destroy, &mut NeverCrash)
        .unwrap();
    crate::finish_accepted_kill_v1(
        journal,
        local.sandbox,
        core::OperationId(2),
        &mut workflow_fixture::worker,
    )
    .unwrap();
    assert_eq!(
        journal
            .capture_internal_progress(&destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
    destroy
}

fn clean_native(
    journal: &mut ControllerJournal,
    destroy: &LifecycleRequest,
    local: InternalLocalBinding,
) -> LifecycleResponse {
    let context = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    let result = journal
        .capture_internal_progress(destroy, &mut NeverCrash)
        .unwrap();
    assert_eq!(result.fact, LifecycleFact::Cleaned);
    result
}

#[test]
fn capacity_reserves_each_future_exact_confirmation_and_the_window_frontier() {
    let cfg = config();
    let slots = usize::from(cfg.max_sandboxes) * usize::from(cfg.operations_per_sandbox);
    let original = slots * (cfg.max_options_bytes as usize * 4 + 8192) + 8192;
    assert_eq!(
        cfg.required_bytes(),
        original
            + slots * (RESULT_CONFIRMATION_BODY_BYTES + b",\"confirmed\":".len())
            + WINDOW_CLOSE_BODY_BYTES
    );
}

#[test]
fn unknown_running_stopped_and_unconfirmed_windows_keep_all_history_and_native_pins() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let mut create = local_request();
    create.identity.operation_id = "create-1".try_into().unwrap();
    journal
        .record_internal_intent(&create, &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let (create, local) = running_native(&mut journal);
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let destroy = stop_native(&mut journal, &create, local);
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let context = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    assert!(
        journal
            .release_host_slot(
                context,
                |_| Err(crate::WorkerTransportError::ReplyLost),
                &mut NeverCrash
            )
            .is_err()
    );
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    let cleaned = clean_native(&mut journal, &destroy, local);
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert!(journal.retirement_candidates().unwrap().is_empty());
    assert_eq!(
        journal.internal_window_frontier(&config().node).unwrap().0,
        0
    );
    journal
        .confirm_internal_result(&cleaned, &mut NeverCrash)
        .unwrap();
    journal
        .close_internal_window(&close(1), &mut NeverCrash)
        .unwrap();
    assert!(journal.internal_pending(&config().node).unwrap().is_empty());
    // Window closure removes only inbox references. Original native cleanup,
    // release witness and retirement remain separately inspectable until swept.
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(local.sandbox)
            .is_some()
    );
    assert_eq!(
        journal.retirement_candidates().unwrap()[0].sandbox,
        local.sandbox
    );
}

#[test]
fn durable_confirmations_and_close_crashes_reopen_the_exact_same_window_identity() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (destroy, result) = terminal_without_native(&mut journal, 1);
        let mut wrong = result.clone();
        wrong.observation_version = wrong.observation_version.checked_next().unwrap();
        assert!(matches!(
            journal.confirm_internal_result(&wrong, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
        journal
            .confirm_internal_result(&result, &mut NeverCrash)
            .unwrap();
        assert!(matches!(
            journal.close_internal_window(&close(1), &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(journal);
        let mut journal = open(&path);
        let committed = point == CrashPoint::AfterCommit;
        let inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
        assert_eq!(inbox.retired_through, u64::from(committed));
        assert_eq!(inbox.entries.is_empty(), committed);
        if !committed {
            assert_eq!(inbox.entries[0].confirmed.as_ref(), Some(&result));
        }
        let answer = journal
            .close_internal_window(&close(1), &mut NeverCrash)
            .unwrap();
        assert_eq!(
            answer,
            WindowCloseAck {
                identity: close(1).identity
            }
        );
        assert!(matches!(
            journal.record_internal_intent(&destroy, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
        assert!(matches!(
            journal.confirm_internal_result(&result, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
        assert_eq!(
            journal.internal_window_frontier(&config().node).unwrap(),
            (1, Some(2.try_into().unwrap()))
        );
    }
}

#[test]
fn unknown_close_commit_recovers_both_outcomes_without_a_fresh_request_or_fake_ack() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (_, result) = terminal_without_native(&mut journal, 1);
        journal
            .confirm_internal_result(&result, &mut NeverCrash)
            .unwrap();
        super::super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.close_internal_window(&close(1), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = open(&path);
        assert_eq!(
            journal.internal_window_frontier(&config().node).unwrap().0,
            u64::from(durable)
        );
        assert_eq!(
            journal
                .close_internal_window(&close(1), &mut NeverCrash)
                .unwrap()
                .identity,
            close(1).identity
        );
        assert_eq!(
            journal.internal_window_frontier(&config().node).unwrap().0,
            1
        );
    }
}

#[test]
fn bounded_long_churn_preserves_observation_frontier_rejects_old_and_future_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let mut prior_observation = 1;
    let mut old = vec![];
    for window in 1..=32 {
        let (destroy, result) = terminal_without_native(&mut journal, window);
        assert!(result.observation_version.get() > prior_observation);
        prior_observation = result.observation_version.get();
        journal
            .confirm_internal_result(&result, &mut NeverCrash)
            .unwrap();
        journal
            .close_internal_window(&close(window), &mut NeverCrash)
            .unwrap();
        let inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
        assert!(inbox.entries.is_empty());
        assert!(inbox.tenants.is_empty());
        assert_eq!(inbox.observation.get(), prior_observation);
        old.push(destroy);
        for stale in &old {
            assert!(matches!(
                journal.record_internal_intent(stale, &mut NeverCrash),
                Err(JournalError::Conflict)
            ));
            assert_eq!(
                journal
                    .close_internal_window(
                        &close(stale.identity.request_window.get()),
                        &mut NeverCrash
                    )
                    .unwrap()
                    .identity,
                close(stale.identity.request_window.get()).identity
            );
        }
        let mut future = request(100, 1, Intent::Create);
        future.identity.request_window = (window + 2).try_into().unwrap();
        assert!(matches!(
            journal.record_internal_intent(&future, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
        drop(journal);
        journal = open(&path);
        assert_eq!(
            journal.internal_window_frontier(&config().node).unwrap(),
            (window, Some((window + 1).try_into().unwrap()))
        );
    }
    let mut successor = request(100, 1, Intent::Create);
    successor.identity.request_window = 33.try_into().unwrap();
    journal
        .record_internal_intent(&successor, &mut NeverCrash)
        .unwrap();
    let original = codec::load(&journal.journal.connection).unwrap().unwrap();
    journal
        .close_internal_window(&close(32), &mut NeverCrash)
        .unwrap();
    assert_eq!(
        codec::load(&journal.journal.connection).unwrap().unwrap(),
        original
    );
}

#[test]
fn tenant_mapping_and_native_ids_do_not_recur_when_closed_histories_disappear() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    for window in 1..=3 {
        let (create, local) = running_native(&mut journal);
        assert_eq!(local.owner, core::TenantId(window));
        assert_eq!(local.sandbox, core::SandboxId(window));
        let destroy = stop_native(&mut journal, &create, local);
        let result = clean_native(&mut journal, &destroy, local);
        journal
            .confirm_internal_result(&result, &mut NeverCrash)
            .unwrap();
        journal
            .close_internal_window(&close(window), &mut NeverCrash)
            .unwrap();
        let inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
        assert_eq!(inbox.tenant_highwater, window);
        assert!(inbox.tenants.is_empty());
        assert_eq!(
            journal.next_sandbox_id().unwrap(),
            core::SandboxId(window + 1)
        );
        drop(journal);
        journal = open(&path);
        assert_eq!(
            codec::load(&journal.journal.connection)
                .unwrap()
                .unwrap()
                .tenant_highwater,
            window
        );
    }
    // Original sweep still performs native deletion, rather than close itself.
    for candidate in journal.retirement_candidates().unwrap() {
        journal
            .retire_reclaimed(candidate.sandbox, 700_000, |_| Ok(()), &mut NeverCrash)
            .unwrap();
    }
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(4));
    assert_eq!(
        journal.internal_window_frontier(&config().node).unwrap().0,
        3
    );
}

#[test]
fn maximum_window_retires_to_exhausted_without_counter_wrap_or_reset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let mut inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
    inbox.retired_through = u64::MAX - 1;
    codec::save(&journal.journal.connection, &inbox).unwrap();
    let (destroy, result) = terminal_without_native(&mut journal, u64::MAX);
    journal
        .confirm_internal_result(&result, &mut NeverCrash)
        .unwrap();
    journal
        .close_internal_window(&close(u64::MAX), &mut NeverCrash)
        .unwrap();
    drop(journal);
    let mut journal = open(&path);
    assert_eq!(
        journal.internal_window_frontier(&config().node).unwrap(),
        (u64::MAX, None)
    );
    let audit = journal.internal_audit().unwrap();
    assert_eq!(audit.retired_through, u64::MAX);
    assert_eq!(audit.current_window, None);
    assert!(audit.operations.is_empty());
    journal
        .close_internal_window(&close(u64::MAX), &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        journal.record_internal_intent(&destroy, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert!(matches!(
        journal.record_internal_intent(&request(1, 1, Intent::Create), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
}

#[test]
fn foreign_window_or_external_store_commit_never_acknowledges_or_changes_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (_, result) = terminal_without_native(&mut journal, 1);
    journal
        .confirm_internal_result(&result, &mut NeverCrash)
        .unwrap();
    for field in 0..5 {
        let mut altered = close(1);
        match field {
            0 => altered.identity.protocol_version = 2,
            1 => altered.identity.node.deployment_id = "elsewhere".try_into().unwrap(),
            2 => altered.identity.node.node_id = "elsewhere".try_into().unwrap(),
            3 => altered.identity.node.node_generation = 2.try_into().unwrap(),
            _ => altered.identity.request_window = 2.try_into().unwrap(),
        }
        assert!(matches!(
            journal.close_internal_window(&altered, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
    }
    let external = rusqlite::Connection::open(&path).unwrap();
    external
        .execute("CREATE TABLE foreign_writer(id INTEGER)", [])
        .unwrap();
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::CorruptStore)
    ));
    let original = codec::load(&journal.journal.connection).unwrap().unwrap();
    assert_eq!(original.retired_through, 0);
    assert_eq!(original.entries[0].confirmed.as_ref(), Some(&result));
}

#[test]
fn exact_saved_confirmations_and_current_window_are_required_during_restore() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (_, result) = terminal_without_native(&mut journal, 1);
    journal
        .confirm_internal_result(&result, &mut NeverCrash)
        .unwrap();
    let original = codec::load(&journal.journal.connection).unwrap().unwrap();
    let mut wrong = original.clone();
    wrong.retired_through = 1;
    assert!(matches!(wrong.validate(), Err(JournalError::CorruptStore)));
    let mut wrong = original.clone();
    wrong.entries[0]
        .confirmed
        .as_mut()
        .unwrap()
        .observation_version = 100.try_into().unwrap();
    assert!(matches!(wrong.validate(), Err(JournalError::CorruptStore)));
    let mut wrong = original.clone();
    wrong.entries[0].response.fact = LifecycleFact::Recorded;
    wrong.entries[0].confirmed = Some(wrong.entries[0].response.clone());
    assert!(matches!(wrong.validate(), Err(JournalError::CorruptStore)));
    let mut no_fields = serde_json::to_value(&original).unwrap();
    no_fields.as_object_mut().unwrap().remove("retired_through");
    assert!(serde_json::from_value::<model::Inbox>(no_fields).is_err());
    let mut no_fields = serde_json::to_value(&original).unwrap();
    no_fields["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("confirmed");
    // Option serde missing fields are None, never an implicit confirmation.
    let unconfirmed: model::Inbox = serde_json::from_value(no_fields).unwrap();
    assert!(unconfirmed.entries[0].confirmed.is_none());
    assert_eq!(
        original.current_window(),
        Some(PositiveCounter::try_from(1).unwrap())
    );
}

#[test]
fn active_native_registry_pin_vetoes_close_until_original_certificate_settlement() {
    use crate::template_registry::{Publication, PublicationState, RegistryConfig};
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let cfg = RegistryConfig {
        root: dir.path().join("registry"),
        max_templates_global: 2,
        max_templates_per_tenant: 2,
        max_artifact_bytes_global: 100_000,
        max_artifact_bytes_per_tenant: 100_000,
        validation_timeout_secs: 60,
    };
    journal.enable_template_registry(&cfg).unwrap();
    let mut admission = local_admission();
    admission.template.local.owner = Some(1);
    admission.template.local.template_id = format!("tpl-{}", "a".repeat(32));
    admission.template.registered = admission.template.local.clone();
    admission.template.digest = admission.template.registered.canonical_digest().unwrap();
    let (create, local) = running_native_with_admission(&mut journal, admission);
    let destroy = stop_native(&mut journal, &create, local);
    let result = clean_native(&mut journal, &destroy, local);
    journal
        .confirm_internal_result(&result, &mut NeverCrash)
        .unwrap();
    let value = Publication {
        candidate_id: "a".repeat(32),
        binding: journal
            .create_options(local.sandbox)
            .unwrap()
            .template
            .unwrap(),
        image_digest: format!("sha256:{}", "a".repeat(64)),
        free_bytes: 500,
        min_free_bytes: 100,
        state: PublicationState::Checking,
        checked: false,
        validation_sandbox: None,
        validation_certificate: None,
        failure: None,
    };
    journal.begin_template_publication(&value, &cfg).unwrap();
    // The original trusted registry attachment, not raw SQL or a forged guest
    // certificate, makes this exact terminal native history a live publication pin.
    let tx = super::super::super::transaction::WorldTransaction::begin(
        &mut journal.journal.connection,
        &journal.committed_world,
    )
    .unwrap();
    super::super::super::registry::attach_validation(
        &tx,
        &value.candidate_id,
        1,
        local.sandbox,
        &value.binding,
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(journal.host_slot_release_pinned(local.sandbox).unwrap());
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(
        journal.internal_window_frontier(&config().node).unwrap().0,
        0
    );
    assert_eq!(
        journal
            .template_publication(1, &value.candidate_id)
            .unwrap()
            .unwrap()
            .validation_sandbox,
        Some(local.sandbox.0)
    );
    // A terminal VM cannot be retrospectively declared a successful live
    // validation. The original reconciler settles this interrupted validation
    // as Failed with an independent cleanup certificate, releasing its live pin.
    assert!(matches!(
        journal.mark_template_checked(1, &value.candidate_id),
        Err(JournalError::Conflict)
    ));
    journal.reconcile_template_publications(false).unwrap();
    assert!(!journal.host_slot_release_pinned(local.sandbox).unwrap());
    journal
        .close_internal_window(&close(1), &mut NeverCrash)
        .unwrap();
    let settled = journal
        .template_publication(1, &value.candidate_id)
        .unwrap()
        .unwrap();
    assert!(settled.validation_sandbox.is_none());
    assert!(settled.validation_certificate.is_some());
    assert_eq!(settled.state, PublicationState::Failed);
    assert!(
        journal
            .world()
            .unwrap()
            .sandbox_record(local.sandbox)
            .is_some()
    );
}

#[test]
fn terminal_confirmation_crashes_preserve_exact_stamp_or_refuse_window_close() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (_, result) = terminal_without_native(&mut journal, 1);
        assert!(matches!(
            journal.confirm_internal_result(&result, &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(journal);
        let mut journal = open(&path);
        let inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
        if point == CrashPoint::BeforeCommit {
            assert!(inbox.entries[0].confirmed.is_none());
            assert!(matches!(
                journal.close_internal_window(&close(1), &mut NeverCrash),
                Err(JournalError::Conflict)
            ));
        } else {
            assert_eq!(inbox.entries[0].confirmed.as_ref(), Some(&result));
            journal
                .close_internal_window(&close(1), &mut NeverCrash)
                .unwrap();
        }
    }
}

#[test]
fn native_physical_release_evidence_cannot_be_replaced_by_a_saved_cleaned_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (create, local) = running_native(&mut journal);
    let destroy = stop_native(&mut journal, &create, local);
    let result = clean_native(&mut journal, &destroy, local);
    journal
        .confirm_internal_result(&result, &mut NeverCrash)
        .unwrap();
    // Corrupt the same connection's durable proof, which does not change its
    // data_version. The close path must validate original evidence afresh.
    journal
        .journal
        .connection
        .execute("DELETE FROM controller_release_witnesses", [])
        .unwrap();
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::CorruptStore)
    ));
    let inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
    assert_eq!(inbox.retired_through, 0);
    assert_eq!(
        inbox.entries.last().unwrap().confirmed.as_ref(),
        Some(&result)
    );
}

#[test]
fn not_accepted_without_native_attachment_also_requires_exact_terminal_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let create = request(1, 1, Intent::Create);
    journal
        .record_internal_intent(&create, &mut NeverCrash)
        .unwrap();
    // Pure inbox state fixture only: production reception/capture NEVER accepts
    // a caller-supplied NotAccepted success. No native admission has occurred.
    let mut inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
    inbox.observation = inbox.observation.checked_next().unwrap();
    inbox.entries[0].response.observation_version = inbox.observation;
    inbox.entries[0].response.fact = LifecycleFact::NotAccepted;
    let result = inbox.entries[0].response.clone();
    codec::save(&journal.journal.connection, &inbox).unwrap();
    assert!(matches!(
        journal.close_internal_window(&close(1), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    journal
        .confirm_internal_result(&result, &mut NeverCrash)
        .unwrap();
    journal
        .close_internal_window(&close(1), &mut NeverCrash)
        .unwrap();
    assert!(journal.world().unwrap().sandbox_records().is_empty());
}
