use super::super::{
    config::{ConfigFile, managed_config},
    test_support::{resolve, settings},
};
use super::*;
use sandbox_controller_core::maintenance::slots::{HostBinding, HostSlotId};
use sandproof_controller_service::WorkerTransportError;
use sandproof_node::HostObservation;
use std::{fs, path::PathBuf};

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
    use sandproof_controller_service::{NeverCrash, kill_v1};
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
    let config: ConfigFile = serde_json::from_value(settings()).unwrap();
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
                managed_config(&config, 1024).unwrap().limits,
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
            kill_v1::<HostObservation, _>(
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
