use super::super::{
    config::{NativeSettings, application_config},
    test_support::{resolve, settings},
};
use super::*;
use sandbox_controller_core::maintenance::slots::{HostBinding, HostSlotId};
use sandproof_controller_service::WorkerTransportError;
use sandproof_node::HostObservation;
use std::{fs, path::PathBuf};

struct CredentialScratch(PathBuf);

impl Drop for CredentialScratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn credential_scratch() -> CredentialScratch {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "sandproof-credential-restart-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&directory).unwrap();
    CredentialScratch(directory)
}

fn context(vm: u64, previous: Option<u64>) -> HostSlotContext {
    HostSlotContext {
        position: HostSlotId(0),
        expected_previous: previous.map(core::VmId),
        binding: HostBinding {
            owner: core::TenantId(1),
            sandbox: core::SandboxId(vm),
            generation: 1,
            vm: core::VmId(vm),
            network: core::NetSlotId(vm),
        },
    }
}

fn audit(
    durable: &[(HostSlotContext, HostSlotPhase)],
    observed: &[(HostSlotContext, HostSlotPhase)],
) -> Result<(), Box<dyn Error>> {
    audit_host_slot_contexts(durable, observed, |_| true)
}

#[test]
fn startup_reopens_only_terminal_fence_without_any_launch_dispatch() {
    use sandproof_controller_service::{NeverCrash, kill};
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "sandproof-cancel-binding-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&directory).unwrap();
    let scratch = Scratch(directory);
    let config: NativeSettings = serde_json::from_value(settings()).unwrap();
    let bounded = resolve(settings());
    for claimed_opening_fence in [false, true] {
        let database = scratch.0.join(format!("{claimed_opening_fence}.db"));
        let mut journal = ControllerJournal::open_complete(
            &database,
            &config.namespace,
            bounded.clone(),
            config.maintenance.clone().unwrap(),
            0,
        )
        .unwrap();
        journal
            .admit_create(
                core::CreateRequest {
                    actor: core::Actor::Tenant(core::TenantId(1)),
                    owner: core::TenantId(1),
                    sandbox: core::SandboxId(1),
                    workspace: core::FsId(1),
                    file_policy: core::FilePolicyRef {
                        digest: core::PolicyDigest(1),
                    },
                    policy: core::NetworkPolicy::DenyAll,
                    operation: core::OperationId(1),
                },
                0,
                60,
                1024,
                application_config(&config, 1024).unwrap().limits,
                &mut NeverCrash,
            )
            .unwrap();
        if claimed_opening_fence {
            assert!(matches!(
                journal
                    .dispatch_next::<HostObservation, _>(
                        &mut |_| Err(WorkerTransportError::ConnectionLost),
                        &mut NeverCrash
                    )
                    .unwrap(),
                sandproof_controller_service::DispatchResult::OutcomeUnknown { .. }
            ));
        }
        assert!(
            kill::<HostObservation, _>(
                &mut journal,
                core::KillRequest {
                    actor: core::Actor::Tenant(core::TenantId(1)),
                    sandbox: core::SandboxId(1),
                    operation: core::OperationId(2),
                },
                &mut |_| Err(WorkerTransportError::ConnectionLost)
            )
            .is_err()
        );
        drop(journal);
        let journal = ControllerJournal::open_complete(
            &database,
            &config.namespace,
            bounded.clone(),
            config.maintenance.clone().unwrap(),
            1,
        )
        .unwrap();
        let world = journal.world().unwrap();
        let durable = journal.host_slot_inventory().unwrap();
        assert_eq!(durable.len(), 1);
        let audit = audit_host_slot_contexts(&durable, &[], |context| {
            can_install_host_slot_binding(&world, context.binding.sandbox)
        });
        assert_eq!(audit.is_ok(), !claimed_opening_fence);
        assert_eq!(journal.next_sandbox_id().unwrap().0, 2);
        let record = world.sandbox_record(core::SandboxId(1)).unwrap();
        assert!(record.current_vm.is_none());
        if !claimed_opening_fence {
            assert!(record.terminal);
            assert_eq!(record.phase, core::Phase::Revoking);
        }
    }
}

