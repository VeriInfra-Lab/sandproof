//! Individual durable rejection positions; native cleanup stays in the original sweep.
use super::{
    super::transaction::WorldTransaction,
    codec,
    model::{Entry, Inbox, InternalRetentionLane},
};
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::LifecycleResponse,
    window::{WindowCloseAck, WindowCloseRequest},
};

fn exact_terminal<'a>(
    inbox: &'a Inbox,
    terminal: &LifecycleResponse,
) -> Result<&'a Entry, JournalError> {
    let current = inbox
        .entries
        .iter()
        .rev()
        .find(|entry| entry.request.identity.retention_slot == terminal.identity.retention_slot)
        .ok_or(JournalError::Conflict)?;
    if current.response != *terminal || current.confirmed.as_ref() != Some(terminal) {
        return Err(JournalError::Conflict);
    }
    Ok(current)
}

fn local_origin(
    inbox: &Inbox,
    terminal: &LifecycleResponse,
) -> Option<super::InternalLocalBinding> {
    inbox
        .entries
        .iter()
        .filter(|entry| entry.request.identity.sandbox_id == terminal.identity.sandbox_id)
        .find_map(|entry| entry.local)
}

impl ControllerJournal {
    /// Retire exactly one confirmed terminal incarnation in the ORIGINAL
    /// transaction. Other live positions do not delay it. Original physical
    /// release witnesses, registry/build pins and worker/data quiescence still
    /// apply; this operation never deletes native rows or performs host I/O.
    /// # Errors
    /// Changed terminal stamps, active/unknown facts, original native pins,
    /// shared corruption and uncertain commits cannot acknowledge closure.
    pub fn close_internal_window(
        &mut self,
        request: &WindowCloseRequest,
        crashes: &mut impl CrashInjector,
    ) -> Result<WindowCloseAck, JournalError> {
        codec::validate(&self.journal.connection, &self.world()?)?;
        let original =
            codec::load(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        request
            .validate(&original.config.node)
            .map_err(|_| JournalError::Conflict)?;
        let terminal = &request.terminal;
        let lane = original
            .lane(terminal.identity.retention_slot)
            .ok_or(JournalError::Conflict)?;
        if terminal.identity.request_window.get() <= lane.retired_through {
            // Only the ONE most recent exact stamp is replayable. Older stamps
            // are rejected, never accumulated and never affect a successor.
            return if lane.last_retired.as_ref() == Some(terminal) {
                Ok(WindowCloseAck {
                    terminal: terminal.clone(),
                })
            } else {
                Err(JournalError::Conflict)
            };
        }
        if lane.current_window != Some(terminal.identity.request_window) {
            return Err(JournalError::Conflict);
        }
        exact_terminal(&original, terminal)?;
        let local = local_origin(&original, terminal);
        if let Some(local) = local
            && self.host_slot_release_pinned(local.sandbox)?
        {
            return Err(JournalError::UnresolvedAction);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        codec::validate(&tx, &world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        exact_terminal(&inbox, terminal)?;
        if let Some(local) = local {
            sandbox_controller_core::maintenance::retirement::validate_terminal_cleanup(
                &world,
                local.sandbox,
            )
            .map_err(|_| JournalError::UnresolvedAction)?;
        }
        let lane = inbox
            .lanes
            .iter_mut()
            .find(|lane| lane.retention_slot == terminal.identity.retention_slot)
            .ok_or(JournalError::Conflict)?;
        if lane.current_window != Some(terminal.identity.request_window) {
            return Err(JournalError::Conflict);
        }
        lane.retired_through = terminal.identity.request_window.get();
        lane.current_window = lane
            .retired_through
            .checked_add(1)
            .and_then(|v| v.try_into().ok());
        lane.last_retired = Some(terminal.clone());
        inbox
            .entries
            .retain(|entry| entry.request.identity.sandbox_id != terminal.identity.sandbox_id);
        // Keep shared mappings still referenced by survivors; highwater is never
        // reset when the last retained use of a local tenant disappears.
        inbox.tenants.retain(|binding| {
            inbox.entries.iter().any(|entry| {
                entry
                    .local
                    .is_some_and(|local| local.owner == binding.local)
                    && entry.request.identity.tenant_id == binding.tenant
            })
        });
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(WindowCloseAck {
            terminal: terminal.clone(),
        })
    }

    /// Original bounded durable positions. Exhausted MAX means `None`, not an
    /// empty host or permission to reset. No recovery/maintenance is performed.
    /// # Errors
    /// Missing authority, node mismatch and shared corruption remain explicit.
    pub fn internal_retention_lanes(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<InternalRetentionLane>, JournalError> {
        codec::validate(&self.journal.connection, &self.world()?)?;
        let inbox = codec::load(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node {
            return Err(JournalError::Conflict);
        }
        Ok(inbox.lanes)
    }
}
