//! Read-only original validation evidence under the same application lock.
use super::{ControllerApplication, ControllerHost};
use crate::{
    JournalError,
    template_registry::{Publication, ValidationEvidence},
};
use sandproof_protocol::{ResourceId, internal::ControlIdentity};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Does not receive an intent, refresh time or invoke the host adapter.
    /// # Errors
    /// Exact identities, original mapping and committed terminal evidence are required.
    pub fn internal_validation_evidence(
        &self,
        create: &ControlIdentity,
        terminal: &ControlIdentity,
        revision: &ResourceId,
        publication: &Publication,
    ) -> Result<ValidationEvidence, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .internal_validation_evidence(create, terminal, revision, publication)
    }
}
