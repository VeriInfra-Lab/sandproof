//! Preserve shared journal failure classes at the autonomous host boundary.
use super::{ControllerApplication, ControllerHost, JournalError};
use sandproof_protocol::internal::NodeIdentity;

impl<H: ControllerHost> ControllerApplication<H> {
    /// Inspect the ORIGINAL authority under its application lock. This is not
    /// physical health or a check of every guest credential, and grants no
    /// permission. Unlike public error mapping it retains shared failure class.
    /// # Errors
    /// Poisoned lock, wrong registered identity or invalid original authority.
    pub fn check_internal_authority(&self, node: &NodeIdentity) -> Result<(), JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .internal_pending(node)
            .map(|_| ())
    }
}
