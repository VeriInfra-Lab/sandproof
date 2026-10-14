use crate::model::{Actor, Authority, Endpoint, OperationStatus, Phase, SandboxRecord, World};
use crate::transition::{authorized, mark_operation_outcome_unknown, operation_key};
use crate::{Generation, NetSlotId, SandboxId, VmId};

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AccessInput {
    Exec(Actor, SandboxId, VmId),
    FileRead(Actor, SandboxId, Generation),
    FileWrite(Actor, SandboxId, Generation),
    Ingress(Actor, SandboxId, VmId, NetSlotId),
    Egress(SandboxId, VmId, NetSlotId, Endpoint),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuxiliaryEvent {
    RpcTimeout(SandboxId),
    Exec(Actor, SandboxId),
    FileRead(Actor, SandboxId, Generation),
    FileWrite(Actor, SandboxId, Generation),
    Ingress(Actor, SandboxId),
    Egress(SandboxId, Endpoint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxiliaryReject {
    SandboxNotFound,
    OperationUnavailable,
    OperationAlreadyFinal,
    LedgerMismatch,
    Unauthorized,
    WrongPhase,
    WrongGeneration,
    CapabilityDenied,
    DestinationDenied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuxiliaryDecision {
    pub event: AuxiliaryEvent,
    pub next: World,
}

fn allows_exec(record: &SandboxRecord, sandbox: SandboxId, vm: VmId) -> bool {
    match &record.effective {
        Authority::Running {
            sandbox: authority_sandbox,
            generation,
            vm: authority_vm,
            ..
        } => {
            *authority_sandbox == sandbox && *generation == record.generation && *authority_vm == vm
        }
        Authority::Closed | Authority::Paused { .. } => false,
    }
}

fn allows_file(record: &SandboxRecord, sandbox: SandboxId, generation: Generation) -> bool {
    match &record.effective {
        Authority::Running {
            sandbox: authority_sandbox,
            generation: authority_generation,
            workspace,
            access,
            ..
        } => {
            *authority_sandbox == sandbox
                && *authority_generation == generation
                && *workspace == record.workspace
                && *access == record.workspace_access
        }
        Authority::Closed | Authority::Paused { .. } => false,
    }
}

fn allows_network(record: &SandboxRecord, sandbox: SandboxId, vm: VmId, slot: NetSlotId) -> bool {
    match &record.effective {
        Authority::Running {
            sandbox: authority_sandbox,
            generation,
            vm: authority_vm,
            slot: authority_slot,
            policy,
            ..
        } => {
            *authority_sandbox == sandbox
                && *generation == record.generation
                && *authority_vm == vm
                && *authority_slot == slot
                && policy == &record.policy
        }
        Authority::Closed | Authority::Paused { .. } => false,
    }
}

fn destination_allowed(record: &SandboxRecord, destination: &Endpoint) -> bool {
    record.policy.destination_allowed(destination)
}

/// Record client-side uncertainty without inventing an adapter result.
///
/// # Errors
/// Returns the first failed guard in `AuxController.handleRpcTimeout`.
pub fn handle_rpc_timeout(
    world: &World,
    sandbox: SandboxId,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(AuxiliaryReject::SandboxNotFound);
    };
    let Some(operation) = record.current_operation else {
        return Err(AuxiliaryReject::OperationUnavailable);
    };
    if operation.status != OperationStatus::Pending {
        return Err(AuxiliaryReject::OperationAlreadyFinal);
    }
    let key = operation_key(sandbox, operation.id);
    let Some(ledger) = world.operations.get(&key) else {
        return Err(AuxiliaryReject::OperationUnavailable);
    };
    if ledger.sandbox != sandbox || ledger.operation != operation || ledger.result.is_some() {
        return Err(AuxiliaryReject::LedgerMismatch);
    }
    Ok(AuxiliaryDecision {
        event: AuxiliaryEvent::RpcTimeout(sandbox),
        next: mark_operation_outcome_unknown(world, sandbox, record, operation),
    })
}

/// Admit an execution capability against the effective authority.
///
/// # Errors
/// Returns sandbox, authorization, then capability errors in model order.
pub fn handle_exec(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    vm: VmId,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(AuxiliaryReject::SandboxNotFound);
    };
    if !authorized(&actor, record.owner) {
        return Err(AuxiliaryReject::Unauthorized);
    }
    if !allows_exec(record, sandbox, vm) {
        return Err(AuxiliaryReject::CapabilityDenied);
    }
    Ok(AuxiliaryDecision {
        event: AuxiliaryEvent::Exec(actor, sandbox),
        next: world.clone(),
    })
}

/// Admit a read through the managed workspace authority.
///
/// # Errors
/// Returns the first failed model guard.
pub fn handle_file_read(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    generation: Generation,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(AuxiliaryReject::SandboxNotFound);
    };
    if record.phase != Phase::Running {
        return Err(AuxiliaryReject::WrongPhase);
    }
    if generation != record.generation {
        return Err(AuxiliaryReject::WrongGeneration);
    }
    if !authorized(&actor, record.owner) {
        return Err(AuxiliaryReject::Unauthorized);
    }
    if !allows_file(record, sandbox, generation) {
        return Err(AuxiliaryReject::CapabilityDenied);
    }
    Ok(AuxiliaryDecision {
        event: AuxiliaryEvent::FileRead(actor, sandbox, generation),
        next: world.clone(),
    })
}

/// Admit a write through the managed workspace authority.
///
/// # Errors
/// Returns the first failed model guard.
pub fn handle_file_write(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    generation: Generation,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(AuxiliaryReject::SandboxNotFound);
    };
    if record.phase != Phase::Running {
        return Err(AuxiliaryReject::WrongPhase);
    }
    if generation != record.generation {
        return Err(AuxiliaryReject::WrongGeneration);
    }
    if !authorized(&actor, record.owner) {
        return Err(AuxiliaryReject::Unauthorized);
    }
    if !allows_file(record, sandbox, generation) {
        return Err(AuxiliaryReject::CapabilityDenied);
    }
    Ok(AuxiliaryDecision {
        event: AuxiliaryEvent::FileWrite(actor, sandbox, generation),
        next: world.clone(),
    })
}

