//! Same original Redis CAS for canonical intent, original reservation and outbox.
use super::{AuthorityError, AuthorityState, CenterAuthority};
use sandproof_center_core::{LifecycleSubmission, Sandbox};
use sandproof_protocol::TenantId;
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, SessionId};

impl<S: AuthorityState> CenterAuthority<S> {
    /// # Errors
    /// Same original authority and business pins; no host I/O or public owner bypass.
    pub fn terminate_expired(&mut self, now_ms: u64) -> Result<(), AuthorityError> {
        self.snapshot()?;
        let next = self.state()?.terminate_expired(now_ms)?;
        if &next == self.state()? {
            self.snapshot()?;
        } else {
            self.commit_center(next)?;
        }
        Ok(())
    }
    /// # Errors
    /// A rejected decision writes nothing. Unknown CAS fences every projection;
    /// recovery reads/reaffirms this same block and replays the original key.
    pub fn accept_lifecycle(
        &mut self,
        owner: &TenantId,
        submission: &LifecycleSubmission,
        now_ms: u64,
    ) -> Result<AcceptedLifecycleIntent, AuthorityError> {
        let (next, intent) = self.state()?.accept_lifecycle(owner, submission, now_ms)?;
        if &next == self.state()? {
            // A replay is not permission to return a stale cached acceptance.
            self.snapshot()?;
        } else {
            self.commit_center(next)?;
        }
        Ok(intent)
    }

    /// Detached v3 work is rebuilt from the same durably reaffirmed authority.
    /// # Errors
    /// No in-memory or unknown-commit candidate may escape as dispatch work.
    pub fn lifecycle_dispatch_batch(
        &mut self,
    ) -> Result<Vec<AcceptedLifecycleIntent>, AuthorityError> {
        Ok(self.snapshot()?.pending_lifecycle())
    }

    /// # Errors
    /// Requires current durable authority and the original owner/session pair.
    pub fn query_lifecycle(
        &mut self,
        owner: &TenantId,
        session: &SessionId,
    ) -> Result<Option<Sandbox>, AuthorityError> {
        Ok(self.snapshot()?.lifecycle_session(owner, session).cloned())
    }

    /// Store one checked current observation in the original single session row.
    /// Sender/call/freshness conditions belong to the original caller; no host
    /// observation number is accepted or synthesized by this interface.
    /// # Errors
    /// A stale intent/domain, conflicting facts or unknown CAS retains work.
    pub fn accept_lifecycle_create_observation(
        &mut self,
        observation: &sandproof_protocol::lifecycle::CreateObservation,
    ) -> Result<(), AuthorityError> {
        self.snapshot()?;
        let next = self
            .state()?
            .accept_lifecycle_create_observation(observation)?;
        if &next == self.state()? {
            self.snapshot()?;
        } else {
            self.commit_center(next)?;
        }
        Ok(())
    }

    /// Save the real current terminal result before sending any confirmation.
    /// # Errors
    /// Unknown CAS fences the same block; recovery reuses its original deadline.
    pub fn accept_lifecycle_terminal_observation(
        &mut self,
        result: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        now_ms: u64,
    ) -> Result<(), AuthorityError> {
        self.snapshot()?;
        let next = self
            .state()?
            .accept_lifecycle_terminal_observation(result, now_ms)?;
        if &next == self.state()? {
            self.snapshot()?;
        } else {
            self.commit_center(next)?;
        }
        Ok(())
    }
}
