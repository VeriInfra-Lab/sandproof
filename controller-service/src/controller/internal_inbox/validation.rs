//! Read original native certificate evidence; no admission, expiry or host I/O.
use super::{codec, model::Inbox};
use crate::template_registry::{
    LocalValidationBinding, MAX_VALIDATION_CERTIFICATE_BYTES, Publication, PublicationFailure,
    PublicationState, ValidationEvidence, ValidationOutcome, evidence, state,
};
use crate::{ControllerJournal, JournalError};
use sandbox_controller_core as core;
use sandproof_protocol::{
    ResourceId,
    internal::{
        ControlIdentity,
        lifecycle::{Intent, LifecycleFact},
    },
};

fn original_pair<'a>(
    inbox: &'a Inbox,
    create: &ControlIdentity,
    terminal: &ControlIdentity,
) -> Result<(&'a super::model::Entry, &'a super::model::Entry), JournalError> {
    create
        .validate_node(&inbox.config.node)
        .map_err(|_| JournalError::Conflict)?;
    terminal
        .validate_node(&inbox.config.node)
        .map_err(|_| JournalError::Conflict)?;
    let original = inbox
        .entries
        .iter()
        .find(|entry| entry.request.identity == *create)
        .ok_or(JournalError::Conflict)?;
    let final_entry = inbox
        .entries
        .iter()
        .find(|entry| entry.request.identity == *terminal)
        .ok_or(JournalError::Conflict)?;
    if original.request.intent != Intent::Create
        || create.intent_version.get() != 1
        || final_entry.request.intent != Intent::Destroy
        || final_entry.response.fact != LifecycleFact::Cleaned
        || original.request.create != final_entry.request.create
        || create.node != terminal.node
        || create.tenant_id != terminal.tenant_id
        || create.sandbox_id != terminal.sandbox_id
        || create.request_window != terminal.request_window
        || terminal.intent_version.get() <= create.intent_version.get()
        || terminal.operation_id == create.operation_id
        || inbox.entries.iter().any(|entry| {
            entry.request.identity.sandbox_id == terminal.sandbox_id
                && entry.request.identity.intent_version.get() > terminal.intent_version.get()
        })
    {
        return Err(JournalError::Conflict);
    }
    Ok((original, final_entry))
}

fn checked_create(
    world: &core::World,
    local: super::model::InternalLocalBinding,
) -> Result<bool, JournalError> {
    let ledger = world
        .operation_record(core::OperationKey {
            sandbox: local.sandbox,
            operation: local.operation,
        })
        .ok_or(JournalError::CorruptStore)?;
    if ledger.sandbox != local.sandbox
        || ledger.operation.id != local.operation
        || ledger.operation.kind != core::OperationKind::Create
    {
        return Err(JournalError::CorruptStore);
    }
    // Never downgrade missing/corrupt successful activation to definitive
    // failure. Only the original terminal create ledger decides.
    match ledger.operation.status {
        core::OperationStatus::Succeeded => {
            let activation = evidence::successful_activation(world, local.sandbox)?;
            if !matches!(activation.origin, core::EffectOrigin::Operation(key)
                if key.sandbox == local.sandbox && key.operation == local.operation)
            {
                return Err(JournalError::CorruptStore);
            }
            Ok(true)
        }
        core::OperationStatus::Failed(_) => Ok(false),
        _ => Err(JournalError::UnresolvedAction),
    }
}

impl ControllerJournal {
    /// Issue original engineering evidence for an exact, already cleaned
    /// validation incarnation under one original read transaction. The supplied
    /// publication is center-accepted build metadata, NOT a filesystem command
    /// or independent host verification of image digest/free-space metadata.
    /// # Errors
    /// Missing/retired mappings, unknown activation, incomplete cleanup, changed
    /// descriptors and shared corruption cannot produce a certificate.
    pub fn internal_validation_evidence(
        &self,
        create: &ControlIdentity,
        terminal: &ControlIdentity,
        revision: &ResourceId,
        publication: &Publication,
    ) -> Result<ValidationEvidence, JournalError> {
        state::validate_begin(publication)?;
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let world = super::super::checkpoint::load_world(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let (original, final_entry) = original_pair(&inbox, create, terminal)?;
        if publication
            .binding
            .owner
            .map(|owner| owner.to_string())
            .as_deref()
            != Some(create.tenant_id.as_str())
            || *revision != original.request.create.template_revision
            || publication
                .binding
                .canonical_digest()
                .map_err(|_| JournalError::Conflict)?
                != original.request.create.template_digest
        {
            return Err(JournalError::Conflict);
        }
        let original_options: crate::CreateOptions =
            serde_json::from_slice(&original.request.create.options)
                .map_err(|_| JournalError::CorruptStore)?;
        if !original_options.env_vars.is_empty()
            || !original_options.metadata.is_empty()
            || !original_options.networking.as_ref().is_some_and(|network| {
                network.allow_internet_access == Some(false) && network.network.is_none()
            })
        {
            return Err(JournalError::Conflict);
        }
        let Some(local) = original.local else {
            // A validated persisted Create plus a current durable Destroy
            // fence, never inferred from a missing SQL row or PID. Original
            // atomic admission guarantees no unrecorded native VM.
            let result = ValidationEvidence {
                create: create.clone(),
                cleaned: final_entry.response.clone(),
                outcome: ValidationOutcome::NotAdmitted,
            };
            result.validate_for(create, terminal, publication)?;
            tx.commit()?;
            return Ok(result);
        };
        // Original immutable options were validated by the SAME snapshot above.
        // The caller's paths are never opened, admitted or used as local paths.
        let options =
            super::super::admission::read_create_options(&tx, &self.namespace, local.sandbox)
                .map_err(|_| JournalError::CorruptStore)?;
        let binding = LocalValidationBinding {
            owner: local.owner.0,
            sandbox: local.sandbox,
            create_operation: local.operation,
            descriptor: options.template.ok_or(JournalError::CorruptStore)?,
        };
        if !binding.is_projection_of(&publication.binding) {
            return Err(JournalError::Conflict);
        }
        let checked = checked_create(&world, local)?;
        let mut native = publication.clone();
        native.binding.clone_from(&binding.descriptor);
        native.checked = checked;
        native.state = if checked {
            PublicationState::Ready
        } else {
            PublicationState::Failed
        };
        native.failure = (!checked).then_some(PublicationFailure::Interrupted);
        let certificate = evidence::make_certificate(&world, &native, local.sandbox)?;
        evidence::encode_bounded(&certificate, MAX_VALIDATION_CERTIFICATE_BYTES)?;
        let result = ValidationEvidence {
            create: create.clone(),
            cleaned: final_entry.response.clone(),
            outcome: ValidationOutcome::Admitted {
                local: Box::new(binding),
                certificate: Box::new(certificate),
            },
        };
        result.validate_for(create, terminal, publication)?;
        tx.commit()?;
        Ok(result)
    }
}
