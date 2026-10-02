//! Full-capacity composition: private settlement and validation certificates
//! consume promised envelopes, with real `SQLite` and the production reducer.
use super::*;
use crate::{
    AdmissionLimits, BuildRetryConfig, CreateAdmission, CreateOptions, WorkerTransportError,
    template_registry::{Publication, PublicationState, RegistryConfig},
    templates::ComputeLimits,
};
use std::collections::BTreeMap;

const BUILD: &str = "11111111111111111111111111111111";

fn host(slots: u64) -> BoundedHostConfig {
    let mut host = capacity::host();
    host.maintenance.max_host_slots = slots;
    host.maintenance.max_retained_results = 2 * slots;
    host
}
fn policy(slots: u64) -> MaintenanceConfig {
    let mut policy = capacity::full_config();
    policy.capacity.as_mut().unwrap().max_live_sandboxes = slots;
    // Exact scalar/reference reservation edges for the fixed V1 lifecycle.
    policy.max_records = 1 + slots + 2 + 61 * slots;
    policy.max_references = 86 * slots;
    policy
}
fn open(path: &std::path::Path, slots: u64, policy: &MaintenanceConfig) -> ControllerJournal {
    ControllerJournal::open_maintained(path, NS, host(slots), policy.clone(), 1000).unwrap()
}
fn limits(journal: &ControllerJournal) -> AdmissionLimits {
    let mut limits = super::support::limits();
    let capacity = journal
        .maintenance_config()
        .unwrap()
        .unwrap()
        .capacity
        .unwrap();
    limits.cleanup_reserve_bytes = capacity.max_online_storage_bytes;
    limits.max_disk_bytes_global = limits.cleanup_reserve_bytes + 100_000;
    limits.max_active_per_tenant = capacity.max_live_sandboxes;
    limits.max_disk_bytes_per_tenant = 100_000;
    limits
}
fn budget(journal: &ControllerJournal) -> (u64, u64, u64, u64) {
    super::super::super::capacity::fixture_sql_budget(
        &journal.journal.connection,
        &journal.world().unwrap(),
    )
    .unwrap()
}
fn tighten(policy: &mut MaintenanceConfig, baseline: (u64, u64, u64, u64), slots: u64) {
    let cap = policy.capacity.as_mut().unwrap();
    cap.max_sql_rows = baseline.0 + slots * baseline.2;
    cap.max_sql_bytes = baseline.1 + slots * baseline.3 + 128;
}
fn enable_private(journal: &mut ControllerJournal, slots: u64) {
    journal
        .enable_build_retry_windows(BuildRetryConfig {
            window_duration_ms: 1000,
            max_anchors: slots,
        })
        .unwrap();
}
fn issue(journal: &mut ControllerJournal, now: u64) -> String {
    journal
        .issue_build_window(core::TenantId(1), now, &mut NeverCrash)
        .unwrap()
        .request_window
}
fn build_admit(
    journal: &mut ControllerJournal,
    id: u64,
    window: &str,
    now: u64,
) -> Result<core::ControllerDecision, AdmissionError> {
    let mut request = request(id);
    request.owner = core::TenantId(1);
    request.actor = core::Actor::Tenant(core::TenantId(1));
    let admission = CreateAdmission {
        now_ms: i64::try_from(now).unwrap(),
        timeout_secs: 60,
        disk_bytes: 1000,
        limits: limits(journal),
        options: CreateOptions {
            metadata: BTreeMap::from([
                ("_sandproof_build_id".into(), BUILD.into()),
                ("_sandproof_build_window".into(), window.into()),
                ("_sandproof_build_request".into(), "60:false".into()),
            ]),
            ..CreateOptions::default()
        },
    };
    journal.admit_create_with_options(request, admission, &mut NeverCrash)
}
fn finish(journal: &mut ControllerJournal, id: u64) {
    crate::finish_admitted_create_v1(
        journal,
        core::SandboxId(id),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
}
fn kill(journal: &mut ControllerJournal, id: u64) {
    crate::kill_v1(
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
fn close(journal: &mut ControllerJournal, window: &str, now: u64) {
    journal
        .close_build_window(core::TenantId(1), window, now, &mut NeverCrash)
        .unwrap();
}
fn settle(
    journal: &mut ControllerJournal,
    window: &str,
    id: Option<u64>,
) -> Result<(), JournalError> {
    journal.settle_build_anchor(
        core::TenantId(1),
        window,
        BUILD,
        id.map(core::SandboxId),
        &mut NeverCrash,
    )
}
fn retire_at(journal: &mut ControllerJournal, id: u64, now: u64) {
    journal
        .retire_reclaimed(core::SandboxId(id), now, |_| Ok(()), &mut NeverCrash)
        .unwrap();
}
fn assert_full(journal: &ControllerJournal, policy: &MaintenanceConfig) {
    let charged = budget(journal);
    let cap = policy.capacity.as_ref().unwrap();
    assert_eq!(charged.0, cap.max_sql_rows);
    assert!(charged.1 <= cap.max_sql_bytes);
    assert!(
        cap.max_sql_bytes - charged.1 < 512,
        "unexpected unreserved room: {}",
        cap.max_sql_bytes - charged.1
    );
}
fn assert_history(journal: &ControllerJournal, live: i64, anchors: i64) {
    for table in [
        "sandboxes",
        "sandbox_create_options",
        "controller_capacity_reservations",
    ] {
        assert_eq!(count(journal, table), live, "{table}");
    }
    assert_eq!(count(journal, "private_build_anchors"), anchors);
    assert!(count(journal, "controller_inputs") <= 8);
    let pages: i64 = journal
        .journal
        .connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    assert!(
        pages < 256,
        "small full-capacity fixture must stay below a fixed 1MiB page plateau"
    );
    if live == 0 {
        for table in [
            "outbox",
            "controller_receipt_history",
            "operations",
            "controller_release_witnesses",
        ] {
            assert_eq!(count(journal, table), 0, "{table}");
        }
    }
}
fn private_fixture(dir: &TempDir, slots: u64) -> (ControllerJournal, MaintenanceConfig) {
    let mut policy = policy(slots);
    let mut seed = open(&dir.path().join("seed"), slots, &policy);
    enable_private(&mut seed, slots);
    tighten(&mut policy, budget(&seed), slots);
    drop(seed);
    let mut journal = open(&dir.path().join("db"), slots, &policy);
    enable_private(&mut journal, slots);
    (journal, policy)
}

#[test]
fn private_settlement_reuses_one_slot_at_exact_sql_and_result_reservation_edges() {
    let dir = TempDir::new().unwrap();
    let (mut journal, policy) = private_fixture(&dir, 1);
    let mut previous = 0;
    let mut position = None;
    for id in 1..=32 {
        let now = 1000 + id * 200;
        let window = issue(&mut journal, now);
        build_admit(&mut journal, id, &window, now).unwrap();
        finish(&mut journal, id);
        assert_full(&journal, &policy);
        let context = journal
            .host_slot_context(core::SandboxId(id))
            .unwrap()
            .unwrap();
        assert_eq!(*position.get_or_insert(context.position), context.position);
        close(&mut journal, &window, now + 1);
        let anchor = journal
            .build_anchor(core::TenantId(1), &window, BUILD)
            .unwrap()
            .unwrap();
        assert_eq!(anchor.sandbox, id);
        kill(&mut journal, id);
        assert!(
            journal
                .host_slot_release_pinned(core::SandboxId(id))
                .unwrap()
        );
        settle(&mut journal, &window, Some(id)).unwrap();
        release(&mut journal, id);
        retire_at(&mut journal, id, now + 2);
        assert_history(&journal, 0, 0);
        assert_eq!(
            journal
                .world()
                .unwrap()
                .maintenance_state()
                .unwrap()
                .retained_count(),
            2
        );
        let retry = issue(&mut journal, now + 3);
        assert!(matches!(
            build_admit(&mut journal, id + 1, &retry, now + 3),
            Err(AdmissionError::Journal(JournalError::MaintenanceCapacity))
        ));
        close(&mut journal, &retry, now + 4);
        settle(&mut journal, &retry, None).unwrap();
        assert!(
            journal
                .build_anchor(core::TenantId(1), &window, BUILD)
                .unwrap()
                .is_none()
        );
        assert!(build_admit(&mut journal, id + 1, &window, now + 4).is_err());
        assert_eq!(
            journal.expire_retained(now + 101, &mut NeverCrash).unwrap(),
            0
        );
        assert_eq!(
            journal.expire_retained(now + 102, &mut NeverCrash).unwrap(),
            2
        );
        let high = allocators::input_high_water(&journal.journal.connection).unwrap();
        assert!(high > previous);
        previous = high;
        drop(journal);
        journal = open(&dir.path().join("db"), 1, &policy);
        assert_history(&journal, 0, 0);
        settle(&mut journal, &window, Some(id)).unwrap();
        // Even after credits and retained results are reusable, the original
        // closed tuple cannot allocate another instance after history retirement.
        let high = allocators::input_high_water(&journal.journal.connection).unwrap();
        assert!(build_admit(&mut journal, id + 1, &window, now + 103).is_err());
        assert_eq!(
            allocators::input_high_water(&journal.journal.connection).unwrap(),
            high
        );
        assert_history(&journal, 0, 0);
    }
}

#[test]
fn private_close_query_and_settlement_spend_only_reserved_frontier_credit() {
    let dir = TempDir::new().unwrap();
    let (mut journal, policy) = private_fixture(&dir, 1);
    let window = issue(&mut journal, 1000);
    build_admit(&mut journal, 1, &window, 1000).unwrap();
    finish(&mut journal, 1);
    kill(&mut journal, 1);
    assert_full(&journal, &policy);
    checkpoint_and_prune(&mut journal);
    let remaining: i64 = journal
        .journal
        .connection
        .query_row(
            "SELECT input_left FROM controller_capacity_reservations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let high = i64::MAX as u64 - 1 - u64::try_from(remaining).unwrap() - 1;
    capacity::move_empty_input_frontier(&journal, high);
    drop(journal);
    journal = open(&dir.path().join("db"), 1, &policy);
    assert!(
        journal
            .issue_build_window(core::TenantId(1), 1001, &mut NeverCrash)
            .is_err()
    );
    close(&mut journal, &window, 1001);
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        high + 1
    );
    assert_eq!(
        journal
            .build_anchor(core::TenantId(1), &window, BUILD)
            .unwrap()
            .unwrap()
            .sandbox,
        1
    );
    settle(&mut journal, &window, Some(1)).unwrap();
    release(&mut journal, 1);
    retire_at(&mut journal, 1, 1002);
    checkpoint_and_prune(&mut journal);
    capacity::move_empty_input_frontier(&journal, i64::MAX as u64 - 3);
    drop(journal);
    journal = open(&dir.path().join("db"), 1, &policy);
    assert_eq!(journal.expire_retained(1102, &mut NeverCrash).unwrap(), 2);
    assert_history(&journal, 0, 0);
    settle(&mut journal, &window, Some(1)).unwrap();
}

#[test]
fn unknown_private_anchor_keeps_only_its_own_credit_while_other_slot_churns() {
    let dir = TempDir::new().unwrap();
    let (mut journal, policy) = private_fixture(&dir, 2);
    let window = issue(&mut journal, 1000);
    build_admit(&mut journal, 1, &window, 1000).unwrap();
    journal
        .dispatch_next_for(
            core::SandboxId(1),
            &mut |_| -> Result<sandproof_protocol::WorkReceipt<Evidence>, WorkerTransportError> {
                Err(WorkerTransportError::ConnectionLost)
            },
            &mut NeverCrash,
        )
        .unwrap();
    close(&mut journal, &window, 1001);
    assert!(settle(&mut journal, &window, Some(1)).is_err());
    let old = journal
        .world()
        .unwrap()
        .sandbox_record(core::SandboxId(1))
        .unwrap()
        .clone();
    let credit:Vec<i64>=journal.journal.connection.query_row("SELECT input_left,effect_left,outbox_left,receipt_left FROM controller_capacity_reservations",[],|row|Ok(vec![row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?])).unwrap();
    let mut position = None;
    for id in 2..=17 {
        let now = 1000 + id * 200;
        let fresh = issue(&mut journal, now);
        build_admit(&mut journal, id, &fresh, now).unwrap();
        finish(&mut journal, id);
        assert_full(&journal, &policy);
        let context = journal
            .host_slot_context(core::SandboxId(id))
            .unwrap()
            .unwrap();
        assert_eq!(*position.get_or_insert(context.position), context.position);
        close(&mut journal, &fresh, now + 1);
        kill(&mut journal, id);
        settle(&mut journal, &fresh, Some(id)).unwrap();
        release(&mut journal, id);
        retire_at(&mut journal, id, now + 2);
        assert_eq!(
            journal.expire_retained(now + 102, &mut NeverCrash).unwrap(),
            2
        );
        assert_history(&journal, 1, 1);
        assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
        assert_eq!(
            journal.world().unwrap().sandbox_record(core::SandboxId(1)),
            Some(&old)
        );
        let current:Vec<i64>=journal.journal.connection.query_row("SELECT input_left,effect_left,outbox_left,receipt_left FROM controller_capacity_reservations",[],|row|Ok(vec![row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?])).unwrap();
        assert_eq!(current, credit);
        assert_eq!(
            journal
                .build_anchor(core::TenantId(1), &window, BUILD)
                .unwrap()
                .unwrap()
                .sandbox,
            1
        );
    }
    drop(journal);
    let journal = open(&dir.path().join("db"), 2, &policy);
    assert_history(&journal, 1, 1);
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
}

fn begin_publications(
    journal: &mut ControllerJournal,
    config: &RegistryConfig,
) -> Vec<Publication> {
    ['a', 'b', 'c']
        .into_iter()
        .map(|candidate| {
            let value = publication(candidate);
            assert!(journal.begin_template_publication(&value, config).unwrap());
            value
        })
        .collect()
}
fn validation_admit(
    journal: &mut ControllerJournal,
    value: &Publication,
    id: u64,
    now: u64,
) -> Result<core::ControllerDecision, AdmissionError> {
    let mut request = request(id);
    request.owner = core::TenantId(1);
    request.actor = core::Actor::Tenant(core::TenantId(1));
    let admission = CreateAdmission {
        now_ms: i64::try_from(now).unwrap(),
        timeout_secs: 60,
        disk_bytes: 1000,
        limits: limits(journal),
        options: CreateOptions {
            template: Some(value.binding.clone()),
            ..CreateOptions::default()
        },
    };
    journal.admit_template_validation(
        request,
        admission,
        ComputeLimits {
            max_vcpus_global: 10,
            max_vcpus_per_tenant: 10,
            max_memory_bytes_global: 1_000_000_000,
            max_memory_bytes_per_tenant: 1_000_000_000,
        },
        &value.candidate_id,
        &mut NeverCrash,
    )
}
fn end_validation(journal: &mut ControllerJournal, value: &Publication, id: u64, ready: bool) {
    if ready {
        finish(journal, id);
        journal
            .mark_template_checked(1, &value.candidate_id)
            .unwrap();
        kill(journal, id);
    } else {
        let result = crate::finish_admitted_create_v1(
            journal,
            core::SandboxId(id),
            core::OperationId(1),
            &mut |action: &sandproof_protocol::WorkAction<core::EffectId>| {
                if matches!(action.command.command(), core::Command::ActivateVm { .. }) {
                    Ok(sandproof_protocol::WorkReceipt::<Evidence> {
                        identity: action.identity.clone(),
                        outcome: sandproof_protocol::WorkOutcome::DefinitiveFailure {
                            code: sandproof_protocol::FailureCode::PolicyRejected,
                            retryable: false,
                            partial: None,
                        },
                    })
                } else {
                    success(action)
                }
            },
        );
        assert!(matches!(result, Err(JournalError::OperationFailed)));
    }
}
fn registry_fixture(
    dir: &TempDir,
) -> (
    ControllerJournal,
    MaintenanceConfig,
    RegistryConfig,
    Vec<Publication>,
) {
    let config = registry_config(dir);
    let mut policy = policy(1);
    let mut seed = open(&dir.path().join("seed"), 1, &policy);
    seed.enable_template_registry(&config).unwrap();
    let values = begin_publications(&mut seed, &config);
    for (index, value) in values.iter().enumerate() {
        let id = index as u64 + 1;
        let now = 1000 + id * 200;
        validation_admit(&mut seed, value, id, now).unwrap();
        end_validation(&mut seed, value, id, true);
        seed.reconcile_template_publications(false).unwrap();
        release(&mut seed, id);
        retire_at(&mut seed, id, now + 2);
        seed.expire_retained(now + 102, &mut NeverCrash).unwrap();
    }
    // The immutable template/artifact rows remain charged independently. The
    // only added headroom is one existing lifecycle lease, plus <512 bytes.
    tighten(&mut policy, budget(&seed), 1);
    drop(seed);
    let mut journal = open(&dir.path().join("db"), 1, &policy);
    journal.enable_template_registry(&config).unwrap();
    let values = begin_publications(&mut journal, &config);
    (journal, policy, config, values)
}

#[test]
fn certificates_complete_at_reserved_sql_ceiling_and_outlive_one_slot_history() {
    let dir = TempDir::new().unwrap();
    let (mut journal, policy, config, values) = registry_fixture(&dir);
    let mut position = None;
    for (index, value) in values.iter().enumerate() {
        let id = index as u64 + 1;
        let now = 1000 + id * 200;
        let ready = id != 2;
        validation_admit(&mut journal, value, id, now).unwrap();
        let context = journal
            .host_slot_context(core::SandboxId(id))
            .unwrap()
            .unwrap();
        assert_eq!(*position.get_or_insert(context.position), context.position);
        if id == 3 {
            assert_full(&journal, &policy);
            assert!(matches!(
                journal.begin_template_publication(&publication('d'), &config),
                Err(JournalError::MaintenanceCapacity)
            ));
            assert_eq!(count(&journal, "template_publications"), 3);
            assert!(matches!(
                journal.set_template_alias(1, "over-budget", &values[0].binding.template_id, None),
                Err(JournalError::MaintenanceCapacity)
            ));
        }
        end_validation(&mut journal, value, id, ready);
        assert_eq!(
            journal.active_template_validation_sandboxes().unwrap(),
            vec![id]
        );
        settle_certificate(&mut journal, value, id, ready);
        assert_eq!(journal.template_validation_sandboxes().unwrap(), vec![id]);
        release(&mut journal, id);
        retire_at(&mut journal, id, now + 2);
        assert!(journal.template_validation_sandboxes().unwrap().is_empty());
        for table in [
            "sandboxes",
            "operations",
            "outbox",
            "controller_receipt_history",
            "sandbox_create_options",
            "controller_capacity_reservations",
        ] {
            assert_eq!(count(&journal, table), 0, "{table}");
        }
        assert!(count(&journal, "controller_inputs") <= 8);
        let results = if ready { 2 } else { 1 };
        assert_eq!(
            journal
                .world()
                .unwrap()
                .maintenance_state()
                .unwrap()
                .retained_count(),
            results
        );
        if let Some(next) = values.get(index + 1) {
            assert!(matches!(
                validation_admit(&mut journal, next, id + 1, now + 3),
                Err(AdmissionError::Journal(JournalError::MaintenanceCapacity))
            ));
        }
        assert_eq!(
            journal.expire_retained(now + 101, &mut NeverCrash).unwrap(),
            0
        );
        assert_eq!(
            journal.expire_retained(now + 102, &mut NeverCrash).unwrap(),
            results
        );
        journal.reconcile_template_publications(false).unwrap();
    }
    assert_eq!(count(&journal, "template_publications"), 3);
    assert_eq!(count(&journal, "admitted_templates"), 3);
    assert_eq!(journal.registered_templates(1).unwrap().len(), 2);
    drop(journal);
    let mut journal = open(&dir.path().join("db"), 1, &policy);
    journal.reconcile_template_publications(true).unwrap();
    for value in &values {
        let saved = journal
            .template_publication(1, &value.candidate_id)
            .unwrap()
            .unwrap();
        assert!(saved.validation_certificate.is_some());
        assert!(saved.validation_sandbox.is_none());
    }
    assert_eq!(count(&journal, "sandbox_create_options"), 0);
    assert_eq!(journal.registered_templates(1).unwrap().len(), 2);
}

fn settle_certificate(journal: &mut ControllerJournal, value: &Publication, id: u64, ready: bool) {
    let before = budget(journal);
    journal.reconcile_template_publications(false).unwrap();
    assert_eq!(
        budget(journal),
        before,
        "certificate growth must consume its existing envelope"
    );
    let saved = journal
        .template_publication(1, &value.candidate_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.state,
        if ready {
            PublicationState::Ready
        } else {
            PublicationState::Failed
        }
    );
    assert!(saved.validation_sandbox.is_none());
    let certificate = saved.validation_certificate.as_ref().unwrap();
    assert_eq!(certificate.activation.is_some(), ready);
    assert_eq!(certificate.sandbox, core::SandboxId(id));
    assert!(
        serde_json::to_vec(&saved).unwrap().len() > serde_json::to_vec(value).unwrap().len() + 1000
    );
}

#[test]
fn prevalidation_failure_keeps_its_precharged_outcome_when_sql_rows_are_full() {
    let dir = TempDir::new().unwrap();
    let (mut journal, policy, _, values) = registry_fixture(&dir);
    validation_admit(&mut journal, &values[0], 1, 1000).unwrap();
    end_validation(&mut journal, &values[0], 1, true);
    settle_certificate(&mut journal, &values[0], 1, true);
    release(&mut journal, 1);
    retire_at(&mut journal, 1, 1002);
    journal.expire_retained(1102, &mut NeverCrash).unwrap();
    let mut added = 0;
    loop {
        match journal.set_template_alias(
            1,
            &format!("fill-{added}"),
            &values[0].binding.template_id,
            None,
        ) {
            Ok(()) => added += 1,
            Err(JournalError::MaintenanceCapacity) => break,
            other => panic!("expected exact SQL capacity refusal, got {other:?}"),
        }
    }
    assert!(added > 0);
    assert_eq!(
        budget(&journal).0,
        policy.capacity.as_ref().unwrap().max_sql_rows
    );
    let before = budget(&journal);
    journal
        .reject_template_publication(
            1,
            &values[1].candidate_id,
            crate::template_registry::PublicationFailure::ValidationRejected,
        )
        .unwrap();
    assert_eq!(budget(&journal), before);
    let rejected = journal
        .template_publication(1, &values[1].candidate_id)
        .unwrap()
        .unwrap();
    assert_eq!(rejected.state, PublicationState::Failed);
    assert!(rejected.validation_certificate.is_none());
    assert!(rejected.validation_sandbox.is_none());
    journal.reconcile_template_publications(true).unwrap();
    assert_eq!(budget(&journal), before);
    assert_eq!(
        journal
            .template_publication(1, &values[2].candidate_id)
            .unwrap()
            .unwrap()
            .state,
        PublicationState::Failed
    );
    assert_eq!(count(&journal, "sandboxes"), 0);
    assert_eq!(count(&journal, "template_publications"), 3);
    drop(journal);
    let mut journal = open(&dir.path().join("db"), 1, &policy);
    journal.reconcile_template_publications(true).unwrap();
    assert_eq!(journal.registered_templates(1).unwrap().len(), 1);
}
