//! Exact global-to-local certificate projection; no storage or I/O.
use crate::PublicTemplate;
use sandproof_controller_service::template_registry::{
    Publication, evidence, state::PublicationRecord,
};
use sandproof_protocol::{
    ResourceId,
    lifecycle::{AcceptedLifecycleIntent, ExpectedSession, LifecycleOperation,
        TerminalLifecycleObservation, TerminalStatus},
};
use serde::{Deserialize, Serialize};

pub use sandproof_controller_service::template_registry::LocalValidationBinding;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationLink {
    pub create: AcceptedLifecycleIntent,
    pub local: Option<LocalValidationBinding>,
    pub terminal: Option<AcceptedLifecycleIntent>,
    pub settlement: Option<ValidationSettlement>,
}
/// Immutable global evidence transferred from the SAME committed lifecycle
/// record, not a guest assertion or a replacement for the native certificate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationSettlement {
    /// Exact current Destroy Cleaned. Non-admission is separately attested by
    /// the original journal; no old Create/NotAccepted shortcut exists.
    pub cleaned: TerminalLifecycleObservation,
    pub admission: ValidationAdmission,
}
/// The authenticated host's original admission outcome, never inferred from
/// missing local data. Native mapping/certificate remain unique in their fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationAdmission {
    Admitted,
    NotAdmitted,
}
impl ValidationSettlement {
    pub(super) fn is_valid_for(&self, link: &ValidationLink) -> bool {
        let create = &link.create;
        let cleaned = &self.cleaned;
        if link.terminal.as_ref() != Some(&cleaned.request)
        {
            return false;
        }
        terminal_request(create, cleaned).is_some()
    }
}

/// Recover only the original exact terminal request from immutable creation
/// and its existing protocol response checks. Shared by both business pins;
/// absence, timeouts and an invented destroy never constitute non-admission.
pub(super) fn terminal_request(
    create: &AcceptedLifecycleIntent,
    result: &TerminalLifecycleObservation,
) -> Option<AcceptedLifecycleIntent> {
    let origin = &create.request.binding;
    let terminal = &result.request;
    let binding = &terminal.request.binding;
    if !matches!(create.request.operation, LifecycleOperation::Create { .. })
        || origin.expected != ExpectedSession::Absent || create.accepted_version != 1
        || create.accepted_version_for_request() != Some(1)
        || create.request.validate(&origin.node).is_err() || !create.retention.valid_for(4096)
        || result.status != TerminalStatus::Cleaned || !result.matches_request(terminal)
        || terminal.center != create.center || terminal.retention != create.retention
        || binding.node != origin.node || binding.key.owner != origin.key.owner
        || binding.key.deployment != origin.key.deployment || binding.session != origin.session
        || binding.key == origin.key || terminal.accepted_version <= create.accepted_version
        || terminal.request.validate(&origin.node).is_err()
    {
        return None;
    }
    Some(terminal.clone())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalPublication {
    pub(super) value: Publication,
    pub(super) revision: ResourceId,
    pub(super) validation: Option<ValidationLink>,
}
impl GlobalPublication {
    pub(super) fn initial_publication(&self) -> Publication {
        let mut value = self.value.clone();
        value.state = sandproof_controller_service::template_registry::PublicationState::Checking;
        value.checked = false;
        value.validation_sandbox = None;
        value.validation_certificate = None;
        value.failure = None;
        value
    }
    #[must_use]
    pub fn value(&self) -> &Publication {
        &self.value
    }
    #[must_use]
    pub fn revision(&self) -> &ResourceId {
        &self.revision
    }
    #[must_use]
    pub fn validation(&self) -> Option<&ValidationLink> {
        self.validation.as_ref()
    }
    fn projected(&self) -> Option<Publication> {
        let mut local = self.value.clone();
        let Some(link) = &self.validation else {
            return (!local.checked
                && local.validation_sandbox.is_none()
                && local.validation_certificate.is_none())
            .then_some(local);
        };
        let Some(binding) = &link.local else {
            if let Some(saved) = &link.settlement {
                return (!local.checked && local.validation_sandbox.is_none()
                    && local.validation_certificate.is_none()
                    && local.state == sandproof_controller_service::template_registry::PublicationState::Failed
                    && local.failure == Some(sandproof_controller_service::template_registry::PublicationFailure::ValidationRejected)
                    && saved.admission == ValidationAdmission::NotAdmitted && saved.is_valid_for(link)).then_some(local);
            }
            return (!local.checked
                && local.validation_sandbox.is_none()
                && local.validation_certificate.is_none()
                && link.terminal.is_none()
                && link.settlement.is_none())
            .then_some(local);
        };
        // Explicit local physical projection: only trusted cache paths and
        // persisted local owner may differ; artifacts/shape/startup never do.
        if !binding.is_projection_of(&self.value.binding)
            || local.validation_sandbox.is_some_and(|id| id != binding.sandbox.0)
            || local.validation_certificate.as_ref().is_some_and(|certificate|
                certificate.sandbox != binding.sandbox || certificate.owner != binding.owner
                || certificate.activation.as_ref().is_some_and(|activation|
                    !matches!(activation.origin, sandbox_controller_core::EffectOrigin::Operation(key)
                        if key.sandbox == binding.sandbox && key.operation == binding.create_operation)))
            || local.validation_certificate.as_ref().is_some_and(|certificate|
                certificate.terminal_operation.operation.kind == sandbox_controller_core::OperationKind::Create
                    && certificate.terminal_operation.operation.id != binding.create_operation)
            || (local.validation_certificate.is_some() != link.terminal.is_some())
            || (local.validation_certificate.is_some() != link.settlement.is_some())
            || link.settlement.as_ref().is_some_and(|saved| saved.admission != ValidationAdmission::Admitted || !saved.is_valid_for(link))
        { return None; }
        local.binding.clone_from(&binding.descriptor);
        Some(local)
    }
}
impl PublicationRecord for GlobalPublication {
    fn publication(&self) -> &Publication {
        &self.value
    }
    fn is_valid(&self) -> bool {
        PublicTemplate::from_binding(self.value.binding.clone(), self.revision.clone()).is_ok()
            && self
                .projected()
                .is_some_and(|local| evidence::validate_publication(&local).is_ok())
    }
    fn preserves_binding(&self, next: &Self) -> bool {
        self.revision == next.revision
            && self.validation.as_ref().is_none_or(|old| {
                next.validation.as_ref().is_some_and(|new| {
                    old.create == new.create
                        && old
                            .local
                            .as_ref()
                            .is_none_or(|local| new.local.as_ref() == Some(local))
                        && old
                            .terminal
                            .as_ref()
                            .is_none_or(|terminal| new.terminal.as_ref() == Some(terminal))
                        && old
                            .settlement
                            .as_ref()
                            .is_none_or(|saved| new.settlement.as_ref() == Some(saved))
                })
            })
    }
}
