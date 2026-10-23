//! Original native certificate evidence, bound to the sole current inbox.
use super::{
    access, authenticated, codec,
    model::{Inbox, Session},
};
use crate::template_registry::{
    LocalValidationBinding, MAX_VALIDATION_CERTIFICATE_BYTES, Publication, PublicationFailure,
    PublicationState, ValidationEvidence, ValidationOutcome, evidence, state,
};
use crate::{ControllerJournal, InternalLocalBinding, JournalError};
use sandbox_controller_core as core;
use sandproof_protocol::{
    ResourceId,
    lifecycle::{
        AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation,
        TerminalLifecycleObservation,
    },
};

fn original_session<'a>(
    inbox: &'a Inbox,
    create: &AcceptedLifecycleIntent,
    terminal: &TerminalLifecycleObservation,
    context: &AuthenticatedCenterContext,
) -> Result<&'a Session, JournalError> {
    authenticated(inbox, create, context)?;
    authenticated(inbox, &terminal.request, context)?;
    let (session, _) = inbox.find(create)?.ok_or(JournalError::Conflict)?;
    let (final_session, _) = inbox
        .find(&terminal.request)?
        .ok_or(JournalError::Conflict)?;
    if session != final_session
        || !inbox.sessions[session].terminal
        || inbox.sessions[session].current_version != terminal.request.accepted_version
    {
        return Err(JournalError::Conflict);
    }
    Ok(&inbox.sessions[session])
}

fn checked_create(world: &core::World, local: InternalLocalBinding) -> Result<bool, JournalError> {
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
    // Preserve the original native success/failure distinction. Missing or
    // unknown activation is never downgraded to a definitive failed check.
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

fn check_original_parameters(
    create: &AcceptedLifecycleIntent,
    revision: &ResourceId,
    publication: &Publication,
) -> Result<(), JournalError> {
    let LifecycleOperation::Create {
        create: parameters, ..
    } = &create.request.operation
    else {
        return Err(JournalError::Conflict);
    };
    if *revision != parameters.template_revision
        || publication
            .binding
            .canonical_digest()
            .map_err(|_| JournalError::Conflict)?
            != parameters.template_digest
    {
        return Err(JournalError::Conflict);
    }
    let options: crate::CreateOptions =
        serde_json::from_slice(&parameters.options).map_err(|_| JournalError::CorruptStore)?;
    if !options.env_vars.is_empty()
        || !options.metadata.is_empty()
        || !options.networking.as_ref().is_some_and(|network| {
            network.allow_internet_access == Some(false) && network.network.is_none()
        })
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

impl ControllerJournal {
    /// Issue engineering evidence for the exact current validation incarnation.
    /// All source, native certificate and physical cleanup checks use one read
    /// snapshot; publication paths are metadata and are never opened here.
    /// # Errors
    /// Foreign/retired pairs, unknown effects, incomplete cleanup and changed
    /// descriptors cannot issue evidence. Context comes from independent TLS.
    pub fn lifecycle_validation_evidence(
        &self,
        create: &AcceptedLifecycleIntent,
        terminal: &TerminalLifecycleObservation,
        revision: &ResourceId,
        publication: &Publication,
        context: &AuthenticatedCenterContext,
    ) -> Result<ValidationEvidence, JournalError> {
        state::validate_begin(publication)?;
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let world = super::super::checkpoint::load_world(&tx)?;
        codec::validate(&tx, &world)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let session = original_session(&inbox, create, terminal, context)?;
        check_original_parameters(create, revision, publication)?;
        if !access::destroy_cleaned(&tx, &world, &inbox, &terminal.request, context)? {
            return Err(JournalError::UnresolvedAction);
        }
        let Some(local) = session.local else {
            // This is an existing exact Create and current Destroy fence in
            // the same authority. An absent row or PID alone is insufficient.
            let result = ValidationEvidence {
                create: create.clone(),
                cleaned: terminal.clone(),
                outcome: ValidationOutcome::NotAdmitted,
            };
            result.validate_for(create, terminal, publication)?;
            self.committed_world.check_identity(&tx)?;
            tx.commit()?;
            return Ok(result);
        };
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
            cleaned: terminal.clone(),
            outcome: ValidationOutcome::Admitted {
                local: Box::new(binding),
                certificate: Box::new(certificate),
            },
        };
        result.validate_for(create, terminal, publication)?;
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
