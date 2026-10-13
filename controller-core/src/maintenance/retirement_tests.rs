use super::*;
use crate::maintenance::slots::{HostBinding, HostSlotId, HostSlotPhase, HostSlotRecord};
use crate::maintenance::windows::WindowLimits;
use crate::maintenance::{MaintenanceInput, MaintenanceLimits, handle_maintenance};
use crate::*;

const SID: SandboxId = SandboxId(10);
const OWNER: TenantId = TenantId(7);

fn step(world: &World, input: ControllerInput) -> World {
    handle_controller(world, input)
        .expect("valid test history")
        .next
}

fn enabled(world: &World, limit: u64) -> World {
    handle_maintenance(
        world,
        MaintenanceInput::Enable {
            limits: MaintenanceLimits {
                max_retained_results: limit,
                max_host_slots: 4,
                windows: WindowLimits::new(2, 100).unwrap(),
            },
            now_ms: 5,
        },
    )
    .unwrap()
    .next
}

fn create(world: &World, sid: SandboxId) -> World {
    step(
        world,
        ControllerInput::Request(Request::Create(CreateRequest {
            actor: Actor::Tenant(OWNER),
            owner: OWNER,
            sandbox: sid,
            workspace: FsId(sid.0),
            file_policy: FilePolicyRef {
                digest: PolicyDigest(3),
            },
            policy: NetworkPolicy::DenyAll,
            operation: OperationId(1),
        })),
    )
}

fn effect(world: &World, sid: SandboxId, predicate: impl Fn(&Command) -> bool) -> EffectId {
    world
        .effect_ids()
        .iter()
        .find(|id| id.command().sandbox() == sid && predicate(id.command()))
        .expect("effect exists")
        .clone()
}

fn execute(world: &World, id: EffectId) -> World {
    let world = step(
        world,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let world = step(&world, ControllerInput::Worker(id.clone()));
    step(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            id,
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    )
}

fn execute_matching(world: &World, sid: SandboxId, predicate: impl Fn(&Command) -> bool) -> World {
    execute(world, effect(world, sid, predicate))
}

fn prepared(world: &World, sid: SandboxId) -> World {
    let world = create(world, sid);
    let world = execute_matching(&world, sid, |c| {
        matches!(
            c,
            Command::InstallFence {
                terminal: false,
                ..
            }
        )
    });
    let key = OperationKey {
        sandbox: sid,
        operation: OperationId(1),
    };
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::ReserveStartup(key)),
    );
    let plan = world.operation_plan(key).unwrap();
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            sid,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    execute_matching(&world, sid, |c| matches!(c, Command::PrepareVm { .. }))
}

fn running(world: &World, sid: SandboxId) -> World {
    let world = prepared(world, sid);
    let world = execute_matching(&world, sid, |c| {
        matches!(c, Command::ConfigureNetwork { .. })
    });
    let world = execute_matching(&world, sid, |c| matches!(c, Command::ActivateVm { .. }));
    let world = execute_matching(&world, sid, |c| {
        matches!(c, Command::ApplyFilePolicy { .. })
    });
    step(
        &world,
        ControllerInput::Publication(PublicationInput::Running(sid)),
    )
}

fn stop_all(world: &World, sid: SandboxId) -> World {
    let mut world = world.clone();
    for vm in world.resource_index(sid).vms {
        let record = *world.vm_record(vm).unwrap();
        if record.status != VmStatus::Reclaimed {
            world = step(
                &world,
                ControllerInput::Scheduler(SchedulerInput::StopVm(sid, record.generation, vm)),
            );
            world = execute_matching(
                &world,
                sid,
                |c| matches!(c, Command::StopVm { vm: found, .. } if *found == vm),
            );
        }
    }
    world
}

fn cleanup(world: &World, sid: SandboxId) -> World {
    let mut world = world.clone();
    let index = world.resource_index(sid);
    for vm in index.vms {
        let record = *world.vm_record(vm).unwrap();
        if record.status != VmStatus::Reclaimed {
            world = step(
                &world,
                ControllerInput::Scheduler(SchedulerInput::CleanupVm(sid, record.generation, vm)),
            );
            world = execute_matching(
                &world,
                sid,
                |c| matches!(c, Command::CleanupVm { vm: found, .. } if *found == vm),
            );
        }
    }
    for slot in index.slots {
        let record = world.slot_record(slot).unwrap();
        if !record.reclaimed {
            world = step(
                &world,
                ControllerInput::Scheduler(SchedulerInput::CleanupSlot(
                    sid,
                    record.generation,
                    slot,
                )),
            );
            world = execute_matching(
                &world,
                sid,
                |c| matches!(c, Command::CleanupSlot { slot: found, .. } if *found == slot),
            );
        }
    }
    world
}

fn destroy(world: &World, sid: SandboxId) -> World {
    let world = step(
        world,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: sid,
            operation: OperationId(2),
        })),
    );
    let world = execute_matching(&world, sid, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = stop_all(&world, sid);
    step(
        &world,
        ControllerInput::Publication(PublicationInput::Destroyed(sid)),
    )
}

fn finished() -> World {
    let world = enabled(&initial_world(), 100);
    let world = running(&world, SID);
    cleanup(&destroy(&world, SID), SID)
}

#[test]
fn split_ledger_checks_keep_count_and_ledger_before_plan_rejection() {
    let mut world = finished();
    let record = world.sandbox_record(SID).unwrap().clone();
    assert_eq!(check_ledgers(&world, SID, &record), Ok(2));
    let key = OperationKey {
        sandbox: SID,
        operation: OperationId(1),
    };
    let mut plan = world.operation_plan(key).unwrap().clone();
    plan.restore_from = Some(RestoreSource {
        snapshot: SnapshotId(99),
        manifest: SnapshotManifestRef {
            digest: ManifestDigest(1),
        },
    });
    world.operation_plans.put(key, plan);
    assert_eq!(
        check_ledgers(&world, SID, &record),
        Err(RetirementReject::SnapshotHistoryUnsupported)
    );
    let mut ledger = world.operations.get(&key).unwrap().clone();
    ledger.operation.epoch = record.epoch + 1;
    world.operations.put(key, ledger);
    let before = world.clone();
    assert_eq!(
        check_ledgers(&world, SID, &record),
        Err(RetirementReject::LedgerMismatch)
    );
    assert_eq!(world, before);
    // Current-ledger validation must still precede both traversal helpers.
    let mut changed_record = record;
    changed_record.current_operation = None;
    assert_eq!(
        check_ledgers(&world, SID, &changed_record),
        Err(RetirementReject::LedgerMismatch)
    );
}

