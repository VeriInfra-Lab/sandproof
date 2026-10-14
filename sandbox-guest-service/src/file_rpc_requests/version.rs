use axum::http::HeaderMap;
use std::fmt;

/// First version field, copied only after the complete header budget check.
/// Private construction prevents an unchecked arbitrary error diagnostic.
#[derive(Clone, PartialEq, Eq)]
pub struct InvalidVersion(Vec<u8>);
impl InvalidVersion {
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Debug for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvalidVersion").finish_non_exhaustive()
    }
}

// connect-go v1.18.1's default POST handler does not require the version field.
// Header.Get observes the first value; it neither trims nor joins duplicates.
pub(super) fn check(headers: &HeaderMap) -> Result<(), InvalidVersion> {
    let value = headers
        .get("connect-protocol-version")
        .map_or(b"".as_slice(), |value| value.as_bytes());
    if value.is_empty() || value == b"1" {
        Ok(())
    } else {
        Err(InvalidVersion(value.to_vec()))
    }
}
