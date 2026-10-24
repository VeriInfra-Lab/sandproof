//! The original six certificate/private-owner regressions on current intents.
use super::*;
use crate::controller::{lifecycle_inbox::tests as fixture, tests_support as workflow_fixture};
use crate::{CrashInjector, CrashPoint, InternalAdmission, NeverCrash, templates::TemplateBinding};
use sandproof_protocol::lifecycle::{LifecycleWindowClose, TerminalStatus};

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
    let request = fixture::create("1", "session-a");
    AuthenticatedCenterContext::from_verified_peer_registration(
        request.center,
        request.request.binding.node,
    )
}
fn inputs() -> (AcceptedLifecycleIntent, InternalAdmission, Publication) {
    let mut admission = fixture::admission();
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
    admission.template.local.kernel.path = "/protected-cache/kernel".into();
    admission.template.local.rootfs.path = "/protected-cache/rootfs".into();
    admission.template.registered = value.binding.clone();
    admission.template.digest = value.binding.canonical_digest().unwrap();
    let mut create = fixture::create("1", "session-a");
    let LifecycleOperation::Create {
        create: parameters, ..
    } = &mut create.request.operation
    else {
        panic!()
    };
    parameters.template_digest = admission.template.digest;
    parameters.options = serde_json::to_vec(&crate::CreateOptions {
        networking: Some(crate::CreateNetworking {
            allow_internet_access: Some(false),
            network: None,
        }),
        ..crate::CreateOptions::default()
    })
    .unwrap();
    (create, admission, value)
}
fn revision(create: &AcceptedLifecycleIntent) -> &ResourceId {
    let LifecycleOperation::Create {
        create: parameters, ..
    } = &create.request.operation
    else {
        panic!()
    };
    &parameters.template_revision
}
fn terminal(create: &AcceptedLifecycleIntent) -> TerminalLifecycleObservation {
    let mut request = fixture::destroy(create);
    request.request.binding.key.sequence = create
        .request
        .binding
        .key
        .sequence
        .get()
        .checked_add(1)
        .unwrap()
        .try_into()
        .unwrap();
    TerminalLifecycleObservation {
        request,
        status: TerminalStatus::Cleaned,
    }
}
fn finish(
    journal: &mut ControllerJournal,
    create: &AcceptedLifecycleIntent,
    admission: InternalAdmission,
    activate: bool,
) -> (InternalLocalBinding, TerminalLifecycleObservation) {
    journal
        .record_lifecycle_intent(create, &context(), &mut NeverCrash)
        .unwrap();
    let local = journal
        .admit_lifecycle_create(create, &context(), admission, &mut NeverCrash)
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
    }
    let terminal = terminal(create);
    journal
        .record_lifecycle_intent(&terminal.request, &context(), &mut NeverCrash)
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
    assert!(
        !journal
            .lifecycle_destroy_cleaned(&terminal.request, &context())
            .unwrap()
    );
    (local, terminal)
}
fn release(
    journal: &mut ControllerJournal,
    local: InternalLocalBinding,
    terminal: &TerminalLifecycleObservation,
) {
    let position = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    journal
        .release_host_slot(position, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    assert!(
        journal
            .lifecycle_destroy_cleaned(&terminal.request, &context())
            .unwrap()
    );
}
fn retire(journal: &mut ControllerJournal, terminal: &TerminalLifecycleObservation) {
    journal
        .confirm_lifecycle_terminal(terminal, &context(), &mut NeverCrash)
        .unwrap();
    journal
        .close_lifecycle_window(
            &LifecycleWindowClose {
                terminal: terminal.clone(),
            },
            &context(),
            &mut NeverCrash,
        )
        .unwrap();
}
fn successor(first: &AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    let mut next = first.clone();
    next.retention.window = 2.try_into().unwrap();
    next.request.binding.session = "next-private-sandbox".try_into().unwrap();
    next.request.binding.key.window = 11.try_into().unwrap();
    next
}

#[test]
fn validation_original_success_and_no_activation_failure_require_exact_release() {
    for activate in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let (create, admission, value) = inputs();
        let (local, terminal) = finish(&mut journal, &create, admission, activate);
        assert!(
            journal
                .lifecycle_validation_evidence(
                    &create,
                    &terminal,
                    revision(&create),
                    &value,
                    &context()
                )
                .is_err()
        );
        release(&mut journal, local, &terminal);
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
        native.failure = (!activate).then_some(PublicationFailure::Interrupted);
        let original =
            evidence::make_certificate(&journal.world().unwrap(), &native, local.sandbox).unwrap();
        native.validation_certificate = Some(original.clone());
        evidence::validate_publication(&native).unwrap();
        let before = journal.world().unwrap();
        let changes = journal.journal.connection.total_changes();
        let result = journal
            .lifecycle_validation_evidence(
                &create,
                &terminal,
                revision(&create),
                &value,
                &context(),
            )
            .unwrap();
        let (mapping, certificate) = result.admitted().unwrap();
        assert_eq!(mapping.sandbox, local.sandbox);
        assert_eq!(mapping.create_operation, local.operation);
        assert_eq!(certificate.activation.is_some(), activate);
        assert_eq!(certificate, &original);
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
        result.validate_for(&create, &terminal, &value).unwrap();
        assert_eq!(journal.world().unwrap(), before);
        assert_eq!(journal.journal.connection.total_changes(), changes);
        drop(journal);
        let journal = fixture::open(&path);
        assert_eq!(
            journal
                .lifecycle_validation_evidence(
                    &create,
                    &terminal,
                    revision(&create),
                    &value,
                    &context()
                )
                .unwrap(),
            result
        );
    }
}