// Frozen pre-refactor traversal, regression oracle only (not production or a
// substitute for a source-bound Lean refinement proof).
fn original_external_dependencies_closed(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    ids: &OwnedIds,
) -> bool {
    let workspace = record.workspace;
    for (other, stored) in world.sandboxes.entries() {
        if *other != sandbox
            && (stored.workspace == workspace
                || stored.current_vm.is_some_and(|vm| ids.vms.contains(&vm))
                || stored
                    .net_slot
                    .is_some_and(|slot| ids.slots.contains(&slot))
                || [stored.durable_snapshot, stored.restore_from]
                    .into_iter()
                    .flatten()
                    .any(|source| ids.snapshots.contains(&source.snapshot))
                || authority_references(&stored.desired, sandbox, workspace, ids)
                || authority_references(&stored.effective, sandbox, workspace, ids))
        {
            return false;
        }
    }
    for (key, ledger) in world.operations.entries() {
        if key.sandbox != sandbox
            && (ledger.sandbox == sandbox || ledger_references(ledger, workspace, ids))
        {
            return false;
        }
    }
    for (key, plan) in world.operation_plans.entries() {
        if key.sandbox != sandbox
            && (plan.key.sandbox == sandbox
                || plan.vm.is_some_and(|vm| ids.vms.contains(&vm))
                || plan.slot.is_some_and(|slot| ids.slots.contains(&slot))
                || plan.stop_vms.iter().any(|vm| ids.vms.contains(vm))
                || plan.close_slots.iter().any(|slot| ids.slots.contains(slot))
                || [plan.restore_from, plan.target_snapshot]
                    .into_iter()
                    .flatten()
                    .any(|source| ids.snapshots.contains(&source.snapshot)))
        {
            return false;
        }
    }
    for (other, index) in world.resource_indexes.entries() {
        if *other != sandbox
            && (index.vms.iter().any(|vm| ids.vms.contains(vm))
                || index.slots.iter().any(|slot| ids.slots.contains(slot))
                || index
                    .snapshots
                    .iter()
                    .any(|snapshot| ids.snapshots.contains(snapshot)))
        {
            return false;
        }
    }
    if world
        .net_slots
        .entries()
        .iter()
        .any(|(_, slot)| slot.sandbox != sandbox && ids.vms.contains(&slot.vm))
        || world
            .snapshots
            .entries()
            .iter()
            .any(|(_, snapshot)| snapshot.sandbox != sandbox && snapshot.workspace == workspace)
        || world
            .applied_file_policies
            .entries()
            .iter()
            .any(|(other, policy)| {
                *other != sandbox && (policy.sandbox == sandbox || policy.workspace == workspace)
            })
    {
        return false;
    }
    for (id, logical) in world.logical_effects.entries() {
        if id.command.sandbox() != sandbox
            && (origin_sandbox(logical) == sandbox
                || matches!(logical.result, Some(LogicalEffectResult::RetiredByStop { coverage, .. })
                    if coverage.operation.sandbox == sandbox)
                || command_references(&id.command, workspace, ids))
        {
            return false;
        }
    }
    if world.effects.entries().iter().any(|(id, status)| id.command().sandbox() != sandbox
        && matches!(status, EffectStatus::RetiredByStop(coverage) if coverage.operation.sandbox == sandbox)) {
        return false;
    }
    for id in world
        .effects
        .entries()
        .iter()
        .map(|(id, _)| id)
        .chain(world.effect_keys.iter())
        .chain(world.inflight.iter())
    {
        if id.command().sandbox() != sandbox && command_references(id.command(), workspace, ids) {
            return false;
        }
    }
    true
}

#[test]
#[allow(clippy::too_many_lines)]
fn split_dependency_checks_match_original_tables_and_work_sources() {
    let other = SandboxId(11);
    let baseline = create(&finished(), other);
    let record = baseline.sandbox_record(SID).unwrap().clone();
    let mut ids = owned_ids(&baseline, SID);
    let vm = ids.vms[0];
    let slot = ids.slots[0];
    let source = RestoreSource {
        snapshot: SnapshotId(999),
        manifest: SnapshotManifestRef {
            digest: ManifestDigest(1),
        },
    };
    ids.snapshots.push(source.snapshot);
    let other_key = OperationKey {
        sandbox: other,
        operation: OperationId(1),
    };
    let other_effect = effect(&baseline, other, |c| {
        matches!(c, Command::InstallFence { .. })
    });
    let reference = EffectId {
        logical: LogicalEffectId {
            command: Command::CleanupVm {
                sandbox: other,
                generation: 1,
                vm,
            },
        },
        attempt: 0,
    };
    let coverage = StopCoverage {
        operation: OperationKey {
            sandbox: SID,
            operation: OperationId(2),
        },
        epoch: 2,
        attempt: 0,
        result: ResultDigest(1),
    };
    assert!(external_dependencies_closed(
        &World::default(),
        SID,
        &record,
        &ids
    ));
    for case in 0..19 {
        let mut world = baseline.clone();
        match case {
            1 | 2 | 3 | 15 | 17 => {
                let mut row = world.sandbox_record(other).unwrap().clone();
                match case {
                    1 => row.workspace = record.workspace,
                    2 => row.current_vm = Some(vm),
                    3 => row.net_slot = Some(slot),
                    15 => {
                        row.effective = Authority::Paused {
                            sandbox: SID,
                            source,
                        }
                    }
                    17 => row.durable_snapshot = Some(source),
                    _ => unreachable!(),
                }
                world.sandboxes.put(other, row);
            }
            4 => {
                let mut row = world.operations.get(&other_key).unwrap().clone();
                row.sandbox = SID;
                world.operations.put(other_key, row);
            }
            5 | 16 => {
                let mut row = world.operation_plan(other_key).unwrap().clone();
                if case == 5 {
                    row.stop_vms.push(vm);
                } else {
                    row.restore_from = Some(source);
                }
                world.operation_plans.put(other_key, row);
            }
            6 => {
                let mut row = world.resource_index(other);
                row.vms.push(vm);
                world.resource_indexes.put(other, row);
            }
            7 => {
                let mut row = world.slot_record(slot).unwrap().clone();
                row.sandbox = other;
                world.net_slots.put(NetSlotId(999), row);
            }
            8 => {
                world.snapshots.put(
                    SnapshotId(998),
                    Snapshot {
                        owner: OWNER,
                        sandbox: other,
                        source_generation: 1,
                        workspace: record.workspace,
                        manifest: source.manifest,
                        stage: SnapshotStage::Durable,
                        immutable: true,
                    },
                );
            }
            9 => {
                world.applied_file_policies.put(
                    other,
                    AppliedFilePolicy {
                        sandbox: other,
                        generation: 1,
                        workspace: record.workspace,
                        policy: record.file_policy,
                    },
                );
            }
            10 | 18 => {
                let mut row = world
                    .logical_effects
                    .get_copy(&other_effect.logical)
                    .unwrap();
                if case == 10 {
                    row.origin = EffectOrigin::Maintenance(SID);
                } else {
                    row.result = Some(LogicalEffectResult::RetiredByStop {
                        attempt: 0,
                        coverage,
                    });
                }
                world.logical_effects.put(other_effect.logical.clone(), row);
            }
            11 => world
                .effects
                .put(other_effect.clone(), EffectStatus::RetiredByStop(coverage)),
            12 => world
                .effects
                .put(reference.clone(), EffectStatus::Committed),
            13 => world.effect_keys.push(reference.clone()),
            14 => world.inflight.push(reference.clone()),
            _ => {}
        }
        let before = world.clone();
        let actual = external_dependencies_closed(&world, SID, &record, &ids);
        assert_eq!(
            actual,
            original_external_dependencies_closed(&world, SID, &record, &ids),
            "dependency case {case}"
        );
        assert_eq!(actual, case == 0, "isolated dependency case {case}");
        assert_eq!(world, before);
    }
}

