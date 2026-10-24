//! Original durable V3 inputs, read under the application's existing owner lock.
use super::{ApplicationError, map_journal};
use super::{ControllerApplication, ControllerHost, JournalError};
use crate::{InternalTemplate, template_registry::CandidateVerification, templates::ComputeLimits};
use sandproof_protocol::{internal::NodeIdentity, lifecycle::AcceptedLifecycleIntent};

impl<H: ControllerHost> ControllerApplication<H> {
    /// # Errors
    /// Current protected node and original storage checks are required.
    pub fn lifecycle_pending_renewals(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .lifecycle_pending_renewals(node)
    }

    /// Resume only an original full renewal already saved by the receiver.
    /// # Errors
    /// No network authentication or new authorization is fabricated on replay.
    pub fn apply_recorded_lifecycle_renew(
        &self,
        intent: &AcceptedLifecycleIntent,
        node: &NodeIdentity,
    ) -> Result<(crate::LifecycleWorkState, i64), ApplicationError> {
        let context = self
            .inner
            .lock()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .recorded_lifecycle_context(intent, node)
            .map_err(map_journal)?;
        self.apply_lifecycle_renew(intent, &context)
    }

    /// Query a previously recorded full input with the protected local node.
    /// # Errors
    /// Foreign or unrecorded input is never manufactured as new authority.
    pub fn lifecycle_work_state_for_recorded(
        &self,
        intent: &AcceptedLifecycleIntent,
        node: &NodeIdentity,
    ) -> Result<crate::LifecycleWorkState, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let context = inner
            .journal
            .recorded_lifecycle_context(intent, node)
            .map_err(map_journal)?;
        inner
            .journal
            .lifecycle_work_state(intent, &context)
            .map_err(map_journal)
    }
    /// Advance a full input that was already accepted and saved by this inbox.
    /// Derives its binding from the original protected record, not fake new TLS.
    /// Releasing the read lock grants no permission; the original advance
    /// reacquires the owner lock and rechecks the complete current request.
    /// # Errors
    /// Wrong node/unrecorded input, Busy, material and admission errors persist.
    pub fn advance_recorded_lifecycle_create_with_verification(
        &self,
        intent: &AcceptedLifecycleIntent,
        node: &NodeIdentity,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
        verification: Option<&CandidateVerification>,
    ) -> Result<crate::LifecycleWorkState, ApplicationError> {
        let context = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .recorded_lifecycle_context(intent, node)
                .map_err(map_journal)?
        };
        self.advance_lifecycle_create_with_verification(
            intent,
            &context,
            template,
            compute,
            verification,
        )
    }

    /// At most the configured finite session count, from the original inbox.
    /// The worker uses its protected Node registration, existing gate, template
    /// source and compute limits; every advance must recheck the full request.
    /// Busy is not `NotAccepted`, and enumerating an input is not VM permission.
    /// # Errors
    /// Poisoned lock or invalid original authority refuses the complete scan.
    pub fn lifecycle_pending_creates(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let mut pending = inner.journal.lifecycle_pending_creates(node)?;
        pending.retain(|intent| {
            !inner
                .active_sessions
                .contains(intent.request.binding.session.as_protocol_id().as_str())
        });
        Ok(pending)
    }
}
