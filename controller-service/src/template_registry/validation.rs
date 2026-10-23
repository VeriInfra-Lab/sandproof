//! Exact read-only validation evidence; no host I/O or publication authority.
use super::{Publication, ValidationCertificate, evidence};
use crate::{CreateOptions, JournalError, templates::TemplateBinding};
use sandbox_controller_core::{OperationId, SandboxId};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, LifecycleOperation, TerminalLifecycleObservation, TerminalStatus,
};
use serde::{Deserialize, Serialize};

/// Hard engineering evidence bound, shared by host, transport and center.
/// Smaller protected metadata budgets may still refuse final settlement.
pub const MAX_VALIDATION_CERTIFICATE_BYTES: usize = 16 * 1024 * 1024;

/// The original persisted host mapping, never a client-selected sandbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalValidationBinding {
    pub owner: u64,
    pub sandbox: SandboxId,
    pub create_operation: OperationId,
    pub descriptor: TemplateBinding,
}
impl LocalValidationBinding {
    /// Only protected local paths and the original mapped owner may differ.
    /// Resources, artifacts, startup and all other immutable fields must match.
    #[must_use]
    pub fn is_projection_of(&self, global: &TemplateBinding) -> bool {
        if self.owner == 0 || self.sandbox.0 == 0 || self.create_operation.0 == 0 {
            return false;
        }
        let mut expected = global.clone();
        expected.owner = Some(self.owner);
        expected
            .kernel
            .path
            .clone_from(&self.descriptor.kernel.path);
        expected
            .rootfs
            .path
            .clone_from(&self.descriptor.rootfs.path);
        expected == self.descriptor
            && self.descriptor.is_valid()
            && serde_json::to_vec(&self.descriptor)
                .is_ok_and(|bytes| bytes.len() <= CreateOptions::MAX_ENCODED_BYTES)
    }
}

/// Authenticated original lifecycle and native evidence for one validation.
/// No guest token, API key or user creation environment is part of this value.
/// Immutable template startup defaults remain part of the original descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationEvidence {
    pub create: AcceptedLifecycleIntent,
    pub cleaned: TerminalLifecycleObservation,
    pub outcome: ValidationOutcome,
}
/// No admission is an explicit authenticated original fence attestation, not
/// absence of an SQL row, a guest claim, or permission to publish Ready.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValidationOutcome {
    Admitted {
        local: Box<LocalValidationBinding>,
        certificate: Box<ValidationCertificate>,
    },
    NotAdmitted,
}
impl ValidationEvidence {
    #[must_use]
    pub fn admitted(&self) -> Option<(&LocalValidationBinding, &ValidationCertificate)> {
        match &self.outcome {
            ValidationOutcome::Admitted { local, certificate } => Some((local, certificate)),
            ValidationOutcome::NotAdmitted => None,
        }
    }
    /// Check original certificate rules after the sole permitted projection.
    /// Full saved requests/currentness are additionally checked by each authority.
    /// # Errors
    /// Wrong global pairing, local origin or original evidence cannot settle.
    pub fn validate_for(
        &self,
        create: &AcceptedLifecycleIntent,
        terminal: &TerminalLifecycleObservation,
        publication: &Publication,
    ) -> Result<(), JournalError> {
        super::state::validate_begin(publication)?;
        let original = &create.request.binding;
        let final_request = &terminal.request;
        let final_binding = &final_request.request.binding;
        let LifecycleOperation::Create {
            create: parameters, ..
        } = &create.request.operation
        else {
            return Err(JournalError::Conflict);
        };
        if self.create != *create
            || self.cleaned != *terminal
            || terminal.status != TerminalStatus::Cleaned
            || !terminal.matches_request(final_request)
            || !matches!(
                final_request.request.operation,
                LifecycleOperation::Destroy {}
            )
            || create.accepted_version != 1
            || final_request.accepted_version <= create.accepted_version
            || create.center != final_request.center
            || create.retention != final_request.retention
            || original.node != final_binding.node
            || original.session != final_binding.session
            || original.key.owner != final_binding.key.owner
            || original.key == final_binding.key
            || create.request.validate(&final_binding.node).is_err()
            || final_request.request.validate(&original.node).is_err()
            || publication
                .binding
                .owner
                .map(|owner| owner.to_string())
                .as_deref()
                != Some(original.key.owner.as_str())
            || publication
                .binding
                .canonical_digest()
                .map_err(|_| JournalError::Conflict)?
                != parameters.template_digest
        {
            return Err(JournalError::Conflict);
        }
        let Some((local_binding, certificate)) = self.admitted() else {
            return Ok(());
        };
        if !local_binding.is_projection_of(&publication.binding)
            || certificate.sandbox != local_binding.sandbox
            || certificate.owner != local_binding.owner
            || certificate.activation.as_ref().is_some_and(|activation| {
                !matches!(activation.origin, sandbox_controller_core::EffectOrigin::Operation(key)
                    if key.sandbox == local_binding.sandbox
                    && key.operation == local_binding.create_operation)
            })
            || (certificate.terminal_operation.operation.kind
                == sandbox_controller_core::OperationKind::Create
                && certificate.terminal_operation.operation.id != local_binding.create_operation)
        {
            return Err(JournalError::Conflict);
        }
        let mut local = publication.clone();
        local.binding.clone_from(&local_binding.descriptor);
        local.checked = certificate.activation.is_some();
        local.state = certificate.outcome;
        local.failure = certificate.failure;
        local.validation_sandbox = None;
        local.validation_certificate = Some(certificate.clone());
        evidence::validate_publication(&local)?;
        evidence::encode_bounded(certificate, MAX_VALIDATION_CERTIFICATE_BYTES)?;
        Ok(())
    }
}
