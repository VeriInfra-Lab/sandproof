//! Same original Redis CAS for canonical intent, original reservation and outbox.
use super::{AuthorityError, AuthorityState, CenterAuthority};
use sandproof_center_core::{LifecycleSession, LifecycleSubmission};
use sandproof_protocol::TenantId;
use sandproof_protocol::internal::PositiveCounter;
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, SessionId};

impl<S: AuthorityState> CenterAuthority<S> {
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
    ) -> Result<Option<LifecycleSession>, AuthorityError> {
        Ok(self.snapshot()?.lifecycle_session(owner, session).cloned())
    }

    /// Actual host Ready facts remain a V3 authentication/V5 evidence obligation.
    /// This method saves them only for the exact current canonical Create.
    /// # Errors
    /// Full binding, original lifetime or CAS failures cannot publish Running.
    pub fn observe_lifecycle_ready(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        observation_version: PositiveCounter,
        started_at_ms: u64,
        end_at_ms: u64,
        max_end_at_ms: u64,
    ) -> Result<(), AuthorityError> {
        let next = self.state()?.observe_lifecycle_ready(
            intent,
            observation_version,
            started_at_ms,
            end_at_ms,
            max_end_at_ms,
        )?;
        self.commit_center(next)
    }
}
