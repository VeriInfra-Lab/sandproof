//! A durable credential index locates one sandbox; it grants no authority.
//! Host reads happen only after that trusted lookup, never across the fleet.
mod credential_core;

use super::{
    ApplicationError, ControllerHost, FaultContext, FaultRecord, FaultStage, Inner, map_journal,
    map_unknown_journal,
};
use crate::application::fault::FaultKind;
use crate::{ControllerJournal, NeverCrash};
use sandbox_controller_core as core;
use sha2::{Digest, Sha256};

pub(super) fn digest(token: &str) -> Result<[u8; 32], ApplicationError> {
    if !credential_core::token_format(token.as_bytes()) {
        return Err(ApplicationError::Busy);
    }
    Ok(Sha256::digest(token.as_bytes()).into())
}

// Called after the admitted identity and its host descriptor are durable, but
// before the first worker claim. Any failure now has an accepted identity and
// must remain an unknown create result, not a side-effect-free rejection.
pub(super) fn register_original(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    token: &str,
) -> Result<(), ApplicationError> {
    let context = FaultContext::instance(sandbox, Some(core::OperationId(1)));
    let generation = journal
        .world()
        .map_err(|error| map_unknown_journal(error, FaultStage::CredentialRegistration, context))?
        .sandbox_record(sandbox)
        .ok_or(ApplicationError::OutcomeUnknown)?
        .generation;
    journal
        .register_data_credential(
            sandbox,
            generation,
            digest(token).map_err(|error| {
                super::map_unknown_application(error, FaultStage::CredentialRegistration, context)
            })?,
            &mut NeverCrash,
        )
        .map_err(|error| map_unknown_journal(error, FaultStage::CredentialRegistration, context))
}

pub(super) fn verify_target<H: ControllerHost>(
    inner: &Inner<H>,
    sandbox: core::SandboxId,
    generation: core::Generation,
) -> Result<String, ApplicationError> {
    let context = FaultContext {
        generation: Some(generation),
        ..FaultContext::instance(sandbox, None)
    };
    let stored = inner
        .journal
        .data_credential(sandbox)
        .map_err(map_journal)?;
    let registered_digest =
        credential_core::registered_digest(stored, generation).map_err(|error| {
            match error {
                credential_core::CredentialReject::Missing => {
                    FaultRecord::host(
                        FaultStage::ReadCredential,
                        FaultKind::CredentialUnavailable,
                        context,
                    )
                    .emit();
                }
                credential_core::CredentialReject::Mismatch => {
                    FaultRecord::host(
                        FaultStage::CheckCredential,
                        FaultKind::CredentialMismatch,
                        context,
                    )
                    .emit();
                }
            }
            ApplicationError::Busy
        })?;
    let found = inner
        .journal
        .lookup_data_credential(&registered_digest)
        .map_err(map_journal)?
        .map(|(bound_sandbox, bound_generation)| (bound_sandbox.0, bound_generation));
    if credential_core::inverse_binding(sandbox.0, generation, found).is_err() {
        FaultRecord::host(
            FaultStage::CheckCredential,
            FaultKind::CredentialMismatch,
            context,
        )
        .emit();
        return Err(ApplicationError::Busy);
    }
    let token = inner.host.token_for(sandbox)?;
    if !credential_core::digest_matches(&digest(&token)?, &registered_digest) {
        FaultRecord::host(
            FaultStage::CheckCredential,
            FaultKind::CredentialMismatch,
            context,
        )
        .emit();
        return Err(ApplicationError::Busy);
    }
    Ok(token)
}
