//! Internal intents enter the SAME application lock, host and recovery workflow.
//! Reception is separate from execution; no remote low-level worker commands.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, Inner, JournalError, NeverCrash,
    map_admission, map_journal, recover_locked, release_reclaimed_slots,
};
use crate::{CreateOptions, InternalAdmission, InternalTemplate, templates::ComputeLimits};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{Intent, LifecycleFact, LifecycleRequest, LifecycleResponse},
};

impl<H: ControllerHost> ControllerApplication<H> {
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

    /// One original create/destroy workflow step; the trusted cache descriptor
    /// is needed ONLY before local admission. Recovery uses persisted options.
    /// Caller cancellation cannot safely cancel this synchronous host work.
    /// # Errors
    /// Shared errors abort; unavailable/local failures retain identity and quota.
    /// Renewal assembly is not implemented and is explicitly refused here.
    pub fn advance_internal_lifecycle(
        &self,
        request: &LifecycleRequest,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
    ) -> Result<LifecycleResponse, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let state = inner
            .journal
            .internal_work_state(request)
            .map_err(map_journal)?;
        if !state.current
            || matches!(
                state.response.fact,
                LifecycleFact::Ready { .. } | LifecycleFact::Cleaned | LifecycleFact::NotAccepted
            )
        {
            return Ok(state.response);
        }
        if matches!(request.intent, Intent::Renew { .. }) {
            return Err(ApplicationError::Invalid);
        }
        if request.intent == Intent::Create && state.local.is_none() {
            self.admit_internal_locked(
                &mut inner,
                request,
                template.ok_or(ApplicationError::Busy)?,
                compute,
            )?;
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
    ) -> Result<(), ApplicationError> {
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
        inner.host.validate_template(&template.local)?;
        release_reclaimed_slots(inner)?;
        let required =
            self.physical_admission_bytes(inner, &options, template.local.resources.disk_bytes)?;
        if inner.host.available_disk_bytes()? < required {
            return Err(ApplicationError::QuotaExceeded);
        }
        inner
            .journal
            .admit_internal_create(
                request,
                InternalAdmission {
                    now_ms: self.clock.now_ms()?,
                    limits: self.config.limits,
                    compute,
                    template,
                    file_policy_digest: self.config.file_policy_digest,
                },
                &mut NeverCrash,
            )
            .map_err(map_admission)?;
        Ok(())
    }
}
