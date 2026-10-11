//! Window transitions commit through the same authority envelope and CAS.
//! No network runs under this borrow, no TTL removes unconfirmed history.
use super::{AuthorityError, AuthorityState, CenterAuthority};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::LifecycleResponse,
    window::{WindowCloseAck, WindowCloseRequest},
};

impl<S: AuthorityState> CenterAuthority<S> {
    /// Stop new creates only after the original pure decision is durable.
    /// Existing renew/destroy/expiry work keeps its original request window.
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
    pub fn confirm_terminal(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        let next = self.state()?.confirm_terminal(response)?;
        self.commit_center(next)
    }

    /// The returned request is a dispatch package, not an acknowledgement.
    /// Dispatch must happen after releasing the authority/decision lock.
    /// The complete envelope validates BEFORE commit/dispatch: unresolved
    /// template/publication history pins veto `ClosePending`, not merely its ack.
    /// # Errors
    /// Active, unknown, unconfirmed or uncommitted windows cannot dispatch close.
    pub fn begin_close(
        &mut self,
        node: &NodeIdentity,
    ) -> Result<WindowCloseRequest, AuthorityError> {
        let (next, request) = self.state()?.begin_close(node)?;
        self.commit_center(next)?;
        Ok(request)
    }

    /// Retry only close intents confirmed against authority within this call.
    /// # Errors
    /// Storage failures fence this projection along with lifecycle dispatch.
    pub fn close_batch(&mut self) -> Result<Vec<WindowCloseRequest>, AuthorityError> {
        Ok(self.snapshot()?.pending_closes())
    }

    /// Apply an exact durable host-close acknowledgement. The whole envelope
    /// still validates business/publication references before the CAS; a live
    /// validation pin vetoes deletion, a settled receipt must already be saved.
    /// # Errors
    /// Wrong acknowledgements, outstanding references and unknown commits keep
    /// the original close responsibility. This does not attest physical cleanup.
    pub fn apply_close_ack(
        &mut self,
        acknowledgement: &WindowCloseAck,
    ) -> Result<(), AuthorityError> {
        let next = self.state()?.apply_close_ack(acknowledgement)?;
        self.commit_center(next)
    }
}
