//! Readonly loss observation: original SQL authority, lock-free host, recheck.
use super::{ControllerApplication, ControllerHost, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CurrentResumeReadiness, ResumeObservation,
};
use std::sync::Arc;

impl<H: ControllerHost> ControllerApplication<H> {
    pub(super) fn observe_restore_state_lost(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<Option<ResumeObservation>, JournalError> {
        let (before, host) = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| JournalError::ControllerBusy)?;
            let now = self
                .clock
                .now_ms()
                .map_err(|_| JournalError::UnresolvedAction)?;
            let Some(candidate) = inner
                .journal
                .lifecycle_restore_exit_candidate(intent, context, now)?
            else {
                return Ok(None);
            };
            (candidate, Arc::clone(&inner.host))
        };
        let Some(observed) = host.observe_lifecycle_restore_exit(&before.action, &before.work)
        else {
            return Ok(None);
        };
        if !before.matches(&observed) {
            return Ok(None);
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        let after = inner
            .journal
            .lifecycle_restore_exit_candidate(intent, context, now)?;
        if after.as_ref() != Some(&before) {
            return Ok(None);
        }
        Ok(Some(ResumeObservation {
            request: intent.clone(),
            current_request: before.current_request,
            readiness: CurrentResumeReadiness::StateLost,
        }))
    }
}
