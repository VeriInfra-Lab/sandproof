//! Bounded public upload header/dispatch/part rejections from the fixed envd.
//! Source: upload.go, encoding.go and filesystem/xattr.go at runtime
//! f455fd58fa3f289e78ce1de690c35968d8d29322 (Apache-2.0, E2B-LICENSE).
use super::{Error, GzipHeaderError, GzipReadError, Limits, Owner, Response, errors};
use crate::file_upload::{DuplicatePath, MetadataFault, MetadataReason};
use axum::http::StatusCode;
use sandproof_guest_runtime::file_upload::MetadataViolation;
use sandproof_guest_runtime::multipart::BoundaryRejection;
use tokio::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    MultipartRead(sandproof_guest_runtime::multipart::FramingError),
    RawGzipBody(GzipReadError),
    MultipartSetup(BoundaryRejection),
    DuplicatePath(DuplicatePath),
    MissingFilename,
    UnsupportedEncoding(Vec<u8>),
    UnsupportedContentType(Vec<u8>),
    RawPathRequired,
    Metadata(MetadataFault),
    GzipHeader(GzipHeaderError),
}
impl Rejection {
    fn status(&self) -> StatusCode {
        if matches!(
            self,
            Self::MultipartSetup(_) | Self::RawGzipBody(_) | Self::MultipartRead(_)
        ) {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::BAD_REQUEST
        }
    }
}

/// Source bytes were retained under admitted header/part/result budgets.
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
    errors::send_error(error.status(), limits, deadline, owner, move || {
        payload(&error, limits.wire_bytes)
    })
    .await
}

fn payload(error: &Rejection, limit: usize) -> Result<Vec<u8>, Error> {
    let mut parts: Vec<&[u8]> = Vec::with_capacity(12);
    let size;
    let byte_text;
    let offset_text;
    match error {
        Rejection::MultipartRead(error) => return super::multipart_read::payload(error, limit),
        Rejection::RawGzipBody(error) => parts.extend([b"error writing file: ".as_slice(), error.message()]),
        Rejection::MultipartSetup(error) => parts.push(match error {
            BoundaryRejection::NotMultipart => b"error parsing multipart form: request Content-Type isn't multipart/form-data",
            BoundaryRejection::MissingBoundary => b"error parsing multipart form: no multipart boundary param in Content-Type",
            BoundaryRejection::EmptyBoundary => b"error reading form: multipart: boundary is empty",
        }),
        Rejection::DuplicatePath(error) => return duplicate_payload(error, limit),
        Rejection::MissingFilename => parts.push(b"error getting multipart custom part file name: filename not found in Content-Disposition header"),
        Rejection::GzipHeader(error) => parts.extend([
            b"error decompressing request body: failed to create gzip reader: ".as_slice(),
            error.message(),
        ]),
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
    errors::envelope(&parts.iter().copied(), error.status().as_u16(), limit)
}

fn duplicate_payload(error: &DuplicatePath, limit: usize) -> Result<Vec<u8>, Error> {
    // Stream pieces through both bounded passes; do not allocate a joined message.
    let prefix = [
        b"you cannot upload multiple files to the same path '".as_slice(),
        error.path.as_bytes(),
        b"' in one upload request, only the first specified file was uploaded",
    ];
    // Fixed resolvePath deliberately omits exactly one other completed file.
    let previous = error
        .previous
        .iter()
        .enumerate()
        .filter(|_| error.previous.len() > 1)
        .flat_map(|(index, path)| {
            [
                if index == 0 {
                    b", also the following files were uploaded: ".as_slice()
                } else {
                    b", "
                },
                path.as_bytes(),
            ]
        });
    errors::envelope(&prefix.into_iter().chain(previous), 400, limit)
}

#[cfg(test)]
mod tests;
