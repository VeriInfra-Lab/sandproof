//! Original bounded per-position confirmation and retirement, on the same SQL
//! connection. No native work or host effects are performed by these methods.
use super::{ControllerJournal, LifecycleRetentionLane, access, authenticated, codec};
use crate::controller::transaction::WorldTransaction;
use crate::{CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::{
    internal::NodeIdentity,
    lifecycle::{
        AuthenticatedCenterContext, ClosedLifecycleWindow, ConfirmedLifecycleResult,
        LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
    },
};

fn checked_result(result: &TerminalLifecycleObservation) -> Result<(), JournalError> {
    if result.status != TerminalStatus::Cleaned
        || !result.matches_request(&result.request)
        || serde_json::to_vec(result)?.len() > 8192
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

impl ControllerJournal {
    pub(crate) fn is_recently_retired_terminal(
        &self,
        result: &TerminalLifecycleObservation,
        context: &AuthenticatedCenterContext,
    ) -> Result<bool, JournalError> {
        checked_result(result)?;
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, &result.request, context)?;
        let exact = inbox
            .lanes
            .iter()
            .any(|lane| lane.last_retired.as_ref() == Some(result));
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(exact)
    }

    /// Save the exact authenticated center confirmation only after independently
    /// rechecking our complete original cleanup facts. An input flag is no proof.
    /// # Errors
    /// Changed key/order, pending resources and uncertain commits refuse an ack.
    pub fn confirm_lifecycle_terminal(
        &mut self,
        result: &TerminalLifecycleObservation,
        context: &AuthenticatedCenterContext,
        crashes: &mut impl CrashInjector,
    ) -> Result<ConfirmedLifecycleResult, JournalError> {
        checked_result(result)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        codec::validate(&tx, &world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, &result.request, context)?;
        if inbox
            .lanes
            .iter()
            .any(|lane| lane.last_retired.as_ref() == Some(result))
        {
            return Ok(ConfirmedLifecycleResult {
                result: result.clone(),
            });
        }
        if !access::destroy_cleaned(&tx, &world, &inbox, &result.request, context)? {
            return Err(JournalError::UnresolvedAction);
        }
        let (s, _) = inbox.find(&result.request)?.ok_or(JournalError::Conflict)?;
        inbox.sessions[s].confirmed_terminal = Some(result.clone());
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(ConfirmedLifecycleResult {
            result: result.clone(),
        })
    }

    /// Advance the original lane only after its exact durable confirmation and
    /// complete native cleanup/pin checks. Keeps one recent complete stamp.
    /// # Errors
    /// No absent row, newer frontier or caller success flag can retire a use.
    pub fn close_lifecycle_window(
        &mut self,
        request: &LifecycleWindowClose,
        context: &AuthenticatedCenterContext,
        crashes: &mut impl CrashInjector,
    ) -> Result<ClosedLifecycleWindow, JournalError> {
        checked_result(&request.terminal)?;
        let state = self.lifecycle_work_state(&request.terminal.request, context);
        if let Ok(state) = state.as_ref()
            && let Some(local) = state.local
            && self.host_slot_release_pinned(local.sandbox)?
        {
            return Err(JournalError::UnresolvedAction);
        }
        // A retired replay is checked by the complete recent stamp below, never
        // by treating this missing work-state lookup as proof of closure.
        if let Err(error) = state
            && !matches!(error, JournalError::Conflict)
        {
            return Err(error);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        codec::validate(&tx, &world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let terminal = &request.terminal;
        authenticated(&inbox, &terminal.request, context)?;
        let lane = inbox
            .lanes
            .iter()
            .find(|lane| lane.slot == terminal.request.retention.slot)
            .ok_or(JournalError::Conflict)?;
        let ack = ClosedLifecycleWindow {
            terminal: terminal.clone(),
            retired_through: terminal.request.retention.window,
        };
        if terminal.request.retention.window.get() <= lane.retired_through {
            return if lane.last_retired.as_ref() == Some(terminal) {
                Ok(ack)
            } else {
                Err(JournalError::Conflict)
            };
        }
        let (s, _) = inbox
            .find(&terminal.request)?
            .ok_or(JournalError::Conflict)?;
        if inbox.sessions[s].confirmed_terminal.as_ref() != Some(terminal)
            || lane.current_window != Some(terminal.request.retention.window)
            || !access::destroy_cleaned(&tx, &world, &inbox, &terminal.request, context)?
        {
            return Err(JournalError::UnresolvedAction);
        }
        if let Some(local) = inbox.sessions[s].local {
            sandbox_controller_core::maintenance::retirement::validate_terminal_cleanup(
                &world,
                local.sandbox,
            )
            .map_err(|_| JournalError::UnresolvedAction)?;
        }
        let lane = inbox
            .lanes
            .iter_mut()
            .find(|lane| lane.slot == terminal.request.retention.slot)
            .ok_or(JournalError::Conflict)?;
        lane.retired_through = terminal.request.retention.window.get();
        lane.current_window = lane
            .retired_through
            .checked_add(1)
            .and_then(|next| next.try_into().ok());
        lane.last_retired = Some(terminal.clone());
        inbox.sessions.remove(s);
        inbox.owners.retain(|owner| {
            inbox
                .sessions
                .iter()
                .any(|session| session.owner == owner.external)
        });
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(ack)
    }

    /// # Errors
    /// Only the protected original node may read finite lane state.
    pub fn lifecycle_retention_lanes(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<LifecycleRetentionLane>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node {
            return Err(JournalError::Conflict);
        }
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(inbox.lanes)
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
