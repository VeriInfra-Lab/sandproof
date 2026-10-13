//! Failures observed by the MIME framing machine, before request-specific wrapping.
use super::{Error, HeaderError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FramingError {
    BodyTruncated,
    NextPartEof,
    /// Includes the line ending, retained only after the line budget check.
    Boundary {
        line: Vec<u8>,
        after_separator: bool,
    },
}
impl FramingError {
    #[must_use]
    pub fn coarse(&self) -> Error {
        match self {
            Self::BodyTruncated | Self::NextPartEof => Error::Truncated,
            Self::Boundary { .. } => Error::Malformed,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    Decode(Error),
    Framing(FramingError),
    Headers(HeaderError),
}
impl ReadError {
    #[must_use]
    pub fn coarse(self) -> Error {
        match self {
            Self::Decode(error) => error,
            Self::Headers(_) => Error::Malformed,
            Self::Framing(error) => error.coarse(),
        }
    }
}
