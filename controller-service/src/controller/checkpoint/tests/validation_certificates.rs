//! Exact validation settlement is independent of later physical release and
//! core retirement; fake evidence here is limited to the worker boundary.
use super::*;
use crate::{
    CreateAdmission, CreateOptions,
    template_registry::{Publication, PublicationState},
    templates::ComputeLimits,
};

fn validation(
    journal: &mut ControllerJournal,
    config: &crate::template_registry::RegistryConfig,
    id: u64,
    candidate: char,
) -> Publication {
    let value = publication(candidate);
    assert!(journal.begin_template_publication(&value, config).unwrap());
    let mut request = request(id);
    request.owner = core::TenantId(1);
    request.actor = core::Actor::Tenant(core::TenantId(1));
    journal
        .admit_template_validation(
            request,
            CreateAdmission {
                now_ms: 1000,
                timeout_secs: 60,
                disk_bytes: 1000,
                limits: limits(),
                options: CreateOptions {
                    template: Some(value.binding.clone()),
                    ..CreateOptions::default()
                },
            },
            ComputeLimits {
                max_vcpus_global: 10,
                max_vcpus_per_tenant: 10,
                max_memory_bytes_global: 1_000_000_000,
                max_memory_bytes_per_tenant: 1_000_000_000,
            },
            &value.candidate_id,
            &mut NeverCrash,
        )
        .unwrap();
    crate::finish_admitted_create(
        journal,
        core::SandboxId(id),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
    value
}

fn kill(journal: &mut ControllerJournal, id: u64) {
    crate::kill(
        journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: core::SandboxId(id),
            operation: core::OperationId(2),
        },
        &mut success,
    )
    .unwrap();
}

fn replace_publication(journal: &ControllerJournal, value: &Publication) {
    journal
        .journal
        .connection
        .execute(
            "UPDATE template_publications SET record_json=?1 WHERE candidate_id=?2",
            params![serde_json::to_vec(value).unwrap(), value.candidate_id],
        )
        .unwrap();
}

#[test]
fn certificate_survives_create_history_retirement_and_one_slot_sequential_validation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let config = registry_config(&dir);
    let mut journal =
        ControllerJournal::open_complete(&path, NS, host_config(1), maintenance_config(), 1000)
            .unwrap();
    journal.enable_template_registry(&config).unwrap();
    for (id, candidate) in [(1, 'a'), (2, 'b'), (3, 'c')] {
        let value = validation(&mut journal, &config, id, candidate);
        journal
            .mark_template_checked(1, &value.candidate_id)
            .unwrap();
        kill(&mut journal, id);
        assert_eq!(
            journal.active_template_validation_sandboxes().unwrap(),
            vec![id]
        );
        journal.reconcile_template_publications(false).unwrap();
        let stored = journal
            .template_publication(1, &value.candidate_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.state, PublicationState::Ready);
        assert!(stored.validation_sandbox.is_none());
        let certificate = stored.validation_certificate.unwrap();
        assert_eq!(certificate.sandbox, core::SandboxId(id));
        assert_eq!(certificate.binding, value.binding);
        assert!(certificate.activation.is_some());
        assert!(!certificate.cleanup.is_empty());
        assert!(
            journal
                .active_template_validation_sandboxes()
                .unwrap()
                .is_empty()
        );
        // Settled probes remain private during the release/retire interval.
        assert_eq!(journal.template_validation_sandboxes().unwrap(), vec![id]);
        release(&mut journal, id);
        assert_eq!(journal.template_validation_sandboxes().unwrap(), vec![id]);
        retire(&mut journal, id);
        assert!(journal.template_validation_sandboxes().unwrap().is_empty());
        assert_eq!(count(&journal, "sandbox_create_options"), 0);
        journal.reconcile_template_publications(true).unwrap();
        checkpoint_and_prune(&mut journal);
        drop(journal);
        journal =
            ControllerJournal::open_complete(&path, NS, host_config(1), maintenance_config(), 1001)
                .unwrap();
        journal.reconcile_template_publications(true).unwrap();
        assert!(
            journal
                .resolve_registered_template(1, &value.binding.template_id)
                .unwrap()
                .is_some()
        );
        assert!(
            journal
                .resolve_registered_template(2, &value.binding.template_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            count(&journal, "template_publications"),
            i64::try_from(id).unwrap()
        );
    }
    assert_eq!(journal.registered_templates(1).unwrap().len(), 3);
    let tight = crate::template_registry::RegistryConfig {
        max_templates_global: 3,
        max_templates_per_tenant: 3,
        ..config
    };
    assert!(
        !journal
            .begin_template_publication(&publication('d'), &tight)
            .unwrap()
    );
}

