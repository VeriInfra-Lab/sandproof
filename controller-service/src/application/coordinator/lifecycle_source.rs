//! Existing application owner; every provider call is outside SQL.
use super::{ControllerApplication, ControllerHost, Inner, JournalError, NeverCrash};
use crate::{SourceWork, SourceWorkRecord};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Persist a bounded readonly question on the original accepted Unload.
    /// # Errors
    /// Original binding/source/budget/bootstrap/clock and commit errors persist.
    pub fn prepare_lifecycle_source_work(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<SourceWorkRecord, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        inner.journal.prepare_lifecycle_source_work::<H::Evidence>(
            intent,
            context,
            now,
            &mut NeverCrash,
        )
    }

    /// Recheck a persisted question, collect raw facts without SQL, then save
    /// only if the same current original source still qualifies. A saved result
    /// is replayed as historical bytes, not freshly sampled or a capture permit.
    /// # Errors
    /// Provider/clock/identity/size/late-result and unknown commits stay explicit.
    pub fn observe_lifecycle_source_work(
        &self,
        work: &SourceWork,
        context: &AuthenticatedCenterContext,
    ) -> Result<SourceWorkRecord, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        let record = inner
            .journal
            .source_work_for_observation::<H::Evidence>(work, context, now)?;
        if record.observation.is_some() {
            return Ok(record);
        }
        let observation = inner
            .host
            .observe_lifecycle_source(work.requirement())
            .map_err(|_| JournalError::UnresolvedAction)?;
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        let Inner { journal, .. } = &mut *inner;
        journal.save_source_observation::<H::Evidence>(
            work,
            context,
            observation,
            now,
            &mut NeverCrash,
        )
    }
}
