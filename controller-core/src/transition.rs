use crate::model::{
    Command, EffectId, EffectOrigin, EffectStatus, FailureKind, Filesystem, LogicalEffectId,
    LogicalEffectRecord, Operation, OperationInput, OperationKey, OperationLedgerRecord,
    OperationPlan, OperationResult, OperationStatus, RestoreSource, SandboxRecord,
    SnapshotManifestRef, World,
};
use crate::{
    Actor, Authority, FilePolicyRef, FsId, NetworkPolicy, OperationKind, Phase, SandboxId,
    SnapshotId, TenantId, VmId, WorkspaceApiAccess,
};

pub const DEFAULT_RETRY_LIMIT: u64 = 3;

#[must_use]
pub const fn operation_key(sandbox: SandboxId, operation: crate::OperationId) -> OperationKey {
    OperationKey { sandbox, operation }
}

/// Projection from the original bound ledger, never an adapter-supplied flag.
#[must_use]
pub fn current_capture(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
) -> Option<crate::capture::CaptureHandoff> {
    let operation = record.current_operation?;
    if operation.kind != OperationKind::Pause || operation.epoch != record.epoch {
        return None;
    }
    let key = operation_key(sandbox, operation.id);
    let plan = world.operation_plans.get(&key)?;
    if plan.key != key || plan.epoch != record.epoch || plan.generation != record.generation {
        return None;
    }
    plan.capture
}

#[must_use]
pub fn authorized(actor: &Actor, owner: TenantId) -> bool {
    actor == &Actor::Tenant(owner) || actor == &Actor::SystemController
}

fn insert_unique<T: Clone + PartialEq>(values: &mut Vec<T>, value: T) {
    let mut index = 0;
    let mut found = false;
    while index < values.len() && !found {
        found = values[index] == value;
        index += 1;
    }
    if found {
        return;
    }
    let mut updated = Vec::new();
    updated.push(value);
    let mut old_index = 0;
    while old_index < values.len() {
        updated.push(values[old_index].clone());
        old_index += 1;
    }
    *values = updated;
}

#[must_use]
pub fn initial_operation_plan(
    world: &World,
    sandbox: SandboxId,
    operation: Operation,
    input: &OperationInput,
) -> OperationPlan {
    let index = world.resource_index(sandbox);
    let generation = match input {
        OperationInput::Create { .. } => 1,
        OperationInput::Pause { generation } | OperationInput::Kill { generation } => *generation,
        OperationInput::Resume { .. } => match world.sandboxes.get(&sandbox) {
            Some(record) => record.generation,
            None => 0,
        },
    };
    let restore_from = match input {
        OperationInput::Resume { source } => Some(*source),
        OperationInput::Create { .. }
        | OperationInput::Pause { .. }
        | OperationInput::Kill { .. } => None,
    };

    OperationPlan {
        key: operation_key(sandbox, operation.id),
        epoch: operation.epoch,
        generation,
        vm: None,
        slot: None,
        restore_from,
        target_snapshot: None,
        capture: if operation.kind == OperationKind::Pause {
            Some(crate::capture::CaptureHandoff::accepted())
        } else {
            None
        },
        stop_vms: index.vms,
        close_slots: index.slots,
        retry_limit: DEFAULT_RETRY_LIMIT,
        sealed: operation.kind == OperationKind::Kill,
    }
}

// Terminal Resume compensation does not acquire the already reclaimed predecessor's
// cleanup work. History stays indexed and all global publication checks still
// require its original Stop evidence. Missing, live or current-generation
// resources remain obligations rather than being inferred closed.
fn resume_cleanup_index(
    world: &World,
    sandbox: SandboxId,
    generation: crate::Generation,
    mut index: crate::ResourceIndex,
) -> crate::ResourceIndex {
    let mut vms = Vec::new();
    let mut cursor = 0;
    while cursor < index.vms.len() {
        let id = index.vms[cursor];
        let closed_history = world.vm_record(id).is_some_and(|vm| {
            vm.sandbox == sandbox
                && vm.generation < generation
                && vm.status == crate::VmStatus::Reclaimed
        });
        if !closed_history {
            vms.push(id);
        }
        cursor += 1;
    }
    let mut slots = Vec::new();
    cursor = 0;
    while cursor < index.slots.len() {
        let id = index.slots[cursor];
        let closed_history = world.slot_record(id).is_some_and(|slot| {
            slot.sandbox == sandbox
                && slot.generation < generation
                && slot.reclaimed
                && !slot.active
                && world.vm_record(slot.vm).is_some_and(|vm| {
                    vm.sandbox == sandbox
                        && vm.owner == slot.owner
                        && vm.generation == slot.generation
                        && vm.status == crate::VmStatus::Reclaimed
                })
        });
        if !closed_history {
            slots.push(id);
        }
        cursor += 1;
    }
    index.vms = vms;
    index.slots = slots;
    index
}