#[test]
fn ready_without_certificate_is_rejected_without_migration_or_detaching_validation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    let config = registry_config(&dir);
    journal.enable_template_registry(&config).unwrap();
    let value = validation(&mut journal, &config, 1, 'a');
    journal
        .mark_template_checked(1, &value.candidate_id)
        .unwrap();
    kill(&mut journal, 1);
    let active = journal
        .template_publication(1, &value.candidate_id)
        .unwrap()
        .unwrap();
    let mut unsupported = active.clone();
    unsupported.state = PublicationState::Ready;
    replace_publication(&journal, &unsupported);
    let before = journal
        .journal
        .connection
        .query_row(
            "SELECT record_json FROM template_publications WHERE candidate_id=?1",
            [&value.candidate_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .unwrap();
    assert!(journal.reconcile_template_publications(false).is_err());
    assert_eq!(
        journal
            .journal
            .connection
            .query_row(
                "SELECT record_json FROM template_publications WHERE candidate_id=?1",
                [&value.candidate_id],
                |row| row.get::<_, Vec<u8>>(0)
            )
            .unwrap(),
        before
    );
    assert!(journal.active_template_validation_sandboxes().is_err());
    replace_publication(&journal, &active);
    journal.reconcile_template_publications(false).unwrap();
    let stored = journal
        .template_publication(1, &value.candidate_id)
        .unwrap()
        .unwrap();
    assert!(stored.validation_sandbox.is_none());
    assert!(stored.validation_certificate.is_some());
    assert_eq!(
        journal
            .journal
            .connection
            .query_row("SELECT version FROM controller_meta", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        11
    );
}

#[test]
fn certificate_rejects_owner_identity_digest_startup_and_cleanup_changes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    let config = registry_config(&dir);
    journal.enable_template_registry(&config).unwrap();
    let value = validation(&mut journal, &config, 1, 'a');
    journal
        .mark_template_checked(1, &value.candidate_id)
        .unwrap();
    kill(&mut journal, 1);
    journal.reconcile_template_publications(false).unwrap();
    let settled = journal
        .template_publication(1, &value.candidate_id)
        .unwrap()
        .unwrap();
    let mut mutations = Vec::new();
    let mut changed = settled.clone();
    changed.validation_certificate.as_mut().unwrap().owner = 2;
    mutations.push(changed);
    let mut changed = settled.clone();
    changed.validation_certificate.as_mut().unwrap().sandbox = core::SandboxId(2);
    mutations.push(changed);
    let mut changed = settled.clone();
    changed.binding.kernel.sha256 = "c".repeat(64);
    mutations.push(changed);
    let mut changed = settled.clone();
    changed.binding.resources.memory_mib = 256;
    mutations.push(changed);
    let mut changed = settled.clone();
    changed
        .validation_certificate
        .as_mut()
        .unwrap()
        .cleanup
        .clear();
    mutations.push(changed);
    let mut changed = settled.clone();
    changed
        .validation_certificate
        .as_mut()
        .unwrap()
        .activation
        .as_mut()
        .unwrap()
        .result = core::ResultDigest(999);
    mutations.push(changed);
    let mut changed = settled.clone();
    changed.binding.startup = Some(crate::templates::TemplateStartup {
        env_vars: std::collections::BTreeMap::new(),
        user: "root".into(),
        workdir: "/".into(),
        argv: vec!["true".into()],
    });
    mutations.push(changed);
    for changed in mutations {
        replace_publication(&journal, &changed);
        assert!(journal.reconcile_template_publications(false).is_err());
    }
    replace_publication(&journal, &settled);
    journal.reconcile_template_publications(false).unwrap();
}

#[test]
fn unknown_cleanup_is_never_settled_and_forged_checked_input_is_rejected() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    let config = registry_config(&dir);
    journal.enable_template_registry(&config).unwrap();
    let mut forged = publication('b');
    forged.checked = true;
    assert!(
        journal
            .begin_template_publication(&forged, &config)
            .is_err()
    );
    let value = validation(&mut journal, &config, 1, 'a');
    journal
        .mark_template_checked(1, &value.candidate_id)
        .unwrap();
    let _ = crate::kill(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: core::SandboxId(1),
            operation: core::OperationId(2),
        },
        &mut |action: &sandproof_protocol::WorkAction<core::EffectId>| {
            if matches!(action.command.command(), core::Command::CleanupVm { .. }) {
                Err(crate::WorkerTransportError::ConnectionLost)
            } else {
                success(action)
            }
        },
    );
    journal.reconcile_template_publications(true).unwrap();
    let stored = journal
        .template_publication(1, &value.candidate_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, PublicationState::Checking);
    assert_eq!(stored.validation_sandbox, Some(1));
    assert!(stored.validation_certificate.is_none());
    assert!(
        journal
            .host_slot_release_pinned(core::SandboxId(1))
            .unwrap()
    );
    let context = journal
        .host_slot_context(core::SandboxId(1))
        .unwrap()
        .unwrap();
    let mut called = false;
    assert!(
        journal
            .release_host_slot(
                context,
                |_| {
                    called = true;
                    Ok(())
                },
                &mut NeverCrash
            )
            .is_err()
    );
    assert!(!called);
    // A claimed Ready state with no certificate is corrupt, not an upgrade.
    let mut unsupported = stored.clone();
    unsupported.state = PublicationState::Ready;
    replace_publication(&journal, &unsupported);
    assert!(journal.reconcile_template_publications(false).is_err());
    replace_publication(&journal, &stored);
    assert_eq!(
        journal.active_template_validation_sandboxes().unwrap(),
        vec![1]
    );
    assert!(
        journal
            .template_publication(1, &value.candidate_id)
            .unwrap()
            .unwrap()
            .validation_certificate
            .is_none()
    );
}

