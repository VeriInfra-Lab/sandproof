//! Known gzip errors while copying a raw upload body into an opened file.
//! Header preparation and multipart reads have different public wrappers.
use crate::upload_body::GzipError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GzipReadError {
    Truncated,
    InvalidMember,
    Checksum,
}
impl TryFrom<GzipError> for GzipReadError {
    type Error = GzipError;
    fn try_from(error: GzipError) -> Result<Self, GzipError> {
        match error {
            GzipError::Truncated => Ok(Self::Truncated),
            GzipError::Header => Ok(Self::InvalidMember),
            GzipError::Checksum => Ok(Self::Checksum),
            other => Err(other),
        }
    }
}
impl From<GzipReadError> for GzipError {
    fn from(error: GzipReadError) -> Self {
        match error {
            GzipReadError::Truncated => Self::Truncated,
            GzipReadError::InvalidMember => Self::Header,
            GzipReadError::Checksum => Self::Checksum,
        }
    }
}
impl GzipReadError {
    pub(super) fn message(self) -> &'static [u8] {
        match self {
            Self::Truncated => b"unexpected EOF",
            Self::InvalidMember => b"gzip: invalid header",
            Self::Checksum => b"gzip: invalid checksum",
        }
    }
}

#[cfg(test)]
mod tests;
