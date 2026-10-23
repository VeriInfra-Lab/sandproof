//! Bounded, authenticated engineering evidence, never a public success flag.
use crate::ReceiveError;
pub use sandproof_controller_service::template_registry::{
    LocalValidationBinding, MAX_VALIDATION_CERTIFICATE_BYTES, ValidationEvidence, ValidationOutcome,
};
use sandproof_controller_service::{
    JournalError,
    template_registry::{Publication, evidence, state},
};
use sandproof_protocol::{
    ResourceId,
    internal::{
        NodeIdentity,
        lifecycle::{LIFECYCLE_BODY_BYTES, RESULT_CONFIRMATION_BODY_BYTES},
    },
};
use serde::{Deserialize, Serialize};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext,
    ExpectedSession, LifecycleOperation, TerminalLifecycleObservation, TerminalStatus};

pub const VALIDATION_PATH: &str = "/internal/template-validation-evidence";
pub const VALIDATION_REQUEST_BODY_BYTES: usize = LIFECYCLE_BODY_BYTES;
/// The same cap bounds host encoding and client collection, not just chunks.
pub const VALIDATION_RESPONSE_BODY_BYTES: usize = MAX_VALIDATION_CERTIFICATE_BYTES
    + sandproof_controller_service::CreateOptions::MAX_ENCODED_BYTES
    + RESULT_CONFIRMATION_BODY_BYTES
    + 8192;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRequest {
    pub create: AcceptedLifecycleIntent,
    pub terminal: TerminalLifecycleObservation,
    pub revision: ResourceId,
    /// Initial center-accepted build metadata. No caller local IDs, checked
    /// flag or certificate may be supplied, and its paths are never executed.
    pub publication: Publication,
}
impl ValidationRequest {
    /// # Errors
    /// Refuses foreign pairs and pre-declared validation outcomes.
    pub fn validate(&self, node: &NodeIdentity) -> Result<(), JournalError> {
        let create = &self.create.request.binding;
        let terminal = &self.terminal.request.request.binding;
        if self.create.request.validate(node).is_err()
            || self.terminal.request.request.validate(node).is_err()
            || self.create.center.deployment != node.deployment_id
            || self.create.center != self.terminal.request.center
            || !self.create.retention.valid_for(4096)
            || self.create.retention != self.terminal.request.retention
            || create.node != terminal.node || create.key.owner != terminal.key.owner
            || create.session != terminal.session || create.key.deployment != terminal.key.deployment
            || create.key == terminal.key || create.expected != ExpectedSession::Absent
            || self.create.accepted_version != 1
            || self.create.accepted_version_for_request() != Some(1)
            || !matches!(self.create.request.operation, LifecycleOperation::Create { .. })
            || self.terminal.request.accepted_version <= self.create.accepted_version
            || self.terminal.status != TerminalStatus::Cleaned
            || !self.terminal.matches_request(&self.terminal.request)
            || self.publication.binding.owner.map(|owner| owner.to_string()).as_deref()
                != Some(create.key.owner.as_str())
        {
            return Err(JournalError::Conflict);
        }
        state::validate_begin(&self.publication)?;
        self.publication
            .binding
            .canonical_digest()
            .map_err(|_| JournalError::Conflict)?;
        Ok(())
    }
    /// Full sender/context binding, separate from shape validation used by the client.
    /// # Errors
    /// The original verified TLS peer and protected registration must agree.
    pub fn validate_context(&self, context: &AuthenticatedCenterContext) -> Result<(), JournalError> {
        self.create.validate(context).map_err(|_| JournalError::Conflict)?;
        self.terminal.request.validate(context).map_err(|_| JournalError::Conflict)?;
        self.validate(context.expected_node())
    }
    /// # Errors
    /// Keeps the request buffer within the same server receive limit.
    pub fn encoded(&self) -> Result<Vec<u8>, JournalError> {
        self.validate(&self.create.request.binding.node)?;
        evidence::encode_bounded(self, VALIDATION_REQUEST_BODY_BYTES)
    }
}

/// Mandatory original-journal evidence port. A production receiver cannot
/// inherit an optimistic or unsupported default implementation.
pub trait ValidationReceiver: Send + Sync + 'static {
    fn validation_evidence(
        &self,
        request: ValidationRequest,
        context: AuthenticatedCenterContext,
    ) -> impl std::future::Future<Output = Result<ValidationEvidence, ReceiveError>> + Send;
}
