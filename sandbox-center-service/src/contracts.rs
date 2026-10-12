//! Narrow business dependencies. No public request can supply a host endpoint,
//! certificate, local path, success flag or replacement storage authority.
use sandproof_api::{PortError, PublicationView, TemplatePage, TemplateView};
use sandproof_center_core::CenterState;
use sandproof_center_store::authority::AuthorityError;
use sandproof_controller_service::templates::{ResourceShape, TemplateBinding};
use sandproof_protocol::internal::lifecycle::{CreateBinding, LifecycleRequest, LifecycleResponse};
use sandproof_protocol::internal::{
    NodeIdentity,
    window::{WindowCloseAck, WindowCloseRequest},
};
use sandproof_protocol::{ResourceId, SandboxId, TenantId};
use serde::Deserialize;
use std::time::Duration;

/// Protected engineering time policy, separate from lifecycle completion.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeObservationPolicy {
    pub health_window_ms: u64,
    pub max_clock_skew_ms: u64,
}
impl NodeObservationPolicy {
    /// # Errors
    /// Zero/unbounded freshness or a clock allowance longer than freshness reject.
    pub fn validate(&self, scan_interval: Duration) -> Result<(), PortError> {
        if self.health_window_ms == 0
            || self.health_window_ms > 86_400_000
            || self.max_clock_skew_ms > self.health_window_ms
            || u128::from(self.health_window_ms) <= scan_interval.as_millis()
        {
            return Err(PortError::Invalid);
        }
        Ok(())
    }
    /// A received timestamp must fall within the bounded local call interval,
    /// with the explicit deployment skew. This does not validate a VM result.
    #[must_use]
    pub fn healthy_until(&self, before_ms: u64, after_ms: u64, host_ms: u64) -> Option<u64> {
        if after_ms < before_ms
            || host_ms < before_ms.saturating_sub(self.max_clock_skew_ms)
            || host_ms > after_ms.checked_add(self.max_clock_skew_ms)?
        {
            return None;
        }
        // Expire relative to send time, not delayed receipt: stalled calls
        // cannot extend a stale physical observation's usable interval.
        let deadline = before_ms.checked_add(self.health_window_ms)?;
        (deadline > after_ms).then_some(deadline)
    }
}

#[derive(Clone, Debug)]
pub struct CenterConfig {
    pub domain: String,
    pub max_timeout_secs: u64,
    pub max_lifetime_secs: u64,
    pub startup_timeout_secs: u64,
    /// Covers reply polling, never cancels durable work on expiry.
    pub reply_timeout: Duration,
    pub poll_interval: Duration,
    pub public_ipv4_enabled: bool,
}
impl CenterConfig {
    /// # Errors
    /// Refuses unrepresentable lifetimes, ambiguous routing and zero budgets.
    pub fn validate(&self) -> Result<(), PortError> {
        if self.domain.is_empty()
            || self.domain.len() > 253
            || !self
                .domain
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            || self.max_timeout_secs < sandproof_controller_service::DEFAULT_TIMEOUT_SECS
            || self.max_timeout_secs > self.max_lifetime_secs
            || self.startup_timeout_secs == 0
            || self.max_lifetime_secs.checked_mul(1000).is_none()
            || self.startup_timeout_secs.checked_mul(1000).is_none()
            || self.reply_timeout.is_zero()
            || self.reply_timeout > Duration::from_hours(24)
            || self.poll_interval.is_zero()
            || self.poll_interval > self.reply_timeout
        {
            return Err(PortError::Invalid);
        }
        Ok(())
    }
}

