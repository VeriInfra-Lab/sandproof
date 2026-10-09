//! A durable credential index locates one sandbox; it grants no authority.
//! Host reads happen only after that trusted lookup, never across the fleet.
use super::{ApplicationError, HostAdapter, Inner, map_journal, same_secret};
use crate::{ControllerJournal, NeverCrash};
use sandbox_controller_core as core;
use sha2::{Digest, Sha256};

pub(super) fn digest(token: &str) -> Result<[u8; 32], ApplicationError> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
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
    let generation = journal
        .world()
        .map_err(|_| ApplicationError::OutcomeUnknown)?
        .sandbox_record(sandbox)
        .ok_or(ApplicationError::OutcomeUnknown)?
        .generation;
    journal
        .register_data_credential(
            sandbox,
            generation,
            digest(token).map_err(|_| ApplicationError::OutcomeUnknown)?,
            &mut NeverCrash,
        )
        .map_err(|_| ApplicationError::OutcomeUnknown)
}

pub(super) fn verify_target<H: HostAdapter>(
    inner: &Inner<H>,
    sandbox: core::SandboxId,
    generation: core::Generation,
) -> Result<String, ApplicationError> {
    let (registered_generation, registered_digest) = inner
        .journal
        .data_credential(sandbox)
        .map_err(map_journal)?
        .ok_or(ApplicationError::Busy)?;
    if registered_generation != generation
        || inner
            .journal
            .lookup_data_credential(&registered_digest)
            .map_err(map_journal)?
            != Some((sandbox, generation))
    {
        return Err(ApplicationError::Busy);
    }
    let token = inner.host.token_for(sandbox)?;
    if !same_secret(&digest(&token)?, &registered_digest) {
        return Err(ApplicationError::Busy);
    }
    Ok(token)
}
