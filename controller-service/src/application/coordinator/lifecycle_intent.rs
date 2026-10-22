//! V3 enters the original application owner, journal and host workflow.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, Inner, JournalError, NeverCrash,
    map_admission, map_journal, recover_locked, release_reclaimed_slots,
};
use crate::{
    InternalTemplate, LifecycleWorkState, template_registry::CandidateVerification,
    templates::ComputeLimits,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation,
};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Read the original Create result and this call's current running facts.
    /// Keeps the original application lock, closes every SQL snapshot before
    /// bounded Host I/O, then rechecks all original objects and stored values.
    /// It never executes/reclaims work, changes timeouts or repairs credentials.
    /// # Errors
    /// Invalid full binding, corrupt authority or unavailable clock stays explicit.
    pub fn observe_lifecycle_create(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<crate::LifecycleCreateObservation, JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        let candidate = inner
            .journal
            .lifecycle_ready_candidate::<H::Evidence>(intent, context, now)?;
        if !candidate.can_probe() {
            return Ok(candidate.view);
        }
        let local = candidate.view.local.ok_or(JournalError::CorruptStore)?;
        let generation = candidate
            .view
            .native
            .as_ref()
            .ok_or(JournalError::CorruptStore)?
            .generation;
        let before =
            super::data_credentials::verify_target(&inner, local.sandbox, generation).is_ok();
        let probes = if before {
            candidate
                .actions
                .iter()
                .map(|action| inner.host.observe(action))
                .collect()
        } else {
            vec![]
        };
        let credentials = before
            && super::data_credentials::verify_target(&inner, local.sandbox, generation).is_ok();
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        inner
            .journal
            .finish_lifecycle_ready(&candidate, context, &probes, credentials, now)
    }

    /// Record through the original lock and writer. Reception grants no host
    /// permission; mapped Destroy also withdraws the original live data routes.
    /// # Errors
    /// Conflicts, corrupt authority and uncertain commits remain explicit.
    pub fn record_lifecycle_intent(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<LifecycleWorkState, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let result = inner
            .journal
            .record_lifecycle_intent(intent, context, &mut NeverCrash);
        if matches!(intent.request.operation, LifecycleOperation::Destroy {}) {
            match &result {
                Ok(state) => {
                    if let Some(local) = state.local {
                        inner.revoke_data(local.sandbox);
                    }
                }
                Err(JournalError::CommitUncertain(_)) => {
                    for signals in inner.data_revocations.values() {
                        for signal in signals {
                            signal.send_replace(true);
                        }
                    }
                }
                Err(_) => {}
            }
        }
        result
    }

    /// Read the exact saved intent and mapping, never synthesize Ready/Unloaded.
    /// # Errors
    /// Poisoned lock, missing/full-content mismatch and storage failure.
    pub fn lifecycle_work_state(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<LifecycleWorkState, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .lifecycle_work_state(intent, context)
    }

    /// Use this application's protected host provider, not caller supplied
    /// observations. The journal calls it without an open SQL transaction.
    /// # Errors
    /// An unsupported/mismatched host or unknown commit leaves startup closed.
    pub fn establish_lifecycle_host_epoch(
        &self,
    ) -> Result<crate::capture::HostEpochObservation, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let Inner { journal, host, .. } = &mut *inner;
        journal.establish_host_epoch(
            &mut |requirement| host.observe_host_epoch(requirement),
            &mut NeverCrash,
        )
    }

    /// Run only the original Create admission and startup/recovery workflow.
    /// The optional template is needed before first admission, not on replay.
    /// Returned mapping data is not a public Ready/Unloaded observation.
    /// # Errors
    /// Original template, host, quota, clock and commit failures are preserved.
    pub fn advance_lifecycle_create_with_verification(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
        verification: Option<&CandidateVerification>,
    ) -> Result<LifecycleWorkState, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let state = inner
            .journal
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)?;
        let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
            return Err(ApplicationError::Invalid);
        };
        if !state.current || state.terminal {
            return Ok(state);
        }
        if !inner.journal.startup_epoch_ready().map_err(map_journal)? {
            return Err(ApplicationError::Busy);
        }
        if state.local.is_none() {
            let admission = self
                .prepare_lifecycle_admission_locked(
                    &mut inner,
                    create,
                    &intent.request.binding.key.owner,
                    template.ok_or(ApplicationError::Busy)?,
                    compute,
                    verification,
                )?
                .ok_or(ApplicationError::QuotaExceeded)?;
            inner
                .journal
                .admit_lifecycle_create(intent, context, admission, &mut NeverCrash)
                .map_err(map_admission)?;
        }
        let local = inner
            .journal
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)?
            .local
            .ok_or(ApplicationError::Busy)?;
        inner
            .journal
            .abort_due_startups(self.clock.now_ms()?, &mut NeverCrash)
            .map_err(map_admission)?;
        recover_locked(&mut inner, Some(local.sandbox), self.clock.as_ref())?;
        release_reclaimed_slots(&mut inner)?;
        inner
            .journal
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)
    }
}
