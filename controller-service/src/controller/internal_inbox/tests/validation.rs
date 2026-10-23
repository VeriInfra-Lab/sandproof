//! Original journal evidence, not a synthetic certificate or guest success flag.
use super::*;
use crate::template_registry::{Publication, PublicationState, evidence};

fn inputs() -> (LifecycleRequest, InternalAdmission, Publication) {
    let mut admission = local_admission();
    let mut binding = admission.template.local.clone();
    let candidate_id = "a".repeat(32);
    binding.template_id = format!("tpl-{candidate_id}");
    binding.owner = Some(1);
    binding.kernel.path = "/center-metadata/kernel".into();
    binding.rootfs.path = "/center-metadata/rootfs".into();
    let value = Publication {
        candidate_id,
        binding,
        image_digest: format!("sha256:{}", "c".repeat(64)),
        free_bytes: 2048,
        min_free_bytes: 1024,
        state: PublicationState::Checking,
        checked: false,
        validation_sandbox: None,
        validation_certificate: None,
        failure: None,
    };
    admission
        .template
        .local
        .template_id
        .clone_from(&value.binding.template_id);
    admission.template.local.owner = Some(1);
    admission.template.registered = value.binding.clone();
    admission.template.digest = value.binding.canonical_digest().unwrap();
    let mut create = local_request();
    create.identity.tenant_id = "1".try_into().unwrap();
    create.create.template_digest = admission.template.digest;
    (create, admission, value)
}

