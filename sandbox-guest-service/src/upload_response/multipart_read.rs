//! MIME framing observations, wrapped at the original upload read stage.
use super::Error;
use crate::go_json;
use sandproof_guest_runtime::multipart::{FramingError, HeaderError};
mod quote;
mod tables;
#[cfg(test)]
mod tests;

pub(super) fn payload(error: &FramingError, limit: usize) -> Result<Vec<u8>, Error> {
    let prefix: &[u8] = match error {
        FramingError::BodyTruncated => b"error writing file: unexpected EOF",
        FramingError::NextPartEof => b"error reading form: multipart: NextPart: EOF",
        FramingError::Boundary {
            after_separator: true,
            ..
        } => b"error reading form: multipart: expecting a new Part; got line ",
        FramingError::Boundary {
            after_separator: false,
            ..
        } => b"error reading form: multipart: unexpected line in Next(): ",
    };
    let line = if let FramingError::Boundary { line, .. } = error {
        Some(line.as_slice())
    } else {
        None
    };
    encoded(prefix, line, limit)
}

pub(super) fn headers_payload(error: &HeaderError, limit: usize) -> Result<Vec<u8>, Error> {
    let (prefix, line): (&[u8], Option<&[u8]>) = match error {
        HeaderError::InitialLine(line) => (
            b"error reading form: malformed MIME header initial line: ",
            Some(line),
        ),
        HeaderError::InitialLineTooLong => {
            (b"error reading form: multipart: message too large", None)
        }
        HeaderError::MissingColon(line) => (
            b"error reading form: malformed MIME header: missing colon: ",
            Some(line),
        ),
        HeaderError::InvalidLine(line) => (
            b"error reading form: malformed MIME header line: ",
            Some(line),
        ),
    };
    encoded(prefix, line, limit)
}
fn encoded(prefix: &[u8], line: Option<&[u8]>, limit: usize) -> Result<Vec<u8>, Error> {
    go_json::bounded(limit as u64, |put| {
        put(b"{\"code\":500,\"message\":\"")?;
        go_json::string_content(prefix, put)?;
        if let Some(line) = line {
            quote::quoted(line, &mut |bytes| go_json::string_content(bytes, put))?;
        }
        put(b"\"}\n")
    })
    .map_err(|_| Error::ResponseLimit)
}