/// Admit ingress through the exact VM/slot authority.
///
/// # Errors
/// Returns sandbox, authorization, then capability errors in model order.
pub fn handle_ingress(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    vm: VmId,
    slot: NetSlotId,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(AuxiliaryReject::SandboxNotFound);
    };
    if !authorized(&actor, record.owner) {
        return Err(AuxiliaryReject::Unauthorized);
    }
    if !allows_network(record, sandbox, vm, slot) {
        return Err(AuxiliaryReject::CapabilityDenied);
    }
    Ok(AuxiliaryDecision {
        event: AuxiliaryEvent::Ingress(actor, sandbox),
        next: world.clone(),
    })
}

/// Admit egress only through the exact running authority and recorded policy.
///
/// # Errors
/// Returns sandbox, capability, then destination errors in model order.
pub fn handle_egress(
    world: &World,
    sandbox: SandboxId,
    vm: VmId,
    slot: NetSlotId,
    destination: Endpoint,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(AuxiliaryReject::SandboxNotFound);
    };
    if !allows_network(record, sandbox, vm, slot) {
        return Err(AuxiliaryReject::CapabilityDenied);
    }
    if !destination_allowed(record, &destination) {
        return Err(AuxiliaryReject::DestinationDenied);
    }
    Ok(AuxiliaryDecision {
        event: AuxiliaryEvent::Egress(sandbox, destination),
        next: world.clone(),
    })
}