/// Checked public projection of the ONE original immutable descriptor.
/// Its protected paths are never public input, response fields or filesystem
/// targets at the center. Artifact distribution remains a separate subsystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicTemplate {
    pub(crate) template_id: String,
    pub(crate) revision: ResourceId,
    pub(crate) digest: [u8; 32],
    pub(crate) resources: ResourceShape,
    pub(crate) envd_version: String,
    descriptor: TemplateBinding,
}
impl PublicTemplate {
    /// Construct from the original protected registry descriptor, not a
    /// provider's unrelated public fields plus a claimed digest.
    /// # Errors
    /// Rejects invalid or oversized descriptors before any admission.
    pub fn from_binding(
        descriptor: TemplateBinding,
        revision: ResourceId,
    ) -> Result<Self, PortError> {
        let digest = descriptor
            .canonical_digest()
            .map_err(|_| PortError::Invalid)?;
        Ok(Self {
            template_id: descriptor.template_id.clone(),
            envd_version: descriptor.envd_version.clone(),
            resources: descriptor.resources,
            revision,
            digest,
            descriptor,
        })
    }
    #[must_use]
    pub fn template_id(&self) -> &str {
        &self.template_id
    }
    #[must_use]
    pub const fn revision(&self) -> &ResourceId {
        &self.revision
    }
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }
    #[must_use]
    pub const fn resources(&self) -> ResourceShape {
        self.resources
    }
    #[must_use]
    pub fn envd_version(&self) -> &str {
        &self.envd_version
    }
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.descriptor.is_valid()
            && self.template_id == self.descriptor.template_id
            && self.envd_version == self.descriptor.envd_version
            && self.resources == self.descriptor.resources
            && self
                .descriptor
                .canonical_digest()
                .is_ok_and(|digest| digest == self.digest)
            && !self.template_id.is_empty()
            && self.template_id.len() <= 64
            && self
                .template_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            && self.resources.is_valid()
            && !self.envd_version.is_empty()
            && self.envd_version.len() <= 64
            && !self.envd_version.contains('\0')
    }
}

