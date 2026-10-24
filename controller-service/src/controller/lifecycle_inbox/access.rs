//! Resolve the current authenticated data identity to the original local row.
use super::{ControllerJournal, codec};
use crate::{InternalLocalBinding, JournalError};
use sandproof_protocol::lifecycle::{
    AuthenticatedCenterContext, LifecycleAccessIdentity, LifecycleOperation,
};

impl ControllerJournal {
    /// Observe exact current Destroy against original cleanup/release evidence.
    /// Recording the terminal intent alone is never a positive result.
    /// # Errors
    /// Wrong full key, stale request, changed writer and corrupt facts refuse.
    pub fn lifecycle_destroy_cleaned(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<bool, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let cleaned = destroy_cleaned(&tx, &world, &inbox, intent, context)?;
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(cleaned)
    }

    /// This lookup supplies no native Access permission or relay launch. The
    /// caller must keep its authority lock through the original checks/pinning.
    /// # Errors
    /// Stale order/incarnation, terminal, foreign or unknown full keys refuse.
    pub fn lifecycle_access_local(
        &self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
    ) -> Result<InternalLocalBinding, JournalError> {
        identity
            .validate(context)
            .map_err(|_| JournalError::Conflict)?;
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        inbox.verify_context(context)?;
        let session = inbox
            .sessions
            .iter()
            .find(|session| {
                session.id == identity.session && session.owner == identity.operation.owner
            })
            .ok_or(JournalError::Conflict)?;
        if session.terminal
            || session.current_version != identity.accepted_version
            || session.incarnation != Some(identity.incarnation)
        {
            return Err(JournalError::Conflict);
        }
        let entry = session
            .entries
            .iter()
            .find(|entry| {
                entry.intent.accepted_version == identity.accepted_version
                    && entry.intent.request.binding.key == identity.operation
                    && entry.intent.request.binding.node == identity.node
            })
            .ok_or(JournalError::Conflict)?;
        let local = session.local.ok_or(JournalError::Conflict)?;
        // A continued Unload retains this execution/incarnation but has its
        // own exact current access key. Historical Ready alone grants nothing.
        if !(matches!(
            entry.intent.request.operation,
            LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }
        ) && entry.local_operation == Some(local.operation)
            || matches!(
                entry.intent.request.operation,
                LifecycleOperation::Renew { .. }
            ) && entry.applied_deadline.is_some()
            || matches!(
                entry.intent.request.operation,
                LifecycleOperation::Unload { .. }
            ) && entry
                .unload_resolution
                .as_ref()
                .is_some_and(|r| r.result().is_some()))
        {
            return Err(JournalError::Conflict);
        }
        let record = world
            .sandbox_record(local.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        if record.owner != local.owner
            || record.terminal
            || record.phase != sandbox_controller_core::Phase::Running
        {
            return Err(JournalError::Conflict);
        }
        tx.commit()?;
        Ok(local)
    }
}

// Shared by readonly observation, durable confirmation and retirement. This
// exact native/provider condition cannot be supplied by a wire success flag.
pub(super) fn destroy_cleaned(
    connection: &rusqlite::Connection,
    world: &sandbox_controller_core::World,
    inbox: &super::model::Inbox,
    intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> Result<bool, JournalError> {
    super::authenticated(inbox, intent, context)?;
    if !matches!(intent.request.operation, LifecycleOperation::Destroy {}) {
        return Err(JournalError::Conflict);
    }
    if inbox.lanes.iter().any(|lane| {
        lane.last_retired.as_ref().is_some_and(|last| {
            last.request == *intent
                && last.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned
        })
    }) {
        return Ok(true);
    }
    let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
    let state = inbox.state(s, e);
    if !state.current || !state.terminal {
        return Err(JournalError::Conflict);
    }
    if super::source_work::retains_session(connection, &intent.request.binding.session)? {
        return Ok(false);
    }
    if super::native_retirement::cleaned(&inbox.sessions[s],world)? {return Ok(true)}
    let Some(local) = state.local else {
        return Ok(true);
    };
    let no_extended_work = world.resource_index(local.sandbox).snapshots.is_empty()
        && world.operation_records().iter().all(|(key, _)| {
            key.sandbox != local.sandbox
                || world
                    .operation_plan(*key)
                    .is_some_and(|plan| plan.capture.is_none() && plan.restore_from.is_none())
        })
        && world.effect_ids().iter().all(|effect| {
            effect.command().sandbox() != local.sandbox
                || !matches!(
                    world.effect_status(effect),
                    Some(
                        sandbox_controller_core::EffectStatus::Dispatched
                            | sandbox_controller_core::EffectStatus::OutcomeUnknown
                    )
                )
        });
    Ok(no_extended_work && super::internal_inbox::native_cleaned(connection, world, local)?)
}