#[test]
fn foreign_dependency_rejection_keeps_priority_over_resource_and_effect_faults() {
    let mut world = create(&finished(), SandboxId(11));
    let mut other = world.sandbox_record(SandboxId(11)).unwrap().clone();
    other.workspace = FsId(SID.0);
    world.sandboxes.put(SandboxId(11), other);
    let vm_id = world.resource_index(SID).vms[0];
    let mut vm = *world.vm_record(vm_id).unwrap();
    vm.status = VmStatus::Executing;
    world.vms.put(vm_id, vm);
    let id = effect(&world, SID, |c| matches!(c, Command::ActivateVm { .. }));
    world.effects.put(id, EffectStatus::OutcomeUnknown);
    let before = world.clone();
    assert_eq!(
        validate_terminal_cleanup(&world, SID),
        Err(RetirementReject::DependencyNotClosed)
    );
    assert_eq!(world, before);
    let key = OperationKey {
        sandbox: SID,
        operation: OperationId(2),
    };
    let mut ledger = world.operations.get(&key).unwrap().clone();
    ledger.operation.epoch += 1;
    world.operations.put(key, ledger);
    assert_eq!(
        validate_terminal_cleanup(&world, SID),
        Err(RetirementReject::LedgerMismatch)
    );
}

fn original_check_resources(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    ids: &OwnedIds,
) -> Result<(), RetirementReject> {
    if record.durable_snapshot.is_some()
        || record.restore_from.is_some()
        || !ids.snapshots.is_empty()
        || !world.resource_index(sandbox).snapshots.is_empty()
    {
        return Err(RetirementReject::SnapshotHistoryUnsupported);
    }
    let filesystem = world
        .filesystems
        .get(&record.workspace)
        .ok_or(RetirementReject::LedgerMismatch)?;
    if filesystem.sandbox != sandbox
        || filesystem.owner != record.owner
        || world
            .filesystems
            .entries()
            .iter()
            .any(|(id, fs)| fs.sandbox == sandbox && *id != record.workspace)
    {
        return Err(RetirementReject::LedgerMismatch);
    }
    if world
        .applied_file_policies
        .get(&sandbox)
        .is_some_and(|policy| {
            policy.sandbox != sandbox
                || policy.workspace != record.workspace
                || policy.policy != record.file_policy
        })
    {
        return Err(RetirementReject::LedgerMismatch);
    }
    if ids.vms.iter().any(|id| id.0 >= world.next_vm_id)
        || ids.slots.iter().any(|id| id.0 >= world.next_slot_id)
    {
        return Err(RetirementReject::IdentityBarrierMissing);
    }
    let index = world.resource_index(sandbox);
    for (id, vm) in world.vms.entries() {
        if vm.sandbox == sandbox
            && (vm.owner != record.owner
                || vm.status != VmStatus::Reclaimed
                || !index.vms.contains(id)
                || !reclaimed_vm_receipt(world, sandbox, *id, vm.generation))
        {
            return Err(RetirementReject::ResourcesNotReclaimed);
        }
    }
    for id in &index.vms {
        if world.vms.get(id).is_none_or(|vm| vm.sandbox != sandbox) {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    for (id, slot) in world.net_slots.entries() {
        if slot.sandbox == sandbox
            && (slot.owner != record.owner
                || slot.active
                || !slot.reclaimed
                || !index.slots.contains(id)
                || world
                    .vms
                    .get(&slot.vm)
                    .is_none_or(|vm| vm.sandbox != sandbox || vm.generation != slot.generation)
                || !reclaimed_slot_receipt(world, sandbox, *id, slot.generation, slot.vm))
        {
            return Err(RetirementReject::ResourcesNotReclaimed);
        }
    }
    for id in &index.slots {
        if world
            .net_slots
            .get(id)
            .is_none_or(|slot| slot.sandbox != sandbox)
        {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    // Plans may reserve IDs without ever issuing a worker command. Absent rows
    // are safe only because all effect attempts are separately checked below.
    for vm in &ids.vms {
        if world
            .vms
            .get(vm)
            .is_some_and(|vm| vm.sandbox != sandbox || vm.owner != record.owner)
        {
            return Err(RetirementReject::DependencyNotClosed);
        }
    }
    for slot in &ids.slots {
        if world
            .net_slots
            .get(slot)
            .is_some_and(|slot| slot.sandbox != sandbox || slot.owner != record.owner)
        {
            return Err(RetirementReject::DependencyNotClosed);
        }
    }
    Ok(())
}

fn original_check_effects(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
) -> Result<(), RetirementReject> {
    for id in world.effect_keys.iter().chain(world.inflight.iter()) {
        if id.command().sandbox() == sandbox && !world.effects.contains_key(id) {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    for (logical_id, logical) in world.logical_effects.entries() {
        if logical_id.command.sandbox() != sandbox {
            continue;
        }
        let id = EffectId {
            logical: logical_id.clone(),
            attempt: logical.current_attempt,
        };
        let status = world
            .effects
            .get_copy(&id)
            .ok_or(RetirementReject::LedgerMismatch)?;
        if !command_bound(world, &logical_id.command, logical, record) {
            return Err(RetirementReject::LedgerMismatch);
        }
        let result_matches = match (logical.result, status) {
            (
                None,
                EffectStatus::Committed | EffectStatus::Acknowledged(EffectOutcome::Failed(_)),
            ) => true,
            (
                Some(LogicalEffectResult::Succeeded { attempt, result }),
                EffectStatus::Acknowledged(EffectOutcome::Succeeded(observed)),
            ) => attempt == id.attempt && result == observed,
            (
                Some(LogicalEffectResult::Failed { attempt, failure }),
                EffectStatus::Acknowledged(EffectOutcome::Failed(observed)),
            ) => attempt == id.attempt && failure == observed,
            (
                Some(LogicalEffectResult::RetiredByStop { attempt, coverage }),
                EffectStatus::RetiredByStop(stored),
            ) => {
                attempt == id.attempt
                    && coverage == stored
                    && retired_stop_valid(world, &id, coverage)
            }
            (_, EffectStatus::Dispatched | EffectStatus::OutcomeUnknown) => {
                return Err(RetirementReject::UnresolvedEffects);
            }
            _ => false,
        };
        if !result_matches {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    for (id, status) in world.effects.entries() {
        if id.command().sandbox() != sandbox {
            continue;
        }
        let logical = world
            .logical_effects
            .get(&id.logical)
            .ok_or(RetirementReject::LedgerMismatch)?;
        if !world.effect_keys.contains(id) || id.attempt > logical.current_attempt {
            return Err(RetirementReject::LedgerMismatch);
        }
        match status {
            EffectStatus::Acknowledged(_) => {
                // A successful prepare/abort must have materialized its exact
                // reserved resources. Retired metadata cannot hide a lost row.
                if matches!(
                    status,
                    EffectStatus::Acknowledged(EffectOutcome::Succeeded(_))
                ) && let Command::PrepareVm { vm, slot, .. }
                | Command::AbortPrepare { vm, slot, .. } = id.command()
                    && (!world.vms.contains_key(vm) || !world.net_slots.contains_key(slot))
                {
                    return Err(RetirementReject::LedgerMismatch);
                }
            }
            EffectStatus::RetiredByStop(coverage) if retired_stop_valid(world, id, *coverage) => {}
            EffectStatus::Committed if obsolete_committed(world, id, logical) => {}
            EffectStatus::Dispatched | EffectStatus::OutcomeUnknown => {
                return Err(RetirementReject::UnresolvedEffects);
            }
            _ => return Err(RetirementReject::LedgerMismatch),
        }
    }
    Ok(())
}

#[test]
fn split_resource_and_effect_checks_match_original_first_rejections() {
    let baseline = finished();
    let record = baseline.sandbox_record(SID).unwrap().clone();
    let vm_id = baseline.resource_index(SID).vms[0];
    let slot_id = baseline.resource_index(SID).slots[0];
    let active = effect(&baseline, SID, |c| matches!(c, Command::ActivateVm { .. }));
    for resource_case in 0..9 {
        for effect_case in 0..7 {
            let mut world = baseline.clone();
            let mut current = record.clone();
            match resource_case {
                1 => {
                    let mut vm = *world.vm_record(vm_id).unwrap();
                    vm.status = VmStatus::Executing;
                    world.vms.put(vm_id, vm);
                }
                2 => world.net_slots = Store::default(),
                3 => {
                    let mut slot = world.slot_record(slot_id).unwrap().clone();
                    slot.active = true;
                    world.net_slots.put(slot_id, slot);
                }
                4 => {
                    let mut index = world.resource_index(SID);
                    index.vms.push(VmId(999));
                    world.resource_indexes.put(SID, index);
                }
                5 => {
                    let mut fs = *world.filesystems.get(&record.workspace).unwrap();
                    fs.owner = TenantId(999);
                    world.filesystems.put(record.workspace, fs);
                }
                6 => world.next_vm_id = vm_id.0,
                7 => {
                    current.restore_from = Some(RestoreSource {
                        snapshot: SnapshotId(999),
                        manifest: SnapshotManifestRef {
                            digest: ManifestDigest(1),
                        },
                    });
                }
                8 => world.vms = Store::default(),
                _ => {}
            }
            match effect_case {
                1 => world
                    .effects
                    .put(active.clone(), EffectStatus::OutcomeUnknown),
                2 => world.effects = Store::default(),
                3 => world.logical_effects = Store::default(),
                4 | 6 => {
                    let mut missing = active.clone();
                    missing.attempt += 1;
                    if effect_case == 4 {
                        world.effect_keys.push(missing);
                    } else {
                        world.inflight.push(missing);
                    }
                }
                5 => {
                    let mut logical = world.logical_effects.get_copy(&active.logical).unwrap();
                    logical.result = Some(LogicalEffectResult::Failed {
                        attempt: active.attempt,
                        failure: FailureKind::StorageFailure,
                    });
                    world.logical_effects.put(active.logical.clone(), logical);
                }
                _ => {}
            }
            let before = world.clone();
            let ids = owned_ids(&world, SID);
            assert_eq!(
                check_resources(&world, SID, &current, &ids),
                original_check_resources(&world, SID, &current, &ids),
                "resources={resource_case}, effects={effect_case}"
            );
            assert_eq!(
                check_effects(&world, SID, &current),
                original_check_effects(&world, SID, &current),
                "resources={resource_case}, effects={effect_case}"
            );
            assert_eq!(world, before);
        }
    }
}

#[test]
fn expanded_optional_snapshot_collection_keeps_order_and_deduplication() {
    let baseline = finished();
    let key = OperationKey {
        sandbox: SID,
        operation: OperationId(1),
    };
    let source = |number| RestoreSource {
        snapshot: SnapshotId(number),
        manifest: SnapshotManifestRef {
            digest: ManifestDigest(number),
        },
    };
    for first in [None, Some(source(31)), Some(source(32))] {
        for second in [None, Some(source(31)), Some(source(32))] {
            let mut world = baseline.clone();
            let mut plan = world.operation_plan(key).unwrap().clone();
            plan.restore_from = first;
            plan.target_snapshot = second;
            world.operation_plans.put(key, plan);
            let before = world.clone();
            let mut expected = Vec::new();
            for snapshot in [first, second].into_iter().flatten() {
                push_unique(&mut expected, snapshot.snapshot);
            }
            assert_eq!(owned_ids(&world, SID).snapshots, expected);
            assert_eq!(world, before);
        }
    }
}

fn request(world: &World, sid: SandboxId) -> RetireRequest {
    let record = world.sandbox_record(sid).unwrap();
    let operation = record.current_operation.unwrap().id;
    RetireRequest {
        sandbox: sid,
        owner: record.owner,
        epoch: record.epoch,
        operation,
        retain_until: 100,
        quiescence: QuiescenceCertificate {
            sandbox: sid,
            owner: record.owner,
            epoch: record.epoch,
            operation,
            workspace: record.workspace,
            worker_drained: QuiescenceStatus::Confirmed,
            data_connections_drained: QuiescenceStatus::Confirmed,
            host_resources_reclaimed: QuiescenceStatus::Confirmed,
            external_pins_released: QuiescenceStatus::Confirmed,
        },
    }
}

fn assert_no_detail(world: &World, sid: SandboxId) {
    assert!(!world.sandboxes.contains_key(&sid));
    assert!(
        !world
            .filesystems
            .entries()
            .iter()
            .any(|(_, row)| row.sandbox == sid)
    );
    assert!(
        !world
            .vms
            .entries()
            .iter()
            .any(|(_, row)| row.sandbox == sid)
    );
    assert!(
        !world
            .net_slots
            .entries()
            .iter()
            .any(|(_, row)| row.sandbox == sid)
    );
    assert!(
        !world
            .snapshots
            .entries()
            .iter()
            .any(|(_, row)| row.sandbox == sid)
    );
    assert!(!world.applied_file_policies.contains_key(&sid));
    assert!(!world.worker_fences.contains_key(&sid));
    assert!(!world.resource_indexes.contains_key(&sid));
    assert!(
        !world
            .operations
            .entries()
            .iter()
            .any(|(key, _)| key.sandbox == sid)
    );
    assert!(
        !world
            .operation_plans
            .entries()
            .iter()
            .any(|(key, _)| key.sandbox == sid)
    );
    assert!(
        !world
            .logical_effects
            .entries()
            .iter()
            .any(|(id, _)| id.command.sandbox() == sid)
    );
    assert!(
        !world
            .effects
            .entries()
            .iter()
            .any(|(id, _)| id.command().sandbox() == sid)
    );
    assert!(
        !world
            .effect_keys
            .iter()
            .any(|id| id.command().sandbox() == sid)
    );
    assert!(
        !world
            .inflight
            .iter()
            .any(|id| id.command().sandbox() == sid)
    );
}

#[test]
fn deletes_every_owned_container_but_preserves_exact_results_and_cursors() {
    let world = finished();
    assert!(
        !world.inflight.is_empty(),
        "historical transport rows deliberately persist"
    );
    let next = retire(&world, request(&world, SID)).unwrap();
    assert_no_detail(&next, SID);
    assert_eq!(
        (
            next.next_vm_id,
            next.next_slot_id,
            next.next_snapshot_id,
            next.next_manifest_digest
        ),
        (
            world.next_vm_id,
            world.next_slot_id,
            world.next_snapshot_id,
            world.next_manifest_digest
        )
    );
    assert_eq!(next.controller_up, world.controller_up);
    for (key, ledger) in world.operations.entries() {
        let retained = next
            .maintenance
            .as_ref()
            .unwrap()
            .retained
            .get(key)
            .unwrap();
        assert_eq!(
            retained,
            &RetainedOperation {
                owner: OWNER,
                ledger: ledger.clone(),
                expires_at: 100
            }
        );
        let query = QueryRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: key.operation,
        };
        assert_eq!(handle_query(&next, query).unwrap().next, next);
        let replay = ReplayRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: key.operation,
            kind: ledger.operation.kind,
            input: ledger.input.clone(),
        };
        assert_eq!(handle_replay(&next, replay).unwrap().next, next);
    }
    let expired = handle_maintenance(&next, MaintenanceInput::ExpireResults { now_ms: 100 })
        .unwrap()
        .next;
    assert_eq!(
        expired
            .maintenance
            .as_ref()
            .unwrap()
            .retained
            .entries()
            .len(),
        0
    );
    assert!(
        handle_query(
            &expired,
            QueryRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: OperationId(1)
            }
        )
        .is_err()
    );
}

#[test]
fn newer_retirement_does_not_wait_for_old_live_sandbox_or_mutate_it() {
    let world = enabled(&initial_world(), 100);
    let world = running(&world, SandboxId(1));
    let world = running(&world, SID);
    let world = cleanup(&destroy(&world, SID), SID);
    let old = world.sandbox_record(SandboxId(1)).unwrap().clone();
    let next = retire(&world, request(&world, SID)).unwrap();
    assert_eq!(next.sandbox_record(SandboxId(1)), Some(&old));
    for (key, value) in world
        .effects
        .entries()
        .iter()
        .filter(|(id, _)| id.command().sandbox() == SandboxId(1))
    {
        assert_eq!(next.effects.get(key), Some(value));
    }
    assert_no_detail(&next, SID);
}

#[test]
fn late_delivery_execution_receipts_retries_and_recreation_are_rejected() {
    let world = finished();
    let ids = world.effect_keys.clone();
    let next = retire(&world, request(&world, SID)).unwrap();
    for id in ids {
        for input in [
            ControllerInput::Worker(id.clone()),
            ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
            ControllerInput::Effect(EffectInput::Receipt(
                id.clone(),
                EffectOutcome::Succeeded(ResultDigest(1)),
            )),
            ControllerInput::Effect(EffectInput::Timeout(id.clone())),
            ControllerInput::Effect(EffectInput::Retry(
                id.logical.clone(),
                FailureKind::ResourceFailure,
            )),
            ControllerInput::Runtime(
                crate::recovery_monitor::RuntimeInput::DuplicateOrLateDelivery(id),
            ),
        ] {
            assert!(handle_controller(&next, input).is_err());
        }
    }
    let before = next.clone();
    assert!(
        handle_create(
            &next,
            CreateRequest {
                actor: Actor::Tenant(OWNER),
                owner: OWNER,
                sandbox: SID,
                workspace: FsId(100),
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(3)
                },
                policy: NetworkPolicy::DenyAll,
                operation: OperationId(1)
            }
        )
        .is_err()
    );
    assert_eq!(next, before);
}

#[test]
fn rejects_live_unreclaimed_unknown_missing_index_and_bad_certificate_without_mutation() {
    let initial = enabled(&initial_world(), 100);
    let live = running(&initial, SID);
    assert_eq!(
        retire(&live, request(&live, SID)),
        Err(RetirementReject::NotTerminal)
    );
    let stopped = destroy(&live, SID);
    assert_eq!(
        retire(&stopped, request(&stopped, SID)),
        Err(RetirementReject::ResourcesNotReclaimed)
    );
    let world = cleanup(&stopped, SID);
    let before = world.clone();
    for alter in 0..7 {
        let mut req = request(&world, SID);
        match alter {
            0 => req.quiescence.epoch += 1,
            1 => req.quiescence.owner = TenantId(999),
            2 => req.quiescence.workspace = FsId(999),
            3 => req.quiescence.worker_drained = QuiescenceStatus::Unconfirmed,
            4 => req.quiescence.data_connections_drained = QuiescenceStatus::Unconfirmed,
            5 => req.quiescence.host_resources_reclaimed = QuiescenceStatus::Unconfirmed,
            _ => req.quiescence.external_pins_released = QuiescenceStatus::Unconfirmed,
        }
        assert_eq!(
            retire(&world, req),
            Err(RetirementReject::CertificateMismatch)
        );
    }
    assert_eq!(world, before);
    let mut missing = world.clone();
    missing.resource_indexes.retain(|_, _| false);
    assert!(retire(&missing, request(&missing, SID)).is_err());
    let mut unknown = world.clone();
    let cleanup_id = effect(&unknown, SID, |c| matches!(c, Command::CleanupVm { .. }));
    unknown
        .effects
        .put(cleanup_id.clone(), EffectStatus::OutcomeUnknown);
    assert!(
        retire(&unknown, request(&unknown, SID)).is_err(),
        "reclaimed state is not a cleanup receipt"
    );
}

#[test]
fn never_materialized_reservations_and_unissued_old_fence_are_retirable() {
    let world = create(&enabled(&initial_world(), 100), SID);
    let key = OperationKey {
        sandbox: SID,
        operation: OperationId(1),
    };
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::ReserveStartup(key)),
    );
    let world = destroy(&world, SID);
    assert!(world.vms.entries().is_empty());
    let original = effect(&world, SID, |c| {
        matches!(
            c,
            Command::InstallFence {
                terminal: false,
                ..
            }
        )
    });
    assert_eq!(
        world.effect_status(&original),
        Some(EffectStatus::Committed)
    );
    assert_eq!(
        world.operation_record(key).unwrap().result,
        Some(OperationResult::Failed(FailureKind::Cancelled))
    );
    let next = retire(&world, request(&world, SID)).unwrap();
    assert_no_detail(&next, SID);
    assert_eq!((next.next_vm_id, next.next_slot_id), (1, 1));
}

fn failed_with_unissued_startup() -> World {
    let world = prepared(&enabled(&initial_world(), 100), SID);
    let policy = effect(&world, SID, |c| {
        matches!(c, Command::ApplyFilePolicy { .. })
    });
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(policy.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            policy.clone(),
            EffectOutcome::Failed(FailureKind::PolicyFailure),
        )),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::FinalizeFailure(
            policy.logical.clone(),
            FailureKind::PolicyFailure,
        )),
    );
    let world = step(
        &world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SID,
            failure: FailureKind::PolicyFailure,
            evidence: Some(policy),
        })),
    );
    let world = execute_matching(&world, SID, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = stop_all(&world, SID);
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Failed(SID, FailureKind::PolicyFailure)),
    );
    cleanup(&world, SID)
}

