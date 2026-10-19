//! Fixed processFile + jsonError wrapping of actual structured observations.
// Runtime f455fd58fa3f289e78ce1de690c35968d8d29322 (Apache-2.0):
// ../../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
// xattr error spelling follows pkg/xattr v0.4.12; see XATTR-LICENSE.
use super::{Error, Limits, Owner, Response, send_with_head};
use crate::go_json;
use axum::http::{HeaderValue, StatusCode};
use sandproof_guest_runtime::{
    file_download::LinuxErrno,
    file_upload::{Fault, Stage},
};
use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// The caller supplies the original request Content-Length, including the
/// compressed/multipart framing length. It is only diagnostic for write ENOSPC.
/// Source paths/attributes are already bounded by the admitted file operation.
/// # Errors
/// Missing observations, invalid/insufficient response budget, original expiry
/// or lost worker. Never exposes arbitrary I/O text or replays file operations.
pub async fn io_failure(
    fault: Fault,
    expected: Option<u64>,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Error> {
    let status = StatusCode::from_u16(fault.status()).map_err(|_| Error::UnclassifiedFault)?;
    send_error(status, limits, deadline, owner, move || {
        payload(&fault, expected, limits.wire_bytes)
    })
    .await
}

pub(crate) async fn send_error(
    status: StatusCode,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
    encode: impl FnOnce() -> Result<Vec<u8>, Error> + Send + 'static,
) -> Result<Response, Error> {
    send_error_with_head(status, limits, deadline, owner, false, encode).await
}

pub(crate) async fn send_error_with_head(
    status: StatusCode,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
    head: bool,
    encode: impl FnOnce() -> Result<Vec<u8>, Error> + Send + 'static,
) -> Result<Response, Error> {
    let mut response = send_with_head(limits, deadline, owner, head, encode).await?;
    *response.status_mut() = status;
    response.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn payload(fault: &Fault, expected: Option<u64>, limit: usize) -> Result<Vec<u8>, Error> {
    let path = fault.path.as_slice();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(12);
    let errno;
    let length;
    let operation: &[u8] = match fault.stage {
        Stage::Directory => {
            parts.extend([b"path is a directory: ".as_slice(), path]);
            b""
        }
        Stage::ParentNotDirectory => {
            parts.extend([
                b"error ensuring directories: path is a file: ".as_slice(),
                path,
            ]);
            b""
        }
        Stage::WriteZero => {
            parts.push(b"error writing file: short write");
            b""
        }
        Stage::Write if fault.errno == Some(LinuxErrno(28)) => {
            if let Some(n) = expected.filter(|n| *n > 0) {
                length = n.to_string();
                parts.extend([
                    b"attempted to write ".as_slice(),
                    length.as_bytes(),
                    b" bytes: ",
                ]);
            }
            parts.push(b"not enough disk space available");
            b""
        }
        Stage::ParentStat => {
            parts.push(b"error ensuring directories: failed to stat directory: ");
            b"stat"
        }
        Stage::ParentCreate => {
            parts.push(b"error ensuring directories: failed to create directory: ");
            b"mkdir"
        }
        Stage::ParentChown => {
            parts.push(b"error ensuring directories: failed to chown directory: ");
            b"chown"
        }
        Stage::Stat => {
            parts.push(b"error getting file info: ");
            b"stat"
        }
        Stage::Chown | Stage::FileChown => {
            parts.push(b"error changing file ownership: ");
            b"chown"
        }
        Stage::Open => {
            parts.push(if fault.errno == Some(LinuxErrno(28)) {
                b"not enough inodes available: "
            } else {
                b"error opening file: "
            });
            b"open"
        }
        Stage::Write => {
            parts.push(b"error writing file: ");
            b"write"
        }
        Stage::MetadataList | Stage::MetadataRemove | Stage::MetadataSet => {
            parts.push(if fault.status() == 507 {
                b"not enough space for file metadata: "
            } else {
                b"error writing file metadata: "
            });
            match fault.stage {
                Stage::MetadataList => b"xattr.list",
                Stage::MetadataRemove => b"xattr.FRemove",
                _ => b"xattr.FSet",
            }
        }
    };
    if !operation.is_empty() {
        errno = fault.errno.ok_or(Error::UnclassifiedFault)?.message();
        parts.push(operation);
        let metadata = matches!(
            fault.stage,
            Stage::MetadataList | Stage::MetadataRemove | Stage::MetadataSet
        );
        // os.PathError always includes a space; xattr.Error skips empty fields.
        if !metadata || !path.is_empty() {
            parts.extend([b" ".as_slice(), path]);
        }
        if matches!(fault.stage, Stage::MetadataRemove | Stage::MetadataSet) {
            let name = fault.attribute.as_deref().ok_or(Error::UnclassifiedFault)?;
            if !name.is_empty() {
                parts.extend([b" ".as_slice(), name]);
            }
        }
        parts.extend([b": ".as_slice(), errno.as_bytes()]);
    }
    envelope(&parts.iter().copied(), fault.status(), limit)
}

pub(super) fn envelope<'a>(
    parts: &(impl Iterator<Item = &'a [u8]> + Clone),
    status: u16,
    limit: usize,
) -> Result<Vec<u8>, Error> {
    let code = status.to_string();
    go_json::bounded(limit as u64, |put| {
        put(b"{\"code\":")?;
        put(code.as_bytes())?;
        put(b",\"message\":\"")?;
        for part in parts.clone() {
            go_json::string_content(part, put)?;
        }
        put(b"\"}\n")
    })
    .map_err(|_| Error::ResponseLimit)
}
