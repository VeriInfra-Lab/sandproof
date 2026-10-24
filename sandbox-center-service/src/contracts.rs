//! Narrow business dependencies. No public request can supply a host endpoint,
//! certificate, local path, success flag or replacement storage authority.
use sandproof_api::{PortError, PublicationView, TemplatePage, TemplateView};
use sandproof_center_core::CenterState;
use sandproof_center_store::authority::AuthorityError;
use sandproof_controller_service::templates::{ResourceShape, TemplateBinding};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::numeric::seconds_to_milliseconds;
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
            || seconds_to_milliseconds(self.max_lifetime_secs).is_none()
            || seconds_to_milliseconds(self.startup_timeout_secs).is_none()
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
    /// This performs one bounded attempt; later reconciliation uses the same
    /// complete packet rather than creating another operation.
    fn drive_committed(
        &self,
        request: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    ) -> Result<(), PortError>;
    /// # Errors
    /// Commits/reuses the original Destroy through the original authority.
    fn destroy_exact(
        &self,
        original_create: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    ) -> Result<sandproof_protocol::lifecycle::AcceptedLifecycleIntent, PortError>;
}

/// One high-level network attempt; no automatic new operation/VM allocation.
/// Called from bounded blocking work, with NO center decision lock held.
pub trait HostPort: Send + Sync + 'static {
    /// # Errors
    /// Only the original journal may report durable custody and execution exit.
    fn observe_lifecycle_unload(
        &self,
        _intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::UnloadObservation, PortError> {
        Err(PortError::Busy)
    }

    /// # Errors
    /// A full exact persisted result is required; unsupported ports stay unknown.
    fn confirm_lifecycle_renewal(
        &self,
        _result: &sandproof_protocol::lifecycle::RenewalConfirmation,
        _deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedRenewal, PortError> {
        Err(PortError::OutcomeUnknown)
    }
    /// # Errors
    /// The original center must have durably saved the exact result first.
    fn confirm_lifecycle_terminal(
        &self,
        _result: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        _deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedLifecycleResult, PortError> {
        Err(PortError::OutcomeUnknown)
    }
    /// # Errors
    /// Unsupported/unknown cannot acknowledge a retired retained position.
    fn close_lifecycle_window(
        &self,
        _request: &sandproof_protocol::lifecycle::LifecycleWindowClose,
        _deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ClosedLifecycleWindow, PortError> {
        Err(PortError::OutcomeUnknown)
    }
    /// # Errors
    /// Full Destroy result only; unsupported adapters cannot assert cleanup.
    fn observe_lifecycle_terminal(
        &self,
        _intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::TerminalLifecycleObservation, PortError> {
        Err(PortError::Busy)
    }
    /// # Errors
    /// A missing/currently unavailable observation does not prove VM absence.
    fn observe_lifecycle_create(
        &self,
        _intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::CreateObservation, PortError> {
        Err(PortError::Busy)
    }
    /// # Errors
    /// One bounded recording call only. Default/unknown never acknowledges work.
    fn record_lifecycle(
        &self,
        _intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _deadline: Duration,
    ) -> Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, PortError> {
        Err(PortError::Busy)
    }
}

/// The production business authority owns the coupled lifecycle/metadata view.
/// Tests inject storage faults while using the same pure `CenterState`
/// decisions, not a second model.
pub trait AuthorityPort: Send + 'static {
    /// # Errors
    /// Save only the exact current completed Unload in the original authority.
    fn accept_lifecycle_unload_observation(
        &mut self,
        _observation: &sandproof_protocol::lifecycle::UnloadObservation,
        _now_ms: u64,
    ) -> Result<(), AuthorityError> {
        Err(sandproof_center_core::DecisionError::Invalid.into())
    }

    /// # Errors
    /// Only an authenticated exact refusal may record its original Destroy.
    fn accept_lifecycle_create_refusal(
        &mut self,
        _observation: &sandproof_protocol::lifecycle::CreateObservation,
    ) -> Result<sandproof_protocol::lifecycle::AcceptedLifecycleIntent, AuthorityError> {
        Err(sandproof_center_core::DecisionError::Invalid.into())
    }
    /// Protected workflow termination of its exact original accepted Create.
    /// Public owner/visibility checks remain at their original public entry.
    /// # Errors
    /// Missing, changed or unsupported originals cannot allocate termination.
    fn destroy_bound_create(
        &mut self,
        _original: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    ) -> Result<sandproof_protocol::lifecycle::AcceptedLifecycleIntent, AuthorityError> {
        Err(sandproof_center_core::DecisionError::Invalid.into())
    }
    /// # Errors
    /// Expiry sets current termination in the original CAS, including private rows.
    fn expire_current(&mut self, now_ms: u64) -> Result<(), AuthorityError>;
    /// One fresh, finite projection of the same authoritative managed inbox.
    /// # Errors
    /// Unknown storage cannot supply detached work from a cached candidate.
    fn lifecycle_dispatch_batch(
        &mut self,
    ) -> Result<Vec<sandproof_protocol::lifecycle::AcceptedLifecycleIntent>, AuthorityError> {
        Ok(self.snapshot()?.pending_lifecycle())
    }
    /// # Errors
    /// Unsupported adapters cannot acknowledge a fabricated Ready history.
    fn accept_lifecycle_create_observation(
        &mut self,
        _observation: &sandproof_protocol::lifecycle::CreateObservation,
    ) -> Result<(), AuthorityError> {
        Err(sandproof_center_core::DecisionError::Invalid.into())
    }
    /// # Errors
    /// Only the original authenticated Host acknowledgement can enter the
    /// same business CAS; an unsupported authority never confirms an echo.
    fn confirm_lifecycle_renewal(
        &mut self,
        _acknowledgement: &sandproof_protocol::lifecycle::ConfirmedRenewal,
    ) -> Result<(), AuthorityError> {
        Err(sandproof_center_core::DecisionError::Unavailable.into())
    }
    /// # Errors
    /// Save the original completed fact and fixed retention in the same authority.
    fn accept_lifecycle_terminal_observation(
        &mut self,
        result: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        now_ms: u64,
    ) -> Result<(), AuthorityError>;
    /// # Errors
    /// Must confirm an authority view within this call; cached-only reads fail.
    fn snapshot(&mut self) -> Result<CenterState, AuthorityError>;
    /// # Errors
    /// Public visibility, lifecycle facts and immutable query bindings MUST
    /// share one freshly confirmed envelope. No second template read/fallback.
    fn public_snapshot(&mut self) -> Result<crate::PublicSnapshot, AuthorityError>;
    /// SDK input normalization shares current admission and the original CAS.
    /// # Errors
    /// Unknown commit keeps the original accepted request, never a new key.
    fn create_current(
        &mut self,
        owner: &TenantId,
        parameters: &sandproof_protocol::lifecycle::CreateParameters,
        policy: crate::LifecyclePolicy,
        now_ms: u64,
    ) -> Result<sandproof_protocol::lifecycle::AcceptedLifecycleIntent, PortError>;
    /// # Errors
    /// Original SDK seconds and current version are normalized by the SAME CAS.
    fn renew_current(
        &mut self,
        owner: &TenantId,
        sandbox: &SandboxId,
        timeout_secs: u64,
        extend_only: bool,
        now_ms: u64,
    ) -> Result<sandproof_protocol::lifecycle::AcceptedLifecycleIntent, AuthorityError>;
    /// # Errors
    /// Timely termination neither waits for host work nor invents cleanup.
    fn destroy_current(
        &mut self,
        owner: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<sandproof_protocol::lifecycle::AcceptedLifecycleIntent, AuthorityError>;
    /// # Errors
    /// Requires the exact host-confirmed terminal result, not an absent record.
    fn confirm_terminal(
        &mut self,
        response: &sandproof_protocol::lifecycle::ConfirmedLifecycleResult,
    ) -> Result<(), AuthorityError>;
    /// # Errors
    /// Stops new creates only after the original authority commit succeeds.
    fn begin_drain(&mut self, node: &NodeIdentity) -> Result<(), AuthorityError>;
    /// # Errors
    /// Requires ONE exact terminal confirmation and its business pin veto.
    fn begin_close(
        &mut self,
        terminal: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        now_ms: u64,
    ) -> Result<sandproof_protocol::lifecycle::LifecycleWindowClose, AuthorityError>;
    /// # Errors
    /// Returns only committed close responsibilities from the original authority.
    fn close_batch(
        &mut self,
        now_ms: u64,
    ) -> Result<Vec<sandproof_protocol::lifecycle::LifecycleWindowClose>, AuthorityError>;
    /// # Errors
    /// Wrong acknowledgements or unresolved business pins retain responsibility.
    fn apply_close_ack(
        &mut self,
        ack: &sandproof_protocol::lifecycle::ClosedLifecycleWindow,
        now_ms: u64,
    ) -> Result<(), AuthorityError>;
    /// # Errors
    /// Unknown commits require explicit durable reconciliation.
    fn recover(&mut self) -> Result<(), AuthorityError>;
}
