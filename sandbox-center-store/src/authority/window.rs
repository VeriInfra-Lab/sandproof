//! Single-lane retirement commits through the same authority envelope and CAS.
//! No network runs under this borrow, no TTL removes unconfirmed history.
use super::{AuthorityError, AuthorityState, CenterAuthority};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    ClosedLifecycleWindow, ConfirmedLifecycleResult, LifecycleWindowClose,
    TerminalLifecycleObservation,
};

impl<S: AuthorityState> CenterAuthority<S> {
    /// Explicit protected node drain, not a prerequisite for retiring another
    /// sandbox's terminal lane. Existing lifecycle work retains its identity.
    /// # Errors
    /// Invalid nodes or uncertain commits never manufacture a drained fact.
    pub fn begin_drain(&mut self, node: &NodeIdentity) -> Result<(), AuthorityError> {
        let next = self.state()?.begin_drain(node)?;
        self.commit_center(next)
    }

    /// Persist the host's exact acknowledgement of an already committed final
    /// result. The caller must obtain this through the authenticated host port.
    /// # Errors
    /// Altered, superseded, nonterminal or uncertain results cannot enable close.
    pub fn confirm_terminal(
        &mut self,
        response: &ConfirmedLifecycleResult,
    ) -> Result<(), AuthorityError> {
        self.snapshot()?;
        let next = self.state()?.confirm_terminal(response)?;
        self.commit_center(next)
    }

    /// The returned request contains the exact terminal result of ONE sandbox,
    /// not a deletion list, node drain or acknowledgement.
    /// Dispatch must happen after releasing the authority/decision lock.
    /// The complete envelope validates BEFORE commit/dispatch: unresolved
    /// template/publication pins veto this lane's close, not unrelated lanes.
    /// # Errors
    /// Active, unknown, unconfirmed or uncommitted lanes cannot dispatch close.
    pub fn begin_close(
        &mut self,
        terminal: &TerminalLifecycleObservation,
        now_ms: u64,
    ) -> Result<LifecycleWindowClose, AuthorityError> {
        // Reaffirm the ORIGINAL complete envelope before classifying a known
        // pure business pin. Storage/hash/restore errors keep the original
        // shared fence and cannot be downgraded to this decision refusal.
        let current = self.snapshot_envelope()?;
        if !current.is_valid() {
            self.observed = None;
            return Err(crate::StoreError::InvalidState.into());
        }
        let (center, request) = current.center().begin_close(terminal, now_ms)?;
        if !center.is_valid() {
            self.observed = None;
            return Err(crate::StoreError::InvalidState.into());
        }
        let next = current.with_center(center);
        if !next.is_valid() {
            // The pure core transition has already validated its result. An
            // unchanged valid business envelope can veto this exact deletion
            // without performing any CAS or fencing a healthy sibling lane.
            return Err(sandproof_center_core::DecisionError::Conflict.into());
        }
        self.commit(&next)?;
        Ok(request)
    }

    /// Retry only close intents confirmed against authority within this call.
    /// # Errors
    /// Storage failures fence this projection along with lifecycle dispatch.
    pub fn close_batch(
        &mut self,
        now_ms: u64,
    ) -> Result<Vec<LifecycleWindowClose>, AuthorityError> {
        let state = self.snapshot()?;
        Ok(state
            .pending_closes()
            .into_iter()
            .filter(|close| {
                let binding = &close.terminal.request.request.binding;
                state
                    .lifecycle_session(&binding.key.owner, &binding.session)
                    .is_some_and(|record| record.result_retention_elapsed(now_ms))
            })
            .collect())
    }

    /// Apply an exact durable host-close acknowledgement. The whole envelope
    /// still validates business/publication references before the CAS; a live
    /// validation pin vetoes deletion, a settled receipt must already be saved.
    /// # Errors
    /// Wrong acknowledgements, outstanding references and unknown commits keep
    /// the original close responsibility. This does not attest physical cleanup.
    pub fn apply_close_ack(
        &mut self,
        acknowledgement: &ClosedLifecycleWindow,
        now_ms: u64,
    ) -> Result<(), AuthorityError> {
        let current = self.snapshot_envelope()?;
        let center = current.center().apply_close_ack(acknowledgement, now_ms)?;
        let next = current.with_center(center);
        if !next.is_valid() {
            return Err(sandproof_center_core::DecisionError::Conflict.into());
        }
        self.commit(&next)
    }
}
