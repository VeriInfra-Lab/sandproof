//! Pure decision + durable commit assembly. No host RPC under this borrow.
use crate::{ControlState, Observed, RedisLedger, StoreError};
use sandproof_center_core::{CenterState, DecisionError, NodeRegistration};
use sandproof_protocol::internal::lifecycle::{CreateBinding, LifecycleRequest, LifecycleResponse};
use sandproof_protocol::{SandboxId, TenantId};

mod window;

impl ControlState for CenterState {
    const FORMAT_VERSION: u32 = 4;
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

/// High-level port. Implementations must enforce registered mTLS, full-body
/// timeout and bounded calls. All transport failures mean UNKNOWN, not release.
pub trait LifecycleTransport {
    type Error;
    /// # Errors
    /// No transport error is proof of a side-effect-free business refusal.
    fn send(
        &mut self,
        request: &LifecycleRequest,
    ) -> impl std::future::Future<Output = Result<LifecycleResponse, Self::Error>> + Send;
}
impl<F, Fut, E> LifecycleTransport for F
where
    F: FnMut(LifecycleRequest) -> Fut,
    Fut: std::future::Future<Output = Result<LifecycleResponse, E>> + Send,
{
    type Error = E;
    fn send(
        &mut self,
        request: &LifecycleRequest,
    ) -> impl std::future::Future<Output = Result<LifecycleResponse, Self::Error>> + Send {
        self(request.clone())
    }
}
pub struct Dispatch {
    request: LifecycleRequest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchUnknown;
impl Dispatch {
    #[must_use]
    pub const fn request(&self) -> &LifecycleRequest {
        &self.request
    }
    /// One detached attempt. Replay needs a fresh durable check of the SAME
    /// outbox. No host I/O executes under the center authority lock.
    /// # Errors
    /// Errors/wrong replies are unknown and cannot release reservations.
    pub async fn send(
        self,
        transport: &mut impl LifecycleTransport,
    ) -> Result<LifecycleResponse, DispatchUnknown> {
        let response = transport
            .send(&self.request)
            .await
            .map_err(|_| DispatchUnknown)?;
        response
            .validate_for(&self.request)
            .map_err(|_| DispatchUnknown)?;
        Ok(response)
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
        self.bounds().required_block_bytes()
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
    /// On unknown commit reconcile pending ORIGINAL request, never re-create.
    pub fn create(
        &mut self,
        tenant: &TenantId,
        binding: CreateBinding,
        now_ms: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        let (next, request) = self.state()?.create(tenant, binding, now_ms)?;
        self.commit_center(next)?;
        Ok(request)
    }
    /// # Errors
    /// Current owner, live lifetime and confirmed commit required.
    pub fn renew(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
        end_at_ms: u64,
        now_ms: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        let (next, request) = self.state()?.renew(tenant, sandbox, end_at_ms, now_ms)?;
        self.commit_center(next)?;
        Ok(request)
    }
    /// # Errors
    /// Fixed owner and durable monotone termination required.
    pub fn destroy(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<LifecycleRequest, AuthorityError> {
        let (next, request) = self.state()?.destroy(tenant, sandbox)?;
        self.commit_center(next)?;
        Ok(request)
    }
    /// # Errors
    /// Old/wrong observations cannot update a newer intent.
    pub fn apply_response(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        let next = self.state()?.apply_response(response)?;
        self.commit_center(next)
    }
    /// Called only after the host confirms the EXACT already committed result.
    /// # Errors
    /// Unknown compaction commits require recovery; stale acknowledgements fail.
    pub fn compact_confirmed(
        &mut self,
        response: &LifecycleResponse,
    ) -> Result<(), AuthorityError> {
        let next = self.state()?.compact_confirmed(response)?;
        self.commit_center(next)
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
    /// Durable current packages, sent after releasing the authority lock.
    /// Destroy can overtake a package; the host must fence delayed create.
    /// # Errors
    /// Any storage failure prevents new packages, even from a cached snapshot.
    pub fn dispatch_batch(&mut self) -> Result<Vec<Dispatch>, AuthorityError> {
        let observed = self
            .observed
            .as_ref()
            .ok_or(AuthorityError::RecoveryRequired)?;
        match self.ledger.reaffirm(observed) {
            Ok(committed) => {
                let batch = committed
                    .state()
                    .center()
                    .pending()
                    .into_iter()
                    .map(|request| Dispatch { request })
                    .collect();
                self.observed = Some(committed.into_observed());
                Ok(batch)
            }
            Err(error) => {
                self.observed = None;
                Err(error.into())
            }
        }
    }
}
