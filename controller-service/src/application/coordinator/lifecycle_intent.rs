//! V3 enters the original application owner, journal and host workflow.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, Inner, JournalError, NeverCrash,
    map_admission, map_journal,
};
use crate::{
    InternalTemplate, LifecycleWorkState, template_registry::CandidateVerification,
    templates::ComputeLimits,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation,
};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Apply or replay the exact recorded lifetime update under original SQL
    /// authority. The saved result is historical, never a current Ready claim.
    /// # Errors
    /// Original owner, clock, lifetime, full-key and current-order checks apply.
    pub fn apply_lifecycle_renew(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<(LifecycleWorkState, i64), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        inner
            .journal
            .apply_lifecycle_renew(intent, context, self.clock.now_ms()?, &mut NeverCrash)
            .map_err(map_admission)
    }

    /// Exact current Destroy query. No execution or release is performed here.
    /// # Errors
    /// Refuses stale, foreign or unrecorded requests and uncertain authority.
    pub fn lifecycle_destroy_cleaned(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<bool, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .lifecycle_destroy_cleaned(intent, context)
    }

    /// Read the original Create result and this call's current running facts.
    /// Closes the SQL snapshot and releases the application lock before bounded
    /// Host I/O, then rechecks all original objects and stored values.
    /// It never executes/reclaims work, changes timeouts or repairs credentials.
    /// # Errors
    /// Invalid full binding, corrupt authority or unavailable clock stays explicit.
    pub fn observe_lifecycle_create(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<crate::LifecycleCreateObservation, JournalError> {
        let (candidate, host, before) = {
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
            (candidate, std::sync::Arc::clone(&inner.host), before)
        };
        let probes = if before {
            candidate
                .actions
                .iter()
                .map(|action| host.observe(action))
                .collect()
        } else {
            vec![]
        };
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let local = candidate.view.local.ok_or(JournalError::CorruptStore)?;
        let generation = candidate
            .view
            .native
            .as_ref()
            .ok_or(JournalError::CorruptStore)?
            .generation;
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
        if matches!(
            intent.request.operation,
            LifecycleOperation::Destroy {} | LifecycleOperation::Unload { .. }
        ) {
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
        let state = self
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)?;
        let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
            return Err(ApplicationError::Invalid);
        };
        if !state.current || state.terminal {
            return Ok(state);
        }
        let Some(_session) = self.acquire_lifecycle_session(intent, context)? else {
            return self
                .lifecycle_work_state(intent, context)
                .map_err(map_journal);
        };
        if state.local.is_none() {
            let admission = self.prepare_current_admission(
                create,
                &intent.request.binding.key.owner,
                template.ok_or(ApplicationError::Busy)?,
                compute,
                verification,
            )?;
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            // Full current intent and all native reservations are checked again
            // after template/file I/O; no prior World is written back.
            let current = inner
                .journal
                .lifecycle_work_state(intent, context)
                .map_err(map_journal)?;
            if !current.current || current.terminal {
                return Ok(current);
            }
            if current.local.is_none() {
                inner
                    .journal
                    .admit_lifecycle_create(intent, context, admission, &mut NeverCrash)
                    .map_err(map_admission)?;
            }
        }
        let state = self
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)?;
        if let Some(local) = state.local {
            self.advance_current_workflow(local.sandbox)?;
        }
        self.lifecycle_work_state(intent, context)
            .map_err(map_journal)
    }

    fn prepare_current_admission(
        &self,
        binding: &sandproof_protocol::internal::lifecycle::CreateBinding,
        owner: &sandproof_protocol::TenantId,
        template: InternalTemplate,
        compute: ComputeLimits,
        verification: Option<&CandidateVerification>,
    ) -> Result<crate::InternalAdmission, ApplicationError> {
        let host = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            if !inner.journal.startup_epoch_ready().map_err(map_journal)? {
                return Err(ApplicationError::Busy);
            }
            std::sync::Arc::clone(&inner.host)
        };
        let mut options: crate::CreateOptions =
            serde_json::from_slice(&binding.options).map_err(|_| ApplicationError::Invalid)?;
        options.template = Some(template.local.clone());
        let network = options.networking.as_ref();
        let policy = super::super::selected_policy(&super::super::CreateCommand {
            template_id: String::new(),
            timeout: None,
            metadata: None,
            env_vars: None,
            allow_internet_access: network.and_then(|value| value.allow_internet_access),
            network: network.and_then(|value| value.network.clone()),
        })?;
        if policy != sandbox_controller_core::NetworkPolicy::DenyAll {
            if !self.config.public_ipv4_enabled {
                return Err(ApplicationError::Invalid);
            }
            if !host.public_ipv4_ready()? {
                return Err(ApplicationError::Busy);
            }
        }
        if let Some(verification) = verification {
            host.cache_candidate_verification(&template.local, verification);
        }
        host.validate_template(&template.local)?;
        let mut admission = crate::InternalAdmission {
            now_ms: self.clock.now_ms()?,
            limits: self.config.limits,
            compute,
            template,
            file_policy_digest: self.config.file_policy_digest,
        };
        admission
            .validate_create_binding(binding, owner)
            .map_err(map_admission)?;
        let required = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            self.internal_physical_admission_bytes(
                &inner,
                &options,
                admission.template.local.resources.disk_bytes,
            )?
        };
        if host.available_disk_bytes()? < required {
            return Err(ApplicationError::QuotaExceeded);
        }
        admission.now_ms = self.clock.now_ms()?;
        Ok(admission)
    }
}