#[test]
fn validation_identity_descriptor_and_unknown_mapping_never_issue_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let (create, admission, value) = inputs();
    journal
        .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
        .unwrap();
    let empty_terminal = terminal(&create);
    assert!(
        journal
            .lifecycle_validation_evidence(
                &create,
                &empty_terminal,
                revision(&create),
                &value,
                &context()
            )
            .is_err()
    );
    journal
        .record_lifecycle_intent(&empty_terminal.request, &context(), &mut NeverCrash)
        .unwrap();
    let no_admission = journal
        .lifecycle_validation_evidence(
            &create,
            &empty_terminal,
            revision(&create),
            &value,
            &context(),
        )
        .unwrap();
    assert_eq!(no_admission.outcome, ValidationOutcome::NotAdmitted);
    assert!(no_admission.admitted().is_none());
    let mut create = create;
    create.request.binding.session = "validation-second".try_into().unwrap();
    create.retention.slot = 2;
    create.request.binding.key.sequence = 3.try_into().unwrap();
    let (local, terminal) = finish(&mut journal, &create, admission, true);
    release(&mut journal, local, &terminal);
    let before = journal.world().unwrap();
    let changes = journal.journal.connection.total_changes();
    for index in 0..9 {
        let mut wrong = create.clone();
        match index {
            0 => wrong.request.binding.key.owner = "2".try_into().unwrap(),
            1 => wrong.request.binding.node.node_generation = 2.try_into().unwrap(),
            2 => wrong.request.binding.node.node_id = "wrong".try_into().unwrap(),
            3 => wrong.request.binding.session = "wrong".try_into().unwrap(),
            4 => wrong.request.binding.key.sequence = 9.try_into().unwrap(),
            5 => wrong.accepted_version = 2,
            6 => wrong.request.binding.key.window = 2.try_into().unwrap(),
            7 => wrong.retention.window = 2.try_into().unwrap(),
            _ => wrong.center.center = "foreign".try_into().unwrap(),
        }
        assert!(
            journal
                .lifecycle_validation_evidence(
                    &wrong,
                    &terminal,
                    revision(&create),
                    &value,
                    &context()
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
                .lifecycle_validation_evidence(
                    &create,
                    &terminal,
                    revision(&create),
                    &wrong,
                    &context()
                )
                .is_err()
        );
    }
    assert!(
        journal
            .lifecycle_validation_evidence(
                &create,
                &terminal,
                &"wrong-revision".try_into().unwrap(),
                &value,
                &context()
            )
            .is_err()
    );
    let mut foreign_center = create.center.clone();
    foreign_center.center = "foreign".try_into().unwrap();
    let foreign = AuthenticatedCenterContext::from_verified_peer_registration(
        foreign_center,
        create.request.binding.node.clone(),
    );
    assert!(
        journal
            .lifecycle_validation_evidence(&create, &terminal, revision(&create), &value, &foreign)
            .is_err()
    );
    let mut pending = terminal.clone();
    pending.status = TerminalStatus::Pending;
    assert!(
        journal
            .lifecycle_validation_evidence(&create, &pending, revision(&create), &value, &context())
            .is_err()
    );
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.journal.connection.total_changes(), changes);
}

