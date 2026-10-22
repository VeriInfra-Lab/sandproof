use super::*;

fn reject(mut checkpoint: Checkpoint, mutate: impl FnOnce(&mut CheckpointWire)) {
    mutate(&mut checkpoint.0);
    assert!(restore(checkpoint, LIMITS).is_err());
}

#[test]
fn rejects_unknown_version_and_invalid_or_exhausted_record_budgets() {
    let checkpoint = capture(&enabled(), LIMITS).unwrap();
    reject(checkpoint.clone(), |wire| {
        wire.version = UNLOAD_CHECKPOINT_VERSION + 1;
    });
    for limits in [
        CheckpointLimits {
            max_records: 0,
            ..LIMITS
        },
        CheckpointLimits {
            max_references: 0,
            ..LIMITS
        },
    ] {
        assert_eq!(
            capture(&initial_world(), limits),
            Err(CheckpointReject::InvalidLimits)
        );
        assert_eq!(
            restore(checkpoint.clone(), limits),
            Err(CheckpointReject::InvalidLimits)
        );
    }
    let (world, _) = live();
    let low = CheckpointLimits {
        max_records: 1,
        ..LIMITS
    };
    assert_eq!(capture(&world, low), Err(CheckpointReject::RecordLimit));
    assert_eq!(
        restore(capture(&world, LIMITS).unwrap(), low),
        Err(CheckpointReject::RecordLimit)
    );
    let low = CheckpointLimits {
        max_references: 1,
        ..LIMITS
    };
    assert_eq!(capture(&world, low), Err(CheckpointReject::ReferenceLimit));
}

#[test]
fn duplicate_keys_and_queues_are_never_collapsed_or_reordered() {
    let (world, _) = live();
    let checkpoint = capture(&world, LIMITS).unwrap();
    reject(checkpoint.clone(), |wire| {
        wire.sandboxes.push(wire.sandboxes[0].clone());
    });
    reject(checkpoint.clone(), |wire| wire.vms.push(wire.vms[0]));
    reject(checkpoint.clone(), |wire| {
        wire.operation_plans.push(wire.operation_plans[0].clone());
    });
    reject(checkpoint.clone(), |wire| {
        wire.effects.push(wire.effects[0].clone());
    });
    reject(checkpoint.clone(), |wire| {
        wire.effect_keys.push(wire.effect_keys[0].clone());
    });
    reject(checkpoint.clone(), |wire| {
        wire.inflight.push(wire.inflight[0].clone());
    });
    reject(checkpoint, |wire| {
        let state = wire.maintenance.as_mut().unwrap();
        state.slots.push(state.slots[0]);
    });
    reject(capture(&retired(), LIMITS).unwrap(), |wire| {
        let state = wire.maintenance.as_mut().unwrap();
        state.retained.push(state.retained[0].clone());
    });
}

#[test]
fn rejects_missing_or_foreign_resources_indexes_plans_and_authority() {
    let (world, _) = live();
    let checkpoint = capture(&world, LIMITS).unwrap();
    reject(checkpoint.clone(), |wire| wire.vms.clear());
    reject(checkpoint.clone(), |wire| wire.filesystems.clear());
    reject(checkpoint.clone(), |wire| wire.resource_indexes.clear());
    reject(checkpoint.clone(), |wire| {
        wire.resource_indexes[0].1.vms.push(VmId(99));
    });
    reject(checkpoint.clone(), |wire| wire.net_slots[0].1.vm = VmId(99));
    reject(checkpoint.clone(), |wire| {
        wire.net_slots[0].1.owner = TenantId(99);
    });
    reject(checkpoint.clone(), |wire| {
        wire.operation_plans[0].1.vm = Some(VmId(99));
    });
    reject(checkpoint.clone(), |wire| {
        wire.operations[0].1.sandbox = SandboxId(99);
    });
    reject(checkpoint.clone(), |wire| wire.worker_fences[0].1.epoch = 2);
    reject(checkpoint.clone(), |wire| {
        wire.sandboxes[0].1.effective = Authority::Closed;
    });
    reject(checkpoint, |wire| wire.applied_file_policies.clear());
}