/// Production implementation is the protected center template subsystem.
/// Resolve authorizes an alias once; bound never follows an alias on replay.
pub trait TemplateProvider: Send + Sync + 'static {
    /// # Errors
    /// Missing/foreign/unready templates return `NotFound`, never base fallback.
    fn resolve(&self, tenant: &TenantId, name: &str) -> Result<PublicTemplate, PortError>;
    /// # Errors
    /// Missing or mutated immutable descriptions fail closed.
    fn bound(&self, revision: &ResourceId, digest: &[u8; 32]) -> Result<PublicTemplate, PortError>;
    /// # Errors
    /// The provider enforces the existing publication acceptance/cleanup contract.
    fn publish(
        &self,
        driver: &dyn CommittedDriver,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, PortError>;
    /// # Errors
    /// Missing/foreign publications are indistinguishable.
    fn publication(&self, tenant: &str, candidate: &str) -> Result<PublicationView, PortError>;
    /// # Errors
    /// Returns only the caller's Ready template.
    fn get(&self, tenant: &str, name: &str) -> Result<TemplateView, PortError>;
    /// # Errors
    /// Uses the original template list contract and bounded query.
    fn list(
        &self,
        tenant: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TemplatePage, PortError>;
    /// # Errors
    /// CAS affects only future resolves, never an accepted binding.
    fn set_alias(
        &self,
        tenant: &str,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<TemplateView, PortError>;
    /// # Errors
    /// Deletes only the caller's alias.
    fn delete_alias(&self, tenant: &str, name: &str) -> Result<(), PortError>;
}

/// A borrowed view of the ORIGINAL center lifecycle coordinator. Publication
/// never owns a second application, queue, decision mutex or backreference.
pub trait CommittedDriver: Send + Sync {
    /// # Errors
    /// Invalid/unavailable trusted service time cannot authorize new work.
    fn now_ms(&self) -> Result<u64, PortError>;
    fn validation_config(&self) -> &CenterConfig;
    /// # Errors
    /// Returns a fresh durable INTERNAL view, including validation instances.
    fn snapshot(&self) -> Result<CenterState, PortError>;
    /// # Errors
    /// Only an already committed exact request is driven. Unknown preserves it.
    /// Trusted assembly supplies the exact original request already accepted
    /// by this application's authority, not a caller-constructed lookalike.
    /// A retired terminal receipt rechecks its complete control identity; it
    /// does not retain/revalidate the canonical create body or reopen a window.
    fn drive_committed(&self, request: &LifecycleRequest) -> Result<LifecycleResponse, PortError>;
    /// # Errors
    /// Commits/reuses the original Destroy through the original authority.
    fn destroy_exact(
        &self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<LifecycleRequest, PortError>;
}

/// One high-level network attempt; no automatic new operation/VM allocation.
/// Called from bounded blocking work, with NO center decision lock held.
pub trait HostPort: Send + Sync + 'static {
    /// # Errors
    /// Every error is an unknown transport outcome, never admission rejection.
    fn send(
        &self,
        request: &LifecycleRequest,
        deadline: Duration,
    ) -> Result<LifecycleResponse, PortError>;
    /// # Errors
    /// Confirms only the EXACT already committed result; no implicit support.
    fn confirm(
        &self,
        result: &LifecycleResponse,
        deadline: Duration,
    ) -> Result<LifecycleResponse, PortError>;
    /// # Errors
    /// Only an exact authenticated durable close acknowledgement permits retirement.
    fn close(
        &self,
        request: &WindowCloseRequest,
        deadline: Duration,
    ) -> Result<WindowCloseAck, PortError>;
}

/// The production business authority owns the coupled lifecycle/metadata view.
/// Tests inject storage faults while using the same pure `CenterState`
/// decisions, not a second model.
pub trait AuthorityPort: Send + 'static {
    /// # Errors
    /// Must confirm an authority view within this call; cached-only reads fail.
    fn snapshot(&mut self) -> Result<CenterState, AuthorityError>;
    /// # Errors
    /// Public visibility, lifecycle facts and immutable query bindings MUST
    /// share one freshly confirmed envelope. No second template read/fallback.
    fn public_snapshot(&mut self) -> Result<crate::PublicSnapshot, AuthorityError>;
    /// # Errors
    /// Acceptance and the original outbox commit together.
    fn create(
        &mut self,
        tenant: &TenantId,
        binding: CreateBinding,
        now_ms: u64,
    ) -> Result<LifecycleRequest, AuthorityError>;
    /// # Errors
    /// The original lifetime and a durable intent are required.
    fn renew(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
        end_at_ms: u64,
        now_ms: u64,
    ) -> Result<LifecycleRequest, AuthorityError>;
    /// # Errors
    /// Termination is monotone, retains fixed binding and reservation.
    fn destroy(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<LifecycleRequest, AuthorityError>;
    /// # Errors
    /// Wrong/old observations cannot cover newer decisions.
    fn apply_response(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError>;
    /// # Errors
    /// Requires the exact current final result and actual durable host ack.
    fn compact_confirmed(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError>;
    /// # Errors
    /// Requires the exact host-confirmed terminal result, not an absent record.
    fn confirm_terminal(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError>;
    /// # Errors
    /// Stops new creates only after the original authority commit succeeds.
    fn begin_drain(&mut self, node: &NodeIdentity) -> Result<(), AuthorityError>;
    /// # Errors
    /// Requires ONE exact terminal confirmation and its business pin veto.
    fn begin_close(
        &mut self,
        terminal: &LifecycleResponse,
    ) -> Result<WindowCloseRequest, AuthorityError>;
    /// # Errors
    /// Returns only committed close responsibilities from the original authority.
    fn close_batch(&mut self) -> Result<Vec<WindowCloseRequest>, AuthorityError>;
    /// # Errors
    /// Wrong acknowledgements or unresolved business pins retain responsibility.
    fn apply_close_ack(&mut self, ack: &WindowCloseAck) -> Result<(), AuthorityError>;
    /// # Errors
    /// Unknown commits require explicit durable reconciliation.
    fn recover(&mut self) -> Result<(), AuthorityError>;
}
