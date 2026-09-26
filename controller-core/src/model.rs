use crate::ids::{
    AttemptNo, Epoch, FsId, Generation, ManifestDigest, NetSlotId, OperationId, PolicyDigest,
    ResultDigest, SandboxId, SnapshotId, TenantId, VmId,
};
use crate::store::Store;

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Actor {
    Tenant(TenantId),
    SystemController,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Phase {
    Starting,
    Running,
    Pausing,
    Paused,
    Revoking,
    Destroyed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VmStatus {
    Prepared,
    Executing,
    Frozen,
    Stopped,
    Reclaimed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SnapshotStage {
    LocalSealed,
    Durable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WorkspaceApiAccess {
    ManagedReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FilePolicyRef {
    pub digest: PolicyDigest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SnapshotManifestRef {
    pub digest: ManifestDigest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RestoreSource {
    pub snapshot: SnapshotId,
    pub manifest: SnapshotManifestRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Endpoint {
    pub address: String,
    pub prefix_length: u64,
    pub protocol: Protocol,
    pub port: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NetworkPolicy {
    DenyAll,
    StaticAllowlist(Vec<Endpoint>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OperationInput {
    Create {
        workspace: FsId,
        network: NetworkPolicy,
        file_policy: FilePolicyRef,
    },
    Pause {
        generation: Generation,
    },
    Resume {
        source: RestoreSource,
    },
    Kill {
        generation: Generation,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FailureKind {
    Cancelled,
    DeadlineExceeded,
    ResourceFailure,
    PolicyFailure,
    StorageFailure,
    InternalFailure,
}

impl FailureKind {
    #[must_use]
    pub const fn retryable(self) -> bool {
        match self {
            Self::Cancelled | Self::PolicyFailure => false,
            Self::DeadlineExceeded
            | Self::ResourceFailure
            | Self::StorageFailure
            | Self::InternalFailure => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OperationResult {
    Running(Generation),
    Paused(RestoreSource),
    Destroyed,
    Failed(FailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OperationKind {
    Create,
    Pause,
    Resume,
    Kill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OperationStatus {
    Pending,
    OutcomeUnknown,
    Compensating(FailureKind),
    Succeeded,
    Failed(FailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Operation {
    pub id: OperationId,
    pub epoch: Epoch,
    pub kind: OperationKind,
    pub status: OperationStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Authority {
    Closed,
    Running {
        sandbox: SandboxId,
        generation: Generation,
        vm: VmId,
        workspace: FsId,
        access: WorkspaceApiAccess,
        slot: NetSlotId,
        policy: NetworkPolicy,
    },
    Paused {
        sandbox: SandboxId,
        source: RestoreSource,
    },
}

impl Authority {
    #[must_use]
    pub fn without_resume(&self) -> Self {
        match self {
            Self::Paused { .. } => Self::Closed,
            authority => authority.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Command {
    InstallFence {
        sandbox: SandboxId,
        epoch: Epoch,
        terminal: bool,
    },
    PrepareVm {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        vm: VmId,
        slot: NetSlotId,
        source: Option<RestoreSource>,
    },
    ConfigureNetwork {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        vm: VmId,
        slot: NetSlotId,
        policy: NetworkPolicy,
    },
    ActivateVm {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        vm: VmId,
    },
    FreezeVm {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        vm: VmId,
    },
    ApplyFilePolicy {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        workspace: FsId,
        policy: FilePolicyRef,
    },
    SealSnapshot {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        vm: VmId,
        workspace: FsId,
        snapshot: SnapshotId,
        manifest: SnapshotManifestRef,
    },
    CommitSnapshot {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        snapshot: SnapshotId,
        manifest: SnapshotManifestRef,
    },
    StopVm {
        sandbox: SandboxId,
        epoch: Epoch,
        generation: Generation,
        vm: VmId,
    },
    CleanupVm {
        sandbox: SandboxId,
        generation: Generation,
        vm: VmId,
    },
    CleanupSlot {
        sandbox: SandboxId,
        generation: Generation,
        slot: NetSlotId,
    },
}

impl Command {
    #[must_use]
    pub const fn sandbox(&self) -> SandboxId {
        match self {
            Self::InstallFence { sandbox, .. }
            | Self::PrepareVm { sandbox, .. }
            | Self::ConfigureNetwork { sandbox, .. }
            | Self::ActivateVm { sandbox, .. }
            | Self::FreezeVm { sandbox, .. }
            | Self::ApplyFilePolicy { sandbox, .. }
            | Self::SealSnapshot { sandbox, .. }
            | Self::CommitSnapshot { sandbox, .. }
            | Self::StopVm { sandbox, .. }
            | Self::CleanupVm { sandbox, .. }
            | Self::CleanupSlot { sandbox, .. } => *sandbox,
        }
    }

    #[must_use]
    pub const fn epoch(&self) -> Option<Epoch> {
        match self {
            Self::InstallFence { epoch, .. }
            | Self::PrepareVm { epoch, .. }
            | Self::ConfigureNetwork { epoch, .. }
            | Self::ActivateVm { epoch, .. }
            | Self::FreezeVm { epoch, .. }
            | Self::ApplyFilePolicy { epoch, .. }
            | Self::SealSnapshot { epoch, .. }
            | Self::CommitSnapshot { epoch, .. }
            | Self::StopVm { epoch, .. } => Some(*epoch),
            Self::CleanupVm { .. } | Self::CleanupSlot { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LogicalEffectId {
    pub command: Command,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EffectId {
    pub logical: LogicalEffectId,
    pub attempt: AttemptNo,
}

impl EffectId {
    #[must_use]
    pub fn initial(command: Command) -> Self {
        Self {
            logical: LogicalEffectId { command },
            attempt: 0,
        }
    }

    #[must_use]
    pub const fn command(&self) -> &Command {
        &self.logical.command
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EffectOutcome {
    Succeeded(ResultDigest),
    Failed(FailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EffectStatus {
    Committed,
    Dispatched,
    OutcomeUnknown,
    Acknowledged(EffectOutcome),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OperationKey {
    pub sandbox: SandboxId,
    pub operation: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OperationPlan {
    pub key: OperationKey,
    pub epoch: Epoch,
    pub generation: Generation,
    pub vm: Option<VmId>,
    pub slot: Option<NetSlotId>,
    pub restore_from: Option<RestoreSource>,
    pub target_snapshot: Option<RestoreSource>,
    pub stop_vms: Vec<VmId>,
    pub close_slots: Vec<NetSlotId>,
    pub retry_limit: u64,
    pub sealed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceIndex {
    pub vms: Vec<VmId>,
    pub slots: Vec<NetSlotId>,
    pub snapshots: Vec<SnapshotId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EffectOrigin {
    Operation(OperationKey),
    Maintenance(SandboxId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LogicalEffectResult {
    Succeeded {
        attempt: AttemptNo,
        result: ResultDigest,
    },
    Failed {
        attempt: AttemptNo,
        failure: FailureKind,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LogicalEffectRecord {
    pub origin: EffectOrigin,
    pub current_attempt: AttemptNo,
    pub remaining_retries: u64,
    pub result: Option<LogicalEffectResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OperationLedgerRecord {
    pub sandbox: SandboxId,
    pub operation: Operation,
    pub input: OperationInput,
    pub result: Option<OperationResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SandboxRecord {
    pub owner: TenantId,
    pub phase: Phase,
    pub terminal: bool,
    pub epoch: Epoch,
    pub generation: Generation,
    pub current_vm: Option<VmId>,
    pub workspace: FsId,
    pub workspace_access: WorkspaceApiAccess,
    pub file_policy: FilePolicyRef,
    pub durable_snapshot: Option<RestoreSource>,
    pub restore_from: Option<RestoreSource>,
    pub net_slot: Option<NetSlotId>,
    pub policy: NetworkPolicy,
    pub desired: Authority,
    pub effective: Authority,
    pub current_operation: Option<Operation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Vm {
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub generation: Generation,
    pub status: VmStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Filesystem {
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub created_generation: Generation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Snapshot {
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub source_generation: Generation,
    pub workspace: FsId,
    pub manifest: SnapshotManifestRef,
    pub stage: SnapshotStage,
    pub immutable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NetSlot {
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub generation: Generation,
    pub vm: VmId,
    pub policy: NetworkPolicy,
    pub configured: bool,
    pub active: bool,
    pub reclaimed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AppliedFilePolicy {
    pub sandbox: SandboxId,
    pub generation: Generation,
    pub workspace: FsId,
    pub policy: FilePolicyRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WorkerFence {
    pub epoch: Epoch,
    pub terminal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Event {
    AcceptCreate {
        actor: Actor,
        owner: TenantId,
        sandbox: SandboxId,
        workspace: FsId,
        file_policy: FilePolicyRef,
        policy: NetworkPolicy,
        operation: OperationId,
    },
    AcceptPause {
        actor: Actor,
        sandbox: SandboxId,
        operation: OperationId,
    },
    AcceptResume {
        actor: Actor,
        sandbox: SandboxId,
        operation: OperationId,
    },
    AcceptKill {
        actor: Actor,
        sandbox: SandboxId,
        operation: OperationId,
    },
    RetryOperation {
        actor: Actor,
        sandbox: SandboxId,
        operation: OperationId,
        kind: OperationKind,
        input: OperationInput,
    },
    QueryOperation {
        actor: Actor,
        sandbox: SandboxId,
        operation: OperationId,
        status: OperationStatus,
        result: Option<OperationResult>,
    },
    BeginCompensation {
        sandbox: SandboxId,
        failure: FailureKind,
    },
    ReserveStartupResources {
        key: OperationKey,
        vm: VmId,
        slot: NetSlotId,
    },
    ReservePauseSnapshot {
        key: OperationKey,
        snapshot: SnapshotId,
        manifest: SnapshotManifestRef,
    },
    DispatchPrepare {
        sandbox: SandboxId,
        vm: VmId,
        slot: NetSlotId,
        source: Option<RestoreSource>,
    },
    ScheduleSealSnapshot {
        sandbox: SandboxId,
        snapshot: SnapshotId,
        manifest: SnapshotManifestRef,
    },
    ScheduleCommitSnapshot {
        sandbox: SandboxId,
        snapshot: SnapshotId,
    },
    ScheduleStopVm {
        sandbox: SandboxId,
        generation: Generation,
        vm: VmId,
    },
    ScheduleCleanupVm {
        sandbox: SandboxId,
        generation: Generation,
        vm: VmId,
    },
    ScheduleCleanupSlot {
        sandbox: SandboxId,
        generation: Generation,
        slot: NetSlotId,
    },
    DispatchEffect(EffectId),
    AcknowledgeEffect(EffectId, EffectOutcome),
    EffectTimeout(EffectId),
    StartNextAttempt(EffectId, EffectId),
    FinalizeEffectFailure(EffectId, FailureKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    pub(crate) sandboxes: Store<SandboxId, SandboxRecord>,
    pub(crate) vms: Store<VmId, Vm>,
    pub(crate) filesystems: Store<FsId, Filesystem>,
    pub(crate) snapshots: Store<SnapshotId, Snapshot>,
    pub(crate) net_slots: Store<NetSlotId, NetSlot>,
    pub(crate) applied_file_policies: Store<SandboxId, AppliedFilePolicy>,
    pub(crate) worker_fences: Store<SandboxId, WorkerFence>,
    pub(crate) operations: Store<OperationKey, OperationLedgerRecord>,
    pub(crate) operation_plans: Store<OperationKey, OperationPlan>,
    pub(crate) resource_indexes: Store<SandboxId, ResourceIndex>,
    pub(crate) logical_effects: Store<LogicalEffectId, LogicalEffectRecord>,
    pub(crate) effect_keys: Vec<EffectId>,
    pub(crate) effects: Store<EffectId, EffectStatus>,
    pub(crate) next_vm_id: u64,
    pub(crate) next_slot_id: u64,
    pub(crate) next_snapshot_id: u64,
    pub(crate) next_manifest_digest: u64,
    pub(crate) inflight: Vec<EffectId>,
    pub(crate) controller_up: bool,
}

impl Default for World {
    fn default() -> Self {
        Self {
            sandboxes: Store::default(),
            vms: Store::default(),
            filesystems: Store::default(),
            snapshots: Store::default(),
            net_slots: Store::default(),
            applied_file_policies: Store::default(),
            worker_fences: Store::default(),
            operations: Store::default(),
            operation_plans: Store::default(),
            resource_indexes: Store::default(),
            logical_effects: Store::default(),
            effect_keys: Vec::new(),
            effects: Store::default(),
            next_vm_id: 0,
            next_slot_id: 0,
            next_snapshot_id: 0,
            next_manifest_digest: 0,
            inflight: Vec::new(),
            controller_up: true,
        }
    }
}

/// The only supported construction of a fresh durable controller state.
///
/// Keeping this as an explicit function gives the refinement proof a real
/// executable initialization boundary instead of a hand-written duplicate of
/// `World::default()`.
#[must_use]
pub fn initial_world() -> World {
    World::default()
}

impl World {
    /// Read-only integration views. They do not permit constructing or
    /// mutating a World outside the controller reducer.
    #[must_use]
    pub fn sandbox_records(&self) -> &[(SandboxId, SandboxRecord)] {
        self.sandboxes.entries()
    }

    #[must_use]
    pub fn sandbox_record(&self, sandbox: SandboxId) -> Option<&SandboxRecord> {
        self.sandboxes.get(&sandbox)
    }

    #[must_use]
    pub fn vm_record(&self, vm: VmId) -> Option<&Vm> {
        self.vms.get(&vm)
    }

    #[must_use]
    pub fn slot_record(&self, slot: NetSlotId) -> Option<&NetSlot> {
        self.net_slots.get(&slot)
    }

    #[must_use]
    pub fn operation_record(&self, key: OperationKey) -> Option<&OperationLedgerRecord> {
        self.operations.get(&key)
    }

    #[must_use]
    pub fn operation_records(&self) -> &[(OperationKey, OperationLedgerRecord)] {
        self.operations.entries()
    }

    #[must_use]
    pub fn operation_plan(&self, key: OperationKey) -> Option<&OperationPlan> {
        self.operation_plans.get(&key)
    }

    #[must_use]
    pub fn effect_ids(&self) -> &[EffectId] {
        &self.effect_keys
    }

    #[must_use]
    pub fn effect_status(&self, effect: &EffectId) -> Option<EffectStatus> {
        self.effects.get_copy(effect)
    }

    #[must_use]
    pub fn logical_effect(&self, effect: &LogicalEffectId) -> Option<LogicalEffectRecord> {
        self.logical_effects.get_copy(effect)
    }

    #[must_use]
    pub fn resource_index(&self, sandbox: SandboxId) -> ResourceIndex {
        self.resource_indexes
            .get(&sandbox)
            .cloned()
            .unwrap_or_default()
    }
}