#[test]
fn startup_never_reconstructs_a_binding_after_host_work_was_claimed() {
    use HostSlotPhase::{Free, Occupied};
    let first = context(0, None);
    let second = context(1, Some(0));
    assert!(audit_host_slot_contexts(&[(first, Occupied)], &[], |_| false).is_err());
    assert!(audit_host_slot_contexts(&[(second, Occupied)], &[(first, Free)], |_| false).is_err());
    assert!(
        audit_host_slot_contexts(&[(second, Occupied)], &[(second, Occupied)], |_| false).is_ok()
    );
}

#[test]
fn startup_accepts_only_precise_bind_and_release_transaction_lag() {
    use HostSlotPhase::{Free, Occupied, Releasing};
    let first = context(0, None);
    assert!(audit(&[(first, Occupied)], &[]).is_ok());
    for phase in [Occupied, Releasing, Free] {
        assert!(audit(&[(first, phase)], &[(first, phase)]).is_ok());
    }
    for phase in [Occupied, Releasing, Free] {
        assert!(audit(&[(first, Releasing)], &[(first, phase)]).is_ok());
    }
    let second = context(1, Some(0));
    assert!(audit(&[(second, Occupied)], &[(first, Free)]).is_ok());
    assert!(audit(&[(second, Occupied)], &[]).is_err());
    assert!(audit(&[(second, Occupied)], &[(first, Occupied)]).is_err());
    assert!(audit(&[(second, Occupied)], &[(first, Releasing)]).is_err());
    assert!(audit(&[(second, Free)], &[(second, Releasing)]).is_err());
    assert!(audit(&[(second, Free)], &[]).is_err());
    assert!(audit(&[(second, Releasing)], &[]).is_err());
    assert!(audit(&[(second, Occupied)], &[(second, Releasing)]).is_err());
    assert!(audit(&[], &[(first, Free)]).is_err());
}

#[test]
fn startup_rejects_context_corruption_duplicates_and_stale_rebinding() {
    use HostSlotPhase::{Free, Occupied, Releasing};
    let old = context(1, None);
    let fresh = context(2, Some(1));
    for altered in [
        HostSlotContext {
            expected_previous: Some(core::VmId(99)),
            ..fresh
        },
        HostSlotContext {
            binding: HostBinding {
                owner: core::TenantId(2),
                ..fresh.binding
            },
            ..fresh
        },
        HostSlotContext {
            binding: HostBinding {
                network: core::NetSlotId(99),
                ..fresh.binding
            },
            ..fresh
        },
    ] {
        for phase in [Occupied, Releasing, Free] {
            assert!(audit(&[(fresh, phase)], &[(altered, phase)]).is_err());
        }
    }
    assert!(audit(&[(context(0, Some(1)), Occupied)], &[(old, Free)]).is_err());
    assert!(audit(&[(context(1, Some(1)), Occupied)], &[(old, Free)]).is_err());
    assert!(audit(&[(fresh, Occupied)], &[(fresh, Occupied); 2]).is_err());
    assert!(audit(&[(fresh, Occupied); 2], &[(fresh, Occupied)]).is_err());
    let same_vm = HostSlotContext {
        position: HostSlotId(1),
        ..fresh
    };
    assert!(audit(&[(fresh, Occupied), (same_vm, Occupied)], &[]).is_err());
}

#[test]
fn startup_only_isolates_typed_leaf_faults_with_the_original_generation() {
    for fault in [
        CredentialFault::Missing,
        CredentialFault::Unreadable,
        CredentialFault::Malformed,
        CredentialFault::TokenHashMismatch,
        CredentialFault::OriginalDigestMismatch,
    ] {
        assert_eq!(
            audit_launch_credential_binding(
                Some((7, [3; 32])),
                7,
                || panic!("original registration must not fall back to launch permission"),
                |digest| {
                    assert_eq!(digest, &[3; 32]);
                    Ok(LaunchCredentialStatus::Unavailable(fault))
                },
            )
            .unwrap(),
            Some(fault)
        );
    }
    assert!(
        audit_launch_credential_binding(
            Some((7, [3; 32])),
            8,
            || panic!("generation corruption must not fall back"),
            |_| panic!("generation corruption must be rejected before credential inspection"),
        )
        .is_err()
    );
}

