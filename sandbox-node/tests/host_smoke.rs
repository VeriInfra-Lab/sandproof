#![cfg(target_os = "linux")]

//! Opt-in, root-only KVM test. It never invents a host success receipt.
//! A failure intentionally leaves exact resource claims for inspection.

use rustix::process::{Signal, getpid, kill_process};
use sandbox_controller_core as core;
use sandproof_controller_service::{
    ControllerJournal, CrashInjector, CrashPoint, DispatchResult, JournalError, NeverCrash,
    create_v1, finish_accepted_kill_v1, finish_admitted_create_v1, kill_v1,
};
use sandproof_node::{LinuxWorker, NodeConfig};
use sandproof_protocol::{TenantId, UnknownReason, WorkOutcome, WorkReceipt};
use std::env;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing operator setting {name}"))
}

fn test_config() -> NodeConfig {
    NodeConfig {
        namespace: required("SANDPROOF_TEST_NAMESPACE"),
        state_dir: PathBuf::from(required("SANDPROOF_TEST_STATE_DIR")),
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
    }
}

fn test_token() -> String {
    let mut random = [0_u8; 32];
    File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut random)
        .unwrap();
    let mut token = String::with_capacity(64);
    for value in random {
        write!(&mut token, "{value:02x}").unwrap();
    }
    token
}

