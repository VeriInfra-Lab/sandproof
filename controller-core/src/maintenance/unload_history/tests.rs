use super::*;
use crate::{
    ManifestDigest, NetSlotId, Operation, OperationId, OperationLedgerRecord, OperationPlan,
    SandboxId, SnapshotId, SnapshotManifestRef, TenantId, VmId, capture::CaptureHandoff,
};

#[test]
fn completed_pause_search_stops_at_first_matching_row_even_if_its_plan_is_missing() {
    let binding = HostBinding {
        owner: TenantId(1),
        sandbox: SandboxId(10),
        generation: 1,
        vm: VmId(3),
        network: NetSlotId(4),
    };
    let source = RestoreSource {
        snapshot: SnapshotId(5),
        manifest: SnapshotManifestRef {
            digest: ManifestDigest(6),
        },
    };
    let bad = OperationKey {
        sandbox: binding.sandbox,
        operation: OperationId(11),
    };
    let good = OperationKey {
        operation: OperationId(12),
        ..bad
    };
    let ledger = |key: OperationKey| OperationLedgerRecord {
        sandbox: binding.sandbox,
        operation: Operation {
            id: key.operation,
            epoch: 7,
            kind: OperationKind::Pause,
            status: OperationStatus::Succeeded,
        },
        input: OperationInput::Pause {
            generation: binding.generation,
        },
        result: Some(OperationResult::Paused(source)),
    };
    let mut world = World::default();
    world.operation_plans.put(
        good,
        OperationPlan {
            key: good,
            epoch: 7,
            generation: binding.generation,
            vm: Some(binding.vm),
            slot: Some(binding.network),
            restore_from: None,
            target_snapshot: Some(source),
            capture: Some(
                CaptureHandoff::accepted()
                    .register_bundle()
                    .unwrap()
                    .terminate(),
            ),
            stop_vms: vec![binding.vm],
            close_slots: vec![binding.network],
            retry_limit: 3,
            sealed: true,
        },
    );
    for first_is_good in [false, true] {
        world.operations = crate::Store::default();
        let order = if first_is_good {
            [good, bad]
        } else {
            [bad, good]
        };
        for key in order {
            world.operations.put(key, ledger(key));
        }
        assert_eq!(
            completed_pause_epoch(&world, binding, 8, source),
            first_is_good.then_some(7)
        );
    }
}