#[test]
fn rejects_effect_attempt_origin_receipt_and_membership_corruption() {
    let (world, _) = live();
    let checkpoint = capture(&world, LIMITS).unwrap();
    reject(checkpoint.clone(), |wire| {
        wire.effects.pop().map(|_| ()).unwrap();
    });
    reject(checkpoint.clone(), |wire| {
        wire.inflight.pop();
    });
    reject(checkpoint.clone(), |wire| {
        wire.logical_effects[0].1.origin = EffectOrigin::Maintenance(SandboxId(10));
    });
    reject(checkpoint.clone(), |wire| {
        wire.logical_effects[0].1.current_attempt = u64::MAX;
    });
    reject(checkpoint.clone(), |wire| {
        wire.logical_effects[0].1.remaining_retries = 9;
    });
    reject(checkpoint.clone(), |wire| {
        wire.logical_effects[0].1.result = None;
    });
    reject(checkpoint, |wire| {
        wire.effects[0].1 = EffectStatus::OutcomeUnknown;
    });
}

#[test]
fn rejects_regressed_frontiers_and_mismatched_maintenance_clocks() {
    let (world, _) = live();
    let checkpoint = capture(&world, LIMITS).unwrap();
    reject(checkpoint.clone(), |wire| wire.next_vm_id = 0);
    reject(checkpoint.clone(), |wire| wire.next_slot_id = 0);
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().sandbox_frontier = None;
    });
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().workspace_frontier = Some(0);
    });
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().time_floor += 1;
    });
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().max_host_slots = 0;
    });
    reject(checkpoint, |wire| {
        wire.maintenance.as_mut().unwrap().max_retained_results = 0;
    });
}

#[test]
fn free_tombstones_may_outlive_sandbox_but_occupied_or_releasing_cannot() {
    let checkpoint = capture(&retired(), LIMITS).unwrap();
    assert!(restore(checkpoint.clone(), LIMITS).is_ok());
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().slots[0].1.phase = HostSlotPhase::Occupied;
    });
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().slots[0].1.phase = HostSlotPhase::Releasing;
    });
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().slots[0].1.binding.owner = TenantId(99);
    });
    reject(checkpoint.clone(), |wire| {
        wire.maintenance.as_mut().unwrap().retained[0].1.expires_at = 5;
    });
    reject(checkpoint, |wire| {
        wire.maintenance.as_mut().unwrap().retained[0]
            .1
            .ledger
            .operation
            .status = OperationStatus::Pending;
    });
}

#[test]
fn v1_rejects_all_snapshot_and_pause_resume_surfaces_without_discarding_them() {
    let (world, _) = live();
    let checkpoint = capture(&world, LIMITS).unwrap();
    let source = RestoreSource {
        snapshot: SnapshotId(1),
        manifest: SnapshotManifestRef {
            digest: ManifestDigest(1),
        },
    };
    reject(checkpoint.clone(), |wire| {
        wire.sandboxes[0].1.restore_from = Some(source);
    });
    reject(checkpoint.clone(), |wire| {
        wire.sandboxes[0].1.durable_snapshot = Some(source);
    });
    reject(checkpoint.clone(), |wire| {
        wire.operation_plans[0].1.target_snapshot = Some(source);
    });
    reject(checkpoint.clone(), |wire| {
        wire.resource_indexes[0].1.snapshots.push(source.snapshot);
    });
    reject(checkpoint.clone(), |wire| {
        wire.operations[0].1.operation.kind = OperationKind::Pause;
    });
    reject(checkpoint.clone(), |wire| {
        wire.operations[0].1.input = OperationInput::Resume { source }
    });
    reject(checkpoint, |wire| wire.sandboxes[0].1.phase = Phase::Paused);
    let paused = handle_controller(
        &world,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SandboxId(10),
            operation: OperationId(2),
        })),
    )
    .unwrap()
    .next;
    assert_eq!(
        capture(&paused, LIMITS),
        Err(CheckpointReject::UnsupportedSnapshotHistory)
    );
}