fn finish(
    journal: &mut ControllerJournal,
    create: &LifecycleRequest,
    admission: InternalAdmission,
    activate: bool,
) -> (InternalLocalBinding, LifecycleRequest) {
    journal
        .record_internal_intent(create, &mut NeverCrash)
        .unwrap();
    let local = journal
        .admit_internal_create(create, admission, &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    if activate {
        crate::finish_admitted_create_timed(
            journal,
            local.sandbox,
            local.operation,
            &mut workflow_fixture::worker,
            &mut || Ok(20),
        )
        .unwrap();
        journal
            .capture_internal_progress(create, &mut NeverCrash)
            .unwrap();
    }
    let mut destroy = create.clone();
    destroy.intent = Intent::Destroy;
    destroy.identity.operation_id = format!(
        "destroy-validation-{}-{}",
        create.identity.retention_slot,
        create.identity.request_window.get()
    )
    .try_into()
    .unwrap();
    destroy.identity.intent_version = 2.try_into().unwrap();
    journal
        .record_internal_intent(&destroy, &mut NeverCrash)
        .unwrap();
    if activate {
        crate::finish_accepted_kill(
            journal,
            local.sandbox,
            core::OperationId(2),
            &mut workflow_fixture::worker,
        )
        .unwrap();
    } else {
        crate::finish_compensating_create(
            journal,
            local.sandbox,
            local.operation,
            &mut workflow_fixture::worker,
        )
        .unwrap();
    }
    assert_eq!(
        journal
            .capture_internal_progress(&destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
    (local, destroy)
}

fn release(
    journal: &mut ControllerJournal,
    local: InternalLocalBinding,
    destroy: &LifecycleRequest,
) {
    let context = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    assert_eq!(
        journal
            .capture_internal_progress(destroy, &mut NeverCrash)
            .unwrap()
            .fact,
        LifecycleFact::Cleaned
    );
}

#[test]
fn validation_original_success_and_no_activation_failure_require_exact_release() {
    for activate in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (create, admission, value) = inputs();
        let (local, destroy) = finish(&mut journal, &create, admission, activate);
        assert!(
            journal
                .internal_validation_evidence(
                    &create.identity,
                    &destroy.identity,
                    &create.create.template_revision,
                    &value
                )
                .is_err()
        );
        release(&mut journal, local, &destroy);
        let mut native = value.clone();
        native.binding = journal
            .create_options(local.sandbox)
            .unwrap()
            .template
            .unwrap();
        native.checked = activate;
        native.state = if activate {
            PublicationState::Ready
        } else {
            PublicationState::Failed
        };
        native.failure =
            (!activate).then_some(crate::template_registry::PublicationFailure::Interrupted);
        let original_certificate =
            evidence::make_certificate(&journal.world().unwrap(), &native, local.sandbox).unwrap();
        native.validation_certificate = Some(original_certificate);
        evidence::validate_publication(&native).unwrap();
        let before = journal.internal_audit().unwrap();
        let changes = journal.journal.connection.total_changes();
        let result = journal
            .internal_validation_evidence(
                &create.identity,
                &destroy.identity,
                &create.create.template_revision,
                &value,
            )
            .unwrap_or_else(|error| panic!("activation={activate}: {error:?}"));
        let (mapping, certificate) = result.admitted().unwrap();
        assert_eq!(mapping.sandbox, local.sandbox);
        assert_eq!(mapping.create_operation, local.operation);
        assert_eq!(certificate.activation.is_some(), activate);
        assert_eq!(
            certificate.outcome,
            if activate {
                PublicationState::Ready
            } else {
                PublicationState::Failed
            }
        );
        assert_eq!(
            mapping.descriptor.kernel.path,
            std::path::Path::new("/protected-cache/kernel")
        );
        assert_eq!(certificate.binding, mapping.descriptor);
        result
            .validate_for(&create.identity, &destroy.identity, &value)
            .unwrap();
        assert_eq!(journal.internal_audit().unwrap(), before);
        assert_eq!(journal.journal.connection.total_changes(), changes);
        drop(journal);
        let journal = open(&path);
        assert_eq!(
            journal
                .internal_validation_evidence(
                    &create.identity,
                    &destroy.identity,
                    &create.create.template_revision,
                    &value
                )
                .unwrap(),
            result
        );
    }
}

#[test]
fn validation_identity_descriptor_and_unknown_mapping_never_issue_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (create, admission, value) = inputs();
    journal
        .record_internal_intent(&create, &mut NeverCrash)
        .unwrap();
    let mut empty_destroy = create.clone();
    empty_destroy.intent = Intent::Destroy;
    empty_destroy.identity.operation_id = "empty-destroy".try_into().unwrap();
    empty_destroy.identity.intent_version = 2.try_into().unwrap();
    journal
        .record_internal_intent(&empty_destroy, &mut NeverCrash)
        .unwrap();
    journal
        .capture_internal_progress(&empty_destroy, &mut NeverCrash)
        .unwrap();
    let not_admitted = journal
        .internal_validation_evidence(
            &create.identity,
            &empty_destroy.identity,
            &create.create.template_revision,
            &value,
        )
        .unwrap();
    assert_eq!(
        not_admitted.outcome,
        crate::template_registry::ValidationOutcome::NotAdmitted
    );
    assert!(not_admitted.admitted().is_none());
    let mut create = create;
    create.identity.sandbox_id = "validation-second".try_into().unwrap();
    create.identity.retention_slot = 2;
    create.identity.operation_id = "validation-create-second".try_into().unwrap();
    let (local, destroy) = finish(&mut journal, &create, admission, true);
    release(&mut journal, local, &destroy);
    let before = journal.internal_audit().unwrap();
    for index in 0..7 {
        let mut wrong = create.identity.clone();
        match index {
            0 => wrong.tenant_id = "2".try_into().unwrap(),
            1 => wrong.node.node_generation = 2.try_into().unwrap(),
            2 => wrong.node.node_id = "wrong".try_into().unwrap(),
            3 => wrong.sandbox_id = "wrong".try_into().unwrap(),
            4 => wrong.operation_id = "wrong".try_into().unwrap(),
            5 => wrong.intent_version = 2.try_into().unwrap(),
            _ => wrong.request_window = 2.try_into().unwrap(),
        }
        assert!(
            journal
                .internal_validation_evidence(
                    &wrong,
                    &destroy.identity,
                    &create.create.template_revision,
                    &value
                )
                .is_err()
        );
    }
    for index in 0..4 {
        let mut wrong = value.clone();
        match index {
            0 => wrong.binding.owner = Some(2),
            1 => wrong.binding.kernel.path = "/untrusted-do-not-open/remote".into(),
            2 => wrong.binding.rootfs.sha256 = "d".repeat(64),
            _ => wrong.checked = true,
        }
        assert!(
            journal
                .internal_validation_evidence(
                    &create.identity,
                    &destroy.identity,
                    &create.create.template_revision,
                    &wrong
                )
                .is_err()
        );
    }
    assert!(
        journal
            .internal_validation_evidence(
                &create.identity,
                &destroy.identity,
                &"wrong-revision".try_into().unwrap(),
                &value
            )
            .is_err()
    );
    assert_eq!(journal.internal_audit().unwrap(), before);
}

#[test]
fn validation_after_window_retirement_uses_original_native_owner_not_global_number() {
    use sandproof_protocol::internal::window::WindowCloseRequest;
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (first, admission, value) = inputs();
    let (local, destroy) = finish(&mut journal, &first, admission.clone(), true);
    release(&mut journal, local, &destroy);
    let cleaned = journal.internal_work_state(&destroy).unwrap().response;
    journal
        .confirm_internal_result(&cleaned, &mut NeverCrash)
        .unwrap();
    journal
        .close_internal_window(&WindowCloseRequest::new(cleaned.clone()), &mut NeverCrash)
        .unwrap();
    assert!(
        journal
            .internal_validation_evidence(
                &first.identity,
                &destroy.identity,
                &first.create.template_revision,
                &value
            )
            .is_err()
    );
    let mut next = first;
    next.identity.request_window = 2.try_into().unwrap();
    next.identity.sandbox_id = "global-has-no-native-suffix".try_into().unwrap();
    next.identity.operation_id = "second-window-create".try_into().unwrap();
    let (local, destroy) = finish(&mut journal, &next, admission, true);
    assert_eq!(local.owner, core::TenantId(2));
    release(&mut journal, local, &destroy);
    let result = journal
        .internal_validation_evidence(
            &next.identity,
            &destroy.identity,
            &next.create.template_revision,
            &value,
        )
        .unwrap();
    assert_eq!(next.identity.tenant_id.as_str(), "1");
    let (mapping, certificate) = result.admitted().unwrap();
    assert_eq!(mapping.owner, 2);
    assert_eq!(mapping.descriptor.owner, Some(2));
    assert_eq!(certificate.owner, 2);
    assert_eq!(certificate.binding, mapping.descriptor);
}

#[test]
fn validation_encoding_exact_limit_succeeds_and_overflow_fails_before_full_buffer() {
    let value = vec!["finite"; 100];
    let ordinary = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        evidence::encode_bounded(&value, ordinary.len()).unwrap(),
        ordinary
    );
    assert!(matches!(
        evidence::encode_bounded(&value, ordinary.len() - 1),
        Err(JournalError::MaintenanceCapacity)
    ));
}

