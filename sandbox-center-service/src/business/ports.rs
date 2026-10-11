//! Required protected artifact/evidence dependencies; no optimistic defaults.
use crate::PublicTemplate;
use sandproof_api::PortError;
use sandproof_controller_service::template_registry::{Publication, ValidationEvidence, state};
use sandproof_internal::template::ValidationRequest;
use sandproof_protocol::{ResourceId, TenantId};
use std::time::Duration;

/// Only the protected original candidate verifier supplies this description.
/// Public HTTP callers still provide the candidate ID, never these fields.
#[derive(Clone, Debug)]
pub struct ProtectedCandidate {
    pub publication: Publication,
    pub revision: ResourceId,
}
impl ProtectedCandidate {
    /// # Errors
    /// A candidate may not declare checked/Ready or exceed the original bound.
    pub fn new(publication: Publication, revision: ResourceId) -> Result<Self, PortError> {
        state::validate_begin(&publication).map_err(|_| PortError::Invalid)?;
        PublicTemplate::from_binding(publication.binding.clone(), revision.clone())?;
        Ok(Self {
            publication,
            revision,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationError {
    /// Definitive artifact rejection BEFORE any validation lifecycle exists.
    Rejected,
    /// Transport/I/O uncertainty is not a no-side-effect rejection.
    Unknown,
}

pub trait CandidatePort: Send + Sync + 'static {
    /// # Errors
    /// The original protected verifier must check owner, bytes and free space.
    fn verified(&self, tenant: &TenantId, candidate: &str)
    -> Result<ProtectedCandidate, PortError>;
    /// Seal/cache using original artifact rules, outside the decision lock.
    /// Replays use the SAME accepted description. Success requires sealed
    /// artifacts and exact authenticated cache evidence durably registered in
    /// the original authority; successful I/O alone is not an availability fact.
    /// # Errors
    /// Only explicit `Rejected` may fail an unattached publication.
    fn prepare(
        &self,
        candidate: &ProtectedCandidate,
        deadline: Duration,
    ) -> Result<(), PreparationError>;
}

pub trait ValidationEvidencePort: Send + Sync + 'static {
    /// # Errors
    /// Return only authenticated original-journal evidence for the exact request.
    fn evidence(
        &self,
        request: &ValidationRequest,
        deadline: Duration,
    ) -> Result<ValidationEvidence, PortError>;
}
