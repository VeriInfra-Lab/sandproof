//! Only errors observed while preparing the FIRST gzip header map here.
//! Later members, body/trailer errors and transport failures have other phases.
use crate::upload_body::GzipError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GzipHeaderError {
    Empty,
    Truncated,
    Invalid,
}
impl TryFrom<GzipError> for GzipHeaderError {
    type Error = GzipError;
    fn try_from(error: GzipError) -> Result<Self, GzipError> {
        match error {
            GzipError::EmptyHeader => Ok(Self::Empty),
            GzipError::Truncated => Ok(Self::Truncated),
            GzipError::Header => Ok(Self::Invalid),
            other => Err(other),
        }
    }
}
impl From<GzipHeaderError> for GzipError {
    fn from(error: GzipHeaderError) -> Self {
        match error {
            GzipHeaderError::Empty => Self::EmptyHeader,
            GzipHeaderError::Truncated => Self::Truncated,
            GzipHeaderError::Invalid => Self::Header,
        }
    }
}
impl GzipHeaderError {
    pub(super) fn message(self) -> &'static [u8] {
        match self {
            Self::Empty => b"EOF",
            Self::Truncated => b"unexpected EOF",
            Self::Invalid => b"gzip: invalid header",
        }
    }
}

#[cfg(test)]
mod tests;
