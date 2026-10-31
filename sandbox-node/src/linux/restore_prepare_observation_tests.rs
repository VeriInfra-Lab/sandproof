//! Original Node guard and identity regressions; physical observations are
//! controlled inputs. The separate public/KVM representative owns real linkage.
use super::*;
use sandproof_protocol::{
    ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
    ResourceRef, SandboxId, TenantId,
};
use std::{cell::Cell, os::unix::fs::PermissionsExt};

fn candidate() -> LoadedSnapshotObservation {
    LoadedSnapshotObservation {
        process: sandbox_runtime::RuntimeProcessIdentity {
            pid: 17,
            start_time_ticks: 23,
            boot_id: "00000000-0000-0000-0000-000000000001".into(),
        },
        state: LoadedSnapshotState::Paused,
    }
}

fn fence_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.path().join("original-fence.json");
    write_fence(
        &path,
        super::super::Fence {
            epoch: 4,
            terminal: true,
        },
    );
    (root, path)
}
fn write_fence(path: &std::path::Path, fence: super::super::Fence) {
    std::fs::write(path, serde_json::to_vec(&fence).unwrap()).unwrap();
}
fn read_fence(path: &std::path::Path) -> NodeResult<super::super::Fence> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

#[test]
fn terminal_fence_keeps_original_prepared_facts_readable_without_execution() {
    let (_root, path) = fence_file();
    for epoch in [3, 4] {
        write_fence(
            &path,
            super::super::Fence {
                epoch,
                terminal: true,
            },
        );
        let calls = Cell::new(0);
        let actual = observe_prepared_under_fence(
            3,
            || read_fence(&path),
            || {
                calls.set(calls.get() + 1);
                Ok(candidate())
            },
        );
        assert!(
            actual.is_ok(),
            "original physically loaded candidate must remain observable after terminal fence: {actual:?}"
        );
        assert_eq!(actual.unwrap(), candidate());
        assert_eq!(calls.get(), 2);
        assert_eq!(
            read_fence(&path).unwrap(),
            super::super::Fence {
                epoch,
                terminal: true
            }
        );
    }
}

#[test]
fn normal_open_fence_still_observes_the_same_candidate() {
    let (_root, path) = fence_file();
    write_fence(
        &path,
        super::super::Fence {
            epoch: 3,
            terminal: false,
        },
    );
    assert_eq!(
        observe_prepared_under_fence(3, || read_fence(&path), || Ok(candidate())).unwrap(),
        candidate()
    );
}

#[test]
fn old_fence_and_new_open_fence_reject_before_any_physical_observation() {
    for fence in [
        super::super::Fence {
            epoch: 2,
            terminal: true,
        },
        super::super::Fence {
            epoch: 2,
            terminal: false,
        },
        super::super::Fence {
            epoch: 4,
            terminal: false,
        },
    ] {
        assert!(
            observe_prepared_under_fence(
                3,
                || Ok(fence),
                || panic!("stale or new nonterminal authority cannot observe old preparation")
            )
            .is_err()
        );
    }
}

#[test]
fn changed_current_fence_or_candidate_never_returns_original_prepared() {
    for part in 0..5 {
        let (_root, path) = fence_file();
        let calls = Cell::new(0);
        let result = observe_prepared_under_fence(
            3,
            || read_fence(&path),
            || {
                calls.set(calls.get() + 1);
                let mut observed = candidate();
                if calls.get() == 2 {
                    match part {
                        0 => observed.process.pid += 1,
                        1 => observed.process.start_time_ticks += 1,
                        2 => {
                            observed.process.boot_id =
                                "00000000-0000-0000-0000-000000000002".into();
                        }
                        3 => observed.state = LoadedSnapshotState::Running,
                        _ => write_fence(
                            &path,
                            super::super::Fence {
                                epoch: 5,
                                terminal: true,
                            },
                        ),
                    }
                }
                Ok(observed)
            },
        );
        assert!(result.is_err());
    }
}

