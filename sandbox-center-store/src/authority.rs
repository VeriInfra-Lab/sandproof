//! Pure decision + durable commit assembly. No host RPC under this borrow.
use crate::{ControlState, Observed, RedisLedger, StoreError};
use sandproof_center_core::{CenterState, DecisionError, NodeRegistration};

mod lifecycle;
mod window;

impl ControlState for CenterState {
    const FORMAT_VERSION: u32 = sandproof_protocol::internal::CENTER_STORE_FORMAT_VERSION;
    fn check_supported_profiles(&self) -> Result<(), StoreError> {
        if self.lifecycle_profile_version() != sandproof_center_core::LIFECYCLE_PROFILE_VERSION {
            return Err(StoreError::UnsupportedProfile);
        }
        Ok(())
    }
    fn is_valid(&self) -> bool {
        self.is_valid()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityError {
    Store(StoreError),
    Decision(DecisionError),
    RecoveryRequired,
}
impl From<StoreError> for AuthorityError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}
impl From<DecisionError> for AuthorityError {
    fn from(e: DecisionError) -> Self {
        Self::Decision(e)
    }
}

/// A single authoritative envelope. The default keeps the existing pure
/// center interface; business envelopes replace only its center projection.
pub trait AuthorityState: ControlState + Clone {
    fn center(&self) -> &CenterState;
    #[must_use]
    fn with_center(&self, center: CenterState) -> Self;
    fn required_block_bytes(&self) -> usize;
}
impl AuthorityState for CenterState {
    fn center(&self) -> &CenterState {
        self
    }
    fn with_center(&self, center: CenterState) -> Self {
        center
    }
    fn required_block_bytes(&self) -> usize {
        self.required_authority_bytes()
    }
}
pub struct CenterAuthority<S = CenterState> {
    ledger: RedisLedger<S>,
    observed: Option<Observed<S>>,
}
impl<S: AuthorityState> CenterAuthority<S> {
    /// Attaches an EXISTING block, never initializes from missing storage.
    /// # Errors
    /// Refuses insufficient full-future capacity or deployment mismatch.
    pub fn attach(mut ledger: RedisLedger<S>) -> Result<Self, AuthorityError> {
        let observed = ledger.read()?;
        if observed.state().required_block_bytes() > ledger.max_block_bytes()
            || observed.state().center().deployment() != ledger.deployment()
        {
            return Err(StoreError::Configuration.into());
        }
        let observed = ledger.reaffirm(&observed)?.into_observed();
        Ok(Self {
            ledger,
            observed: Some(observed),
        })
    }
    /// # Errors
    /// Unknown commit invalidates even cached reads until reconciliation.
    pub fn state(&self) -> Result<&CenterState, AuthorityError> {
        self.observed
            .as_ref()
            .map(|value| value.state().center())
            .ok_or(AuthorityError::RecoveryRequired)
    }
    /// A public read/authorization view must be confirmed during this call,
    /// not taken from a pre-request cache. Reaffirm never lifts an unknown
    /// write fence; that requires explicit recovery of the original block.
    /// # Errors
    /// Storage failures invalidate even the previously observed view.
    pub fn snapshot(&mut self) -> Result<CenterState, AuthorityError> {
        Ok(self.snapshot_envelope()?.center().clone())
    }
    /// Fresh durable view of both lifecycle and business metadata, never a
    /// business-only cache or a second Redis key.
    /// # Errors
    /// Any shared storage fault fences every projection of this authority.
    pub fn snapshot_envelope(&mut self) -> Result<S, AuthorityError> {
        let observed = self
            .observed
            .as_ref()
            .ok_or(AuthorityError::RecoveryRequired)?;
        match self.ledger.reaffirm(observed) {
            Ok(committed) => {
                let state = committed.state().clone();
                self.observed = Some(committed.into_observed());
                Ok(state)
            }
            Err(error) => {
                self.observed = None;
                Err(error.into())
            }
        }
    }
    fn commit(&mut self, next: &S) -> Result<(), AuthorityError> {
        if next.required_block_bytes() > self.ledger.max_block_bytes() {
            return Err(DecisionError::Capacity.into());
        }
        let previous = self
            .observed
            .as_ref()
            .ok_or(AuthorityError::RecoveryRequired)?;
        match self.ledger.commit(previous, next) {
            Ok(committed) => {
                self.observed = Some(committed.into_observed());
                Ok(())
            }
            Err(error) => {
                self.observed = None;
                Err(error.into())
            }
        }
    }
    /// Apply one pure whole-envelope decision with the original CAS/unknown
    /// boundary. Callers must not perform external I/O in the decision.
    /// # Errors
    /// A rejected decision writes nothing; any commit error fences all views.
    pub fn decide<R>(
        &mut self,
        decision: impl FnOnce(&S) -> Result<(S, R), AuthorityError>,
    ) -> Result<R, AuthorityError> {
        let current = self
            .observed
            .as_ref()
            .ok_or(AuthorityError::RecoveryRequired)?
            .state();
        let (next, result) = decision(current)?;
        self.commit(&next)?;
        Ok(result)
    }
    fn commit_center(&mut self, center: CenterState) -> Result<(), AuthorityError> {
        let current = self
            .observed
            .as_ref()
            .ok_or(AuthorityError::RecoveryRequired)?
            .state();
        self.commit(&current.with_center(center))
    }
    /// # Errors
    /// Trusted observation cannot change fixed identity or capacity.
    pub fn observe_node(&mut self, observation: NodeRegistration) -> Result<(), AuthorityError> {
        let next = self.state()?.observe_node(observation)?;
        self.commit_center(next)
    }
    /// # Errors
    /// Read/reconnect alone cannot lift an unknown-write fence: reaffirm durably.
    pub fn recover(&mut self) -> Result<(), AuthorityError> {
        self.observed = None;
        self.ledger.reconnect()?;
        let observed = self.ledger.read()?;
        let committed = self.ledger.reaffirm(&observed)?;
        self.observed = Some(committed.into_observed());
        Ok(())
    }
}