/// Execute one capability request through the same split as Lean.
///
/// # Errors
/// Returns the selected handler's first failed guard.
pub fn handle_access(
    world: &World,
    input: AccessInput,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    match input {
        AccessInput::Exec(actor, sandbox, vm) => handle_exec(world, actor, sandbox, vm),
        AccessInput::FileRead(actor, sandbox, generation) => {
            handle_file_read(world, actor, sandbox, generation)
        }
        AccessInput::FileWrite(actor, sandbox, generation) => {
            handle_file_write(world, actor, sandbox, generation)
        }
        AccessInput::Ingress(actor, sandbox, vm, slot) => {
            handle_ingress(world, actor, sandbox, vm, slot)
        }
        AccessInput::Egress(sandbox, vm, slot, destination) => {
            handle_egress(world, sandbox, vm, slot, destination)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccessInput, AuxiliaryEvent, AuxiliaryReject, handle_access, handle_egress,
        handle_file_read, handle_rpc_timeout,
    };
    use crate::model::{
        Actor, Authority, Endpoint, NetworkPolicy, OperationResult, OperationStatus, Protocol,
        WorkspaceApiAccess,
    };
    use crate::request::{CreateRequest, handle_create};
    use crate::transition::operation_key;
    use crate::{
        FilePolicyRef, FsId, NetSlotId, OperationId, PolicyDigest, SandboxId, TenantId, VmId, World,
    };

    const OWNER: TenantId = TenantId(1);
    const SID: SandboxId = SandboxId(2);
    const VM: VmId = VmId(3);
    const SLOT: NetSlotId = NetSlotId(4);
    const OPERATION: OperationId = OperationId(5);
    const WORKSPACE: FsId = FsId(6);

    fn endpoint(port: u64) -> Endpoint {
        Endpoint {
            address: "203.0.113.1".to_owned(),
            prefix_length: 32,
            protocol: Protocol::Tcp,
            port,
        }
    }

    fn created() -> World {
        handle_create(
            &World::default(),
            CreateRequest {
                actor: Actor::Tenant(OWNER),
                owner: OWNER,
                sandbox: SID,
                workspace: WORKSPACE,
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(7),
                },
                policy: NetworkPolicy::DenyAll,
                operation: OPERATION,
            },
        )
        .expect("create")
        .next
    }

    fn running() -> World {
        let mut world = created();
        let destination = endpoint(443);
        let record = world.sandboxes.get(&SID).expect("sandbox").clone();
        let mut running = record;
        running.phase = crate::Phase::Running;
        running.effective = Authority::Running {
            sandbox: SID,
            generation: 1,
            vm: VM,
            workspace: WORKSPACE,
            access: WorkspaceApiAccess::ManagedReadWrite,
            slot: SLOT,
            policy: NetworkPolicy::StaticAllowlist(vec![destination]),
        };
        running.policy = NetworkPolicy::StaticAllowlist(vec![endpoint(443)]);
        world.sandboxes.put(SID, running);
        world
    }

    #[test]
    fn ipv4_policy_denies_platform_before_user_allow_and_keeps_exceptions() {
        let mut world = running();
        let policy = NetworkPolicy::from_sdk_ipv4(
            Some(false),
            &["1.1.1.1".into(), "169.254.169.254".into()],
            &["0.0.0.0/0".into()],
        )
        .unwrap();
        let mut record = world.sandboxes.get(&SID).unwrap().clone();
        record.policy = policy.clone();
        if let Authority::Running {
            policy: effective, ..
        } = &mut record.effective
        {
            *effective = policy;
        }
        world.sandboxes.put(SID, record);
        let mut destination = endpoint(53);
        destination.address = "1.1.1.1".into();
        assert!(handle_egress(&world, SID, VM, SLOT, destination.clone()).is_ok());
        destination.address = "8.8.8.8".into();
        assert_eq!(
            handle_egress(&world, SID, VM, SLOT, destination.clone()),
            Err(AuxiliaryReject::DestinationDenied)
        );
        destination.address = "169.254.169.254".into();
        assert_eq!(
            handle_egress(&world, SID, VM, SLOT, destination),
            Err(AuxiliaryReject::DestinationDenied)
        );
    }

    #[test]
    fn timeout_marks_both_current_operation_and_ledger_unknown() {
        let world = created();
        let decision = handle_rpc_timeout(&world, SID).expect("timeout");
        assert_eq!(decision.event, AuxiliaryEvent::RpcTimeout(SID));
        assert_eq!(
            decision
                .next
                .sandboxes
                .get(&SID)
                .and_then(|record| record.current_operation)
                .map(|operation| operation.status),
            Some(OperationStatus::OutcomeUnknown)
        );
        let ledger = decision
            .next
            .operations
            .get(&operation_key(SID, OPERATION))
            .expect("ledger");
        assert_eq!(ledger.operation.status, OperationStatus::OutcomeUnknown);
        assert_eq!(ledger.result, None::<OperationResult>);
    }

    #[test]
    fn timeout_changes_only_the_two_operation_records() {
        let world = created();
        let mut expected = world.clone();
        let mut record = expected.sandboxes.get(&SID).expect("sandbox").clone();
        record.current_operation.as_mut().expect("operation").status =
            OperationStatus::OutcomeUnknown;
        expected.sandboxes.put(SID, record);
        let key = operation_key(SID, OPERATION);
        let mut ledger = expected.operations.get(&key).expect("ledger").clone();
        ledger.operation.status = OperationStatus::OutcomeUnknown;
        expected.operations.put(key, ledger);

        let decision = handle_rpc_timeout(&world, SID).expect("timeout");
        assert_eq!(decision.next, expected);
        assert_eq!(world, created());
        assert_eq!(
            handle_rpc_timeout(&decision.next, SID),
            Err(AuxiliaryReject::OperationAlreadyFinal)
        );
    }

    #[test]
    fn timeout_rejects_missing_sandbox_operation_and_ledger() {
        assert_eq!(
            handle_rpc_timeout(&World::default(), SID),
            Err(AuxiliaryReject::SandboxNotFound)
        );
        let mut world = created();
        let mut record = world.sandboxes.get(&SID).expect("sandbox").clone();
        record.current_operation = None;
        world.sandboxes.put(SID, record);
        assert_eq!(
            handle_rpc_timeout(&world, SID),
            Err(AuxiliaryReject::OperationUnavailable)
        );

        let mut world = created();
        world.operations = crate::store::Store::default();
        assert_eq!(
            handle_rpc_timeout(&world, SID),
            Err(AuxiliaryReject::OperationUnavailable)
        );
    }

    #[test]
    fn timeout_rejects_each_ledger_inconsistency_without_mutation() {
        let key = operation_key(SID, OPERATION);
        for mismatch in 0..3 {
            let mut world = created();
            let mut ledger = world.operations.get(&key).expect("ledger").clone();
            match mismatch {
                0 => ledger.sandbox = SandboxId(99),
                1 => ledger.operation.id = OperationId(99),
                _ => ledger.result = Some(OperationResult::Destroyed),
            }
            world.operations.put(key, ledger);
            let before = world.clone();
            assert_eq!(
                handle_rpc_timeout(&world, SID),
                Err(AuxiliaryReject::LedgerMismatch)
            );
            assert_eq!(world, before);
        }
    }

    #[test]
    fn timeout_checks_nonpending_status_before_missing_ledger() {
        for status in [
            OperationStatus::OutcomeUnknown,
            OperationStatus::Succeeded,
            OperationStatus::Failed(crate::FailureKind::PolicyFailure),
            OperationStatus::Compensating(crate::FailureKind::PolicyFailure),
        ] {
            let mut world = created();
            let mut record = world.sandboxes.get(&SID).expect("sandbox").clone();
            record.current_operation.as_mut().expect("operation").status = status;
            world.sandboxes.put(SID, record);
            world.operations = crate::store::Store::default();
            assert_eq!(
                handle_rpc_timeout(&world, SID),
                Err(AuxiliaryReject::OperationAlreadyFinal)
            );
        }
    }

    #[test]
    fn access_requires_exact_effective_authority_and_owner() {
        let world = running();
        assert_eq!(
            handle_access(&world, AccessInput::Exec(Actor::Tenant(OWNER), SID, VM))
                .map(|decision| decision.event),
            Ok(AuxiliaryEvent::Exec(Actor::Tenant(OWNER), SID))
        );
        assert_eq!(
            handle_access(
                &world,
                AccessInput::Exec(Actor::Tenant(TenantId(99)), SID, VM)
            ),
            Err(AuxiliaryReject::Unauthorized)
        );
        assert_eq!(
            handle_file_read(&world, Actor::Tenant(OWNER), SID, 2),
            Err(AuxiliaryReject::WrongGeneration)
        );
    }

    #[test]
    fn egress_is_fail_closed_and_uses_static_exact_membership() {
        let world = running();
        assert_eq!(
            handle_egress(&world, SID, VM, SLOT, endpoint(443)).map(|decision| decision.event),
            Ok(AuxiliaryEvent::Egress(SID, endpoint(443)))
        );
        assert_eq!(
            handle_egress(&world, SID, VM, SLOT, endpoint(80)),
            Err(AuxiliaryReject::DestinationDenied)
        );
        assert_eq!(
            handle_egress(&world, SID, VmId(99), SLOT, endpoint(443)),
            Err(AuxiliaryReject::CapabilityDenied)
        );
    }
}
