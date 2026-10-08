//! A durable credential index locates one sandbox; it grants no authority.
//! Host reads happen only after that trusted lookup, never across the fleet.
use super::{HostAdapter, Inner, PortError, map_journal, same_secret};
use sandbox_controller_core as core;
use sandproof_controller_service::{ControllerJournal, NeverCrash};
use sha2::{Digest, Sha256};

pub(super) fn digest(token: &str) -> Result<[u8; 32], PortError> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PortError::Busy);
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
) -> Result<(), PortError> {
    let generation = journal
        .world()
        .map_err(|_| PortError::OutcomeUnknown)?
        .sandbox_record(sandbox)
        .ok_or(PortError::OutcomeUnknown)?
        .generation;
    journal
        .register_data_credential(
            sandbox,
            generation,
            digest(token).map_err(|_| PortError::OutcomeUnknown)?,
            &mut NeverCrash,
        )
        .map_err(|_| PortError::OutcomeUnknown)
}

pub(super) fn verify_target<H: HostAdapter>(
    inner: &Inner<H>,
    sandbox: core::SandboxId,
    generation: core::Generation,
) -> Result<String, PortError> {
    let (registered_generation, registered_digest) = inner
        .journal
        .data_credential(sandbox)
        .map_err(map_journal)?
        .ok_or(PortError::Busy)?;
    if registered_generation != generation
        || inner
            .journal
            .lookup_data_credential(&registered_digest)
            .map_err(map_journal)?
            != Some((sandbox, generation))
    {
        return Err(PortError::Busy);
    }
    let token = inner.host.token_for(sandbox)?;
    if !same_secret(&digest(&token)?, &registered_digest) {
        return Err(PortError::Busy);
    }
    Ok(token)
}
