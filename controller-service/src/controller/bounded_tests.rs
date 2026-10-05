use super::*;
use crate::{AdmissionLimits, NeverCrash, finish_admitted_create_v1, kill_v1};
use sandbox_controller_core::maintenance::{
    MaintenanceLimits, slots::HostSlotPhase, windows::WindowLimits,
};
use sandproof_protocol::ResourceRef;
use serde::Deserialize;
use tempfile::TempDir;

const NS: &str = "0123456789abcdef";
fn config(slots: u64) -> BoundedHostConfig {
    BoundedHostConfig {
        physical_storage: None,
        console_policy: None,
        uid_base: 1000,
        uplink_base: 8,
        max_data_routes_per_slot: 4,
        max_data_routes_global: 8,
        data_route_control_reserve: 1,
        maintenance: MaintenanceLimits {
            max_host_slots: slots,
            max_retained_results: 16,
            windows: WindowLimits::new(2, 1000).unwrap(),
        },
    }
}
fn request(id: u64) -> core::CreateRequest {
    core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(id)),
        owner: core::TenantId(id),
        sandbox: core::SandboxId(id),
        workspace: core::FsId(id),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(1),
    }
}
fn limits() -> AdmissionLimits {
    AdmissionLimits {
        max_vm_id: 1,
        max_active_global: 4,
        max_active_per_tenant: 1,
        max_disk_bytes_global: 100_000,
        max_disk_bytes_per_tenant: 10_000,
        cleanup_reserve_bytes: 1000,
        max_timeout_secs: 600,
        max_lifetime_secs: 600,
        startup_timeout_secs: 60,
    }
}
fn admit(
    journal: &mut ControllerJournal,
    id: u64,
    crash: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, AdmissionError> {
    journal.admit_create(request(id), 1000, 60, 1000, limits(), crash)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Evidence {
    resource: ResourceRef,
    command: core::Command,
}
impl ResourceEvidence for Evidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}
impl HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.command == *command
    }
}
fn success(
    action: &WorkAction<core::EffectId>,
) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
    action
        .identity
        .validate_version()
        .map_err(|_| WorkerTransportError::ConnectionLost)?;
    Ok(WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(Evidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
    })
}
struct FailAt(CrashPoint);
impl CrashInjector for FailAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}
fn create(journal: &mut ControllerJournal, id: u64) {
    admit(journal, id, &mut NeverCrash).unwrap();
    finish_admitted_create_v1(
        journal,
        core::SandboxId(id),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
}
fn destroy(journal: &mut ControllerJournal, id: u64) {
    kill_v1(
        journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(id)),
            sandbox: core::SandboxId(id),
            operation: core::OperationId(2),
        },
        &mut success,
    )
    .unwrap();
}

