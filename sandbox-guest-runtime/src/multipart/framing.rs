//! Failures observed by the MIME framing machine, before request-specific wrapping.
use super::{Error, HeaderError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FramingError {
    BodyTruncated,
    NextPartEof,
    /// The original `NextPart` `ReadSlice` reached its fixed 4096-byte buffer.
    ReadBufferFull,
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
            Self::Boundary { .. } | Self::ReadBufferFull => Error::Malformed,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    Decode(Error),
    Framing(FramingError),
    Headers(HeaderError),
    /// Actual source failure, whose cause is retained by the caller.
    Source(SourceStage),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStage {
    NextPart,
    Headers,
    Body,
    /// The caller is draining the source after MIME End.
    Complete,
}
impl ReadError {
    #[must_use]
    pub fn coarse(self) -> Error {
        match self {
            Self::Decode(error) => error,
            Self::Headers(_) => Error::Malformed,
            Self::Framing(error) => error.coarse(),
            Self::Source(_) => Error::Truncated,
        }
    }
}