#[test]
fn validation_after_window_retirement_uses_original_native_owner_not_global_number() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let (first, admission, value) = inputs();
    let (local, terminal) = finish(&mut journal, &first, admission.clone(), true);
    release(&mut journal, local, &terminal);
    retire(&mut journal, &terminal);
    assert!(
        journal
            .lifecycle_validation_evidence(&first, &terminal, revision(&first), &value, &context())
            .is_err()
    );
    let next = successor(&first);
    let (local, terminal) = finish(&mut journal, &next, admission, true);
    assert_eq!(local.owner, core::TenantId(2));
    release(&mut journal, local, &terminal);
    let result = journal
        .lifecycle_validation_evidence(&next, &terminal, revision(&next), &value, &context())
        .unwrap();
    assert_eq!(next.request.binding.key.owner.as_str(), "1");
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
fn validation_without_admission_still_requires_the_original_restricted_create_options() {
    for index in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = fixture::initialize(&dir.path().join("db"));
        let (mut create, _, value) = inputs();
        let LifecycleOperation::Create {
            create: parameters, ..
        } = &mut create.request.operation
        else {
            panic!()
        };
        let mut options: crate::CreateOptions =
            serde_json::from_slice(&parameters.options).unwrap();
        match index {
            0 => {
                options
                    .env_vars
                    .insert("USER_INPUT".into(), "not-validation".into());
            }
            1 => {
                options
                    .metadata
                    .insert("user-input".into(), "not-validation".into());
            }
            2 => options.networking = None,
            _ => options.networking.as_mut().unwrap().allow_internet_access = Some(true),
        }
        parameters.options = serde_json::to_vec(&options).unwrap();
        journal
            .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
            .unwrap();
        let terminal = terminal(&create);
        journal
            .record_lifecycle_intent(&terminal.request, &context(), &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .lifecycle_destroy_cleaned(&terminal.request, &context())
                .unwrap()
        );
        assert!(
            journal
                .lifecycle_validation_evidence(
                    &create,
                    &terminal,
                    revision(&create),
                    &value,
                    &context()
                )
                .is_err()
        );
    }
}

#[test]
fn private_template_across_windows_rejects_foreign_or_changed_global_binding_without_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let (first, admission, _) = inputs();
    let (local, terminal) = finish(&mut journal, &first, admission.clone(), true);
    release(&mut journal, local, &terminal);
    retire(&mut journal, &terminal);
    let next = successor(&first);
    journal
        .record_lifecycle_intent(&next, &context(), &mut NeverCrash)
        .unwrap();
    let before = journal.world().unwrap();
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
                .admit_lifecycle_create(&next, &context(), changed, &mut NeverCrash)
                .is_err()
        );
        assert_eq!(journal.world().unwrap(), before);
        assert_eq!(journal.next_sandbox_id().unwrap(), allocator);
    }
    let local = journal
        .admit_lifecycle_create(&next, &context(), admission, &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    assert_eq!(local.owner, core::TenantId(2));
}

#[test]
fn private_template_global_binding_and_native_projection_commit_together_across_windows() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = fixture::initialize(&path);
        let (first, admission, _) = inputs();
        let (local, terminal) = finish(&mut journal, &first, admission.clone(), true);
        release(&mut journal, local, &terminal);
        retire(&mut journal, &terminal);
        let next = successor(&first);
        journal
            .record_lifecycle_intent(&next, &context(), &mut NeverCrash)
            .unwrap();
        assert!(matches!(
            journal.admit_lifecycle_create(&next, &context(), admission.clone(), &mut Stop(point)),
            Err(crate::AdmissionError::Journal(JournalError::InjectedCrash(
                _
            )))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        let attached = journal
            .lifecycle_work_state(&next, &context())
            .unwrap()
            .local;
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
        fixture::refresh_epoch(&mut journal);
        let retry = journal
            .admit_lifecycle_create(&next, &context(), admission, &mut NeverCrash)
            .unwrap();
        let local = retry.local.unwrap();
        if let Some(original) = attached {
            assert_eq!(local, original);
        }
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
