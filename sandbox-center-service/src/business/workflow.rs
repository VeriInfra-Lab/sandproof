//! Finite publication progress derived from Checking records, never a new queue.
use super::{
    BusinessTemplates, CandidatePort, PreparationError, ProtectedCandidate, ValidationEvidencePort,
};
use crate::{CommittedDriver, TemplateProvider};
use sandproof_api::{PortError, PublicationView};
use sandproof_controller_service::{
    CreateNetworking, CreateOptions,
    template_registry::{PublicationFailure, PublicationState},
};
use sandproof_internal::template::ValidationRequest;
use sandproof_protocol::lifecycle::{CreateParameters, LifecycleOperation};

impl<C: CandidatePort, E: ValidationEvidencePort> BusinessTemplates<C, E> {
    pub(super) fn publish_with(
        &self,
        driver: &dyn CommittedDriver,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, PortError> {
        let owner = Self::owner(tenant)?;
        Self::candidate_id(candidate)?;
        driver.validation_config().validate()?;
        let initial = self.authority.snapshot()?;
        if initial.metadata.owned_record(owner, candidate).is_some() {
            return self.publication(tenant, candidate);
        }
        let tenant = crate::application::tenant_id(tenant)?;
        let accepted = self.candidates.verified(&tenant, candidate)?;
        if accepted.publication.candidate_id != candidate
            || accepted.publication.binding.owner != Some(owner)
        {
            return Err(PortError::NotFound);
        }
        ProtectedCandidate::new(accepted.publication.clone(), accepted.revision.clone())?;
        if initial.metadata.registry.config().validation_timeout_secs
            > driver.validation_config().max_timeout_secs
            || initial.metadata.registry.config().validation_timeout_secs
                > driver.validation_config().max_lifetime_secs
        {
            return Err(PortError::Invalid);
        }
        self.authority
            .begin_publication(accepted.publication, accepted.revision)?;
        // Once accepted, failures never claim that no publication side effect
        // occurred. Its original record remains discoverable by candidate ID.
        self.progress_one(driver, owner, candidate)
            .map_err(|_| PortError::OutcomeUnknown)?;
        self.publication(tenant.as_str(), candidate)
            .map_err(|_| PortError::OutcomeUnknown)
    }

    /// Call from the existing single bounded background worker. The finite
    /// registry is the only work source; host-local errors do not stop peers.
    /// # Errors
    /// Shared authority faults stop scanning and never authorize cached work.
    pub fn reconcile_once(&self, driver: &dyn CommittedDriver) -> Result<usize, PortError> {
        let pending: Vec<_> = self
            .authority
            .snapshot()?
            .metadata
            .registry
            .records()
            .iter()
            .filter(|record| record.value().state == PublicationState::Checking)
            .map(|record| {
                (
                    record.value().binding.owner,
                    record.value().candidate_id.clone(),
                )
            })
            .collect();
        let mut completed = 0;
        for (owner, candidate) in pending {
            let owner = owner.ok_or(PortError::OutcomeUnknown)?;
            match self.progress_one(driver, owner, &candidate) {
                Ok(true) => completed += 1,
                Ok(false) => {}
                Err(_) => {
                    self.authority.snapshot()?;
                }
            }
        }
        Ok(completed)
    }

    fn progress_one(
        &self,
        driver: &dyn CommittedDriver,
        owner: u64,
        candidate: &str,
    ) -> Result<bool, PortError> {
        let value = self.authority.snapshot()?;
        let record = value
            .metadata
            .owned_record(owner, candidate)
            .ok_or(PortError::NotFound)?;
        if record.value().state != PublicationState::Checking {
            return Ok(true);
        }
        let Some(link) = record.validation() else {
            let candidate =
                ProtectedCandidate::new(record.initial_publication(), record.revision().clone())?;
            match self
                .candidates
                .prepare(&candidate, driver.validation_config().reply_timeout)
            {
                Ok(()) => {}
                Err(PreparationError::Unknown) => return Ok(false),
                Err(PreparationError::Rejected) => {
                    self.authority.reject_unattached(
                        &candidate.publication.candidate_id,
                        PublicationFailure::ArtifactCheck,
                    )?;
                    return Ok(true);
                }
            }
            let binding = validation_binding(&value.metadata, record, driver)?;
            match self.authority.create_validation(
                &candidate.publication.candidate_id,
                &binding,
                driver.now_ms()?,
            ) {
                Ok(request) => {
                    let _ = driver.drive_committed(&request);
                }
                Err(PortError::QuotaExceeded | PortError::Invalid | PortError::Busy) => {
                    self.authority.reject_unattached(
                        &candidate.publication.candidate_id,
                        PublicationFailure::ValidationRejected,
                    )?;
                    return Ok(true);
                }
                Err(error) => return Err(error),
            }
            return self.progress_attached(driver, owner, &candidate.publication.candidate_id);
        };
        let current = value
            .center
            .sandboxes()
            .iter()
            .find(|sandbox| {
                sandbox
                    .operations
                    .first()
                    .is_some_and(|first| first.intent() == &link.create)
            })
            .and_then(|sandbox| sandbox.operations.last())
            .ok_or(PortError::OutcomeUnknown)?;
        let _ = driver.drive_committed(current.intent());
        self.progress_attached(driver, owner, candidate)
    }

    fn progress_attached(
        &self,
        driver: &dyn CommittedDriver,
        owner: u64,
        candidate: &str,
    ) -> Result<bool, PortError> {
        let value = self.authority.snapshot()?;
        let record = value
            .metadata
            .owned_record(owner, candidate)
            .ok_or(PortError::NotFound)?;
        if record.value().state != PublicationState::Checking {
            return Ok(true);
        }
        let link = record.validation().ok_or(PortError::OutcomeUnknown)?;
        let sandbox = value
            .center
            .sandboxes()
            .iter()
            .find(|sandbox| {
                sandbox
                    .operations
                    .first()
                    .is_some_and(|first| first.intent() == &link.create)
            })
            .ok_or(PortError::OutcomeUnknown)?;
        let original = sandbox.operations[0].intent();
        let current = sandbox.operations.last().ok_or(PortError::OutcomeUnknown)?;
        if matches!(
            current.intent().request.operation,
            LifecycleOperation::Create { .. }
        ) {
            // A transport timeout is not a failed VM. Only original Ready or
            // the original startup deadline authorizes ending this validation.
            let LifecycleOperation::Create { create, .. } = &original.request.operation else {
                return Err(PortError::OutcomeUnknown);
            };
            if current.running().is_none() && driver.now_ms()? < create.startup_deadline_ms {
                return Ok(false);
            }
            let destroy = driver.destroy_exact(&link.create)?;
            let _ = driver.drive_committed(&destroy);
            return self.finish_terminal(driver, candidate);
        }
        self.finish_terminal(driver, candidate)
    }

    fn finish_terminal(
        &self,
        driver: &dyn CommittedDriver,
        candidate: &str,
    ) -> Result<bool, PortError> {
        let value = self.authority.snapshot()?;
        let record = value
            .metadata
            .registry
            .record(candidate)
            .ok_or(PortError::OutcomeUnknown)?;
        let link = record.validation().ok_or(PortError::OutcomeUnknown)?;
        let sandbox = value
            .center
            .sandboxes()
            .iter()
            .find(|sandbox| {
                sandbox
                    .operations
                    .first()
                    .is_some_and(|first| first.intent() == &link.create)
            })
            .ok_or(PortError::OutcomeUnknown)?;
        let current = sandbox.operations.last().ok_or(PortError::OutcomeUnknown)?;
        let Some(cleaned) = current.terminal() else {
            return Ok(false);
        };
        let original = sandbox.operations[0].intent();
        if super::publication::terminal_request(original, cleaned).as_ref()
            != Some(current.intent())
        {
            return Ok(false);
        }
        // A saved capacity refusal and its exact host acknowledgement are
        // separate durable facts. Reply expiry/loss may leave only the first.
        // Keep Checking until reconciliation confirms this SAME result; do not
        // misclassify pending confirmation as an uncertain authority failure.
        if sandbox.terminal_confirmation.as_ref() != Some(cleaned) {
            return Ok(false);
        }
        let request = ValidationRequest {
            create: link.create.clone(),
            terminal: cleaned.clone(),
            revision: record.revision().clone(),
            publication: record.initial_publication(),
        };
        // Evidence network I/O is detached from the original decision lock.
        // The caller's original driver deadline is passed below, not reset by
        // a new lifecycle or reconstructed wall-clock start.
        let observed = self
            .evidence
            .evidence(&request, driver.validation_config().reply_timeout)?;
        if observed.cleaned != *cleaned {
            return Err(PortError::OutcomeUnknown);
        }
        self.authority.settle_evidence(&request, &observed)?;
        Ok(true)
    }
}

fn validation_binding(
    metadata: &super::RegistryMetadata,
    record: &super::GlobalPublication,
    driver: &dyn CommittedDriver,
) -> Result<CreateParameters, PortError> {
    let config = driver.validation_config();
    let public = crate::PublicTemplate::from_binding(
        record.value().binding.clone(),
        record.revision().clone(),
    )?;
    let options = CreateOptions {
        networking: Some(CreateNetworking {
            allow_internet_access: Some(false),
            network: None,
        }),
        ..CreateOptions::default()
    };
    crate::create_binding::create_parameters(
        &public,
        &options,
        metadata.registry.config().validation_timeout_secs,
        config.into(),
    )
}
