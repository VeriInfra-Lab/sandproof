//! Bounded public upload header/dispatch rejections from the fixed envd.
//! Source: upload.go, encoding.go and filesystem/xattr.go at runtime
//! f455fd58fa3f289e78ce1de690c35968d8d29322 (Apache-2.0, E2B-LICENSE).
use super::{Error, Limits, Owner, Response, errors};
use crate::file_upload::{MetadataFault, MetadataReason};
use axum::http::StatusCode;
use sandproof_guest_runtime::file_upload::MetadataViolation;
use tokio::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    UnsupportedEncoding(Vec<u8>),
    UnsupportedContentType(Vec<u8>),
    RawPathRequired,
    Metadata(MetadataFault),
}

/// Source bytes were retained only after complete request-header admission.
/// Like I/O errors, rejected uploads keep the original slot/deadline through
/// response bytes and do not acquire a replacement authorization or slot.
/// # Errors
/// Invalid/insufficient response budget, missing observations or delivery error.
pub async fn rejected(
    error: Rejection,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Error> {
    errors::send_error(
        StatusCode::BAD_REQUEST,
        limits,
        deadline,
        owner,
        move || payload(&error, limits.wire_bytes),
    )
    .await
}

fn payload(error: &Rejection, limit: usize) -> Result<Vec<u8>, Error> {
    let mut parts: Vec<&[u8]> = Vec::with_capacity(12);
    let size;
    let byte_text;
    let offset_text;
    match error {
        Rejection::UnsupportedEncoding(value) => parts.extend([
            b"error decompressing request body: unsupported Content-Encoding: ".as_slice(),
            value,
            b", supported: [gzip]",
        ]),
        Rejection::UnsupportedContentType(value) => parts.extend([
            b"unsupported content type: ".as_slice(),
            value,
            b", expected multipart/form-data or application/octet-stream",
        ]),
        Rejection::RawPathRequired => {
            parts.push(b"path query parameter is required for raw body upload");
        }
        Rejection::Metadata(fault) => {
            parts.push(b"invalid metadata: ");
            match fault.reason {
                MetadataReason::Total(total) => {
                    size = total.to_string();
                    parts.extend([
                        b"total metadata size ".as_slice(),
                        size.as_bytes(),
                        b" exceeds 4096 bytes",
                    ]);
                }
                MetadataReason::Entry(MetadataViolation::KeyLength) => parts.extend([
                    b"metadata key \"".as_slice(),
                    fault.key.as_bytes(),
                    b"\" exceeds 246 bytes",
                ]),
                MetadataReason::Entry(MetadataViolation::ValueByte { byte, offset }) => {
                    byte_text = format!("{byte:02x}");
                    offset_text = offset.to_string();
                    // HeaderName restricts the normalized key to ASCII token
                    // characters, so Go %q adds quotes without inner escapes.
                    parts.extend([
                        b"metadata value for key \"".as_slice(),
                        fault.key.as_bytes(),
                        b"\" contains non-printable-ASCII byte 0x",
                        byte_text.as_bytes(),
                        b" at offset ",
                        offset_text.as_bytes(),
                    ]);
                }
                // HeaderName and the empty-suffix filter exclude these cases.
                MetadataReason::Entry(_) => return Err(Error::UnclassifiedFault),
            }
        }
    }
    errors::envelope(&parts, 400, limit)
}

#[cfg(test)]
mod tests;
