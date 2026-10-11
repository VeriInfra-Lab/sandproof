//! Narrow business dependencies. No public request can supply a host endpoint,
//! certificate, local path, success flag or replacement storage authority.
use sandproof_api::{PortError, PublicationView, TemplatePage, TemplateView};
use sandproof_center_core::CenterState;
use sandproof_center_store::authority::{AuthorityError, CenterAuthority};
use sandproof_controller_service::templates::{ResourceShape, TemplateBinding};
use sandproof_protocol::internal::lifecycle::{CreateBinding, LifecycleRequest, LifecycleResponse};
use sandproof_protocol::{ResourceId, SandboxId, TenantId};
use std::time::Duration;

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
        if !descriptor.is_valid() {
            return Err(PortError::Invalid);
        }
        let bytes = serde_json::to_vec(&descriptor).map_err(|_| PortError::Invalid)?;
        if bytes.len() > sandproof_controller_service::CreateOptions::MAX_ENCODED_BYTES {
            return Err(PortError::Invalid);
        }
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes)
            .as_ref()
            .try_into()
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
            && serde_json::to_vec(&self.descriptor).is_ok_and(|bytes| {
                bytes.len() <= sandproof_controller_service::CreateOptions::MAX_ENCODED_BYTES
                    && ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref() == self.digest
            })
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
    fn publish(&self, tenant: &str, candidate: &str) -> Result<PublicationView, PortError>;
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
}

/// Only `CenterAuthority` implements this in production. Tests inject storage
/// faults while using the same pure `CenterState` decisions, not a second model.
pub trait AuthorityPort: Send + 'static {
    /// # Errors
    /// Must confirm an authority view within this call; cached-only reads fail.
    fn snapshot(&mut self) -> Result<CenterState, AuthorityError>;
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
    /// Unknown commits require explicit durable reconciliation.
    fn recover(&mut self) -> Result<(), AuthorityError>;
}
impl AuthorityPort for CenterAuthority {
    fn snapshot(&mut self) -> Result<CenterState, AuthorityError> {
        Self::snapshot(self)
    }
    fn create(
        &mut self,
        tenant: &TenantId,
        binding: CreateBinding,
        now_ms: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        Self::create(self, tenant, binding, now_ms)
    }
    fn renew(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
        end_at_ms: u64,
        now_ms: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        Self::renew(self, tenant, sandbox, end_at_ms, now_ms)
    }
    fn destroy(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<LifecycleRequest, AuthorityError> {
        Self::destroy(self, tenant, sandbox)
    }
    fn apply_response(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        Self::apply_response(self, response)
    }
    fn compact_confirmed(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        Self::compact_confirmed(self, response)
    }
    fn recover(&mut self) -> Result<(), AuthorityError> {
        Self::recover(self)
    }
}