#[test]
fn compensated_failure_can_retire_obsolete_committed_startup_without_fabricating_receipts() {
    let world = failed_with_unissued_startup();
    assert!(world.effects.entries().iter().any(|(id, status)| matches!(
        id.command(),
        Command::ActivateVm { .. }
    ) && *status
        == EffectStatus::Committed));
    let next = retire(&world, request(&world, SID)).unwrap();
    assert_no_detail(&next, SID);
    assert_eq!(
        next.maintenance
            .as_ref()
            .unwrap()
            .retained
            .get(&OperationKey {
                sandbox: SID,
                operation: OperationId(1)
            })
            .unwrap()
            .ledger
            .result,
        Some(OperationResult::Failed(FailureKind::PolicyFailure))
    );
}

#[test]
fn stopped_activation_retirement_remains_distinct_from_a_receipt() {
    let world = prepared(&enabled(&initial_world(), 100), SID);
    let world = execute_matching(&world, SID, |c| {
        matches!(c, Command::ConfigureNetwork { .. })
    });
    let activation = effect(&world, SID, |c| matches!(c, Command::ActivateVm { .. }));
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(activation.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(activation.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SID,
            failure: FailureKind::DeadlineExceeded,
            evidence: None,
        })),
    );
    let world = execute_matching(&world, SID, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = stop_all(&world, SID);
    let stop = effect(&world, SID, |c| matches!(c, Command::StopVm { .. }));
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::RetireActivation {
            activation: activation.clone(),
            stop,
        }),
    );
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Failed(SID, FailureKind::DeadlineExceeded)),
    );
    let world = cleanup(&world, SID);
    assert!(matches!(
        world.effect_status(&activation),
        Some(EffectStatus::RetiredByStop(_))
    ));
    assert!(!effect_succeeded_check(&world, activation.command()));
    let next = retire(&world, request(&world, SID)).unwrap();
    assert_no_detail(&next, SID);
}

