//! Checked current-state checkpoints using one supported storage format.
//!
//! This is a recovery boundary, not an arbitrary `World` deserializer. The
//! opaque wire value cannot expose a World until `restore` checks its bounds,
//! identities and references. Capture preserves record and queue order, all
//! allocation cursors, unknown work and maintenance tombstones. It does not
//! discard history: the checked retirement reducer must compact it first.
//!
//! Pause/capture state uses the same format and validation as startup and kill.
//! Resume and repeated captures check consecutive epochs, native generations,
//! exact original sources and each resource's own allocation plan. This validator
//! checks structure under the host contracts; it is
//! not a proof of historical reachability or authenticity of host evidence.
//! Record bounds alone do not establish a byte bound. The service must bound
//! encoded input BEFORE parsing and use a bounded writer when encoding, then
//! bind this value to its input cut, SQL state and immutable configuration in
//! one database snapshot. No production limit defaults are supplied here.

mod bounds;
#[cfg(feature = "serde")]
mod strict;
#[cfg(test)]
mod tests;
mod validate;

use crate::maintenance::{
    MaintenanceState,
    retirement::RetainedOperation,
    slots::{HostSlotId, HostSlotRecord},
    windows::WindowState,
};
use crate::{
    AppliedFilePolicy, EffectId, EffectStatus, Filesystem, FsId, LogicalEffectId,
    LogicalEffectRecord, NetSlot, NetSlotId, OperationKey, OperationLedgerRecord, OperationPlan,
    ResourceIndex, SandboxId, SandboxRecord, Snapshot, SnapshotId, Store, Vm, VmId, WorkerFence,
    World,
};

pub const CHECKPOINT_VERSION: u16 = 2;
/// Explicit aggregate record and nested-reference budgets. Neither value may
/// be zero. Strings and encoded bytes require a separate service-side bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointLimits {
    pub max_records: u64,
    pub max_references: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointReject {
    InvalidLimits,
    RecordLimit,
    ReferenceLimit,
    UnsupportedVersion,
    UnsupportedSnapshotHistory,
    DuplicateKey,
    InvalidReference,
    InvalidLedger,
    InvalidResource,
    InvalidAuthority,
    InvalidEffect,
    InvalidMaintenance,
    InvalidFrontier,
}

/// Opaque, untrusted checkpoint encoding. Deserialization checks syntax and
/// rejects unknown fields recursively; only `restore` validates the semantic
/// state against the caller's explicit bounds and produces a World.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Checkpoint(CheckpointWire);