#[test]
fn lost_certificate_commit_atomically_selects_active_pin_or_settled_outcome() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        let config = registry_config(&dir);
        journal.enable_template_registry(&config).unwrap();
        let value = validation(&mut journal, &config, 1, 'a');
        journal
            .mark_template_checked(1, &value.candidate_id)
            .unwrap();
        kill(&mut journal, 1);
        crate::controller::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.reconcile_template_publications(false),
            Err(JournalError::CommitUncertain(_))
        ));
        let stored = journal
            .template_publication(1, &value.candidate_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.validation_certificate.is_some(), durable);
        assert_eq!(stored.validation_sandbox.is_some(), !durable);
        assert_eq!(stored.state == PublicationState::Ready, durable);
        assert_eq!(
            journal
                .host_slot_release_pinned(core::SandboxId(1))
                .unwrap(),
            !durable
        );
        assert_eq!(journal.template_validation_sandboxes().unwrap(), vec![1]);
        let version: i64 = journal
            .journal
            .connection
            .query_row("SELECT version FROM controller_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, if durable { 11 } else { 10 });
        drop(journal);
        let mut journal = open(&path);
        journal.reconcile_template_publications(true).unwrap();
        assert!(
            !journal
                .host_slot_release_pinned(core::SandboxId(1))
                .unwrap()
        );
        release(&mut journal, 1);
        retire(&mut journal, 1);
        assert!(journal.template_validation_sandboxes().unwrap().is_empty());
    }
}
