//! Fixed SHA-256 library use; no lifecycle decision or object access.
use sha2::{Digest, Sha256};

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