#[test]
fn exact_abort_prepare_receipts_cover_both_resources_without_cleanup_commands() {
    let world = create(&enabled(&initial_world(), 100), SID);
    let world = execute_matching(&world, SID, |c| {
        matches!(
            c,
            Command::InstallFence {
                terminal: false,
                ..
            }
        )
    });
    let key = OperationKey {
        sandbox: SID,
        operation: OperationId(1),
    };
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::ReserveStartup(key)),
    );
    let plan = world.operation_plan(key).unwrap().clone();
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    let prepare = effect(&world, SID, |c| matches!(c, Command::PrepareVm { .. }));
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(prepare.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            prepare.clone(),
            EffectOutcome::Failed(FailureKind::PolicyFailure),
        )),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::FinalizeFailure(
            prepare.logical.clone(),
            FailureKind::PolicyFailure,
        )),
    );
    let world = step(
        &world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SID,
            failure: FailureKind::PolicyFailure,
            evidence: Some(prepare),
        })),
    );
    let world = execute_matching(&world, SID, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::AbortPrepare(
            SID,
            1,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    let abort = effect(&world, SID, |c| matches!(c, Command::AbortPrepare { .. }));
    let dispatched = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(abort.clone())),
    );
    assert_abort_prepare_rejection_order(&dispatched, &abort, key);
    let world = execute(&world, abort);
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Failed(SID, FailureKind::PolicyFailure)),
    );
    assert!(!world.effect_ids().iter().any(|id| matches!(
        id.command(),
        Command::CleanupVm { .. } | Command::CleanupSlot { .. }
    )));
    let next = retire(&world, request(&world, SID)).unwrap();
    assert_no_detail(&next, SID);
}

