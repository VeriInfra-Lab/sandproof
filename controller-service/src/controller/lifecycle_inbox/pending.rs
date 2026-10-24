//! Bounded original persisted inputs for the existing local work gate.
use super::{ControllerJournal, codec};
use crate::JournalError;
use sandbox_controller_core as core;
use sandproof_protocol::{
    internal::NodeIdentity,
    lifecycle::{AcceptedLifecycleIntent, LifecycleOperation},
};

impl ControllerJournal {
    /// Current, already received renewals without a committed application.
    /// Listing is not permission; apply rechecks the full original transaction.
    /// # Errors
    /// Wrong node and unreadable original authority refuse the scan.
    pub fn lifecycle_pending_renewals(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node {
            return Err(JournalError::Conflict);
        }
        let pending = inbox
            .sessions
            .iter()
            .filter(|session| !session.terminal)
            .filter_map(|session| {
                session.entries.iter().find(|entry| {
                    entry.intent.accepted_version == session.current_version
                        && entry.applied_deadline.is_none()
                        && matches!(
                            entry.intent.request.operation,
                            LifecycleOperation::Renew { .. }
                        )
                })
            })
            .map(|entry| entry.intent.clone())
            .collect();
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(pending)
    }

    pub(crate) fn recorded_lifecycle_context(
        &self,
        intent: &AcceptedLifecycleIntent,
        node: &NodeIdentity,
    ) -> Result<sandproof_protocol::lifecycle::AuthenticatedCenterContext, JournalError> {
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node || inbox.find(intent)?.is_none() {
            return Err(JournalError::Conflict);
        }
        // This reuses an already persisted, received full input. It does not
        // authenticate a new network sender or authorize an unrecorded request.
        let context = inbox.config.context();
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(context)
    }

    /// Read current Create requests from the original finite inbox. Returned
    /// values authorize nothing: advance rechecks the same original request.
    /// This local worker reads its protected Node configuration, not new TLS.
    /// # Errors
    /// Missing/foreign configuration and corrupt mapped objects fail closed.
    pub fn lifecycle_pending_creates(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node {
            return Err(JournalError::Conflict);
        }
        let mut pending = Vec::new();
        for session in &inbox.sessions {
            if session.terminal || session.refusal.is_some() {
                continue;
            }
            let entry = session
                .entries
                .iter()
                .find(|entry| entry.intent.accepted_version == session.current_version)
                .ok_or(JournalError::CorruptStore)?;
            if !matches!(
                entry.intent.request.operation,
                LifecycleOperation::Create { .. }
            ) {
                continue;
            }
            let needs_advance = if let Some(local) = session.local {
                let record = world
                    .sandbox_record(local.sandbox)
                    .ok_or(JournalError::CorruptStore)?;
                if record.owner != local.owner {
                    return Err(JournalError::CorruptStore);
                }
                !record.terminal
                    && record.phase == core::Phase::Starting
                    && matches!(record.current_operation,Some(operation)
                        if operation.id==local.operation && operation.kind==core::OperationKind::Create)
            } else {
                true
            };
            if needs_advance {
                pending.push(entry.intent.clone());
            }
        }
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(pending)
    }
}

impl ControllerJournal {
    /// Bounded current Unload intents from the original durable inbox.
    /// # Errors
    /// Missing/foreign protected registration or corrupted state refuses.
    pub fn lifecycle_pending_unloads(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node {
            return Err(JournalError::Conflict);
        }
        let pending = inbox
            .sessions
            .iter()
            .filter(|s| !s.terminal)
            .filter_map(|s| {
                s.entries.iter().find(|e| {
                    e.intent.accepted_version == s.current_version
                        && matches!(
                            e.intent.request.operation,
                            LifecycleOperation::Unload { .. }
                        )
                })
            })
            .map(|e| e.intent.clone())
            .collect();
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(pending)
    }
}
