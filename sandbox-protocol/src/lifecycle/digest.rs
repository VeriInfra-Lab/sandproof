use serde::{Deserialize, Serialize};

/// The full 256-bit SHA-256 value of explicitly selected content.
///
/// This is a value carrier, not a verification receipt. Its constructor neither
/// reads files nor computes a hash. Callers must bind the value to the exact
/// bytes, original object and format that their operation actually uses.
/// The local controller's numeric `ManifestDigest` is a different identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    /// Preserve all digest bytes, including leading zero bytes.
    #[must_use]
    pub const fn from_sha256(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}
