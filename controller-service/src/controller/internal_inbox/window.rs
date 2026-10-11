//! Durable rejection frontier only; native cleanup stays in the original sweep.
use super::{super::transaction::WorldTransaction, codec, model::Inbox};
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::internal::{
    PositiveCounter,
    lifecycle::LifecycleFact,
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::collections::BTreeMap;

fn terminal_confirmed(inbox: &Inbox) -> bool {
    let mut latest = BTreeMap::new();
    for entry in &inbox.entries {
        latest.insert(entry.request.identity.sandbox_id.as_str(), entry);
    }
    latest.values().all(|entry| {
        matches!(
            entry.response.fact,
            LifecycleFact::Cleaned | LifecycleFact::NotAccepted
        ) && entry.confirmed.as_ref() == Some(&entry.response)
    })
}

impl ControllerJournal {
    /// Reject the complete window permanently only after exact confirmed final
    /// facts and the original native physical-release checks. This does NOT
    /// delete native objects or registry/build evidence and does NOT run host I/O.
    /// The original independent maintain/retire path remains responsible for
    /// worker/data quiescence and all other retained local references.
    /// # Errors
    /// Foreign/future windows, active/unknown/unconfirmed facts, original pins,
    /// corrupt native evidence and unknown commits cannot acknowledge closure.
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
        let target = request.identity.request_window;
        if target.get() <= original.retired_through {
            // The full node/protocol binding has already been checked. No old
            // duplicate can erase or confirm any entry of the active successor.
            return Ok(WindowCloseAck {
                identity: request.identity.clone(),
            });
        }
        if original.current_window() != Some(target) || !terminal_confirmed(&original) {
            return Err(JournalError::Conflict);
        }
        // Reuse the ONE original registry/private-build pin rule. This journal's
        // exclusive borrow/lease makes it stable until BEGIN IMMEDIATE; begin
        // also rejects an external connection data_version change before writes.
        for entry in &original.entries {
            if let Some(local) = entry.local
                && self.host_slot_release_pinned(local.sandbox)?
            {
                return Err(JournalError::UnresolvedAction);
            }
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        // Reconstruct original saved native cleanup plus exact release witnesses
        // BEFORE deleting inbox pins; a caller-provided Cleaned is not evidence.
        let world = tx.world();
        codec::validate(&tx, &world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if inbox.current_window() != Some(target) || !terminal_confirmed(&inbox) {
            return Err(JournalError::Conflict);
        }
        for entry in &inbox.entries {
            if let Some(local) = entry.local {
                sandbox_controller_core::maintenance::retirement::validate_terminal_cleanup(
                    &world,
                    local.sandbox,
                )
                .map_err(|_| JournalError::UnresolvedAction)?;
            }
        }
        inbox.retired_through = target.get();
        inbox.entries.clear();
        // Mappings describe retained distributed origins, not a permanent tenant
        // directory. The scalar highwater survives, so old local IDs never recur.
        inbox.tenants.clear();
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(WindowCloseAck {
            identity: request.identity.clone(),
        })
    }

    /// Read-only validated durable window frontier; None means exhausted MAX,
    /// not an empty/unprovisioned host and not permission to reset numbering.
    /// # Errors
    /// Missing configuration, node mismatch and shared corruption stay explicit.
    pub fn internal_window_frontier(
        &self,
        node: &sandproof_protocol::internal::NodeIdentity,
    ) -> Result<(u64, Option<PositiveCounter>), JournalError> {
        codec::validate(&self.journal.connection, &self.world()?)?;
        let inbox = codec::load(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node {
            return Err(JournalError::Conflict);
        }
        Ok((inbox.retired_through, inbox.current_window()))
    }
}
