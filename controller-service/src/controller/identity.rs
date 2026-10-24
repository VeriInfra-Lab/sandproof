use super::allocators::{self, SqlId};
use crate::JournalError;
use rusqlite::{OptionalExtension, Transaction, params};
use sandbox_controller_core as core;
use sandproof_protocol::{
    ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
    ResourceRef, SandboxId, TenantId, WorkAction,
};

pub(super) fn sandbox_id(
    namespace: &str,
    sandbox: core::SandboxId,
) -> Result<SandboxId, JournalError> {
    Ok(SandboxId::try_from(format!(
        "sb-{namespace}-{}",
        sandbox.0
    ))?)
}

pub(super) fn operation_id(
    namespace: &str,
    key: core::OperationKey,
) -> Result<OperationId, JournalError> {
    Ok(OperationId::try_from(format!(
        "op-{namespace}-{}-{}",
        key.sandbox.0, key.operation.0
    ))?)
}

pub(super) fn owner_id(owner: core::TenantId) -> Result<TenantId, JournalError> {
    Ok(TenantId::try_from(format!("tenant-{}", owner.0))?)
}

pub(super) fn resource(
    namespace: &str,
    command: &core::Command,
) -> Result<ResourceRef, JournalError> {
    use core::Command;
    let (kind, prefix, number) = match command {
        Command::InstallFence { sandbox, .. } => (ResourceKind::Sandbox, "sb", sandbox.0),
        Command::PrepareVm { vm, .. }
        | Command::AbortPrepare { vm, .. }
        | Command::ActivateVm { vm, .. }
        | Command::FreezeVm { vm, .. }
        | Command::StopVm { vm, .. }
        | Command::CleanupVm { vm, .. } => (ResourceKind::Vm, "vm", vm.0),
        Command::ConfigureNetwork { slot, .. } | Command::CleanupSlot { slot, .. } => {
            (ResourceKind::NetworkSlot, "slot", slot.0)
        }
        Command::ApplyFilePolicy { workspace, .. } => (ResourceKind::Workspace, "fs", workspace.0),
        Command::SealSnapshot { snapshot, .. } | Command::CommitSnapshot { snapshot, .. } => {
            (ResourceKind::Snapshot, "snapshot", snapshot.0)
        }
    };
    Ok(ResourceRef {
        kind,
        id: ResourceId::try_from(format!("{prefix}-{namespace}-{number}"))?,
    })
}

pub(super) fn new_actions(
    transaction: &Transaction<'_>,
    namespace: &str,
    previous: &core::World,
    next: &core::World,
    sandbox: core::SandboxId,
) -> Result<Vec<WorkAction<core::EffectId>>, JournalError> {
    let record = next
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let operation = record.current_operation.ok_or(JournalError::CorruptStore)?;
    let mut actions = Vec::new();
    for effect in next.effect_ids().iter().rev() {
        if previous.effect_status(effect).is_some() {
            continue;
        }
        if effect.command().sandbox() != sandbox {
            return Err(JournalError::CorruptStore);
        }
        let logical = serde_json::to_vec(&effect.logical)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM controller_effect_keys WHERE logical_json=?1",
                [&logical],
                |row| row.get(0),
            )
            .optional()?;
        let logical_number = if let Some(number) = existing {
            number
        } else {
            let number = allocators::next_sql(transaction, SqlId::Effect)?;
            transaction.execute(
                "INSERT INTO controller_effect_keys(id,logical_json) VALUES (?1,?2)",
                params![number, &logical],
            )?;
            number
        };
        let generation = match effect.command() {
            core::Command::PrepareVm { generation, .. }
            | core::Command::AbortPrepare { generation, .. }
            | core::Command::ConfigureNetwork { generation, .. }
            | core::Command::ActivateVm { generation, .. }
            | core::Command::FreezeVm { generation, .. }
            | core::Command::SealSnapshot { generation, .. }
            | core::Command::CommitSnapshot { generation, .. }
            | core::Command::ApplyFilePolicy { generation, .. }
            | core::Command::StopVm { generation, .. }
            | core::Command::CleanupVm { generation, .. }
            | core::Command::CleanupSlot { generation, .. } => *generation,
            core::Command::InstallFence { .. } => record.generation,
        };
        actions.push(WorkAction {
            identity: ActionIdentity {
                protocol_version: PROTOCOL_VERSION,
                owner: owner_id(record.owner)?,
                sandbox: sandbox_id(namespace, sandbox)?,
                operation: operation_id(
                    namespace,
                    core::OperationKey {
                        sandbox,
                        operation: operation.id,
                    },
                )?,
                epoch: effect.command().epoch().unwrap_or(record.epoch),
                generation,
                logical_action: LogicalActionId::try_from(format!("effect-{logical_number}"))?,
                attempt: effect.attempt,
                resource: resource(namespace, effect.command())?,
            },
            command: effect.clone(),
        });
    }
    Ok(actions)
}

pub(super) fn update_operation_projections(
    transaction: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<(), JournalError> {
    let owner = owner_id(
        world
            .sandbox_record(sandbox)
            .ok_or(JournalError::CorruptStore)?
            .owner,
    )?;
    let public_sandbox = sandbox_id(namespace, sandbox)?;
    for (key, record) in world.operation_records() {
        if key.sandbox != sandbox {
            continue;
        }
        let changed = transaction.execute(
            "UPDATE operations SET state_blob=?1 WHERE operation_id=?2 AND sandbox_id=?3 AND owner_id=?4",
            params![serde_json::to_vec(record)?, operation_id(namespace, *key)?.as_str(), public_sandbox.as_str(), owner.as_str()],
        )?;
        if changed != 1 {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

pub(super) fn input_sandbox(
    input: &core::ControllerInput,
) -> Result<core::SandboxId, JournalError> {
    use core::{
        ControllerInput as I, EffectInput as E, PublicationInput as P, Request as R,
        SchedulerInput as S,
    };
    Ok(match input {
        I::Request(R::Create(request)) => request.sandbox,
        I::Request(R::Kill(request)) => request.sandbox,
        I::Request(R::Pause(request)) => request.sandbox,
        I::Request(R::Compensate(request)) => request.sandbox,
        I::Scheduler(S::ReserveStartup(key) | S::ReservePauseSnapshot(key)) => key.sandbox,
        I::Scheduler(
            S::DispatchPrepare(sandbox, ..)
            | S::AbortPrepare(sandbox, ..)
            | S::StopVm(sandbox, ..)
            | S::CleanupVm(sandbox, ..)
            | S::CleanupSlot(sandbox, ..)
            | S::SealSnapshot(sandbox, ..)
            | S::CommitSnapshot(sandbox, ..),
        )
        | I::Publication(
            P::Running(sandbox)
            | P::Destroyed(sandbox)
            | P::Failed(sandbox, _)
            | P::Paused(sandbox, _),
        )
        | I::RpcTimeout(sandbox) => *sandbox,
        I::Worker(effect)
        | I::Effect(E::Dispatch(effect) | E::Receipt(effect, _) | E::Timeout(effect)) => {
            effect.command().sandbox()
        }
        I::Effect(E::Retry(effect, _) | E::FinalizeFailure(effect, _)) => effect.command.sandbox(),
        I::Effect(E::RetireActivation { activation, .. }) => activation.command().sandbox(),
        _ => return Err(JournalError::UnsupportedInput),
    })
}
