//! Pure comparisons, not authority to execute or release a physical resource.
//! Callers must obtain fresh evidence under the original host/application lock
//! and retain all adjacent I/O checks. These functions never collect evidence.
use super::{
    Fence, HostObservation, NodeResult, ObservedResult, ResourceId, ResourceKind, ResourceRef,
    SlotBinding, WorkAction, core,
};

pub(super) fn fence_can_advance(prior: Fence, next: Fence) -> bool {
    prior.epoch <= next.epoch && (next.terminal || !prior.terminal)
}

pub(super) fn fence_matches(observed: Fence, epoch: core::Epoch, terminal: bool) -> bool {
    observed.epoch == epoch && observed.terminal == terminal
}

pub(super) fn command_generation(command: &core::Command) -> Option<core::Generation> {
    match command {
        core::Command::PrepareVm { generation, .. }
        | core::Command::AbortPrepare { generation, .. }
        | core::Command::ConfigureNetwork { generation, .. }
        | core::Command::ActivateVm { generation, .. }
        | core::Command::FreezeVm { generation, .. }
        | core::Command::SealSnapshot { generation, .. }
        | core::Command::ApplyFilePolicy { generation, .. }
        | core::Command::StopVm { generation, .. }
        | core::Command::CleanupVm { generation, .. }
        | core::Command::CleanupSlot { generation, .. } => Some(*generation),
        _ => None,
    }
}

pub(super) fn action_fields_match(action: &WorkAction<core::EffectId>, namespace: &str) -> bool {
    let command = action.command.command();
    action.identity.attempt == action.command.attempt
        && action.identity.epoch == command.epoch().unwrap_or(action.identity.epoch)
        && command_generation(command).is_none_or(|value| value == action.identity.generation)
        && action.identity.sandbox.as_str() == format!("sb-{namespace}-{}", command.sandbox().0)
}

pub(super) fn slot_coordinates_match(
    binding: &SlotBinding,
    sandbox: core::SandboxId,
    generation: core::Generation,
    vm: core::VmId,
    slot: core::NetSlotId,
) -> bool {
    binding.sandbox == sandbox
        && binding.generation == generation
        && binding.vm == vm
        && binding.slot == slot
}

pub(super) fn process_identity_matches(
    before: &super::Observation,
    after: &super::Observation,
) -> bool {
    before.pid == after.pid && before.start_time_ticks == after.start_time_ticks
}

pub(super) fn evidence_matches(observation: &HostObservation, command: &core::Command) -> bool {
    if &observation.command != command {
        return false;
    }
    match (command, &observation.result) {
        (
            core::Command::InstallFence {
                epoch, terminal, ..
            },
            ObservedResult::Fence(fence),
        ) => fence.epoch == *epoch && fence.terminal == *terminal,
        (
            core::Command::PrepareVm { .. },
            ObservedResult::Prepared {
                pid, start_ticks, ..
            },
        )
        | (
            core::Command::ActivateVm { .. },
            ObservedResult::Activated {
                pid, start_ticks, ..
            },
        )
        | (core::Command::StopVm { .. }, ObservedResult::Stopped { pid, start_ticks }) => {
            *pid != 0 && *start_ticks != 0
        }
        (
            core::Command::FreezeVm { .. },
            ObservedResult::Frozen {
                pid,
                start_ticks,
                boot_id,
            },
        ) => sandbox_runtime::RuntimeProcessIdentity {
            pid: *pid,
            start_time_ticks: *start_ticks,
            boot_id: boot_id.clone(),
        }
        .validate_fields()
        .is_ok(),
        (core::Command::ConfigureNetwork { .. }, ObservedResult::Network { namespace_inode })
        | (
            core::Command::ApplyFilePolicy { .. },
            ObservedResult::FilePolicy {
                disk_inode: namespace_inode,
            },
        ) => *namespace_inode != 0,
        (core::Command::CleanupVm { .. }, ObservedResult::VmCleaned)
        | (core::Command::CleanupSlot { .. }, ObservedResult::SlotClosed)
        | (core::Command::AbortPrepare { .. }, ObservedResult::PrepareAborted) => true,
        _ => false,
    }
}