fn update_guest_guard(namespace: &str, script: &str) {
    let mut child = Process::new("/usr/bin/ip")
        .args(["netns", "exec", namespace, "/usr/sbin/nft", "-f", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

fn test_create_request(config: &NodeConfig) -> core::CreateRequest {
    core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: core::SandboxId(1),
        workspace: core::FsId(1),
        file_policy: core::FilePolicyRef {
            digest: config.file_policy_digest,
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(1),
    }
}

#[test]
#[ignore = "requires root, KVM, pinned envd image and dedicated state directory"]
#[expect(
    clippy::too_many_lines,
    reason = "one real VM is carried through two deliberately lost receipts, restart reconciliation and exact cleanup"
)]
fn create_and_kill_use_real_host_receipts() {
    let config = test_config();
    let namespace = config.namespace.clone();
    let state_dir = config.state_dir.clone();
    let mut worker = LinuxWorker::new(config.clone()).unwrap();
    let mut journal = ControllerJournal::open(state_dir.join("controller.db"), &namespace).unwrap();
    let token = test_token();
    let sandbox = core::SandboxId(1);
    worker.put_secret(sandbox, &token).unwrap();
    let request = test_create_request(&config);
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
    let late_prepare = worker.observe_action(&uncertain[0]).unwrap();
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox,
                operation: core::OperationId(2),
            })),
            &mut NeverCrash,
        ),
        Err(JournalError::UnresolvedAction)
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
    assert_eq!(
        journal.complete(&late_prepare, &mut NeverCrash).unwrap(),
        running
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
    // Even an extra restrictive rule is a changed policy. The next startup
    // inventory must reject it, rather than treating the namespace inode as
    // proof that the original guard still exists. Both changes are atomic
    // nft transactions; the test never opens an unguarded interval.
    update_guest_guard(
        &vm_name,
        "add rule inet sandproof_guard guest_out iifname \"tap0\" drop\n",
    );
    assert!(worker.inventory().is_err());
    update_guest_guard(
        &vm_name,
        "flush chain inet sandproof_guard guest_out\n\
         add rule inet sandproof_guard guest_out iifname \"tap0\" ct state established,related accept\n\
         add rule inet sandproof_guard guest_out iifname \"tap0\" drop\n",
    );
    assert_eq!(worker.inventory().unwrap().slots.len(), 1);
    let lost = kill_v1(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox,
            operation: core::OperationId(2),
        },
        &mut |action| {
            if matches!(
                action.command.command(),
                core::Command::StopVm { .. }
                    | core::Command::CleanupVm { .. }
                    | core::Command::CleanupSlot { .. }
            ) {
                let mut wrong_owner = action.clone();
                wrong_owner.identity.owner = TenantId::try_from("tenant-999").unwrap();
                assert!(worker.observe_action(&wrong_owner).is_err());
                assert!(matches!(
                    worker.execute(&wrong_owner).unwrap().outcome,
                    WorkOutcome::Unknown { .. }
                ));
            }
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
    let late_stop = worker.observe_action(&uncertain[0]).unwrap();
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
    assert_eq!(
        journal.complete(&late_stop, &mut NeverCrash).unwrap(),
        killed
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

struct TerminateAfterWorker {
    sigkill: bool,
}

impl CrashInjector for TerminateAfterWorker {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == CrashPoint::AfterWorkerCall {
            // Neither termination path runs destructors or commits a receipt.
            if self.sigkill {
                kill_process(getpid(), Signal::KILL).unwrap();
                unreachable!("SIGKILL returned to the worker");
            }
            std::process::exit(91);
        }
        Ok(())
    }
}

fn exit_child(stage: &str, sigkill: bool) {
    let config = test_config();
    let mut worker = LinuxWorker::new(config.clone()).unwrap();
    let mut journal =
        ControllerJournal::open(config.state_dir.join("controller.db"), &config.namespace).unwrap();
    journal
        .dispatch_next(
            &mut |action| {
                assert!(matches!(
                    (stage, action.command.command()),
                    ("prepare", core::Command::PrepareVm { .. })
                        | ("stop", core::Command::StopVm { .. })
                ));
                worker.execute(action)
            },
            &mut TerminateAfterWorker { sigkill },
        )
        .unwrap();
    panic!("worker child did not exit at the injected boundary");
}

fn spawn_exit_child(stage: &str, test_name: &str, sigkill: bool) {
    let output = Process::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--ignored", "--nocapture"])
        .env("SANDPROOF_HOST_EXIT_CHILD", stage)
        .output()
        .unwrap();
    if sigkill {
        assert_eq!(output.status.signal(), Some(9), "{stage}: {output:?}");
    } else {
        assert_eq!(output.status.code(), Some(91), "{stage}: {output:?}");
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "real worker process terminates separately after VM launch and exact stop, before either receipt is committed"
)]
fn worker_termination_reconciles_prepare_and_stop(test_name: &str, sigkill: bool) {
    if let Ok(stage) = env::var("SANDPROOF_HOST_EXIT_CHILD") {
        exit_child(&stage, sigkill);
        return;
    }
    let config = test_config();
    let sandbox = core::SandboxId(1);
    let mut worker = LinuxWorker::new(config.clone()).unwrap();
    worker.put_secret(sandbox, &test_token()).unwrap();
    let mut journal =
        ControllerJournal::open(config.state_dir.join("controller.db"), &config.namespace).unwrap();
    journal
        .apply(
            core::ControllerInput::Request(core::Request::Create(test_create_request(&config))),
            &mut NeverCrash,
        )
        .unwrap();
    let create_key = core::OperationKey {
        sandbox,
        operation: core::OperationId(1),
    };
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(create_key)),
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(
        journal
            .dispatch_next(&mut |action| worker.execute(action), &mut NeverCrash)
            .unwrap(),
        DispatchResult::ReceiptRecorded(_)
    ));
    let plan = journal
        .world()
        .unwrap()
        .operation_plan(create_key)
        .unwrap()
        .clone();
    let vm = plan.vm.unwrap();
    let slot = plan.slot.unwrap();
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
                sandbox, vm, slot,
            )),
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal);
    drop(worker);
    spawn_exit_child("prepare", test_name, sigkill);

    let mut worker = LinuxWorker::new(config.clone()).unwrap();
    let mut journal =
        ControllerJournal::open(config.state_dir.join("controller.db"), &config.namespace).unwrap();
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::PrepareVm { .. }
    ));
    let late_prepare = worker.observe_action(&uncertain[0]).unwrap();
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox,
                operation: core::OperationId(2),
            })),
            &mut NeverCrash,
        ),
        Err(JournalError::UnresolvedAction)
    ));
    let inventory = worker.inventory().unwrap();
    assert_eq!(inventory.slots.len(), 1);
    assert_eq!(inventory.slots[0].vm, vm);
    assert_eq!(
        inventory.slots[0].process.as_ref().unwrap().state,
        sandbox_runtime::VmState::VmmRunning
    );
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
        .unwrap_or_else(|error| panic!("resumed create: {error}; host: {:?}", worker.last_error()));
    assert!(!repeated_prepare);
    assert_eq!(
        running.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Running
    );
    assert_eq!(
        journal.complete(&late_prepare, &mut NeverCrash).unwrap(),
        running
    );

    journal
        .apply(
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox,
                operation: core::OperationId(2),
            })),
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(
        journal
            .dispatch_next(&mut |action| worker.execute(action), &mut NeverCrash)
            .unwrap(),
        DispatchResult::ReceiptRecorded(_)
    ));
    let world = journal.world().unwrap();
    let record = world.sandbox_record(sandbox).unwrap();
    let kill_key = core::OperationKey {
        sandbox,
        operation: core::OperationId(2),
    };
    let stop_vm = world.operation_plan(kill_key).unwrap().stop_vms[0];
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
                sandbox,
                record.generation,
                stop_vm,
            )),
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal);
    drop(worker);
    spawn_exit_child("stop", test_name, sigkill);

    let mut worker = LinuxWorker::new(config.clone()).unwrap();
    let mut journal =
        ControllerJournal::open(config.state_dir.join("controller.db"), &config.namespace).unwrap();
    let uncertain = journal.uncertain_actions().unwrap();
    assert_eq!(uncertain.len(), 1);
    assert!(matches!(
        uncertain[0].command.command(),
        core::Command::StopVm { .. }
    ));
    let late_stop = worker.observe_action(&uncertain[0]).unwrap();
    assert_eq!(
        worker.inventory().unwrap().slots[0]
            .process
            .as_ref()
            .unwrap()
            .state,
        sandbox_runtime::VmState::Stopped
    );
    assert_eq!(
        journal
            .reconcile_uncertain(&mut |action| worker.observe_action(action).ok())
            .unwrap(),
        1
    );
    let mut repeated_stop = false;
    let destroyed =
        finish_accepted_kill_v1(&mut journal, sandbox, core::OperationId(2), &mut |action| {
            repeated_stop |= matches!(action.command.command(), core::Command::StopVm { .. });
            worker.execute(action)
        })
        .unwrap_or_else(|error| panic!("resumed kill: {error}; host: {:?}", worker.last_error()));
    assert!(!repeated_stop);
    assert_eq!(
        destroyed.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    assert_eq!(
        journal.complete(&late_stop, &mut NeverCrash).unwrap(),
        destroyed
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
    let inventory = worker.inventory().unwrap();
    assert!(inventory.unbound_claims.is_empty());
    assert!(inventory.unbound_networks.is_empty());
    assert_eq!(inventory.slots.len(), 1);
    assert!(inventory.slots[0].network_inode.is_none());
    assert!(inventory.slots[0].disk_inode.is_none());
    assert!(!PathBuf::from(format!("/run/netns/vm-{}-{}", config.namespace, vm.0)).exists());
}

#[test]
#[ignore = "requires root, KVM, pinned envd image and dedicated state directory"]
fn real_worker_process_exit_reconciles_prepare_and_stop() {
    worker_termination_reconciles_prepare_and_stop(
        "real_worker_process_exit_reconciles_prepare_and_stop",
        false,
    );
}

#[test]
#[ignore = "requires root, KVM, pinned envd image and dedicated state directory"]
fn real_worker_sigkill_reconciles_prepare_and_stop() {
    worker_termination_reconciles_prepare_and_stop(
        "real_worker_sigkill_reconciles_prepare_and_stop",
        true,
    );
}
