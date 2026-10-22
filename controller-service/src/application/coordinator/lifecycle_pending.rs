//! Original durable V3 inputs, read under the application's existing owner lock.
use super::{ApplicationError, map_journal};
use super::{ControllerApplication, ControllerHost, JournalError};
use crate::{InternalTemplate, template_registry::CandidateVerification, templates::ComputeLimits};
use sandproof_protocol::{internal::NodeIdentity, lifecycle::AcceptedLifecycleIntent};

impl<H: ControllerHost> ControllerApplication<H> {
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
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .lifecycle_pending_creates(node)
    }
}
