//! MIME framing observations, wrapped at the original upload read stage.
use super::Error;
use crate::go_json;
use sandproof_guest_runtime::multipart::FramingError;
mod quote;
mod tables;
#[cfg(test)]
mod tests;

pub(super) fn payload(error: &FramingError, limit: usize) -> Result<Vec<u8>, Error> {
    go_json::bounded(limit as u64, |put| {
        put(b"{\"code\":500,\"message\":\"")?;
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
        go_json::string_content(prefix, put)?;
        if let FramingError::Boundary { line, .. } = error {
            quote::quoted(line, &mut |bytes| go_json::string_content(bytes, put))?;
        }
        put(b"\"}\n")
    })
    .map_err(|_| Error::ResponseLimit)
}