#[test]
fn bounded_admission_reservation_rolls_back_with_every_projection_and_identity() {
    let dir = TempDir::new().unwrap();
    let mut j = ControllerJournal::open_bounded(dir.path().join("db"), NS, config(1), 0).unwrap();
    assert!(admit(&mut j, 1, &mut FailAt(CrashPoint::BeforeCommit)).is_err());
    assert!(j.host_slot_contexts().unwrap().is_empty());
    assert!(j.world().unwrap().sandbox_records().is_empty());
    assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(1));
    create(&mut j, 1);
    let before = j.world().unwrap();
    assert!(matches!(
        admit(&mut j, 2, &mut NeverCrash),
        Err(AdmissionError::QuotaExceeded)
    ));
    assert_eq!(j.world().unwrap(), before);
    assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(2));
    assert_eq!(
        j.journal
            .connection
            .query_row("SELECT count(*) FROM sandbox_lifetimes", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn one_older_live_instance_survives_repeated_other_position_reuse() {
    let dir = TempDir::new().unwrap();
    let mut j = ControllerJournal::open_bounded(dir.path().join("db"), NS, config(2), 0).unwrap();
    create(&mut j, 1);
    let older = j.host_slot_context(core::SandboxId(1)).unwrap().unwrap();
    for id in 2..=18 {
        create(&mut j, id);
        assert!(
            j.host_slot_context(core::SandboxId(id))
                .unwrap()
                .unwrap()
                .binding
                .vm
                .0
                > 0
        );
        destroy(&mut j, id);
        let context = j.host_slot_context(core::SandboxId(id)).unwrap().unwrap();
        j.release_host_slot(context, |_| Ok(()), &mut NeverCrash)
            .unwrap();
        assert_eq!(
            j.host_slot_context(core::SandboxId(1)).unwrap(),
            Some(older)
        );
        assert_eq!(
            j.world()
                .unwrap()
                .sandbox_record(core::SandboxId(1))
                .unwrap()
                .phase,
            core::Phase::Running
        );
    }
    assert_eq!(j.host_slot_contexts().unwrap().len(), 2);
}

#[test]
fn host_release_never_runs_before_committed_intent_and_lost_ack_is_replayable() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut j = ControllerJournal::open_bounded(&path, NS, config(1), 0).unwrap();
    create(&mut j, 1);
    let context = j.host_slot_context(core::SandboxId(1)).unwrap().unwrap();
    assert!(
        j.release_host_slot(
            context,
            |_| panic!("running VM cannot reach release"),
            &mut NeverCrash
        )
        .is_err()
    );
    destroy(&mut j, 1);
    assert!(
        j.release_host_slot(
            context,
            |_| panic!("uncommitted intent cannot reach host"),
            &mut FailAt(CrashPoint::BeforeCommit)
        )
        .is_err()
    );
    assert_eq!(
        j.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Occupied
    );
    assert!(
        j.release_host_slot(
            context,
            |_| Ok(()),
            &mut FailAt(CrashPoint::AfterWorkerCall)
        )
        .is_err()
    );
    assert_eq!(
        j.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Releasing
    );
    drop(j);
    let mut j = ControllerJournal::open_bounded(&path, NS, config(1), 0).unwrap();
    assert_eq!(
        j.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Releasing
    );
    j.release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    assert_eq!(j.host_slot_inventory().unwrap()[0].1, HostSlotPhase::Free);
}

#[test]
fn predecessor_context_corruption_and_missing_used_row_fail_closed_on_reopen() {
    for mutation in [
        "DELETE FROM controller_host_slots",
        "UPDATE controller_host_slots SET context_json=json_set(context_json,'$.expected_previous',0)",
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut j = ControllerJournal::open_bounded(&path, NS, config(1), 0).unwrap();
        for id in 1..=3 {
            create(&mut j, id);
            destroy(&mut j, id);
            let context = j.host_slot_context(core::SandboxId(id)).unwrap().unwrap();
            j.release_host_slot(context, |_| Ok(()), &mut NeverCrash)
                .unwrap();
        }
        drop(j);
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        assert!(ControllerJournal::open_bounded(&path, NS, config(1), 0).is_err());
    }
}

#[test]
fn commit_uncertainty_reloads_exact_candidate_not_the_assumed_release_result() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut j = ControllerJournal::open_bounded(&path, NS, config(1), 0).unwrap();
        create(&mut j, 1);
        destroy(&mut j, 1);
        let context = j.host_slot_context(core::SandboxId(1)).unwrap().unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            j.release_host_slot(
                context,
                |_| panic!("uncertain commit cannot reach host"),
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        assert_eq!(
            j.host_slot_inventory().unwrap()[0].1,
            if durable {
                HostSlotPhase::Releasing
            } else {
                HostSlotPhase::Occupied
            }
        );
    }
}

#[test]
fn all_bounded_limits_and_nested_fields_are_explicit_and_checked() {
    let valid = serde_json::to_value(config(1)).unwrap();
    for field in [
        "uid_base",
        "uplink_base",
        "maintenance",
        "max_data_routes_per_slot",
    ] {
        let mut changed = valid.clone();
        changed.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<BoundedHostConfig>(changed).is_err());
    }
    let mut unknown = valid;
    unknown["maintenance"]["invented"] = true.into();
    assert!(serde_json::from_value::<BoundedHostConfig>(unknown).is_err());
    for invalid in [
        BoundedHostConfig {
            max_data_routes_per_slot: 0,
            ..config(1)
        },
        BoundedHostConfig {
            physical_storage: None,
            console_policy: None,
            uid_base: 0,
            ..config(1)
        },
        BoundedHostConfig {
            physical_storage: None,
            console_policy: None,
            uid_base: u32::MAX,
            ..config(1)
        },
        BoundedHostConfig {
            uplink_base: u64::MAX,
            ..config(1)
        },
        config(0),
        config(1_048_577),
    ] {
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn console_policy_is_explicit_and_cannot_change_an_existing_host_mapping() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let legacy = config(1);
    let encoded = serde_json::to_vec(&legacy).unwrap();
    assert!(
        !String::from_utf8(encoded.clone())
            .unwrap()
            .contains("console_policy")
    );
    assert_eq!(
        serde_json::from_slice::<BoundedHostConfig>(&encoded).unwrap(),
        legacy
    );
    drop(ControllerJournal::open_bounded(&path, NS, legacy.clone(), 0).unwrap());
    let mut discard = legacy.clone();
    discard.console_policy = Some(ConsolePolicy::Discard);
    assert!(ControllerJournal::open_bounded(&path, NS, discard, 0).is_err());
    assert!(ControllerJournal::open_bounded(&path, NS, legacy, 0).is_ok());
}