fn assert_abort_prepare_rejection_order(dispatched: &World, abort: &EffectId, key: OperationKey) {
    // The extracted value-only compensation guard must still precede the plan
    // checks at the public worker entry, but follow missing-plan rejection.
    for status in [
        OperationStatus::Pending,
        OperationStatus::OutcomeUnknown,
        OperationStatus::Succeeded,
        OperationStatus::Failed(FailureKind::PolicyFailure),
        OperationStatus::Compensating(FailureKind::PolicyFailure),
    ] {
        let mut invalid = dispatched.clone();
        let mut record = invalid.sandbox_record(SID).unwrap().clone();
        record.current_operation.as_mut().unwrap().status = status;
        invalid.sandboxes.put(SID, record);
        let mut invalid_plan = invalid.operation_plan(key).unwrap().clone();
        invalid_plan.sealed = false;
        invalid.operation_plans.put(key, invalid_plan);
        let before = invalid.clone();
        let expected = if matches!(status, OperationStatus::Compensating(_)) {
            WorkerReject::PlanMismatch
        } else {
            WorkerReject::WrongPhase
        };
        assert_eq!(crate::worker::handle_worker(&invalid, abort), Err(expected));
        assert_eq!(invalid, before);
        invalid.operation_plans = Store::default();
        assert_eq!(
            crate::worker::handle_worker(&invalid, abort),
            Err(WorkerReject::PlanNotFound)
        );
    }
}