pub(super) fn command_resource(
    command: &core::Command,
    namespace: &str,
) -> NodeResult<ResourceRef> {
    let (kind, prefix, number) = match command {
        core::Command::InstallFence { sandbox, .. } => (ResourceKind::Sandbox, "sb", sandbox.0),
        core::Command::PrepareVm { vm, .. }
        | core::Command::AbortPrepare { vm, .. }
        | core::Command::ActivateVm { vm, .. }
        | core::Command::FreezeVm { vm, .. }
        | core::Command::StopVm { vm, .. }
        | core::Command::CleanupVm { vm, .. } => (ResourceKind::Vm, "vm", vm.0),
        core::Command::ConfigureNetwork { slot, .. } | core::Command::CleanupSlot { slot, .. } => {
            (ResourceKind::NetworkSlot, "slot", slot.0)
        }
        core::Command::SealSnapshot { snapshot, .. } => {
            (ResourceKind::Snapshot, "snapshot", snapshot.0)
        }
        core::Command::ApplyFilePolicy { workspace, .. } => {
            (ResourceKind::Workspace, "fs", workspace.0)
        }
        _ => return Err("unsupported host resource command".into()),
    };
    Ok(ResourceRef {
        kind,
        id: ResourceId::try_from(format!("{prefix}-{namespace}-{number}"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_protocol::{
        ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
        ResourceRef, SandboxId, TenantId,
    };

    fn action(command: core::Command) -> WorkAction<core::EffectId> {
        WorkAction {
            identity: ActionIdentity {
                protocol_version: PROTOCOL_VERSION,
                owner: TenantId::try_from("tenant-7").unwrap(),
                sandbox: SandboxId::try_from("sb-test-1").unwrap(),
                operation: OperationId::try_from("op-test-1").unwrap(),
                epoch: 2,
                generation: 3,
                logical_action: LogicalActionId::try_from("effect-1").unwrap(),
                attempt: 0,
                resource: ResourceRef {
                    kind: ResourceKind::Vm,
                    id: ResourceId::try_from("vm-test-4").unwrap(),
                },
            },
            command: core::EffectId::initial(command),
        }
    }

    #[test]
    fn fields_reject_old_attempt_epoch_generation_and_noncanonical_sandbox() {
        let original = action(core::Command::ActivateVm {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 3,
            vm: core::VmId(4),
        });
        assert!(action_fields_match(&original, "test"));
        for field in ["attempt", "epoch", "generation"] {
            let mut changed = serde_json::to_value(&original).unwrap();
            changed["identity"][field] = serde_json::json!(99);
            let changed = serde_json::from_value(changed).unwrap();
            assert!(!action_fields_match(&changed, "test"), "{field}");
        }
        let mut changed = original.clone();
        changed.identity.sandbox = SandboxId::try_from("sb-test-01").unwrap();
        assert!(!action_fields_match(&changed, "test"));
        assert!(!action_fields_match(&original, "other"));
        // Matching these fields is deliberately not a resource/owner/currentness certificate.
    }

    #[test]
    fn freeze_and_full_retain_original_generation_and_distinct_resource_roles() {
        let freeze = core::Command::FreezeVm {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 3,
            vm: core::VmId(4),
        };
        let seal = core::Command::SealSnapshot {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 3,
            vm: core::VmId(4),
            workspace: core::FsId(5),
            snapshot: core::SnapshotId(6),
            manifest: core::SnapshotManifestRef {
                digest: core::ManifestDigest(7),
            },
        };
        for command in [&freeze, &seal] {
            let mut work = action(command.clone());
            assert!(action_fields_match(&work, "test"));
            work.identity.generation += 1;
            assert!(!action_fields_match(&work, "test"));
        }
        let resource = command_resource(&freeze, "test").unwrap();
        assert_eq!(resource.kind, ResourceKind::Vm);
        assert_eq!(resource.id.as_str(), "vm-test-4");
        let resource = command_resource(&seal, "test").unwrap();
        assert_eq!(resource.kind, ResourceKind::Snapshot);
        assert_eq!(resource.id.as_str(), "snapshot-test-6");
    }

    #[test]
    fn cleanup_without_epoch_still_checks_generation_and_attempt() {
        let mut cleanup = action(core::Command::CleanupVm {
            sandbox: core::SandboxId(1),
            generation: 3,
            vm: core::VmId(4),
        });
        cleanup.identity.epoch = 99;
        assert!(action_fields_match(&cleanup, "test"));
        cleanup.identity.generation = 2;
        assert!(!action_fields_match(&cleanup, "test"));
    }

    #[test]
    fn exact_slot_coordinates_never_accept_a_reused_position_binding() {
        let original = SlotBinding {
            owner: TenantId::try_from("tenant-7").unwrap(),
            sandbox: core::SandboxId(1),
            generation: 3,
            vm: core::VmId(4),
            slot: core::NetSlotId(5),
        };
        assert!(slot_coordinates_match(
            &original,
            core::SandboxId(1),
            3,
            core::VmId(4),
            core::NetSlotId(5)
        ));
        for field in ["sandbox", "generation", "vm", "slot"] {
            let mut changed = serde_json::to_value(&original).unwrap();
            changed[field] = serde_json::json!(99);
            let changed = serde_json::from_value(changed).unwrap();
            assert!(
                !slot_coordinates_match(
                    &changed,
                    core::SandboxId(1),
                    3,
                    core::VmId(4),
                    core::NetSlotId(5)
                ),
                "{field}"
            );
        }
    }

    #[test]
    fn terminal_evidence_cannot_validate_another_command_or_reopen_fence() {
        let command = core::Command::CleanupVm {
            sandbox: core::SandboxId(1),
            generation: 3,
            vm: core::VmId(4),
        };
        let action = action(command.clone());
        let mut observation = HostObservation {
            resource: action.identity.resource,
            command: command.clone(),
            result: ObservedResult::VmCleaned,
        };
        assert!(evidence_matches(&observation, &command));
        let mut changed = command.clone();
        if let core::Command::CleanupVm { generation, .. } = &mut changed {
            *generation = 2;
        }
        assert!(!evidence_matches(&observation, &changed));
        observation.result = ObservedResult::SlotClosed;
        assert!(!evidence_matches(&observation, &command));
        let closed = Fence {
            epoch: 2,
            terminal: true,
        };
        assert!(fence_matches(closed, 2, true));
        assert!(!fence_matches(closed, 2, false));
        assert!(!fence_matches(closed, 1, true));
    }

    #[test]
    fn resource_mapping_binds_kind_namespace_and_exact_number() {
        let vm = core::Command::CleanupVm {
            sandbox: core::SandboxId(1),
            generation: 3,
            vm: core::VmId(4),
        };
        let slot = core::Command::CleanupSlot {
            sandbox: core::SandboxId(1),
            generation: 3,
            slot: core::NetSlotId(4),
        };
        let resource = command_resource(&vm, "test").unwrap();
        assert_eq!(resource.kind, ResourceKind::Vm);
        assert_eq!(resource.id.as_str(), "vm-test-4");
        assert_ne!(resource, command_resource(&slot, "test").unwrap());
        assert_ne!(resource, command_resource(&vm, "other").unwrap());
    }

    #[test]
    fn process_comparison_detects_pid_reuse_not_just_pid_change() {
        let before = super::super::Observation {
            resource_id: "vm-test-4".into(),
            pid: 12,
            start_time_ticks: 100,
            state: super::super::VmState::VmmRunning,
        };
        assert!(process_identity_matches(&before, &before));
        let mut after = before.clone();
        after.start_time_ticks = 101;
        assert!(!process_identity_matches(&before, &after));
        after = before.clone();
        after.pid = 13;
        assert!(!process_identity_matches(&before, &after));
        // Both running observations and resource binding are checked by the caller.
    }
}
