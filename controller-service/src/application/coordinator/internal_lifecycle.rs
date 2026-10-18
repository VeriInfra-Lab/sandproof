//! Internal intents enter the SAME application lock, host and recovery workflow.
//! Reception is separate from execution; no remote low-level worker commands.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, Inner, JournalError, NeverCrash,
    map_admission, map_journal, recover_locked, release_reclaimed_slots,
};
use crate::{
    CreateOptions, InternalAdmission, InternalTemplate, template_registry::CandidateVerification,
    templates::ComputeLimits,
};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{Intent, LifecycleFact, LifecycleRequest, LifecycleResponse},
    window::{WindowCloseAck, WindowCloseRequest},
};

/// A definitive original capacity decision, not an adapter error category.
enum InternalAdmissionDecision {
    Admitted,
    CapacityVeto,
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// Original exclusive writer/host lock, with no alternate cleanup algorithm.
    /// # Errors
    /// Unknown/active/unconfirmed/pinned responsibilities cannot close a window.
    pub fn close_internal_window(
        &self,
        request: &WindowCloseRequest,
    ) -> Result<WindowCloseAck, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .close_internal_window(request, &mut NeverCrash)
    }
    /// Exact result acknowledgement shares the original writer and lock.
    /// # Errors
    /// Result mismatches/storage errors never erase unknown responsibility.
    pub fn confirm_internal_result(
        &self,
        result: &LifecycleResponse,
    ) -> Result<LifecycleResponse, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .confirm_internal_result(result, &mut NeverCrash)
    }
    /// Authenticated internal reception only. A mapped destroy withdraws local
    /// data routes under the original lock, including an uncertain commit.
    /// # Errors
    /// Exact duplicates replay; conflicts/capacity/storage errors stay explicit.
    pub fn record_internal_intent(
        &self,
        request: &LifecycleRequest,
    ) -> Result<LifecycleResponse, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let result = inner
            .journal
            .record_internal_intent(request, &mut NeverCrash);
        if request.intent == Intent::Destroy {
            match &result {
                Ok(_) => {
                    if let Some(local) = inner.journal.internal_work_state(request)?.local {
                        inner.revoke_data(local.sandbox);
                    }
                }
                Err(JournalError::CommitUncertain(_)) => {
                    // The journal is fenced and its commit may be durable.
                    // Conservatively close current routes, never grant new ones.
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

    /// Bounded pending identities from the original configured inbox.
    /// # Errors
    /// Wrong node or invalid shared authority aborts the scan.
    pub fn internal_pending(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<LifecycleRequest>, ApplicationError> {
        self.inner
            .lock()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .internal_pending(node)
            .map_err(map_journal)
    }

    /// One original create/renew/destroy workflow step; the trusted cache descriptor
    /// is needed ONLY before local admission. Recovery uses persisted options.
    /// Caller cancellation cannot safely cancel this synchronous host work.
    /// # Errors
    /// Shared errors abort; unavailable/local failures retain identity and quota.
    pub fn advance_internal_lifecycle(
        &self,
        request: &LifecycleRequest,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
    ) -> Result<LifecycleResponse, ApplicationError> {
        self.advance_internal_lifecycle_with_verification(request, template, compute, None)
    }

    /// Same original workflow, optionally reusing a protected cache's completed
    /// artifact reads. The node checks the evidence against the local files;
    /// normal template validation and admission remain mandatory. This hint is
    /// process-local, never a remote authorization or a persisted Ready fact.
    /// # Errors
    /// Same authority, material, admission and recovery errors as normal advance.
    pub fn advance_internal_lifecycle_with_verification(
        &self,
        request: &LifecycleRequest,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
        verification: Option<&CandidateVerification>,
    ) -> Result<LifecycleResponse, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let state = inner
            .journal
            .internal_work_state(request)
            .map_err(map_journal)?;
        if !state.current
            || matches!(
                state.response.fact,
                LifecycleFact::Ready { .. }
                    | LifecycleFact::Renewed { .. }
                    | LifecycleFact::Cleaned
                    | LifecycleFact::NotAccepted
            )
        {
            return Ok(state.response);
        }
        if let Intent::Renew { end_at_ms } = request.intent {
            let now_ms = self.clock.now_ms()?;
            let result = inner
                .journal
                .apply_internal_renew(request, now_ms, &mut NeverCrash);
            if end_at_ms <= u64::try_from(now_ms).map_err(|_| ApplicationError::Invalid)?
                && (result.is_ok()
                    || matches!(
                        result,
                        Err(crate::AdmissionError::Journal(
                            JournalError::CommitUncertain(_)
                        ))
                    ))
                && let Some(local) = state.local
            {
                // As on the original set_timeout path, immediate expiry closes
                // current data before the independent worker stops the VM.
                inner.revoke_data(local.sandbox);
            }
            return result.map_err(map_admission);
        }
        if request.intent == Intent::Create && state.local.is_none() {
            let decision = self.admit_internal_locked(
                &mut inner,
                request,
                template.ok_or(ApplicationError::Busy)?,
                compute,
                verification,
            )?;
            if matches!(decision, InternalAdmissionDecision::CapacityVeto) {
                // Only the original definitive pre-admission capacity veto is
                // final. Missing material, host I/O and uncertain commits still
                // retain Recorded/unknown responsibility and never use this path.
                return inner
                    .journal
                    .reject_internal_capacity(request, &mut NeverCrash)
                    .map_err(map_journal);
            }
        }
        let local = inner
            .journal
            .internal_work_state(request)
            .map_err(map_journal)?
            .local;
        // Reuse the original independent startup expiry BEFORE any resumption.
        inner
            .journal
            .abort_due_startups(self.clock.now_ms()?, &mut NeverCrash)
            .map_err(map_admission)?;
        if let Some(local) = local {
            if request.intent == Intent::Destroy {
                inner.revoke_data(local.sandbox);
            }
            recover_locked(&mut inner, Some(local.sandbox), self.clock.as_ref())?;
            release_reclaimed_slots(&mut inner)?;
        }
        inner
            .journal
            .capture_internal_progress(request, &mut NeverCrash)
            .map_err(map_journal)
    }

    fn admit_internal_locked(
        &self,
        inner: &mut Inner<H>,
        request: &LifecycleRequest,
        template: InternalTemplate,
        compute: ComputeLimits,
        verification: Option<&CandidateVerification>,
    ) -> Result<InternalAdmissionDecision, ApplicationError> {
        let mut options: CreateOptions = serde_json::from_slice(&request.create.options)
            .map_err(|_| ApplicationError::Invalid)?;
        // The original journal admission validates the full canonical request,
        // revision/digest, owner and shape before committing local responsibility.
        options.template = Some(template.local.clone());
        let network = options.networking.as_ref();
        let policy = super::super::selected_policy(&super::super::CreateCommand {
            template_id: String::new(),
            timeout: None,
            metadata: None,
            env_vars: None,
            allow_internet_access: network.and_then(|v| v.allow_internet_access),
            network: network.and_then(|v| v.network.clone()),
        })?;
        if policy != sandbox_controller_core::NetworkPolicy::DenyAll {
            if !self.config.public_ipv4_enabled {
                return Err(ApplicationError::Invalid);
            }
            if !inner.host.public_ipv4_ready()? {
                return Err(ApplicationError::Busy);
            }
        }
        if let Some(verification) = verification {
            inner
                .host
                .cache_candidate_verification(&template.local, verification);
        }
        inner.host.validate_template(&template.local)?;
        release_reclaimed_slots(inner)?;
        let required = self.internal_physical_admission_bytes(
            inner,
            &options,
            template.local.resources.disk_bytes,
        )?;
        let mut admission = InternalAdmission {
            now_ms: self.clock.now_ms()?,
            limits: self.config.limits,
            compute,
            template,
            file_policy_digest: self.config.file_policy_digest,
        };
        // A physical veto is definitive only for this valid original immutable
        // attachment. Bad material cannot turn into a successful quota release
        // merely because the same host currently has little free space.
        admission
            .validate_attachment(request)
            .map_err(map_admission)?;
        if inner.host.available_disk_bytes()? < required {
            return Ok(InternalAdmissionDecision::CapacityVeto);
        }
        // Preserve the original journal's fresh clock/rejection checks after
        // the potentially slow host observation, rather than reusing its time.
        admission.now_ms = self.clock.now_ms()?;
        let decision = inner
            .journal
            .admit_internal_create(request, admission, &mut NeverCrash);
        match decision {
            Ok(_) => Ok(InternalAdmissionDecision::Admitted),
            Err(crate::AdmissionError::QuotaExceeded) => {
                Ok(InternalAdmissionDecision::CapacityVeto)
            }
            Err(error) => Err(map_admission(error)),
        }
    }
}
