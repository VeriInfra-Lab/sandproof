//! A checked, secret-free view of the current inbox and original native world.
use super::{codec, model::Inbox};
use crate::{
    ControllerJournal, HostSlotContext, InternalLocalBinding, InternalNativeAudit, JournalError,
    LifecycleInboxConfig,
};
use sandbox_controller_core as core;
use sandproof_protocol::{internal::NodeIdentity, lifecycle::{OperationBinding, TerminalLifecycleObservation}};
use serde::Serialize;

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LifecycleAuditOperation {
    pub binding: OperationBinding,
    pub accepted_version: u64,
    pub current: bool,
    pub terminal: bool,
    /// Exact original saved confirmation on the current operation only.
    /// The terminal intent flag alone is never a completed-cleanup result.
    pub confirmed_terminal: Option<TerminalLifecycleObservation>,
    pub local: Option<InternalLocalBinding>,
    pub local_operation: Option<core::OperationId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LifecycleAudit {
    pub node: NodeIdentity,
    pub source_observation: crate::capture::SourceObservationConfig,
    pub operations: Vec<LifecycleAuditOperation>,
    pub native: Vec<InternalNativeAudit>,
    pub host_slots: Vec<(HostSlotContext, core::maintenance::slots::HostSlotPhase)>,
    pub retention_lanes: Vec<super::LifecycleRetentionLane>,
}

pub(in crate::controller) fn snapshot(
    connection: &rusqlite::Connection,
    world: &core::World,
    expected: &LifecycleInboxConfig,
) -> Result<LifecycleAudit, JournalError> {
    codec::validate(connection, world)?;
    let inbox: Inbox = codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    if inbox.config != *expected {
        return Err(JournalError::UnsupportedStore);
    }
    let host = super::super::bounded::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let host_slots = super::super::bounded::validate_contexts(connection, world, &host)?
        .into_iter()
        .map(|context| {
            let phase = world
                .maintenance_state()
                .and_then(|state| state.host_slot(context.position))
                .ok_or(JournalError::CorruptStore)?
                .phase;
            Ok((context, phase))
        })
        .collect::<Result<_, JournalError>>()?;
    let operations = inbox
        .sessions
        .iter()
        .flat_map(|session| {
            session.entries.iter().map(|entry| LifecycleAuditOperation {
                binding: entry.intent.request.binding.clone(),
                accepted_version: entry.intent.accepted_version,
                current: entry.intent.accepted_version == session.current_version,
                terminal: session.terminal,
                confirmed_terminal: if entry.intent.accepted_version == session.current_version {
                    session.confirmed_terminal.clone()
                } else { None },
                local: session.local,
                local_operation: entry.local_operation,
            })
        })
        .collect();
    let native = world
        .sandbox_records()
        .iter()
        .map(|(sandbox, record)| InternalNativeAudit {
            sandbox: *sandbox,
            owner: record.owner,
            phase: record.phase,
            terminal: record.terminal,
            epoch: record.epoch,
            generation: record.generation,
            operation: record.current_operation.map(|operation| operation.id),
            current_vm: record.current_vm,
            resources: world.resource_index(*sandbox),
        })
        .collect();
    Ok(LifecycleAudit {
        node: inbox.config.node,
        source_observation: super::source_work::configuration_snapshot(connection)?,
        operations,
        native,
        host_slots,
        retention_lanes: inbox.lanes,
    })
}

impl ControllerJournal {
    /// Checked current-format capacity for new logical sessions, not permission
    /// to release a previous session or overwrite retained operation identities.
    /// # Errors
    /// Foreign node, missing registration or corrupt authority refuses.
    pub fn lifecycle_available_instances(&self, node: &NodeIdentity) -> Result<u64, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if inbox.config.node != *node {
            return Err(JournalError::Conflict);
        }
        let host = super::super::bounded::config(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let physical = super::super::bounded::available_positions(&world, &host)?;
        let unused = inbox
            .lanes
            .iter()
            .filter(|lane| {
                lane.current_window.is_some()
                    && !inbox
                        .sessions
                        .iter()
                        .any(|session| session.retention.slot == lane.slot)
            })
            .count();
        tx.commit()?;
        Ok(physical.min(u64::try_from(unused).map_err(|_| JournalError::CorruptStore)?))
    }
}
