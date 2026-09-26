#![cfg(target_os = "linux")]

//! Opt-in, root-only KVM test. It never invents a host success receipt.
//! A failure intentionally leaves exact resource claims for inspection.

use sandbox_controller_core as core;
use sandproof_controller_service::{ControllerJournal, create_v1, kill_v1};
use sandproof_node::{LinuxWorker, NodeConfig};
use std::env;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing operator setting {name}"))
}

#[test]
#[ignore = "requires root, KVM, pinned envd image and dedicated state directory"]
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
    let running = create_v1(&mut journal, request, &mut |action| worker.execute(action))
        .unwrap_or_else(|error| panic!("create failed: {error}; host: {:?}", worker.last_error()));
    assert_eq!(
        running.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Running
    );
    let resources = running.resource_index(sandbox);
    assert_eq!(resources.vms.len(), 1);
    assert_eq!(resources.slots.len(), 1);
    let vm_name = format!("vm-{namespace}-{}", resources.vms[0].0);
    assert!(PathBuf::from(format!("/run/netns/{vm_name}")).exists());
    let killed = kill_v1(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox,
            operation: core::OperationId(2),
        },
        &mut |action| worker.execute(action),
    )
    .unwrap_or_else(|error| panic!("kill failed: {error}; host: {:?}", worker.last_error()));
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
}