pub fn store_new_operation(
    world: &mut World,
    sandbox: SandboxId,
    operation: Operation,
    input: OperationInput,
) {
    let key = operation_key(sandbox, operation.id);
    let plan = initial_operation_plan(world, sandbox, operation, &input);
    world.operations.put(
        key,
        OperationLedgerRecord {
            sandbox,
            operation,
            input,
            result: None,
        },
    );
    world.operation_plans.put(key, plan);
}

#[must_use]
pub fn inferred_effect_origin(world: &World, command: &Command) -> EffectOrigin {
    let sandbox = command.sandbox();
    if let (Some(epoch), Some(record)) = (command.epoch(), world.sandboxes.get(&sandbox))
        && let Some(operation) = record.current_operation
        && operation.epoch == epoch
    {
        return EffectOrigin::Operation(operation_key(sandbox, operation.id));
    }
    EffectOrigin::Maintenance(sandbox)
}

pub fn commit_effect(world: &mut World, command: Command) {
    let id = EffectId::initial(command.clone());
    if world.effects.contains_key(&id) {
        return;
    }

    let logical = LogicalEffectId { command };
    world.effects.put(id.clone(), EffectStatus::Committed);
    insert_unique(&mut world.effect_keys, id);

    if !world.logical_effects.contains_key(&logical) {
        let origin = inferred_effect_origin(world, &logical.command);
        world.logical_effects.put(
            logical,
            LogicalEffectRecord {
                origin,
                current_attempt: 0,
                remaining_retries: DEFAULT_RETRY_LIMIT,
                result: None,
            },
        );
    }
}

pub fn enqueue(world: &mut World, command: Command) {
    let id = EffectId::initial(command.clone());
    commit_effect(world, command);
    insert_unique(&mut world.inflight, id);
}

#[must_use]
pub fn begin_create(
    world: &World,
    sandbox: SandboxId,
    owner: TenantId,
    workspace: FsId,
    file_policy: FilePolicyRef,
    policy: NetworkPolicy,
    operation_id: crate::OperationId,
) -> World {
    let mut next = world.clone();
    crate::maintenance::record_identity(&mut next, sandbox, workspace);
    let operation = Operation {
        id: operation_id,
        epoch: 1,
        kind: OperationKind::Create,
        status: OperationStatus::Pending,
    };
    let record = SandboxRecord {
        owner,
        phase: crate::Phase::Starting,
        terminal: false,
        epoch: 1,
        generation: 1,
        current_vm: None,
        workspace,
        workspace_access: WorkspaceApiAccess::ManagedReadWrite,
        file_policy,
        durable_snapshot: None,
        restore_from: None,
        net_slot: None,
        policy: policy.clone(),
        desired: Authority::Closed,
        effective: Authority::Closed,
        current_operation: Some(operation),
    };
    next.filesystems.put(
        workspace,
        Filesystem {
            owner,
            sandbox,
            created_generation: 1,
        },
    );
    next.sandboxes.put(sandbox, record);
    store_new_operation(
        &mut next,
        sandbox,
        operation,
        OperationInput::Create {
            workspace,
            network: policy,
            file_policy,
        },
    );
    enqueue(
        &mut next,
        Command::InstallFence {
            sandbox,
            epoch: 1,
            terminal: false,
        },
    );
    next
}

/// Persist the pause operation and its capture responsibilities before
/// committing its fence and freeze effects. The added capture fields are
/// outside the historical `SandboxModel.beginPause` correspondence.
#[must_use]
pub fn begin_pause(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    operation_id: crate::OperationId,
    vm: VmId,
    epoch: crate::Epoch,
) -> World {
    let mut next = world.clone();
    let operation = Operation {
        id: operation_id,
        epoch,
        kind: OperationKind::Pause,
        status: OperationStatus::Pending,
    };
    let mut updated = record.clone();
    updated.phase = Phase::Pausing;
    updated.epoch = epoch;
    updated.desired = Authority::Closed;
    updated.current_operation = Some(operation);
    next.sandboxes.put(sandbox, updated);
    store_new_operation(
        &mut next,
        sandbox,
        operation,
        OperationInput::Pause {
            generation: record.generation,
        },
    );
    enqueue(
        &mut next,
        Command::InstallFence {
            sandbox,
            epoch,
            terminal: false,
        },
    );
    enqueue(
        &mut next,
        Command::FreezeVm {
            sandbox,
            epoch,
            generation: record.generation,
            vm,
        },
    );
    next
}