#[test]
fn wrong_source_unavailable_or_exited_candidate_stays_unknown() {
    let fence = super::super::Fence {
        epoch: 4,
        terminal: true,
    };
    for fail_at in [1, 2] {
        let calls = Cell::new(0);
        let result = observe_prepared_under_fence(
            3,
            || Ok(fence),
            || {
                calls.set(calls.get() + 1);
                if calls.get() == fail_at {
                    return Err(sandbox_runtime::RuntimeError::IdentityMismatch(
                        "original source/intent/work differs or API unavailable",
                    )
                    .into());
                }
                Ok(candidate())
            },
        );
        assert!(result.is_err());
    }
    let mut exited = candidate();
    exited.state = LoadedSnapshotState::Exited;
    assert!(observe_prepared_under_fence(3, || Ok(fence), || Ok(exited.clone())).is_err());
}

fn action(
    command: core::Command,
    operation: u64,
    generation: u64,
    epoch: u64,
) -> WorkAction<core::EffectId> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: TenantId::try_from("tenant-7").unwrap(),
            sandbox: SandboxId::try_from("sb-0123456789abcdef-1").unwrap(),
            operation: OperationId::try_from(format!("op-0123456789abcdef-1-{operation}")).unwrap(),
            epoch,
            generation,
            logical_action: LogicalActionId::try_from("effect-1").unwrap(),
            attempt: 0,
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: ResourceId::try_from("vm-0123456789abcdef-2").unwrap(),
            },
        },
        command: core::EffectId::initial(command),
    }
}
fn prepare() -> WorkAction<core::EffectId> {
    action(
        core::Command::PrepareVm {
            sandbox: core::SandboxId(1),
            epoch: 3,
            generation: 2,
            vm: core::VmId(2),
            slot: core::NetSlotId(2),
            source: Some(core::RestoreSource {
                snapshot: core::SnapshotId(1),
                manifest: core::SnapshotManifestRef {
                    digest: core::ManifestDigest(7),
                },
            }),
        },
        3,
        2,
        3,
    )
}
fn seal() -> WorkAction<core::EffectId> {
    action(
        core::Command::SealSnapshot {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 1,
            vm: core::VmId(1),
            workspace: core::FsId(1),
            snapshot: core::SnapshotId(1),
            manifest: core::SnapshotManifestRef {
                digest: core::ManifestDigest(7),
            },
        },
        2,
        1,
        2,
    )
}

#[test]
fn terminal_exception_never_selects_load_execution_activation_or_sourceless_prepare() {
    let action = prepare();
    assert!(readonly_restore_prepare(action.command.command(), true));
    assert!(!readonly_restore_prepare(action.command.command(), false));
    assert!(!readonly_restore_prepare(
        &core::Command::ActivateVm {
            sandbox: core::SandboxId(1),
            epoch: 3,
            generation: 2,
            vm: core::VmId(2)
        },
        true
    ));
    let mut command = action.command.command().clone();
    if let core::Command::PrepareVm { source, .. } = &mut command {
        *source = None;
    }
    assert!(!readonly_restore_prepare(&command, true));
}

#[test]
fn original_scope_rejects_wrong_snapshot_manifest_owner_operation_and_old_generation() {
    let source = seal();
    let target = prepare();
    let check = |action: &WorkAction<core::EffectId>| {
        scope_target(
            action,
            &source,
            core::OperationId(3),
            2,
            "0123456789abcdef",
            None,
        )
    };
    assert!(check(&target).is_ok());
    for part in 0..5 {
        let mut changed = target.clone();
        match part {
            0 | 1 => {
                let mut command = changed.command.command().clone();
                if let core::Command::PrepareVm {
                    source: Some(ref mut value),
                    ..
                } = command
                {
                    if part == 0 {
                        value.snapshot = core::SnapshotId(999);
                    } else {
                        value.manifest.digest = core::ManifestDigest(999);
                    }
                }
                changed.command = core::EffectId::initial(command);
            }
            2 => changed.identity.owner = TenantId::try_from("tenant-8").unwrap(),
            3 => {
                changed.identity.operation =
                    OperationId::try_from("op-0123456789abcdef-1-4").unwrap();
            }
            _ => {
                let mut command = changed.command.command().clone();
                if let core::Command::PrepareVm { generation, .. } = &mut command {
                    *generation = 1;
                }
                changed.command = core::EffectId::initial(command);
                changed.identity.generation = 1;
            }
        }
        assert!(check(&changed).is_err());
    }
}
