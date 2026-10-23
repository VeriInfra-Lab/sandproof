//! Read-only original validation evidence under the same application lock.
use super::{ControllerApplication, ControllerHost};
use crate::{
    JournalError,
    template_registry::{Publication, ValidationEvidence},
};
use sandproof_protocol::{
    ResourceId,
    lifecycle::{
        AcceptedLifecycleIntent, AuthenticatedCenterContext, TerminalLifecycleObservation,
    },
};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Does not receive an intent, refresh time or invoke the host adapter.
    /// # Errors
    /// Exact identities, original mapping and committed terminal evidence are required.
    pub fn lifecycle_validation_evidence(
        &self,
        create: &AcceptedLifecycleIntent,
        terminal: &TerminalLifecycleObservation,
        revision: &ResourceId,
        publication: &Publication,
        context: &AuthenticatedCenterContext,
    ) -> Result<ValidationEvidence, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .lifecycle_validation_evidence(create, terminal, revision, publication, context)
    }
}