/// Begin a resume exactly as `SandboxModel.beginResume`.
#[must_use]
pub fn begin_resume(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    operation_id: crate::OperationId,
    source: RestoreSource,
    epoch: crate::Epoch,
    generation: crate::Generation,
) -> World {
    let mut next = world.clone();
    let operation = Operation {
        id: operation_id,
        epoch,
        kind: OperationKind::Resume,
        status: OperationStatus::Pending,
    };
    let mut updated = record.clone();
    updated.phase = Phase::Starting;
    updated.epoch = epoch;
    updated.generation = generation;
    updated.current_vm = None;
    updated.restore_from = Some(source);
    updated.net_slot = None;
    updated.desired = Authority::Closed;
    updated.effective = Authority::Closed;
    updated.current_operation = Some(operation);
    next.sandboxes.put(sandbox, updated);
    store_new_operation(
        &mut next,
        sandbox,
        operation,
        OperationInput::Resume { source },
    );
    enqueue(
        &mut next,
        Command::InstallFence {
            sandbox,
            epoch,
            terminal: false,
        },
    );
    next
}

/// Return whether a kill may replace the current operation.  The comparison
/// and lookup order mirrors `Controller.supersedeReadyCheck`.
#[must_use]
pub fn supersede_ready_check(
    world: &World,
    sandbox: SandboxId,
    current: Option<Operation>,
) -> bool {
    let Some(operation) = current else {
        return true;
    };
    match operation.status {
        OperationStatus::Pending | OperationStatus::OutcomeUnknown => {
            match world.operations.get(&operation_key(sandbox, operation.id)) {
                Some(ledger) => ledger.operation == operation && ledger.result.is_none(),
                None => false,
            }
        }
        OperationStatus::Compensating(_)
        | OperationStatus::Succeeded
        | OperationStatus::Failed(_) => true,
    }
}

fn supersede_current_operation(world: &mut World, sandbox: SandboxId, current: Option<Operation>) {
    let Some(operation) = current else {
        return;
    };
    let failure = match operation.status {
        OperationStatus::Pending | OperationStatus::OutcomeUnknown => FailureKind::Cancelled,
        OperationStatus::Compensating(failure) if operation.kind == OperationKind::Pause => failure,
        _ => return,
    };
    let key = operation_key(sandbox, operation.id);
    let Some(ledger) = world.operations.get(&key) else {
        return;
    };
    if ledger.operation != operation || ledger.result.is_some() {
        return;
    }
    let mut superseded = ledger.clone();
    superseded.operation.status = OperationStatus::Failed(failure);
    superseded.result = Some(OperationResult::Failed(failure));
    world.operations.put(key, superseded);
}

/// Begin a kill, finalize a superseded operation, and revoke capture
/// eligibility without releasing its material responsibilities. The capture
/// revocation is outside the historical `SandboxModel.beginKill` proof.
#[must_use]
pub fn begin_kill(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    operation_id: crate::OperationId,
    epoch: crate::Epoch,
) -> World {
    let mut next = world.clone();
    // Destroy revokes future recovery, including old sources, without
    // releasing any capture/loader/running-backing material responsibility.
    for (key, stored) in world.operation_plans.entries() {
        if key.sandbox == sandbox && stored.capture.is_some() {
            let mut plan = stored.clone();
            if let Some(capture) = plan.capture {
                plan.capture = Some(capture.terminate());
            }
            next.operation_plans.put(*key, plan);
        }
    }
    let operation = Operation {
        id: operation_id,
        epoch,
        kind: OperationKind::Kill,
        status: OperationStatus::Pending,
    };
    supersede_current_operation(&mut next, sandbox, record.current_operation);
    let mut updated = record.clone();
    updated.phase = Phase::Revoking;
    updated.terminal = true;
    updated.epoch = epoch;
    updated.desired = Authority::Closed;
    updated.effective = record.effective.without_resume();
    updated.current_operation = Some(operation);
    next.sandboxes.put(sandbox, updated);
    store_new_operation(
        &mut next,
        sandbox,
        operation,
        OperationInput::Kill {
            generation: record.generation,
        },
    );
    enqueue(
        &mut next,
        Command::InstallFence {
            sandbox,
            epoch,
            terminal: true,
        },
    );
    next
}

