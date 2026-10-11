//! Exact global-to-local certificate projection; no storage or I/O.
use crate::PublicTemplate;
use sandproof_controller_service::template_registry::{
    Publication, evidence, state::PublicationRecord,
};
use sandproof_protocol::{
    ResourceId,
    internal::ControlIdentity,
    internal::lifecycle::{Intent, LifecycleFact, LifecycleRequest, LifecycleResponse},
};
use serde::{Deserialize, Serialize};

pub use sandproof_controller_service::template_registry::LocalValidationBinding;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationLink {
    pub create: ControlIdentity,
    pub local: Option<LocalValidationBinding>,
    pub terminal: Option<ControlIdentity>,
    pub settlement: Option<ValidationSettlement>,
}
/// Immutable global evidence transferred from the SAME committed lifecycle
/// record, not a guest assertion or a replacement for the native certificate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationSettlement {
    pub original_create: LifecycleRequest,
    pub cleaned: LifecycleResponse,
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
        let create = &self.original_create;
        let cleaned = &self.cleaned;
        if create.identity != link.create
            || create.intent != Intent::Create
            || create.identity.intent_version.get() != 1
            || create.validate(&link.create.node).is_err()
            || link.terminal.as_ref() != Some(&cleaned.identity)
            || cleaned.fact != LifecycleFact::Cleaned
            || cleaned.identity.intent_version.get() <= create.identity.intent_version.get()
            || cleaned.identity.operation_id == create.identity.operation_id
        {
            return false;
        }
        // Reuse exact response validation with the immutable original binding.
        // Only the operation and monotone intent version may differ.
        let mut terminal = create.clone();
        terminal.intent = Intent::Destroy;
        terminal
            .identity
            .operation_id
            .clone_from(&cleaned.identity.operation_id);
        terminal.identity.intent_version = cleaned.identity.intent_version;
        cleaned.validate_for(&terminal).is_ok()
    }
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
