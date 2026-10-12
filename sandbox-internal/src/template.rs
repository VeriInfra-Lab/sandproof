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
        ControlIdentity, NodeIdentity,
        lifecycle::{LIFECYCLE_BODY_BYTES, RESULT_CONFIRMATION_BODY_BYTES},
    },
};
use serde::{Deserialize, Serialize};

pub const VALIDATION_PATH: &str = "/internal/v2/template-validation-evidence";
pub const VALIDATION_REQUEST_BODY_BYTES: usize = LIFECYCLE_BODY_BYTES;
/// The same cap bounds host encoding and client collection, not just chunks.
pub const VALIDATION_RESPONSE_BODY_BYTES: usize = MAX_VALIDATION_CERTIFICATE_BYTES
    + sandproof_controller_service::CreateOptions::MAX_ENCODED_BYTES
    + RESULT_CONFIRMATION_BODY_BYTES
    + 8192;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRequest {
    pub create: ControlIdentity,
    pub terminal: ControlIdentity,
    pub revision: ResourceId,
    /// Initial center-accepted build metadata. No caller local IDs, checked
    /// flag or certificate may be supplied, and its paths are never executed.
    pub publication: Publication,
}
impl ValidationRequest {
    /// # Errors
    /// Refuses foreign pairs and pre-declared validation outcomes.
    pub fn validate(&self, node: &NodeIdentity) -> Result<(), JournalError> {
        self.create
            .validate_node(node)
            .map_err(|_| JournalError::Conflict)?;
        self.terminal
            .validate_node(node)
            .map_err(|_| JournalError::Conflict)?;
        if self.create.node != self.terminal.node
            || self.create.tenant_id != self.terminal.tenant_id
            || self.create.sandbox_id != self.terminal.sandbox_id
            || self.create.request_window != self.terminal.request_window
            || self.create.retention_slot != self.terminal.retention_slot
            || self.create.intent_version.get() != 1
            || (self.create != self.terminal
                && (self.terminal.intent_version.get() <= self.create.intent_version.get()
                    || self.terminal.operation_id == self.create.operation_id))
            || self
                .publication
                .binding
                .owner
                .map(|owner| owner.to_string())
                .as_deref()
                != Some(self.create.tenant_id.as_str())
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
    /// # Errors
    /// Keeps the request buffer within the same server receive limit.
    pub fn encoded(&self) -> Result<Vec<u8>, JournalError> {
        self.validate(&self.create.node)?;
        evidence::encode_bounded(self, VALIDATION_REQUEST_BODY_BYTES)
    }
}

/// Mandatory original-journal evidence port. A production receiver cannot
/// inherit an optimistic or unsupported default implementation.
pub trait ValidationReceiver: Send + Sync + 'static {
    fn validation_evidence(
        &self,
        request: ValidationRequest,
    ) -> impl std::future::Future<Output = Result<ValidationEvidence, ReceiveError>> + Send;
}