/// Ordinary pause failure must preserve the original live/frozen VM. Only
/// create/resume startup compensation uses the terminal cleanup path below.
/// Pause recovery remains parked until its original commit/work is reconciled;
/// it is never equivalent to owner destroy or expiry.
#[must_use]
pub fn begin_compensation(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    operation: Operation,
    failure: FailureKind,
) -> World {
    let mut next = world.clone();
    let mut compensating = operation;
    compensating.status = OperationStatus::Compensating(failure);

    if operation.kind == OperationKind::Pause {
        let mut updated = record.clone();
        updated.desired = Authority::Closed;
        // Only the original InstallFence worker may change effective access.
        updated.current_operation = Some(compensating);
        next.sandboxes.put(sandbox, updated);
        let key = operation_key(sandbox, operation.id);
        if let Some(stored) = next.operations.get(&key) {
            let mut ledger = stored.clone();
            ledger.operation = compensating;
            next.operations.put(key, ledger);
        }
        if let Some(stored) = next.operation_plans.get(&key) {
            let mut plan = stored.clone();
            if let Some(capture) = plan.capture {
                plan.capture = Some(capture.ordinary_failure());
            }
            next.operation_plans.put(key, plan);
        }
        return next;
    }

    let mut updated = record.clone();
    updated.phase = Phase::Revoking;
    updated.terminal = true;
    updated.desired = Authority::Closed;
    updated.effective = Authority::Closed;
    updated.current_operation = Some(compensating);
    next.sandboxes.put(sandbox, updated);

    let key = operation_key(sandbox, operation.id);
    if let Some(stored) = next.operations.get(&key) {
        let mut ledger = stored.clone();
        ledger.operation = compensating;
        ledger.result = None;
        next.operations.put(key, ledger);
    }

    if operation.kind == OperationKind::Resume {
        if let Some(stored) = next.operation_plans.get(&key) {
            let mut plan = stored.clone();
            let index = resume_cleanup_index(
                world,
                sandbox,
                plan.generation,
                crate::ResourceIndex {
                    vms: plan.stop_vms.clone(),
                    slots: plan.close_slots.clone(),
                    snapshots: Vec::new(),
                },
            );
            plan.stop_vms = index.vms;
            plan.close_slots = index.slots;
            next.operation_plans.put(key, plan);
        }
    }

    enqueue(
        &mut next,
        Command::InstallFence {
            sandbox,
            epoch: record.epoch,
            terminal: true,
        },
    );
    next
}

/// Mark the current RPC operation as outcome-unknown in both the sandbox row
/// and its operation ledger.  This is a journal-only transition: no worker
/// command is invented from a client-side timeout.
#[must_use]
pub fn mark_operation_outcome_unknown(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    operation: Operation,
) -> World {
    let mut next = world.clone();
    let mut unknown = operation;
    unknown.status = OperationStatus::OutcomeUnknown;

    let mut updated = record.clone();
    updated.current_operation = Some(unknown);
    next.sandboxes.put(sandbox, updated);

    let key = operation_key(sandbox, operation.id);
    if let Some(stored) = world.operations.get(&key) {
        let mut ledger = stored.clone();
        ledger.operation = unknown;
        ledger.result = None;
        next.operations.put(key, ledger);
    }
    next
}

/// Reserve the next startup VM and slot in the durable operation plan.
#[must_use]
pub fn reserve_startup_resources(
    world: &World,
    key: OperationKey,
    plan: &OperationPlan,
) -> Option<World> {
    if world.next_vm_id == u64::MAX || world.next_slot_id == u64::MAX {
        return None;
    }
    let mut next = world.clone();
    let vm = VmId(world.next_vm_id);
    let slot = crate::NetSlotId(world.next_slot_id);
    next.next_vm_id = world.next_vm_id + 1;
    next.next_slot_id = world.next_slot_id + 1;
    let mut reserved = plan.clone();
    reserved.vm = Some(vm);
    reserved.slot = Some(slot);
    insert_unique(&mut reserved.stop_vms, vm);
    insert_unique(&mut reserved.close_slots, slot);
    reserved.sealed = true;
    next.operation_plans.put(key, reserved);
    Some(next)
}