#[test]
fn snapshot_history_cross_references_and_inconsistent_effect_origins_fail_closed() {
    let world = finished();
    let mut snapshot = world.clone();
    snapshot.snapshots.put(
        SnapshotId(99),
        Snapshot {
            owner: OWNER,
            sandbox: SID,
            source_generation: 1,
            workspace: FsId(SID.0),
            manifest: SnapshotManifestRef {
                digest: ManifestDigest(99),
            },
            stage: SnapshotStage::Durable,
            immutable: true,
        },
    );
    assert_eq!(
        retire(&snapshot, request(&snapshot, SID)),
        Err(RetirementReject::SnapshotHistoryUnsupported)
    );
    let mut cross = create(&world, SandboxId(11));
    let mut other = cross.sandbox_record(SandboxId(11)).unwrap().clone();
    other.current_vm = Some(world.resource_index(SID).vms[0]);
    cross.sandboxes.put(SandboxId(11), other);
    assert_eq!(
        retire(&cross, request(&cross, SID)),
        Err(RetirementReject::DependencyNotClosed)
    );
    let mut origin = world.clone();
    let id = effect(&origin, SID, |c| matches!(c, Command::PrepareVm { .. }));
    let mut logical = origin.logical_effects.get_copy(&id.logical).unwrap();
    logical.origin = EffectOrigin::Maintenance(SandboxId(999));
    origin.logical_effects.put(id.logical, logical);
    assert_eq!(
        retire(&origin, request(&origin, SID)),
        Err(RetirementReject::LedgerMismatch)
    );
}

#[test]
fn occupied_and_releasing_positions_block_but_free_last_binding_survives() {
    let world = finished();
    let index = world.resource_index(SID);
    let binding = HostBinding {
        owner: OWNER,
        sandbox: SID,
        generation: 1,
        vm: index.vms[0],
        network: index.slots[0],
    };
    for phase in [
        HostSlotPhase::Occupied,
        HostSlotPhase::Releasing,
        HostSlotPhase::Free,
    ] {
        let mut candidate = world.clone();
        candidate
            .maintenance
            .as_mut()
            .unwrap()
            .slots
            .put(HostSlotId(0), HostSlotRecord { binding, phase });
        let result = retire(&candidate, request(&candidate, SID));
        if phase == HostSlotPhase::Free {
            assert_eq!(
                result
                    .unwrap()
                    .maintenance
                    .as_ref()
                    .unwrap()
                    .slots
                    .get_copy(&HostSlotId(0)),
                Some(HostSlotRecord { binding, phase })
            );
        } else {
            assert_eq!(result, Err(RetirementReject::ResourcesNotReclaimed));
        }
    }
}

#[test]
fn retention_budget_and_expiry_are_atomic_and_duplicate_retirement_rejects() {
    let mut world = finished();
    world.maintenance.as_mut().unwrap().max_retained_results = 1;
    let before = world.clone();
    assert_eq!(
        retire(&world, request(&world, SID)),
        Err(RetirementReject::RetentionCapacity)
    );
    assert_eq!(world, before);
    world.maintenance.as_mut().unwrap().max_retained_results = 2;
    let mut expired = request(&world, SID);
    for deadline in [4, 5] {
        expired.retain_until = deadline;
        assert_eq!(
            retire(&world, expired),
            Err(RetirementReject::RetentionExpired)
        );
    }
    let req = request(&world, SID);
    let next = retire(&world, req).unwrap();
    assert_eq!(retire(&next, req), Err(RetirementReject::SandboxNotFound));
}

#[test]
fn retained_queries_preserve_authorization_exact_replay_and_expiry_boundary() {
    let world = finished();
    let retired = retire(&world, request(&world, SID)).unwrap();
    let query = QueryRequest {
        actor: Actor::Tenant(OWNER),
        sandbox: SID,
        operation: OperationId(1),
    };
    let ledger = world
        .operation_record(OperationKey {
            sandbox: SID,
            operation: OperationId(1),
        })
        .unwrap();
    let replay = ReplayRequest {
        actor: Actor::Tenant(OWNER),
        sandbox: SID,
        operation: OperationId(1),
        kind: ledger.operation.kind,
        input: ledger.input.clone(),
    };
    assert_eq!(
        handle_query(
            &retired,
            QueryRequest {
                actor: Actor::Tenant(TenantId(999)),
                ..query.clone()
            }
        ),
        Err(Reject::Unauthorized)
    );
    assert_eq!(
        handle_replay(
            &retired,
            ReplayRequest {
                actor: Actor::Tenant(TenantId(999)),
                ..replay.clone()
            }
        ),
        Err(Reject::Unauthorized)
    );
    assert_eq!(
        handle_replay(
            &retired,
            ReplayRequest {
                kind: OperationKind::Kill,
                ..replay.clone()
            }
        ),
        Err(Reject::LedgerMismatch)
    );
    assert_eq!(
        handle_replay(
            &retired,
            ReplayRequest {
                input: OperationInput::Create {
                    workspace: FsId(999),
                    network: NetworkPolicy::DenyAll,
                    file_policy: FilePolicyRef {
                        digest: PolicyDigest(3)
                    },
                },
                ..replay.clone()
            }
        ),
        Err(Reject::LedgerMismatch)
    );
    let before_expiry =
        handle_maintenance(&retired, MaintenanceInput::ExpireResults { now_ms: 99 })
            .unwrap()
            .next;
    assert!(handle_query(&before_expiry, query.clone()).is_ok());
    assert!(handle_replay(&before_expiry, replay.clone()).is_ok());
    let mut down = before_expiry.clone();
    down.controller_up = false;
    assert!(handle_query(&down, query.clone()).is_ok());
    assert!(handle_replay(&down, replay.clone()).is_ok());
    let expired = handle_maintenance(
        &before_expiry,
        MaintenanceInput::ExpireResults { now_ms: 100 },
    )
    .unwrap()
    .next;
    assert_eq!(
        handle_query(&expired, query.clone()),
        Err(Reject::ResultExpired)
    );
    assert_eq!(handle_replay(&expired, replay), Err(Reject::ResultExpired));
    // Once deleted, the compact admission frontier discloses no old owner or
    // result. All unavailable identities below it receive the same refusal.
    assert_eq!(
        handle_query(
            &expired,
            QueryRequest {
                actor: Actor::Tenant(TenantId(999)),
                ..query
            }
        ),
        Err(Reject::ResultExpired)
    );
}

