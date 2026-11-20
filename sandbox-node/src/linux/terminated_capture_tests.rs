//! Native binding guards and actual ordinary child exit. Not a VMM, Full
//! worker handoff, protected slot-gate observation or end-to-end drain proof.
use super::*;
use sandproof_protocol::{
    ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
    ResourceRef, SandboxId, TenantId, lifecycle::ContentDigest,
};
const NS: &str = "0123456789abcdef";
fn action(command: core::Command, operation: u64, name: &str) -> WorkAction<core::EffectId> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: TenantId::try_from("tenant-7").unwrap(),
            sandbox: SandboxId::try_from(format!("sb-{NS}-0")).unwrap(),
            operation: OperationId::try_from(format!("op-{NS}-0-{operation}")).unwrap(),
            epoch: command.epoch().unwrap(),
            generation: 1,
            logical_action: LogicalActionId::try_from(name).unwrap(),
            attempt: 0,
            resource: match &command {
                core::Command::SealSnapshot { snapshot, .. } => ResourceRef {
                    kind: ResourceKind::Snapshot,
                    id: ResourceId::try_from(format!("snapshot-{NS}-{}", snapshot.0)).unwrap(),
                },
                core::Command::StopVm { vm, .. } => ResourceRef {
                    kind: ResourceKind::Vm,
                    id: ResourceId::try_from(format!("vm-{NS}-{}", vm.0)).unwrap(),
                },
                _ => unreachable!(),
            },
        },
        command: core::EffectId::initial(command),
    }
}
fn pair() -> (
    core::capture::OriginalContinueIdentity,
    WorkAction<core::EffectId>,
    WorkAction<core::EffectId>,
    core::StopCoverage,
) {
    let id = core::capture::OriginalContinueIdentity {
        key: core::OperationKey {
            sandbox: core::SandboxId(0),
            operation: core::OperationId(2),
        },
        epoch: 2,
        generation: 1,
        vm: core::VmId(0),
    };
    let seal = action(
        core::Command::SealSnapshot {
            sandbox: id.key.sandbox,
            epoch: id.epoch,
            generation: id.generation,
            vm: id.vm,
            workspace: core::FsId(0),
            snapshot: core::SnapshotId(0),
            manifest: core::SnapshotManifestRef {
                digest: core::ManifestDigest(0),
            },
        },
        2,
        "effect-seal",
    );
    let stop = action(
        core::Command::StopVm {
            sandbox: id.key.sandbox,
            epoch: 3,
            generation: 1,
            vm: id.vm,
        },
        9,
        "effect-stop",
    );
    let coverage = core::StopCoverage {
        operation: core::OperationKey {
            sandbox: id.key.sandbox,
            operation: core::OperationId(9),
        },
        epoch: 3,
        attempt: 0,
        result: core::ResultDigest(1),
    };
    (id, seal, stop, coverage)
}
#[test]
fn terminal_pair_binds_original_source_and_separate_later_stop_without_rejecting_zero_ids() {
    let (id, seal, stop, coverage) = pair();
    assert_ne!(seal.identity.resource, stop.identity.resource);
    assert!(require_terminal_pair(NS, id, &seal, &stop, coverage).is_ok());
    let mut wrong_seal = seal.clone();
    wrong_seal.identity.resource = stop.identity.resource.clone();
    assert!(require_terminal_pair(NS, id, &wrong_seal, &stop, coverage).is_err());
    for field in [
        "original-key",
        "original-epoch",
        "generation",
        "vm",
        "stop-key",
        "stop-epoch",
        "stop-attempt",
        "owner",
        "resource",
    ] {
        let mut changed_id = id;
        let mut changed_stop = stop.clone();
        let mut changed = coverage;
        match field {
            "original-key" => changed_id.key.operation.0 += 1,
            "original-epoch" => changed_id.epoch += 1,
            "generation" => changed_id.generation += 1,
            "vm" => changed_id.vm.0 += 1,
            "stop-key" => changed.operation.operation.0 += 1,
            "stop-epoch" => changed.epoch += 1,
            "stop-attempt" => changed.attempt += 1,
            "owner" => changed_stop.identity.owner = TenantId::try_from("tenant-8").unwrap(),
            "resource" => {
                changed_stop.identity.resource.id =
                    ResourceId::try_from(format!("vm-{NS}-1")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            require_terminal_pair(NS, changed_id, &seal, &changed_stop, changed).is_err(),
            "accepted {field}"
        );
    }
}
#[test]
fn terminal_worker_probe_rejects_a_live_or_foreign_boot_worker_and_accepts_actual_exit() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("/bin/sh")
        .args(["-c", "read token; exit 0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let process = RuntimeProcessIdentity::capture(child.id()).unwrap();
    let worker = CaptureWorkerBinding::new(
        sandproof_controller_service::capture::SourceProcessIdentity {
            pid: process.pid,
            start_time_ticks: process.start_time_ticks,
            boot_id: process.boot_id.clone(),
        },
        ContentDigest::from_sha256([29; 32]),
    )
    .unwrap();
    assert!(require_worker_ended(Some(&worker), &process.boot_id).is_err());
    child.stdin.take().unwrap().write_all(b"done\n").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(require_worker_ended(Some(&worker), "00000000-0000-0000-0000-000000000000").is_err());
    assert!(require_worker_ended(Some(&worker), &process.boot_id).is_ok());
}