/// Reserve the next pause snapshot and manifest in the durable operation plan.
#[must_use]
pub fn reserve_pause_snapshot(
    world: &World,
    key: OperationKey,
    plan: &OperationPlan,
    vm: VmId,
) -> Option<World> {
    if world.next_snapshot_id == u64::MAX || world.next_manifest_digest == u64::MAX {
        return None;
    }
    let mut next = world.clone();
    let source = RestoreSource {
        snapshot: SnapshotId(world.next_snapshot_id),
        manifest: SnapshotManifestRef {
            digest: crate::ManifestDigest(world.next_manifest_digest),
        },
    };
    next.next_snapshot_id = world.next_snapshot_id + 1;
    next.next_manifest_digest = world.next_manifest_digest + 1;
    let mut reserved = plan.clone();
    reserved.vm = Some(vm);
    reserved.target_snapshot = Some(source);
    reserved.sealed = true;
    next.operation_plans.put(key, reserved);
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::{operation_key, reserve_pause_snapshot, reserve_startup_resources};
    use crate::model::{OperationPlan, RestoreSource, World};
    use crate::{
        ManifestDigest, NetSlotId, OperationId, SandboxId, SnapshotId, SnapshotManifestRef, VmId,
    };

    fn plan() -> OperationPlan {
        OperationPlan {
            key: operation_key(SandboxId(1), OperationId(2)),
            epoch: 3,
            generation: 4,
            vm: None,
            slot: None,
            restore_from: None,
            target_snapshot: None,
            capture: Some(crate::capture::CaptureHandoff::accepted()),
            stop_vms: vec![VmId(8)],
            close_slots: vec![NetSlotId(9)],
            retry_limit: 3,
            sealed: false,
        }
    }

    #[test]
    fn startup_reservation_advances_cursors_and_prepends_cleanup_resources() {
        let mut world = World {
            next_vm_id: 21,
            next_slot_id: 34,
            ..World::default()
        };
        let plan = plan();
        world.operation_plans.put(plan.key, plan.clone());

        let next = reserve_startup_resources(&world, plan.key, &plan).expect("capacity");
        let reserved = next.operation_plans.get(&plan.key).expect("reserved plan");
        assert_eq!(next.next_vm_id, 22);
        assert_eq!(next.next_slot_id, 35);
        assert_eq!(reserved.vm, Some(VmId(21)));
        assert_eq!(reserved.slot, Some(NetSlotId(34)));
        assert_eq!(reserved.stop_vms, vec![VmId(21), VmId(8)]);
        assert_eq!(reserved.close_slots, vec![NetSlotId(34), NetSlotId(9)]);
        assert!(reserved.sealed);
        assert_eq!(world.next_vm_id, 21);
        assert_eq!(world.next_slot_id, 34);
    }

    #[test]
    fn pause_reservation_advances_snapshot_and_manifest_cursors() {
        let world = World {
            next_snapshot_id: 55,
            next_manifest_digest: 89,
            ..World::default()
        };
        let plan = plan();
        let next = reserve_pause_snapshot(&world, plan.key, &plan, VmId(8)).expect("capacity");
        let reserved = next.operation_plans.get(&plan.key).expect("reserved plan");
        assert_eq!(next.next_snapshot_id, 56);
        assert_eq!(next.next_manifest_digest, 90);
        assert_eq!(reserved.vm, Some(VmId(8)));
        assert_eq!(
            reserved.target_snapshot,
            Some(RestoreSource {
                snapshot: SnapshotId(55),
                manifest: SnapshotManifestRef {
                    digest: ManifestDigest(89),
                },
            })
        );
        assert!(reserved.sealed);
    }

    #[test]
    fn reservations_fail_closed_at_the_machine_counter_boundary() {
        let plan = plan();
        let startup_world = World {
            next_vm_id: u64::MAX,
            ..World::default()
        };
        assert_eq!(
            reserve_startup_resources(&startup_world, plan.key, &plan),
            None
        );

        let snapshot_world = World {
            next_manifest_digest: u64::MAX,
            ..World::default()
        };
        assert_eq!(
            reserve_pause_snapshot(&snapshot_world, plan.key, &plan, VmId(8)),
            None
        );
    }

    #[test]
    fn insertion_stops_at_first_equal_element_and_preserves_prepend_order() {
        use std::{cell::RefCell, rc::Rc};
        #[derive(Clone)]
        struct Compared {
            value: u64,
            visited: Rc<RefCell<Vec<u64>>>,
        }
        impl PartialEq for Compared {
            fn eq(&self, other: &Self) -> bool {
                self.visited.borrow_mut().push(self.value);
                self.value == other.value
            }
        }
        let visited = Rc::new(RefCell::new(Vec::new()));
        let item = |value| Compared {
            value,
            visited: visited.clone(),
        };
        let mut values = vec![item(1), item(2), item(3)];
        super::insert_unique(&mut values, item(2));
        assert_eq!(*visited.borrow(), vec![1, 2]);
        assert_eq!(
            values.iter().map(|entry| entry.value).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        visited.borrow_mut().clear();
        super::insert_unique(&mut values, item(4));
        assert_eq!(*visited.borrow(), vec![1, 2, 3]);
        assert_eq!(
            values.iter().map(|entry| entry.value).collect::<Vec<_>>(),
            vec![4, 1, 2, 3]
        );
    }
}
