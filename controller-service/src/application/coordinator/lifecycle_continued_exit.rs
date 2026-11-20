//! Current failed-Unload qualification is rechecked around readonly Host I/O.
use super::{ControllerApplication, ControllerHost, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CurrentResumeReadiness, ResumeObservation,
};
use std::sync::Arc;

impl<H: ControllerHost> ControllerApplication<H> {
    pub(super) fn observe_continued_state_lost(
        &self,
        original: &AcceptedLifecycleIntent,
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
            let candidates = inner
                .journal
                .lifecycle_continued_exit_candidates::<H::Evidence>(original, context, now)?;
            if candidates.0.is_none() && candidates.1.is_none() {
                return Ok(None);
            }
            (candidates, Arc::clone(&inner.host))
        };
        let current_request = match (&before.0, &before.1) {
            (Some(candidate), None) => {
                let Some(observed) =
                    host.observe_lifecycle_restore_exit(&candidate.action, &candidate.work)
                else {
                    return Ok(None);
                };
                if !candidate.matches(&observed) {
                    return Ok(None);
                }
                candidate.current_request().clone()
            }
            (None, Some(candidate)) => {
                let Some(observed) = host
                    .observe_lifecycle_original_continue_exit(&candidate.action, &candidate.work)
                else {
                    return Ok(None);
                };
                if !candidate.matches(&observed) {
                    return Ok(None);
                }
                candidate.current_request().clone()
            }
            _ => return Err(JournalError::CorruptStore),
        };
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
            .lifecycle_continued_exit_candidates::<H::Evidence>(original, context, now)?;
        if after != before {
            return Ok(None);
        }
        Ok(Some(ResumeObservation {
            request: original.clone(),
            current_request,
            readiness: CurrentResumeReadiness::StateLost,
        }))
    }
}
