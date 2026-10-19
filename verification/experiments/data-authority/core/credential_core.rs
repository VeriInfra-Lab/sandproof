//! Value checks on actual credential rows and host-token bytes.
#[path = "byte_core.rs"]
mod byte_core;
pub use byte_core::token_format;

#[must_use]
pub fn digest_matches(actual: &[u8; 32], registered: &[u8; 32]) -> bool {
    byte_core::same_secret(actual, registered)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialReject {
    Missing,
    Mismatch,
}

/// # Errors
/// Rejects a missing row or a generation mismatch before any inverse query.
pub fn registered_digest(
    stored: Option<(u64, [u8; 32])>,
    generation: u64,
) -> Result<[u8; 32], CredentialReject> {
    let (registered_generation, registered_digest) = stored.ok_or(CredentialReject::Missing)?;
    if registered_generation != generation {
        return Err(CredentialReject::Mismatch);
    }
    Ok(registered_digest)
}

/// # Errors
/// Rejects a missing inverse row or either mismatched identity field.
pub fn inverse_binding(
    sandbox: u64,
    generation: u64,
    found: Option<(u64, u64)>,
) -> Result<(), CredentialReject> {
    match found {
        Some((bound_sandbox, bound_generation))
            if bound_sandbox == sandbox && bound_generation == generation =>
        {
            Ok(())
        }
        _ => Err(CredentialReject::Mismatch),
    }
}
