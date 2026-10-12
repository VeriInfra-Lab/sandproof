//! Read-only projection under the same application lock, with no host calls.
use super::{ControllerApplication, ControllerHost};
use crate::{InternalAudit, JournalError};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Return a checked, bounded original snapshot, not an execution permission
    /// or a guarantee that the last physical fact is still current.
    /// # Errors
    /// Shared corruption/lock failure refuses the complete audit.
    pub fn internal_audit(&self) -> Result<InternalAudit, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .internal_audit()
    }
}