impl Checkpoint {
    /// The untrusted schema tag, used to check the service envelope binding.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.0.version
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Checkpoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <CheckpointWire as serde::Deserialize>::deserialize(strict::Strict(deserializer)).map(Self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
struct CheckpointWire {
    version: u16,
    maintenance: Option<MaintenanceWire>,
    sandboxes: Vec<(SandboxId, SandboxRecord)>,
    vms: Vec<(VmId, Vm)>,
    filesystems: Vec<(FsId, Filesystem)>,
    snapshots: Vec<(SnapshotId, Snapshot)>,
    net_slots: Vec<(NetSlotId, NetSlot)>,
    applied_file_policies: Vec<(SandboxId, AppliedFilePolicy)>,
    worker_fences: Vec<(SandboxId, WorkerFence)>,
    operations: Vec<(OperationKey, OperationLedgerRecord)>,
    operation_plans: Vec<(OperationKey, OperationPlan)>,
    resource_indexes: Vec<(SandboxId, ResourceIndex)>,
    logical_effects: Vec<(LogicalEffectId, LogicalEffectRecord)>,
    effect_keys: Vec<EffectId>,
    effects: Vec<(EffectId, EffectStatus)>,
    next_vm_id: u64,
    next_slot_id: u64,
    next_snapshot_id: u64,
    next_manifest_digest: u64,
    inflight: Vec<EffectId>,
    controller_up: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
struct MaintenanceWire {
    sandbox_frontier: Option<u64>,
    workspace_frontier: Option<u64>,
    retained: Vec<(OperationKey, RetainedOperation)>,
    time_floor: u64,
    max_retained_results: u64,
    max_host_slots: u64,
    slots: Vec<(HostSlotId, HostSlotRecord)>,
    windows: WindowState,
}

impl From<&MaintenanceState> for MaintenanceWire {
    fn from(value: &MaintenanceState) -> Self {
        Self {
            sandbox_frontier: value.sandbox_frontier,
            workspace_frontier: value.workspace_frontier,
            retained: value.retained.entries().to_vec(),
            time_floor: value.time_floor,
            max_retained_results: value.max_retained_results,
            max_host_slots: value.max_host_slots,
            slots: value.slots.entries().to_vec(),
            windows: value.windows.clone(),
        }
    }
}

impl From<&World> for CheckpointWire {
    fn from(world: &World) -> Self {
        Self {
            version: CHECKPOINT_VERSION,
            maintenance: world.maintenance.as_ref().map(MaintenanceWire::from),
            sandboxes: world.sandboxes.entries().to_vec(),
            vms: world.vms.entries().to_vec(),
            filesystems: world.filesystems.entries().to_vec(),
            snapshots: world.snapshots.entries().to_vec(),
            net_slots: world.net_slots.entries().to_vec(),
            applied_file_policies: world.applied_file_policies.entries().to_vec(),
            worker_fences: world.worker_fences.entries().to_vec(),
            operations: world.operations.entries().to_vec(),
            operation_plans: world.operation_plans.entries().to_vec(),
            resource_indexes: world.resource_indexes.entries().to_vec(),
            logical_effects: world.logical_effects.entries().to_vec(),
            effect_keys: world.effect_keys.clone(),
            effects: world.effects.entries().to_vec(),
            next_vm_id: world.next_vm_id,
            next_slot_id: world.next_slot_id,
            next_snapshot_id: world.next_snapshot_id,
            next_manifest_digest: world.next_manifest_digest,
            inflight: world.inflight.clone(),
            controller_up: world.controller_up,
        }
    }
}

fn restore_store<K: PartialEq, V>(rows: Vec<(K, V)>) -> Result<Store<K, V>, CheckpointReject> {
    let mut store = Store::default();
    for (key, value) in rows {
        if store.contains_key(&key) {
            return Err(CheckpointReject::DuplicateKey);
        }
        store.put(key, value);
    }
    Ok(store)
}

impl CheckpointWire {
    fn into_world(self) -> Result<World, CheckpointReject> {
        let maintenance = self
            .maintenance
            .map(|state| {
                Ok(MaintenanceState {
                    sandbox_frontier: state.sandbox_frontier,
                    workspace_frontier: state.workspace_frontier,
                    retained: restore_store(state.retained)?,
                    time_floor: state.time_floor,
                    max_retained_results: state.max_retained_results,
                    max_host_slots: state.max_host_slots,
                    slots: restore_store(state.slots)?,
                    windows: state.windows,
                })
            })
            .transpose()?;
        Ok(World {
            maintenance,
            sandboxes: restore_store(self.sandboxes)?,
            vms: restore_store(self.vms)?,
            filesystems: restore_store(self.filesystems)?,
            snapshots: restore_store(self.snapshots)?,
            net_slots: restore_store(self.net_slots)?,
            applied_file_policies: restore_store(self.applied_file_policies)?,
            worker_fences: restore_store(self.worker_fences)?,
            operations: restore_store(self.operations)?,
            operation_plans: restore_store(self.operation_plans)?,
            resource_indexes: restore_store(self.resource_indexes)?,
            logical_effects: restore_store(self.logical_effects)?,
            effect_keys: self.effect_keys,
            effects: restore_store(self.effects)?,
            next_vm_id: self.next_vm_id,
            next_slot_id: self.next_slot_id,
            next_snapshot_id: self.next_snapshot_id,
            next_manifest_digest: self.next_manifest_digest,
            inflight: self.inflight,
            controller_up: self.controller_up,
        })
    }
}

/// Capture a structurally valid, already-compacted World, preserving order.
/// # Errors
/// Rejects unsupported lifecycle history, invalid references or exceeded bounds.
pub fn capture(world: &World, limits: CheckpointLimits) -> Result<Checkpoint, CheckpointReject> {
    bounds::world(world, limits)?;
    validate::world(world)?;
    Ok(Checkpoint(CheckpointWire::from(world)))
}

/// Restore only after validating the version, bounds and complete references.
/// No partially restored World escapes on rejection.
/// # Errors
/// Rejects malformed, unsupported or over-budget checkpoint values.
pub fn restore(
    checkpoint: Checkpoint,
    limits: CheckpointLimits,
) -> Result<World, CheckpointReject> {
    let wire = checkpoint.0;
    if wire.version != CHECKPOINT_VERSION {
        return Err(CheckpointReject::UnsupportedVersion);
    }
    bounds::wire(&wire, limits)?;
    let world = wire.into_world()?;
    validate::world(&world)?;
    Ok(world)
}

// Metadata retirement of snapshot histories uses these same source, epoch,
// generation and reference checks. This is structural validation only: the
// original retirement certificate must separately attest actual quiescence.
pub(crate) fn valid_snapshot_retirement_history(world: &World, sandbox: SandboxId) -> bool {
    validate::world(world).is_ok()
        && world.operation_plans.entries().iter().all(|(key, plan)| {
            key.sandbox != sandbox
                || plan.capture.is_none_or(|capture| {
                    capture.eligibility() == crate::capture::RestoreEligibility::Revoked
                })
        })
}
