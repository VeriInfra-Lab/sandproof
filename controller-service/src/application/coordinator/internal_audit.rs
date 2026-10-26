//! Read-only projection under the same application lock, with no host calls.
use super::{ControllerApplication, ControllerHost};
use crate::{InternalAudit, JournalError};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Debug-only observation of this recorded formal Unload before data drain.
    /// The one application lock protects the current request/native/Full reads.
    /// This creates no capture, dispatch, exit or resource-release permission.
    /// # Errors
    /// Foreign, changed or unrecorded requests and storage/lock failures refuse.
    #[cfg(debug_assertions)]
    pub fn lifecycle_unload_is_pausing(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        node: &sandproof_protocol::internal::NodeIdentity,
    ) -> Result<bool, JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let context = inner.journal.recorded_lifecycle_context(intent, node)?;
        let state = inner.journal.lifecycle_work_state(intent, &context)?;
        if !state.current
            || state.terminal
            || !matches!(
                intent.request.operation,
                sandproof_protocol::lifecycle::LifecycleOperation::Unload { .. }
            )
        {
            return Ok(false);
        }
        let Some(local) = state.local else {
            return Ok(false);
        };
        let world = inner.journal.world()?;
        let Some(native) = world.sandbox_record(local.sandbox) else {
            return Ok(false);
        };
        if native.phase != sandbox_controller_core::Phase::Pausing
            || native.terminal
            || native.owner != local.owner
            || native.current_operation.map(|op| op.id) != state.local_operation
        {
            return Ok(false);
        }
        let Some(record) = inner.journal.lifecycle_capture_record(local.sandbox)? else {
            return Ok(false);
        };
        Ok(record.work.request() == intent
            && record.capture_operation() == state.local_operation
            && record.full_capture().is_some_and(|full| {
                record
                    .work
                    .pause_request()
                    .is_ok_and(|pause| full.pause == pause)
            }))
    }

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