#[test]
fn private_template_across_windows_rejects_foreign_or_changed_global_binding_without_allocation() {
    use sandproof_protocol::internal::window::WindowCloseRequest;
    let dir = tempfile::tempdir().unwrap();
    let mut journal = open(&dir.path().join("db"));
    journal
        .initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let (first, admission, _) = inputs();
    let (local, destroy) = finish(&mut journal, &first, admission.clone(), true);
    release(&mut journal, local, &destroy);
    let cleaned = journal.internal_work_state(&destroy).unwrap().response;
    journal
        .confirm_internal_result(&cleaned, &mut NeverCrash)
        .unwrap();
    journal
        .close_internal_window(&WindowCloseRequest::new(cleaned.clone()), &mut NeverCrash)
        .unwrap();
    let mut next = first;
    next.identity.request_window = 2.try_into().unwrap();
    next.identity.sandbox_id = "next-private-sandbox".try_into().unwrap();
    next.identity.operation_id = "next-private-create".try_into().unwrap();
    journal
        .record_internal_intent(&next, &mut NeverCrash)
        .unwrap();
    let before = journal.internal_audit().unwrap();
    let allocator = journal.next_sandbox_id().unwrap();
    for index in 0..4 {
        let mut changed = admission.clone();
        match index {
            0 => changed.template.local.owner = Some(2),
            1 => changed.template.local.revision = "r2".into(),
            2 => changed.template.local.kernel.sha256 = "d".repeat(64),
            _ => changed.template.registered.rootfs.path = "/changed-center-rootfs".into(),
        }
        assert!(
            journal
                .admit_internal_create(&next, changed, &mut NeverCrash)
                .is_err()
        );
        assert_eq!(journal.internal_audit().unwrap(), before);
        assert_eq!(journal.next_sandbox_id().unwrap(), allocator);
    }
    let local = journal
        .admit_internal_create(&next, admission, &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    assert_eq!(local.owner, core::TenantId(2));
}

#[test]
fn private_template_global_binding_and_native_projection_commit_together_across_windows() {
    use sandproof_protocol::internal::window::WindowCloseRequest;
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        journal
            .initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let (first, admission, _) = inputs();
        let (local, destroy) = finish(&mut journal, &first, admission.clone(), true);
        release(&mut journal, local, &destroy);
        let cleaned = journal.internal_work_state(&destroy).unwrap().response;
        journal
            .confirm_internal_result(&cleaned, &mut NeverCrash)
            .unwrap();
        journal
            .close_internal_window(&WindowCloseRequest::new(cleaned.clone()), &mut NeverCrash)
            .unwrap();
        let mut next = first;
        next.identity.request_window = 2.try_into().unwrap();
        next.identity.sandbox_id = "second-window-sandbox".try_into().unwrap();
        next.identity.operation_id = "second-window-create".try_into().unwrap();
        journal
            .record_internal_intent(&next, &mut NeverCrash)
            .unwrap();
        assert!(matches!(
            journal.admit_internal_create(&next, admission.clone(), &mut Stop(point)),
            Err(AdmissionError::Journal(JournalError::InjectedCrash(_)))
        ));
        drop(journal);
        let mut journal = open(&path);
        let attached = journal.internal_work_state(&next).unwrap().local;
        assert_eq!(attached.is_some(), point == CrashPoint::AfterCommit);
        let immutable: Vec<u8> = journal
            .journal
            .connection
            .query_row(
                "SELECT binding_json FROM admitted_templates WHERE template_id=?1",
                [&admission.template.local.template_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<TemplateBinding>(&immutable).unwrap(),
            admission.template.registered
        );
        let retry = journal
            .admit_internal_create(&next, admission, &mut NeverCrash)
            .unwrap();
        assert_eq!(retry.local.is_some(), point == CrashPoint::BeforeCommit);
        let local = journal.internal_work_state(&next).unwrap().local.unwrap();
        assert_eq!(local.owner, core::TenantId(2));
        assert_eq!(
            journal
                .create_options(local.sandbox)
                .unwrap()
                .template
                .unwrap()
                .owner,
            Some(2)
        );
        assert_eq!(journal.world().unwrap().sandbox_records().len(), 2);
    }
}