#[test]
fn retirement_rejects_current_epoch_or_generation_disagreement() {
    let world = finished();
    let key = OperationKey {
        sandbox: SID,
        operation: OperationId(2),
    };
    let mut epoch = world.clone();
    let mut record = epoch.sandbox_record(SID).unwrap().clone();
    record.epoch += 1;
    epoch.sandboxes.put(SID, record);
    assert_eq!(
        retire(&epoch, request(&epoch, SID)),
        Err(RetirementReject::LedgerMismatch)
    );
    let mut generation = world.clone();
    let mut plan = generation.operation_plan(key).unwrap().clone();
    plan.generation += 1;
    generation.operation_plans.put(key, plan);
    assert_eq!(
        retire(&generation, request(&generation, SID)),
        Err(RetirementReject::LedgerMismatch)
    );
}

#[test]
fn zero_and_maximum_explicit_id_can_retire_without_counter_arithmetic() {
    let mut world = enabled(&initial_world(), 10);
    for sid in [SandboxId(0), SandboxId(u64::MAX)] {
        world = create(&world, sid);
        world = destroy(&world, sid);
        world = retire(&world, request(&world, sid)).unwrap();
        assert_no_detail(&world, sid);
        assert_eq!(
            world.maintenance.as_ref().unwrap().sandbox_frontier,
            Some(sid.0)
        );
        assert_eq!(
            world.maintenance.as_ref().unwrap().workspace_frontier,
            Some(sid.0)
        );
    }
    assert_eq!(world.next_vm_id, 0);
    assert_eq!(
        world.maintenance.as_ref().unwrap().retained.entries().len(),
        4
    );
}

#[test]
fn repeated_retirement_keeps_details_bounded_while_an_older_sandbox_stays_live() {
    let mut world = running(&enabled(&initial_world(), 2), SandboxId(1));
    let older = world.sandbox_record(SandboxId(1)).unwrap().clone();
    let baseline = (
        world.effects.entries().len(),
        world.logical_effects.entries().len(),
        world.inflight.len(),
    );
    for identity in 2..34 {
        let sid = SandboxId(identity);
        world = running(&world, sid);
        world = cleanup(&destroy(&world, sid), sid);
        let mut retirement = request(&world, sid);
        retirement.retain_until = 100 + identity;
        world = retire(&world, retirement).unwrap();
        assert_no_detail(&world, sid);
        assert_eq!(world.sandbox_record(SandboxId(1)), Some(&older));
        assert_eq!(world.sandboxes.entries().len(), 1);
        assert_eq!(world.filesystems.entries().len(), 1);
        assert_eq!(world.vms.entries().len(), 1);
        assert_eq!(world.net_slots.entries().len(), 1);
        assert_eq!(world.operations.entries().len(), 1);
        assert_eq!(world.operation_plans.entries().len(), 1);
        assert_eq!(
            (
                world.effects.entries().len(),
                world.logical_effects.entries().len(),
                world.inflight.len()
            ),
            baseline
        );
        assert_eq!(
            world.maintenance.as_ref().unwrap().retained.entries().len(),
            2
        );
        world = handle_maintenance(
            &world,
            MaintenanceInput::ExpireResults {
                now_ms: 100 + identity,
            },
        )
        .unwrap()
        .next;
        assert_eq!(
            world.maintenance.as_ref().unwrap().retained.entries().len(),
            0
        );
    }
    assert_eq!(world.next_vm_id, 33);
}

#[test]
fn foreign_stop_coverage_and_any_unknown_attempt_prevent_deleting_dependencies() {
    let finished = finished();
    let foreign = create(&finished, SandboxId(11));
    let id = effect(&foreign, SandboxId(11), |c| {
        matches!(c, Command::InstallFence { .. })
    });
    let coverage = StopCoverage {
        operation: OperationKey {
            sandbox: SID,
            operation: OperationId(2),
        },
        epoch: 2,
        attempt: 0,
        result: ResultDigest(1),
    };
    let mut foreign_status = foreign.clone();
    foreign_status
        .effects
        .put(id.clone(), EffectStatus::RetiredByStop(coverage));
    assert_eq!(
        retire(&foreign_status, request(&foreign_status, SID)),
        Err(RetirementReject::DependencyNotClosed)
    );
    let mut foreign_logical = foreign.clone();
    let mut logical = foreign_logical
        .logical_effects
        .get_copy(&id.logical)
        .unwrap();
    logical.result = Some(LogicalEffectResult::RetiredByStop {
        attempt: id.attempt,
        coverage,
    });
    foreign_logical.logical_effects.put(id.logical, logical);
    assert_eq!(
        retire(&foreign_logical, request(&foreign_logical, SID)),
        Err(RetirementReject::DependencyNotClosed)
    );
    for status in [EffectStatus::Dispatched, EffectStatus::OutcomeUnknown] {
        let mut unknown = finished.clone();
        let id = effect(&unknown, SID, |c| matches!(c, Command::ActivateVm { .. }));
        unknown.effects.put(id, status);
        assert_eq!(
            retire(&unknown, request(&unknown, SID)),
            Err(RetirementReject::UnresolvedEffects)
        );
    }
}

#[test]
fn logical_cleanup_precedes_free_without_weakening_exact_evidence() {
    let world = finished();
    let index = world.resource_index(SID);
    let binding = HostBinding {
        owner: OWNER,
        sandbox: SID,
        generation: 1,
        vm: index.vms[0],
        network: index.slots[0],
    };
    for phase in [
        HostSlotPhase::Occupied,
        HostSlotPhase::Releasing,
        HostSlotPhase::Free,
    ] {
        let mut candidate = world.clone();
        candidate
            .maintenance
            .as_mut()
            .unwrap()
            .slots
            .put(HostSlotId(0), HostSlotRecord { binding, phase });
        let before = candidate.clone();
        validate_terminal_cleanup(&candidate, SID).unwrap();
        assert_eq!(candidate, before);
        if phase != HostSlotPhase::Free {
            assert!(retire(&candidate, request(&candidate, SID)).is_err());
        }
    }
    for status in [EffectStatus::Dispatched, EffectStatus::OutcomeUnknown] {
        let mut unknown = world.clone();
        let id = effect(&unknown, SID, |c| matches!(c, Command::CleanupVm { .. }));
        unknown.effects.put(id, status);
        assert!(validate_terminal_cleanup(&unknown, SID).is_err());
    }
    let failed = failed_with_unissued_startup();
    validate_terminal_cleanup(&failed, SID).unwrap();
    let mut foreign = create(&world, SandboxId(11));
    let mut row = foreign.sandbox_record(SandboxId(11)).unwrap().clone();
    row.current_vm = Some(index.vms[0]);
    foreign.sandboxes.put(SandboxId(11), row);
    assert_eq!(
        validate_terminal_cleanup(&foreign, SID),
        Err(RetirementReject::DependencyNotClosed)
    );
    let mut mismatch = world.clone();
    let mut vm = *mismatch.vm_record(index.vms[0]).unwrap();
    vm.owner = TenantId(99);
    mismatch.vms.put(index.vms[0], vm);
    assert!(validate_terminal_cleanup(&mismatch, SID).is_err());
}
