#![cfg(target_os = "linux")]

//! Opt-in, root-only KVM test. It never invents a host success receipt.
//! A failure intentionally leaves exact resource claims for inspection.

use sandbox_controller_core as core;
use sandproof_controller_service::{
    ControllerJournal, JournalError, create_v1, finish_accepted_kill_v1, finish_admitted_create_v1,
    kill_v1,
};
use sandproof_node::{LinuxWorker, NodeConfig};
use sandproof_protocol::{UnknownReason, WorkOutcome, WorkReceipt};
use std::env;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command as Process;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing operator setting {name}"))
}

#[test]
#[ignore = "requires root, KVM, pinned envd image and dedicated state directory"]
#[expect(
    clippy::too_many_lines,
    reason = "one real VM is carried through two deliberately lost receipts, restart reconciliation and exact cleanup"
)]
fn create_and_kill_use_real_host_receipts() {
    let namespace = required("SANDPROOF_TEST_NAMESPACE");
    let state_dir = PathBuf::from(required("SANDPROOF_TEST_STATE_DIR"));
    let config = NodeConfig {
        namespace: namespace.clone(),
        state_dir: state_dir.clone(),
        jailer: PathBuf::from(required("SANDPROOF_TEST_JAILER")),
        firecracker: PathBuf::from(required("SANDPROOF_TEST_FIRECRACKER")),
        chroot_base: PathBuf::from(required("SANDPROOF_TEST_CHROOT")),
        kernel: PathBuf::from(required("SANDPROOF_TEST_KERNEL")),
        rootfs: PathBuf::from(required("SANDPROOF_TEST_ROOTFS")),
        uid_base: required("SANDPROOF_TEST_UID_BASE").parse().unwrap(),
        max_vm_id: 1000,
        vcpu_count: 1,
        memory_mib: 256,
        file_policy_digest: core::PolicyDigest(1),
    };
    let mut worker = LinuxWorker::new(config.clone()).unwrap();
    let mut journal = ControllerJournal::open(state_dir.join("controller.db"), &namespace).unwrap();
    let mut random = [0_u8; 32];
    File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut random)
        .unwrap();
    let mut token = String::with_capacity(64);
    for value in random {
        write!(&mut token, "{value:02x}").unwrap();
    }
    let sandbox = core::SandboxId(1);
    worker.put_secret(sandbox, &token).unwrap();
    let request = core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox,
        workspace: core::FsId(1),
        file_policy: core::FilePolicyRef {
            digest: config.file_policy_digest,
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(1),
    };
    let lost = create_v1(&mut journal, request, &mut |action| {
        let receipt = worker.execute(action)?;
        if matches!(receipt.outcome, WorkOutcome::ObservedSuccess(_)) {
            assert_eq!(worker.observe_action(action).unwrap(), receipt);
        }
        if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
            return Ok(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            });
        }
        Ok(receipt)
    });
    assert!(
        matches!(lost, Err(JournalError::UnresolvedAction)),
        "{lost:?}; host: {:?}",
        worker.last_error()
    );
    drop(journal); // simulated controller process restart after the VM launched
    let mut journal = ControllerJournal::open(state_dir.join("controller.db"), &namespace).unwrap();
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::PrepareVm { .. }
    ));
    assert_eq!(
        journal
            .reconcile_uncertain(&mut |action| worker.observe_action(action).ok())
            .unwrap(),
        1
    );
    let mut repeated_prepare = false;
    let running =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            repeated_prepare |= matches!(action.command.command(), core::Command::PrepareVm { .. });
            worker.execute(action)
        })
        .unwrap_or_else(|error| {
            panic!(
                "resumed create failed: {error}; host: {:?}",
                worker.last_error()
            )
        });
    assert!(!repeated_prepare);
    assert_eq!(
        running.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Running
    );
    let resources = running.resource_index(sandbox);
    assert_eq!(resources.vms.len(), 1);
    assert_eq!(resources.slots.len(), 1);
    let inventory = worker.inventory().unwrap();
    assert!(inventory.unbound_claims.is_empty());
    assert!(inventory.unbound_networks.is_empty());
    assert_eq!(inventory.slots.len(), 1);
    assert_eq!(inventory.slots[0].sandbox, sandbox);
    assert_eq!(inventory.slots[0].vm, resources.vms[0]);
    assert_eq!(inventory.slots[0].slot, resources.slots[0]);
    assert!(inventory.slots[0].claim_present);
    assert!(inventory.slots[0].process.is_some());
    assert!(inventory.slots[0].network_inode.is_some());
    assert!(inventory.slots[0].disk_inode.is_some());
    let vm_name = format!("vm-{namespace}-{}", resources.vms[0].0);
    assert!(PathBuf::from(format!("/run/netns/{vm_name}")).exists());
    let lost = kill_v1(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox,
            operation: core::OperationId(2),
        },
        &mut |action| {
            let receipt = worker.execute(action)?;
            if matches!(receipt.outcome, WorkOutcome::ObservedSuccess(_)) {
                assert_eq!(worker.observe_action(action).unwrap(), receipt);
            }
            if matches!(action.command.command(), core::Command::StopVm { .. }) {
                return Ok(WorkReceipt {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                });
            }
            Ok(receipt)
        },
    );
    assert!(
        matches!(lost, Err(JournalError::UnresolvedAction)),
        "{lost:?}; host: {:?}",
        worker.last_error()
    );
    drop(journal);
    let mut journal = ControllerJournal::open(state_dir.join("controller.db"), &namespace).unwrap();
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::StopVm { .. }
    ));
    assert_eq!(
        journal
            .reconcile_uncertain(&mut |action| worker.observe_action(action).ok())
            .unwrap(),
        1
    );
    let mut repeated_stop = false;
    let killed =
        finish_accepted_kill_v1(&mut journal, sandbox, core::OperationId(2), &mut |action| {
            repeated_stop |= matches!(action.command.command(), core::Command::StopVm { .. });
            worker.execute(action)
        })
        .unwrap_or_else(|error| {
            panic!(
                "resumed kill failed: {error}; host: {:?}",
                worker.last_error()
            )
        });
    assert!(!repeated_stop);
    assert_eq!(
        killed.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    assert!(!PathBuf::from(format!("/run/netns/{vm_name}")).exists());
    assert!(
        !config
            .chroot_base
            .join(config.firecracker.file_name().unwrap())
            .join(vm_name)
            .exists()
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
    let inventory = worker.inventory().unwrap();
    assert_eq!(inventory.slots.len(), 1);
    assert!(inventory.unbound_networks.is_empty());
    assert_eq!(
        inventory.slots[0].process.as_ref().unwrap().state,
        sandbox_runtime::VmState::Stopped
    );
    assert!(inventory.slots[0].network_inode.is_none());
    assert!(inventory.slots[0].disk_inode.is_none());
    let unbound = config
        .chroot_base
        .join(".claims")
        .join(format!("vm-{namespace}-999"));
    std::fs::create_dir(&unbound).unwrap();
    std::fs::set_permissions(&unbound, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        worker.inventory().unwrap().unbound_claims,
        vec![format!("vm-{namespace}-999")]
    );
    std::fs::remove_dir(&unbound).unwrap();
    let orphan_network = format!("vm-{namespace}-998");
    assert!(
        Process::new("/usr/bin/ip")
            .args(["netns", "add", &orphan_network])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        worker.inventory().unwrap().unbound_networks,
        vec![orphan_network.clone()]
    );
    assert!(
        Process::new("/usr/bin/ip")
            .args(["netns", "delete", &orphan_network])
            .status()
            .unwrap()
            .success()
    );
    assert!(worker.inventory().unwrap().unbound_networks.is_empty());
}