#[test]
fn startup_credential_exceptions_never_swallow_authority_errors_or_missing_bindings() {
    assert!(
        audit_launch_credential_binding(
            Some((7, [3; 32])),
            7,
            || panic!("has original registration"),
            |_| Err("untrusted parent or changed launch configuration".into()),
        )
        .is_err()
    );
    for readable in [
        Ok(false),
        Err("malformed credential without original index".into()),
    ] {
        assert!(
            audit_launch_credential_binding(
                None,
                7,
                || readable,
                |_| panic!("missing original index cannot permit a local exception"),
            )
            .is_err()
        );
    }
    assert_eq!(
        audit_launch_credential_binding(
            None,
            7,
            || Ok(true),
            |_| panic!("no original digest exists"),
        )
        .unwrap(),
        None
    );
}

#[test]
fn a_bad_target_does_not_turn_an_intact_peer_into_a_startup_failure() {
    let statuses = [
        LaunchCredentialStatus::Unavailable(CredentialFault::Malformed),
        LaunchCredentialStatus::Intact,
    ];
    let faults = statuses
        .into_iter()
        .map(|status| {
            audit_launch_credential_binding(
                Some((1, [9; 32])),
                1,
                || panic!("original registration already available"),
                |_| Ok(status),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(faults, [Some(CredentialFault::Malformed), None]);
}

#[test]
fn reopening_with_a_local_credential_fault_preserves_the_original_unknown_occupancy() {
    use sandproof_controller_service::NeverCrash;
    const SANDBOXES: u64 = 2;
    let directory = credential_scratch();
    let database = directory.0.join("controller.db");
    let mut value = settings();
    let operations = value["maintenance"]["max_operations_per_sandbox"]
        .as_u64()
        .unwrap();
    // Each admitted instance reserves its entire N=8 result graph, even while
    // its original Create is pending/unknown: two instances need 16, not 10.
    value["bounded_host"]["maintenance"]["max_retained_results"] =
        serde_json::json!(SANDBOXES * operations);
    let config: NativeSettings = serde_json::from_value(value.clone()).unwrap();
    let bounded = resolve(value);
    let mut journal = ControllerJournal::open_complete(
        &database,
        &config.namespace,
        bounded.clone(),
        config.maintenance.clone().unwrap(),
        0,
    )
    .unwrap();
    for id in 1..=SANDBOXES {
        journal
            .admit_create(
                core::CreateRequest {
                    actor: core::Actor::Tenant(core::TenantId(1)),
                    owner: core::TenantId(1),
                    sandbox: core::SandboxId(id),
                    workspace: core::FsId(id),
                    file_policy: core::FilePolicyRef {
                        digest: core::PolicyDigest(1),
                    },
                    policy: core::NetworkPolicy::DenyAll,
                    operation: core::OperationId(id),
                },
                0,
                60,
                1024,
                application_config(&config, 1024).unwrap().limits,
                &mut NeverCrash,
            )
            .unwrap();
        journal
            .register_data_credential(
                core::SandboxId(id),
                1,
                [u8::try_from(id).unwrap(); 32],
                &mut NeverCrash,
            )
            .unwrap();
    }
    assert!(matches!(
        journal
            .dispatch_next::<HostObservation, _>(
                &mut |_| Err(WorkerTransportError::ConnectionLost),
                &mut NeverCrash,
            )
            .unwrap(),
        sandproof_controller_service::DispatchResult::OutcomeUnknown { .. }
    ));
    let before = journal.world().unwrap();
    let slots = journal.host_slot_inventory().unwrap();
    let unknown = journal.uncertain_actions().unwrap();
    drop(journal);
    let journal = ControllerJournal::open_complete(
        &database,
        &config.namespace,
        bounded,
        config.maintenance.clone().unwrap(),
        1,
    )
    .unwrap();
    for (id, status, fault) in [
        (
            1,
            LaunchCredentialStatus::Unavailable(CredentialFault::Missing),
            Some(CredentialFault::Missing),
        ),
        (2, LaunchCredentialStatus::Intact, None),
    ] {
        assert_eq!(
            audit_launch_credential_binding(
                journal
                    .original_data_credential(core::SandboxId(id))
                    .unwrap(),
                1,
                || panic!("original credential remains registered after restart"),
                |_| Ok(status),
            )
            .unwrap(),
            fault
        );
        assert_eq!(
            journal
                .original_data_credential(core::SandboxId(id))
                .unwrap(),
            Some((1, [u8::try_from(id).unwrap(); 32]))
        );
    }
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.host_slot_inventory().unwrap(), slots);
    assert_eq!(journal.uncertain_actions().unwrap(), unknown);
}
