//! Bounded, secret-free projections of the original validated authority.
use super::{
    codec,
    model::{InternalLocalBinding, InternalRetentionLane},
};
use crate::{ControllerJournal, HostSlotContext, JournalError};
use rusqlite::Connection;
use sandbox_controller_core as core;
use sandproof_protocol::internal::{
    ControlIdentity, NodeIdentity,
    lifecycle::{Intent, LifecycleResponse},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// A checked database snapshot, not a current physical observation or permit.
/// Counts are bounded by the original checkpoint and inbox configurations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InternalAudit {
    pub node: NodeIdentity,
    /// Fixed original durable frontiers and exact most recent terminal stamps.
    pub lanes: Vec<InternalRetentionLane>,
    pub operations: Vec<InternalAuditOperation>,
    pub native: Vec<InternalNativeAudit>,
    /// Includes original last bindings of already-free physical positions.
    pub host_slots: Vec<(HostSlotContext, core::maintenance::slots::HostSlotPhase)>,
}

/// Retained authenticated identity and saved fact, without creation payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InternalAuditOperation {
    pub identity: ControlIdentity,
    pub intent: Intent,
    pub local: Option<InternalLocalBinding>,
    pub current: bool,
    pub response: LifecycleResponse,
    /// Original exact saved center acknowledgement; no inferred confirmation.
    pub confirmed: Option<LifecycleResponse>,
}

/// Original local state and exact resource references. No secret, environment,
/// launch path or guest-controlled text is exported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InternalNativeAudit {
    pub sandbox: core::SandboxId,
    pub owner: core::TenantId,
    pub phase: core::Phase,
    pub terminal: bool,
    pub epoch: u64,
    pub generation: u64,
    pub operation: Option<core::OperationId>,
    pub current_vm: Option<core::VmId>,
    pub resources: core::ResourceIndex,
}

pub(in crate::controller) fn snapshot(
    connection: &Connection,
    world: &core::World,
) -> Result<InternalAudit, JournalError> {
    // Reuse the exact persisted inbox and native mapping/fact checks. A missing
    // provisioned inbox is not an empty deployment for the authenticated host.
    codec::validate(connection, world)?;
    let inbox = codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let config =
        super::super::bounded::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let host_slots = super::super::bounded::validate_contexts(connection, world, &config)?
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
    // Temporary bounded view only, never persisted or used for permission.
    // Build once instead of scanning every retained entry for every result.
    let mut latest = BTreeMap::new();
    for entry in &inbox.entries {
        let summary = latest
            .entry(entry.request.identity.sandbox_id.as_str())
            .or_insert((0_u64, None));
        summary.0 = summary.0.max(entry.request.identity.intent_version.get());
        if let Some(local) = entry.local {
            summary.1 = Some(local);
        }
    }
    let operations = inbox
        .entries
        .iter()
        .map(|entry| {
            let identity = &entry.request.identity;
            let (version, local) = latest[identity.sandbox_id.as_str()];
            InternalAuditOperation {
                identity: identity.clone(),
                intent: entry.request.intent,
                local,
                current: identity.intent_version.get() == version,
                response: entry.response.clone(),
                confirmed: entry.confirmed.clone(),
            }
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
            operation: record
                .current_operation
                .as_ref()
                .map(|operation| operation.id),
            current_vm: record.current_vm,
            resources: world.resource_index(*sandbox),
        })
        .collect();
    Ok(InternalAudit {
        lanes: inbox.lanes,
        node: inbox.config.node,
        operations,
        native,
        host_slots,
    })
}

impl ControllerJournal {
    /// Available original physical positions and retained-history lanes under
    /// one checked read transaction. No row means never bound only in the
    /// complete protected physical-position domain, not absence of responsibility.
    /// Unretired requests and exhausted lane generations remain unavailable.
    /// # Errors
    /// Missing, changed or corrupt authority and wrong node fail closed.
    pub fn internal_available_instances(&self, node: &NodeIdentity) -> Result<u64, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let world = super::super::checkpoint::load_world(&tx)?;
        let audit = snapshot(&tx, &world)?;
        if audit.node != *node {
            return Err(JournalError::Conflict);
        }
        let config = super::super::bounded::config(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let physical = super::super::bounded::available_positions(&world, &config)?;
        let retained: BTreeSet<_> = audit
            .operations
            .iter()
            .map(|operation| operation.identity.retention_slot)
            .collect();
        let lanes = u64::try_from(
            audit
                .lanes
                .iter()
                .filter(|lane| {
                    lane.current_window.is_some() && !retained.contains(&lane.retention_slot)
                })
                .count(),
        )
        .map_err(|_| JournalError::CorruptStore)?;
        tx.commit()?;
        Ok(physical.min(lanes))
    }

    /// Inspect the original authority under one read transaction. This never
    /// receives requests, advances time, recovers work or accesses host secrets.
    /// # Errors
    /// Rejects invalid or missing authority rather than returning a partial view.
    pub fn internal_audit(&self) -> Result<InternalAudit, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let world = super::super::checkpoint::load_world(&tx)?;
        let audit = snapshot(&tx, &world)?;
        tx.commit()?;
        Ok(audit)
    }
}
