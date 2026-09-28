use crate::model::{
    Actor, EffectId, EffectOrigin, EffectOutcome, EffectStatus, Event, FailureKind, FilePolicyRef,
    LogicalEffectResult, NetworkPolicy, OperationInput, OperationKind, OperationStatus, Phase,
    World,
};
use crate::transition::{
    authorized, begin_compensation, begin_create, begin_kill, begin_pause, begin_resume,
    operation_key, supersede_ready_check,
};
use crate::{FsId, OperationId, SandboxId, TenantId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    ControllerDown,
    SandboxExists,
    SandboxNotFound,
    WorkspaceExists,
    OperationExists,
    OperationPlanExists,
    Unauthorized,
    WrongPhase,
    Terminal,
    NoCurrentVm,
    SnapshotUnavailable,
    OperationUnavailable,
    OperationNotCompensable,
    OperationAlreadyFinal,
    LedgerMismatch,
    FailureUnjustified,
    CounterExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub event: Event,
    pub next: World,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CreateRequest {
    pub actor: Actor,
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub workspace: FsId,
    pub file_policy: FilePolicyRef,
    pub policy: NetworkPolicy,
    pub operation: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PauseRequest {
    pub actor: Actor,
    pub sandbox: SandboxId,
    pub operation: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResumeRequest {
    pub actor: Actor,
    pub sandbox: SandboxId,
    pub operation: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KillRequest {
    pub actor: Actor,
    pub sandbox: SandboxId,
    pub operation: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReplayRequest {
    pub actor: Actor,
    pub sandbox: SandboxId,
    pub operation: OperationId,
    pub kind: OperationKind,
    pub input: OperationInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QueryRequest {
    pub actor: Actor,
    pub sandbox: SandboxId,
    pub operation: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CompensationRequest {
    pub sandbox: SandboxId,
    pub failure: FailureKind,
    pub evidence: Option<EffectId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Request {
    Create(CreateRequest),
    Pause(PauseRequest),
    Resume(ResumeRequest),
    Kill(KillRequest),
    Replay(ReplayRequest),
    Query(QueryRequest),
    Compensate(CompensationRequest),
}

impl From<CreateRequest> for Request {
    fn from(input: CreateRequest) -> Self {
        Self::Create(input)
    }
}

impl From<PauseRequest> for Request {
    fn from(input: PauseRequest) -> Self {
        Self::Pause(input)
    }
}

impl From<ResumeRequest> for Request {
    fn from(input: ResumeRequest) -> Self {
        Self::Resume(input)
    }
}

impl From<KillRequest> for Request {
    fn from(input: KillRequest) -> Self {
        Self::Kill(input)
    }
}

impl From<ReplayRequest> for Request {
    fn from(input: ReplayRequest) -> Self {
        Self::Replay(input)
    }
}

impl From<QueryRequest> for Request {
    fn from(input: QueryRequest) -> Self {
        Self::Query(input)
    }
}

impl From<CompensationRequest> for Request {
    fn from(input: CompensationRequest) -> Self {
        Self::Compensate(input)
    }
}

/// Rust implementation of Lean's `Controller.handleCreate`.
///
/// Check order is part of the observable contract: malformed requests receive
/// the same first rejection as the executable Lean reducer.
///
/// # Errors
///
/// Returns the first `Reject` produced by the same ordered checks as the Lean
/// reducer.  Rejection never mutates `world`.
pub fn handle_create(world: &World, input: CreateRequest) -> Result<Decision, Reject> {
    let CreateRequest {
        actor,
        owner,
        sandbox,
        workspace,
        file_policy,
        policy,
        operation,
    } = input;
    if !world.controller_up {
        return Err(Reject::ControllerDown);
    }
    if world.sandboxes.contains_key(&sandbox) {
        return Err(Reject::SandboxExists);
    }
    if world.filesystems.contains_key(&workspace) {
        return Err(Reject::WorkspaceExists);
    }
    let key = operation_key(sandbox, operation);
    if world.operations.contains_key(&key) {
        return Err(Reject::OperationExists);
    }
    if world.operation_plans.contains_key(&key) {
        return Err(Reject::OperationPlanExists);
    }
    if !authorized(&actor, owner) {
        return Err(Reject::Unauthorized);
    }

    Ok(Decision {
        event: Event::AcceptCreate {
            actor,
            owner,
            sandbox,
            workspace,
            file_policy,
            policy: policy.clone(),
            operation,
        },
        next: begin_create(
            world,
            sandbox,
            owner,
            workspace,
            file_policy,
            policy,
            operation,
        ),
    })
}

/// Rust implementation of Lean's `Controller.handlePause`.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered admission checks.
pub fn handle_pause(world: &World, input: PauseRequest) -> Result<Decision, Reject> {
    let PauseRequest {
        actor,
        sandbox,
        operation,
    } = input;
    if !world.controller_up {
        return Err(Reject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(Reject::SandboxNotFound);
    };
    if record.phase != Phase::Running {
        return Err(Reject::WrongPhase);
    }
    if record.terminal {
        return Err(Reject::Terminal);
    }
    let Some(vm) = record.current_vm else {
        return Err(Reject::NoCurrentVm);
    };
    let key = operation_key(sandbox, operation);
    if world.operations.contains_key(&key) {
        return Err(Reject::OperationExists);
    }
    if world.operation_plans.contains_key(&key) {
        return Err(Reject::OperationPlanExists);
    }
    if !authorized(&actor, record.owner) {
        return Err(Reject::Unauthorized);
    }
    if record.epoch == u64::MAX {
        return Err(Reject::CounterExhausted);
    }
    let epoch = record.epoch + 1;

    Ok(Decision {
        event: Event::AcceptPause {
            actor,
            sandbox,
            operation,
        },
        next: begin_pause(world, sandbox, record, operation, vm, epoch),
    })
}

fn usable_snapshot_check(
    world: &World,
    sandbox: SandboxId,
    record: &crate::SandboxRecord,
    source: crate::RestoreSource,
) -> bool {
    match world.snapshots.get(&source.snapshot) {
        Some(snapshot) => {
            snapshot.owner == record.owner
                && snapshot.sandbox == sandbox
                && snapshot.workspace == record.workspace
                && snapshot.manifest == source.manifest
                && snapshot.stage == crate::SnapshotStage::Durable
                && snapshot.immutable
        }
        None => false,
    }
}

/// Rust implementation of Lean's `Controller.handleResume`.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered admission checks.
pub fn handle_resume(world: &World, input: ResumeRequest) -> Result<Decision, Reject> {
    let ResumeRequest {
        actor,
        sandbox,
        operation,
    } = input;
    if !world.controller_up {
        return Err(Reject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(Reject::SandboxNotFound);
    };
    if record.phase != Phase::Paused {
        return Err(Reject::WrongPhase);
    }
    if record.terminal {
        return Err(Reject::Terminal);
    }
    let Some(source) = record.durable_snapshot else {
        return Err(Reject::SnapshotUnavailable);
    };
    if !usable_snapshot_check(world, sandbox, record, source) {
        return Err(Reject::SnapshotUnavailable);
    }
    let key = operation_key(sandbox, operation);
    if world.operations.contains_key(&key) {
        return Err(Reject::OperationExists);
    }
    if world.operation_plans.contains_key(&key) {
        return Err(Reject::OperationPlanExists);
    }
    if !authorized(&actor, record.owner) {
        return Err(Reject::Unauthorized);
    }
    if record.epoch == u64::MAX || record.generation == u64::MAX {
        return Err(Reject::CounterExhausted);
    }
    let epoch = record.epoch + 1;
    let generation = record.generation + 1;

    Ok(Decision {
        event: Event::AcceptResume {
            actor,
            sandbox,
            operation,
        },
        next: begin_resume(world, sandbox, record, operation, source, epoch, generation),
    })
}

/// Rust implementation of Lean's `Controller.handleKill`.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered admission checks.
pub fn handle_kill(world: &World, input: KillRequest) -> Result<Decision, Reject> {
    let KillRequest {
        actor,
        sandbox,
        operation,
    } = input;
    if !world.controller_up {
        return Err(Reject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(Reject::SandboxNotFound);
    };
    if record.phase == Phase::Destroyed {
        return Err(Reject::Terminal);
    }
    if record.terminal {
        return Err(Reject::Terminal);
    }
    if !supersede_ready_check(world, sandbox, record.current_operation) {
        return Err(Reject::LedgerMismatch);
    }
    let key = operation_key(sandbox, operation);
    if world.operations.contains_key(&key) {
        return Err(Reject::OperationExists);
    }
    if world.operation_plans.contains_key(&key) {
        return Err(Reject::OperationPlanExists);
    }
    if !authorized(&actor, record.owner) {
        return Err(Reject::Unauthorized);
    }
    if record.epoch == u64::MAX {
        return Err(Reject::CounterExhausted);
    }
    let epoch = record.epoch + 1;

    Ok(Decision {
        event: Event::AcceptKill {
            actor,
            sandbox,
            operation,
        },
        next: begin_kill(world, sandbox, record, operation, epoch),
    })
}

/// Return a previously persisted operation without creating a new operation
/// or changing durable state.  This intentionally remains available while the
/// controller process is marked down, matching Lean's `handleReplay`.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered replay checks.
pub fn handle_replay(world: &World, input: ReplayRequest) -> Result<Decision, Reject> {
    let ReplayRequest {
        actor,
        sandbox,
        operation,
        kind,
        input,
    } = input;
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(Reject::SandboxNotFound);
    };
    if !authorized(&actor, record.owner) {
        return Err(Reject::Unauthorized);
    }
    let key = operation_key(sandbox, operation);
    let Some(ledger) = world.operations.get(&key) else {
        return Err(Reject::OperationUnavailable);
    };
    if ledger.operation.kind != kind {
        return Err(Reject::LedgerMismatch);
    }
    if ledger.input != input {
        return Err(Reject::LedgerMismatch);
    }

    Ok(Decision {
        event: Event::RetryOperation {
            actor,
            sandbox,
            operation,
            kind,
            input,
        },
        next: world.clone(),
    })
}

/// Read an operation ledger row without changing durable state.  Like replay,
/// query has no controller-up guard in the Lean reducer.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered query checks.
pub fn handle_query(world: &World, input: QueryRequest) -> Result<Decision, Reject> {
    let QueryRequest {
        actor,
        sandbox,
        operation,
    } = input;
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(Reject::SandboxNotFound);
    };
    if !authorized(&actor, record.owner) {
        return Err(Reject::Unauthorized);
    }
    let key = operation_key(sandbox, operation);
    let Some(ledger) = world.operations.get(&key) else {
        return Err(Reject::OperationUnavailable);
    };

    Ok(Decision {
        event: Event::QueryOperation {
            actor,
            sandbox,
            operation,
            status: ledger.operation.status,
            result: ledger.result,
        },
        next: world.clone(),
    })
}

fn supplied_compensation_evidence(
    world: &World,
    key: crate::OperationKey,
    epoch: crate::Epoch,
    failure: FailureKind,
    evidence: Option<EffectId>,
) -> bool {
    let Some(id) = evidence else {
        return matches!(
            failure,
            FailureKind::Cancelled | FailureKind::DeadlineExceeded
        );
    };
    let Some(logical) = world.logical_effects.get(&id.logical) else {
        return false;
    };

    logical.origin == EffectOrigin::Operation(key)
        && id.attempt == logical.current_attempt
        && logical.result
            == Some(LogicalEffectResult::Failed {
                attempt: id.attempt,
                failure,
            })
        && world.effects.get(&id)
            == Some(&EffectStatus::Acknowledged(EffectOutcome::Failed(failure)))
        && id.command().sandbox() == key.sandbox
        && id.command().epoch() == Some(epoch)
}

/// Start the terminal compensation protocol for the current create, resume,
/// or pause operation.  Worker-caused compensation requires the finalized
/// failure of the exact current logical-effect attempt owned by that operation.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered compensation checks.
pub fn handle_compensation(world: &World, input: CompensationRequest) -> Result<Decision, Reject> {
    let CompensationRequest {
        sandbox,
        failure,
        evidence,
    } = input;
    if !world.controller_up {
        return Err(Reject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(Reject::SandboxNotFound);
    };
    if !matches!(record.phase, Phase::Starting | Phase::Pausing) {
        return Err(Reject::WrongPhase);
    }
    if record.terminal {
        return Err(Reject::Terminal);
    }
    let Some(operation) = record.current_operation else {
        return Err(Reject::OperationUnavailable);
    };
    if !matches!(
        operation.kind,
        OperationKind::Create | OperationKind::Resume | OperationKind::Pause
    ) {
        return Err(Reject::OperationNotCompensable);
    }
    if !matches!(
        operation.status,
        OperationStatus::Pending | OperationStatus::OutcomeUnknown
    ) {
        return Err(Reject::OperationAlreadyFinal);
    }
    let key = operation_key(sandbox, operation.id);
    let Some(ledger) = world.operations.get(&key) else {
        return Err(Reject::OperationUnavailable);
    };
    if ledger.sandbox != sandbox || ledger.operation != operation || ledger.result.is_some() {
        return Err(Reject::LedgerMismatch);
    }
    if !supplied_compensation_evidence(world, key, operation.epoch, failure, evidence) {
        return Err(Reject::FailureUnjustified);
    }

    Ok(Decision {
        event: Event::BeginCompensation { sandbox, failure },
        next: begin_compensation(world, sandbox, record, operation, failure),
    })
}

/// Unified executable entry point corresponding to Lean's `Controller.handle`.
///
/// # Errors
///
/// Returns the first rejection produced by the selected request handler.
pub fn handle(world: &World, input: Request) -> Result<Decision, Reject> {
    match input {
        Request::Create(input) => handle_create(world, input),
        Request::Pause(input) => handle_pause(world, input),
        Request::Resume(input) => handle_resume(world, input),
        Request::Kill(input) => handle_kill(world, input),
        Request::Replay(input) => handle_replay(world, input),
        Request::Query(input) => handle_query(world, input),
        Request::Compensate(input) => handle_compensation(world, input),
    }
}

#[cfg(test)]
mod tests {
    use super::{Reject, handle_create, handle_kill, handle_pause, handle_resume};
    use crate::model::{
        Actor, Authority, Command, EffectOrigin, EffectStatus, Event, FilePolicyRef,
        LogicalEffectId, NetworkPolicy, Operation, OperationInput, OperationKind,
        OperationLedgerRecord, OperationResult, OperationStatus, Phase, RestoreSource,
        SandboxRecord, Snapshot, SnapshotManifestRef, SnapshotStage, WorkspaceApiAccess, World,
    };
    use crate::transition::operation_key;
    use crate::{
        FailureKind, FsId, ManifestDigest, OperationId, PolicyDigest, SandboxId, SnapshotId,
        TenantId, VmId,
    };

    const OWNER: TenantId = TenantId(7);
    const SID: SandboxId = SandboxId(11);
    const WORKSPACE: FsId = FsId(13);
    const VM: VmId = VmId(23);

    fn sandbox_record(phase: Phase) -> SandboxRecord {
        SandboxRecord {
            owner: OWNER,
            phase,
            terminal: false,
            epoch: 5,
            generation: 3,
            current_vm: Some(VM),
            workspace: WORKSPACE,
            workspace_access: WorkspaceApiAccess::ManagedReadWrite,
            file_policy: FilePolicyRef {
                digest: PolicyDigest(17),
            },
            durable_snapshot: None,
            restore_from: None,
            net_slot: None,
            policy: NetworkPolicy::DenyAll,
            desired: Authority::Closed,
            effective: Authority::Closed,
            current_operation: None,
        }
    }

    fn world_with_record(record: SandboxRecord) -> World {
        let mut world = World::default();
        world.sandboxes.put(SID, record);
        world
    }

    fn accepted_create() -> super::Decision {
        handle_create(
            &World::default(),
            super::CreateRequest {
                actor: Actor::Tenant(TenantId(7)),
                owner: TenantId(7),
                sandbox: SandboxId(11),
                workspace: FsId(13),
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(17),
                },
                policy: NetworkPolicy::DenyAll,
                operation: OperationId(19),
            },
        )
        .expect("valid create must be accepted")
    }

    #[test]
    fn create_atomically_records_intent_plan_and_initial_effect() {
        let decision = accepted_create();
        let key = operation_key(SandboxId(11), OperationId(19));
        let sandbox = decision
            .next
            .sandboxes
            .get(&SandboxId(11))
            .expect("sandbox record");
        assert_eq!(sandbox.phase, Phase::Starting);
        assert_eq!(
            sandbox.current_operation.expect("operation").status,
            OperationStatus::Pending
        );

        let ledger = decision.next.operations.get(&key).expect("ledger");
        assert_eq!(ledger.sandbox, SandboxId(11));
        assert_eq!(
            ledger.input,
            OperationInput::Create {
                workspace: FsId(13),
                network: NetworkPolicy::DenyAll,
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(17),
                },
            }
        );
        let plan = decision.next.operation_plans.get(&key).expect("plan");
        assert_eq!(plan.key, key);
        assert_eq!(plan.retry_limit, 3);

        let command = Command::InstallFence {
            sandbox: SandboxId(11),
            epoch: 1,
            terminal: false,
        };
        let effect = crate::EffectId::initial(command.clone());
        assert_eq!(
            decision.next.effects.get(&effect),
            Some(&EffectStatus::Committed)
        );
        assert!(decision.next.inflight.contains(&effect));
        let logical = LogicalEffectId { command };
        assert_eq!(
            decision
                .next
                .logical_effects
                .get(&logical)
                .expect("logical effect")
                .origin,
            EffectOrigin::Operation(key)
        );
    }

    #[test]
    fn create_rejects_an_actor_from_another_tenant_without_state_change() {
        let world = World::default();
        let result = handle_create(
            &world,
            super::CreateRequest {
                actor: Actor::Tenant(TenantId(8)),
                owner: TenantId(7),
                sandbox: SandboxId(11),
                workspace: FsId(13),
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(17),
                },
                policy: NetworkPolicy::DenyAll,
                operation: OperationId(19),
            },
        );
        assert_eq!(result, Err(Reject::Unauthorized));
        assert_eq!(world, World::default());
    }

    #[test]
    fn pause_records_intent_and_prepends_fence_and_freeze_attempts() {
        let world = world_with_record(sandbox_record(Phase::Running));
        let operation_id = OperationId(29);
        let decision = handle_pause(
            &world,
            super::PauseRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: operation_id,
            },
        )
        .expect("valid pause must be accepted");

        assert_eq!(
            decision.event,
            Event::AcceptPause {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: operation_id,
            }
        );
        let record = decision.next.sandboxes.get(&SID).expect("sandbox");
        assert_eq!(record.phase, Phase::Pausing);
        assert_eq!(record.epoch, 6);
        assert_eq!(record.desired, Authority::Closed);
        let operation = record.current_operation.expect("current operation");
        assert_eq!(operation.kind, OperationKind::Pause);
        assert_eq!(operation.status, OperationStatus::Pending);

        let key = operation_key(SID, operation_id);
        assert_eq!(
            decision.next.operations.get(&key).expect("ledger").input,
            OperationInput::Pause { generation: 3 }
        );
        let freeze = crate::EffectId::initial(Command::FreezeVm {
            sandbox: SID,
            epoch: 6,
            generation: 3,
            vm: VM,
        });
        let fence = crate::EffectId::initial(Command::InstallFence {
            sandbox: SID,
            epoch: 6,
            terminal: false,
        });
        assert_eq!(
            decision.next.effect_keys,
            vec![freeze.clone(), fence.clone()]
        );
        assert_eq!(decision.next.inflight, vec![freeze, fence]);
    }

    #[test]
    fn pause_rejections_follow_lean_order() {
        let mut record = sandbox_record(Phase::Starting);
        record.terminal = true;
        record.current_vm = None;
        let world = world_with_record(record);
        assert_eq!(
            handle_pause(
                &world,
                super::PauseRequest {
                    actor: Actor::Tenant(TenantId(99)),
                    sandbox: SID,
                    operation: OperationId(29),
                },
            ),
            Err(Reject::WrongPhase)
        );

        let mut record = sandbox_record(Phase::Running);
        record.terminal = true;
        record.current_vm = None;
        let world = world_with_record(record);
        assert_eq!(
            handle_pause(
                &world,
                super::PauseRequest {
                    actor: Actor::Tenant(OWNER),
                    sandbox: SID,
                    operation: OperationId(29),
                },
            ),
            Err(Reject::Terminal)
        );
    }

    fn resumable_world() -> (World, RestoreSource) {
        let source = RestoreSource {
            snapshot: SnapshotId(31),
            manifest: SnapshotManifestRef {
                digest: ManifestDigest(37),
            },
        };
        let mut record = sandbox_record(Phase::Paused);
        record.current_vm = None;
        record.durable_snapshot = Some(source);
        record.desired = Authority::Paused {
            sandbox: SID,
            source,
        };
        record.effective = record.desired.clone();
        let mut world = world_with_record(record);
        world.snapshots.put(
            source.snapshot,
            Snapshot {
                owner: OWNER,
                sandbox: SID,
                source_generation: 3,
                workspace: WORKSPACE,
                manifest: source.manifest,
                stage: SnapshotStage::Durable,
                immutable: true,
            },
        );
        (world, source)
    }

    #[test]
    fn resume_validates_snapshot_and_starts_next_generation() {
        let (world, source) = resumable_world();
        let operation_id = OperationId(41);
        let decision = handle_resume(
            &world,
            super::ResumeRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: operation_id,
            },
        )
        .expect("valid resume must be accepted");

        assert_eq!(
            decision.event,
            Event::AcceptResume {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: operation_id,
            }
        );
        let record = decision.next.sandboxes.get(&SID).expect("sandbox");
        assert_eq!(record.phase, Phase::Starting);
        assert_eq!(record.epoch, 6);
        assert_eq!(record.generation, 4);
        assert_eq!(record.restore_from, Some(source));
        assert_eq!(record.desired, Authority::Closed);
        assert_eq!(record.effective, Authority::Closed);
        let key = operation_key(SID, operation_id);
        assert_eq!(
            decision.next.operations.get(&key).expect("ledger").input,
            OperationInput::Resume { source }
        );
        assert_eq!(
            decision
                .next
                .operation_plans
                .get(&key)
                .expect("plan")
                .generation,
            4
        );
    }

    #[test]
    fn resume_rejects_an_invalid_snapshot_before_duplicate_operation() {
        let (mut world, source) = resumable_world();
        let operation_id = OperationId(41);
        let mut snapshot = *world.snapshots.get(&source.snapshot).expect("snapshot");
        snapshot.immutable = false;
        world.snapshots.put(source.snapshot, snapshot);
        world.operations.put(
            operation_key(SID, operation_id),
            OperationLedgerRecord {
                sandbox: SID,
                operation: Operation {
                    id: operation_id,
                    epoch: 99,
                    kind: OperationKind::Resume,
                    status: OperationStatus::Pending,
                },
                input: OperationInput::Resume { source },
                result: None,
            },
        );
        assert_eq!(
            handle_resume(
                &world,
                super::ResumeRequest {
                    actor: Actor::Tenant(OWNER),
                    sandbox: SID,
                    operation: operation_id,
                },
            ),
            Err(Reject::SnapshotUnavailable)
        );
    }

    #[test]
    fn kill_supersedes_unfinished_operation_and_writes_tombstone() {
        let old_operation = Operation {
            id: OperationId(43),
            epoch: 5,
            kind: OperationKind::Resume,
            status: OperationStatus::OutcomeUnknown,
        };
        let mut record = sandbox_record(Phase::Starting);
        record.current_operation = Some(old_operation);
        let mut world = world_with_record(record);
        let old_key = operation_key(SID, old_operation.id);
        world.operations.put(
            old_key,
            OperationLedgerRecord {
                sandbox: SID,
                operation: old_operation,
                input: OperationInput::Resume {
                    source: RestoreSource {
                        snapshot: SnapshotId(47),
                        manifest: SnapshotManifestRef {
                            digest: ManifestDigest(53),
                        },
                    },
                },
                result: None,
            },
        );
        let kill_id = OperationId(59);
        let decision = handle_kill(
            &world,
            super::KillRequest {
                actor: Actor::SystemController,
                sandbox: SID,
                operation: kill_id,
            },
        )
        .expect("valid kill must be accepted");

        let record = decision.next.sandboxes.get(&SID).expect("sandbox");
        assert_eq!(record.phase, Phase::Revoking);
        assert!(record.terminal);
        assert_eq!(record.epoch, 6);
        assert_eq!(record.desired, Authority::Closed);
        assert_eq!(record.effective, Authority::Closed);
        let superseded = decision.next.operations.get(&old_key).expect("old ledger");
        assert_eq!(
            superseded.operation.status,
            OperationStatus::Failed(FailureKind::Cancelled)
        );
        assert_eq!(
            superseded.result,
            Some(OperationResult::Failed(FailureKind::Cancelled))
        );
        let kill_key = operation_key(SID, kill_id);
        assert_eq!(
            decision
                .next
                .operations
                .get(&kill_key)
                .expect("kill ledger")
                .input,
            OperationInput::Kill { generation: 3 }
        );
        assert!(
            decision
                .next
                .operation_plans
                .get(&kill_key)
                .expect("kill plan")
                .sealed
        );
    }

    #[test]
    fn kill_rejects_ledger_mismatch_before_freshness_and_authorization() {
        let operation = Operation {
            id: OperationId(61),
            epoch: 5,
            kind: OperationKind::Pause,
            status: OperationStatus::Pending,
        };
        let mut record = sandbox_record(Phase::Pausing);
        record.current_operation = Some(operation);
        let mut world = world_with_record(record);
        let requested = OperationId(67);
        world.operations.put(
            operation_key(SID, requested),
            OperationLedgerRecord {
                sandbox: SID,
                operation,
                input: OperationInput::Pause { generation: 3 },
                result: None,
            },
        );
        assert_eq!(
            handle_kill(
                &world,
                super::KillRequest {
                    actor: Actor::Tenant(TenantId(99)),
                    sandbox: SID,
                    operation: requested,
                },
            ),
            Err(Reject::LedgerMismatch)
        );
    }

    #[test]
    fn lifecycle_requests_fail_closed_before_machine_counters_wrap() {
        let mut pause_record = sandbox_record(Phase::Running);
        pause_record.epoch = u64::MAX;
        let pause_world = world_with_record(pause_record);
        assert_eq!(
            handle_pause(
                &pause_world,
                super::PauseRequest {
                    actor: Actor::Tenant(OWNER),
                    sandbox: SID,
                    operation: OperationId(71),
                },
            ),
            Err(Reject::CounterExhausted)
        );

        let (mut resume_world, _) = resumable_world();
        let mut resume_record = resume_world.sandboxes.get(&SID).expect("sandbox").clone();
        resume_record.generation = u64::MAX;
        resume_world.sandboxes.put(SID, resume_record);
        assert_eq!(
            handle_resume(
                &resume_world,
                super::ResumeRequest {
                    actor: Actor::Tenant(OWNER),
                    sandbox: SID,
                    operation: OperationId(73),
                },
            ),
            Err(Reject::CounterExhausted)
        );

        let mut kill_record = sandbox_record(Phase::Starting);
        kill_record.epoch = u64::MAX;
        let kill_world = world_with_record(kill_record);
        assert_eq!(
            handle_kill(
                &kill_world,
                super::KillRequest {
                    actor: Actor::SystemController,
                    sandbox: SID,
                    operation: OperationId(79),
                },
            ),
            Err(Reject::CounterExhausted)
        );
    }
}

#[cfg(test)]
mod operation_ledger_tests {
    use super::{
        CompensationRequest, QueryRequest, Reject, ReplayRequest, Request, handle,
        handle_compensation, handle_query, handle_replay,
    };
    use crate::model::{
        Actor, Authority, Command, EffectId, EffectOrigin, EffectOutcome, EffectStatus, Event,
        FailureKind, FilePolicyRef, LogicalEffectId, LogicalEffectRecord, LogicalEffectResult,
        NetworkPolicy, Operation, OperationInput, OperationKind, OperationLedgerRecord,
        OperationResult, OperationStatus, Phase, SandboxRecord, WorkspaceApiAccess, World,
    };
    use crate::transition::operation_key;
    use crate::{FsId, OperationId, PolicyDigest, SandboxId, TenantId, VmId};

    const OWNER: TenantId = TenantId(101);
    const OTHER: TenantId = TenantId(102);
    const SID: SandboxId = SandboxId(103);
    const OPERATION_ID: OperationId = OperationId(104);
    const WORKSPACE: FsId = FsId(105);
    const EPOCH: u64 = 7;

    fn create_input() -> OperationInput {
        OperationInput::Create {
            workspace: WORKSPACE,
            network: NetworkPolicy::DenyAll,
            file_policy: FilePolicyRef {
                digest: PolicyDigest(106),
            },
        }
    }

    fn ledger_world(status: OperationStatus) -> World {
        let operation = Operation {
            id: OPERATION_ID,
            epoch: EPOCH,
            kind: OperationKind::Create,
            status,
        };
        let record = SandboxRecord {
            owner: OWNER,
            phase: Phase::Starting,
            terminal: false,
            epoch: EPOCH,
            generation: 3,
            current_vm: Some(VmId(107)),
            workspace: WORKSPACE,
            workspace_access: WorkspaceApiAccess::ManagedReadWrite,
            file_policy: FilePolicyRef {
                digest: PolicyDigest(106),
            },
            durable_snapshot: None,
            restore_from: None,
            net_slot: None,
            policy: NetworkPolicy::DenyAll,
            desired: Authority::Closed,
            effective: Authority::Closed,
            current_operation: Some(operation),
        };
        let mut world = World::default();
        world.sandboxes.put(SID, record);
        world.operations.put(
            operation_key(SID, OPERATION_ID),
            OperationLedgerRecord {
                sandbox: SID,
                operation,
                input: create_input(),
                result: None,
            },
        );
        world
    }

    fn evidence_id(attempt: u64, epoch: u64) -> EffectId {
        EffectId {
            logical: LogicalEffectId {
                command: Command::InstallFence {
                    sandbox: SID,
                    epoch,
                    terminal: false,
                },
            },
            attempt,
        }
    }

    fn finalized_failure_world(failure: FailureKind) -> (World, EffectId) {
        let mut world = ledger_world(OperationStatus::Pending);
        let id = evidence_id(2, EPOCH);
        world.logical_effects.put(
            id.logical.clone(),
            LogicalEffectRecord {
                origin: EffectOrigin::Operation(operation_key(SID, OPERATION_ID)),
                current_attempt: id.attempt,
                remaining_retries: 0,
                result: Some(LogicalEffectResult::Failed {
                    attempt: id.attempt,
                    failure,
                }),
            },
        );
        world.effects.put(
            id.clone(),
            EffectStatus::Acknowledged(EffectOutcome::Failed(failure)),
        );
        (world, id)
    }

    fn replay_request() -> ReplayRequest {
        ReplayRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OPERATION_ID,
            kind: OperationKind::Create,
            input: create_input(),
        }
    }

    #[test]
    fn replay_is_read_only_while_controller_is_down_and_never_creates_an_operation() {
        let mut world = ledger_world(OperationStatus::Succeeded);
        world.controller_up = false;
        let before = world.clone();

        let decision = handle_replay(&world, replay_request()).expect("matching replay");

        assert_eq!(decision.next, before);
        assert_eq!(world, before);
        assert_eq!(
            decision.event,
            Event::RetryOperation {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: OPERATION_ID,
                kind: OperationKind::Create,
                input: create_input(),
            }
        );
        assert_eq!(world.operations.entries().len(), 1);
    }

    #[test]
    fn replay_rejections_follow_sandbox_authorization_ledger_kind_input_order() {
        assert_eq!(
            handle_replay(&World::default(), replay_request()),
            Err(Reject::SandboxNotFound)
        );

        let mut unauthorized = ledger_world(OperationStatus::Pending);
        unauthorized.operations = crate::Store::default();
        let mut request = replay_request();
        request.actor = Actor::Tenant(OTHER);
        assert_eq!(
            handle_replay(&unauthorized, request),
            Err(Reject::Unauthorized)
        );

        let mut missing = ledger_world(OperationStatus::Pending);
        missing.operations = crate::Store::default();
        assert_eq!(
            handle_replay(&missing, replay_request()),
            Err(Reject::OperationUnavailable)
        );

        let mut wrong_kind = replay_request();
        wrong_kind.kind = OperationKind::Pause;
        wrong_kind.input = OperationInput::Pause { generation: 3 };
        assert_eq!(
            handle_replay(&ledger_world(OperationStatus::Pending), wrong_kind),
            Err(Reject::LedgerMismatch)
        );

        let mut wrong_input = replay_request();
        wrong_input.input = OperationInput::Kill { generation: 3 };
        assert_eq!(
            handle_replay(&ledger_world(OperationStatus::Pending), wrong_input),
            Err(Reject::LedgerMismatch)
        );
    }

    #[test]
    fn query_is_read_only_and_reports_exact_persisted_status_and_result() {
        let mut world = ledger_world(OperationStatus::Failed(FailureKind::PolicyFailure));
        let key = operation_key(SID, OPERATION_ID);
        let mut ledger = world.operations.get(&key).expect("ledger").clone();
        ledger.result = Some(OperationResult::Failed(FailureKind::PolicyFailure));
        world.operations.put(key, ledger);
        world.controller_up = false;
        let before = world.clone();
        let request = QueryRequest {
            actor: Actor::SystemController,
            sandbox: SID,
            operation: OPERATION_ID,
        };

        let decision = handle_query(&world, request).expect("persisted query");

        assert_eq!(decision.next, before);
        assert_eq!(world, before);
        assert_eq!(
            decision.event,
            Event::QueryOperation {
                actor: Actor::SystemController,
                sandbox: SID,
                operation: OPERATION_ID,
                status: OperationStatus::Failed(FailureKind::PolicyFailure),
                result: Some(OperationResult::Failed(FailureKind::PolicyFailure)),
            }
        );
    }

    #[test]
    fn compensation_accepts_only_the_finalized_current_attempt_for_worker_failure() {
        let failure = FailureKind::PolicyFailure;
        let (world, id) = finalized_failure_world(failure);

        let decision = handle_compensation(
            &world,
            CompensationRequest {
                sandbox: SID,
                failure,
                evidence: Some(id),
            },
        )
        .expect("finalized current failure");

        assert_eq!(
            decision.event,
            Event::BeginCompensation {
                sandbox: SID,
                failure
            }
        );
        let record = decision.next.sandboxes.get(&SID).expect("sandbox");
        assert_eq!(record.phase, Phase::Revoking);
        assert!(record.terminal);
        assert_eq!(record.desired, Authority::Closed);
        assert_eq!(record.effective, Authority::Closed);
        assert_eq!(
            record.current_operation.expect("operation").status,
            OperationStatus::Compensating(failure)
        );
        let ledger = decision
            .next
            .operations
            .get(&operation_key(SID, OPERATION_ID))
            .expect("ledger");
        assert_eq!(
            ledger.operation.status,
            OperationStatus::Compensating(failure)
        );
        assert_eq!(ledger.result, None);
        let fence = EffectId::initial(Command::InstallFence {
            sandbox: SID,
            epoch: EPOCH,
            terminal: true,
        });
        assert_eq!(
            decision.next.effects.get(&fence),
            Some(&EffectStatus::Committed)
        );
        assert!(decision.next.inflight.contains(&fence));
    }

    #[test]
    fn controller_owned_cancel_and_deadline_need_no_worker_evidence() {
        for failure in [FailureKind::Cancelled, FailureKind::DeadlineExceeded] {
            let world = ledger_world(OperationStatus::OutcomeUnknown);
            let result = handle_compensation(
                &world,
                CompensationRequest {
                    sandbox: SID,
                    failure,
                    evidence: None,
                },
            );
            assert!(result.is_ok(), "{failure:?} must be locally justified");
        }

        let world = ledger_world(OperationStatus::Pending);
        assert_eq!(
            handle_compensation(
                &world,
                CompensationRequest {
                    sandbox: SID,
                    failure: FailureKind::InternalFailure,
                    evidence: None,
                },
            ),
            Err(Reject::FailureUnjustified)
        );
    }

    #[test]
    fn a_raw_failure_receipt_is_not_finalized_compensation_evidence() {
        let failure = FailureKind::PolicyFailure;
        let (mut world, id) = finalized_failure_world(failure);
        let logical = world.logical_effects.get(&id.logical).expect("logical");
        world.logical_effects.put(
            id.logical.clone(),
            LogicalEffectRecord {
                result: None,
                ..*logical
            },
        );

        assert_eq!(
            handle_compensation(
                &world,
                CompensationRequest {
                    sandbox: SID,
                    failure,
                    evidence: Some(id),
                },
            ),
            Err(Reject::FailureUnjustified)
        );
    }

    #[test]
    fn stale_attempt_wrong_origin_and_missing_receipt_are_rejected() {
        let failure = FailureKind::PolicyFailure;
        let (base, id) = finalized_failure_world(failure);

        let mut stale = base.clone();
        let stale_id = evidence_id(1, EPOCH);
        stale.effects.put(
            stale_id.clone(),
            EffectStatus::Acknowledged(EffectOutcome::Failed(failure)),
        );
        assert_eq!(
            handle_compensation(
                &stale,
                CompensationRequest {
                    sandbox: SID,
                    failure,
                    evidence: Some(stale_id),
                },
            ),
            Err(Reject::FailureUnjustified)
        );

        let mut wrong_origin = base.clone();
        let logical = *wrong_origin
            .logical_effects
            .get(&id.logical)
            .expect("logical");
        wrong_origin.logical_effects.put(
            id.logical.clone(),
            LogicalEffectRecord {
                origin: EffectOrigin::Operation(operation_key(SID, OperationId(999))),
                ..logical
            },
        );
        assert_eq!(
            handle_compensation(
                &wrong_origin,
                CompensationRequest {
                    sandbox: SID,
                    failure,
                    evidence: Some(id.clone()),
                },
            ),
            Err(Reject::FailureUnjustified)
        );

        let mut missing_receipt = base.clone();
        missing_receipt.effects = crate::Store::default();
        assert_eq!(
            handle_compensation(
                &missing_receipt,
                CompensationRequest {
                    sandbox: SID,
                    failure,
                    evidence: Some(id.clone()),
                },
            ),
            Err(Reject::FailureUnjustified)
        );
    }

    #[test]
    fn successful_result_mismatched_failure_and_wrong_epoch_are_rejected() {
        let failure = FailureKind::PolicyFailure;
        let (base, id) = finalized_failure_world(failure);

        let mut succeeded = base.clone();
        let logical = *succeeded.logical_effects.get(&id.logical).expect("logical");
        succeeded.logical_effects.put(
            id.logical.clone(),
            LogicalEffectRecord {
                result: Some(LogicalEffectResult::Succeeded {
                    attempt: id.attempt,
                    result: crate::ResultDigest(109),
                }),
                ..logical
            },
        );
        assert_eq!(
            handle_compensation(
                &succeeded,
                CompensationRequest {
                    sandbox: SID,
                    failure,
                    evidence: Some(id.clone()),
                },
            ),
            Err(Reject::FailureUnjustified)
        );

        assert_eq!(
            handle_compensation(
                &base,
                CompensationRequest {
                    sandbox: SID,
                    failure: FailureKind::StorageFailure,
                    evidence: Some(id),
                },
            ),
            Err(Reject::FailureUnjustified)
        );

        let wrong_epoch_id = evidence_id(2, EPOCH + 1);
        let mut wrong_epoch = ledger_world(OperationStatus::Pending);
        wrong_epoch.logical_effects.put(
            wrong_epoch_id.logical.clone(),
            LogicalEffectRecord {
                origin: EffectOrigin::Operation(operation_key(SID, OPERATION_ID)),
                current_attempt: 2,
                remaining_retries: 0,
                result: Some(LogicalEffectResult::Failed {
                    attempt: 2,
                    failure,
                }),
            },
        );
        wrong_epoch.effects.put(
            wrong_epoch_id.clone(),
            EffectStatus::Acknowledged(EffectOutcome::Failed(failure)),
        );
        assert_eq!(
            handle_compensation(
                &wrong_epoch,
                CompensationRequest {
                    sandbox: SID,
                    failure,
                    evidence: Some(wrong_epoch_id),
                },
            ),
            Err(Reject::FailureUnjustified)
        );
    }

    #[test]
    fn compensation_guard_priority_and_unified_dispatch_match_lean() {
        let down = World {
            controller_up: false,
            ..World::default()
        };
        assert_eq!(
            handle_compensation(
                &down,
                CompensationRequest {
                    sandbox: SID,
                    failure: FailureKind::Cancelled,
                    evidence: None,
                },
            ),
            Err(Reject::ControllerDown)
        );

        let mut wrong_phase = ledger_world(OperationStatus::Succeeded);
        let record = wrong_phase.sandboxes.get(&SID).expect("sandbox");
        wrong_phase.sandboxes.put(
            SID,
            SandboxRecord {
                phase: Phase::Running,
                terminal: true,
                ..record.clone()
            },
        );
        assert_eq!(
            handle_compensation(
                &wrong_phase,
                CompensationRequest {
                    sandbox: SID,
                    failure: FailureKind::InternalFailure,
                    evidence: None,
                },
            ),
            Err(Reject::WrongPhase)
        );

        let world = ledger_world(OperationStatus::Pending);
        let decision = handle(
            &world,
            Request::Query(QueryRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: OPERATION_ID,
            }),
        )
        .expect("unified query");
        assert_eq!(decision.next, world);
        assert!(matches!(decision.event, Event::QueryOperation { .. }));
    }
}
