//! Fixed envd f455fd58 filesystem/{utils,dir,move}.go and permissions/path.go.
//! Apache-2.0: sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
use super::{Error, Failure, Limits, Owner, Response, StatusCode, send_encoded};
use crate::go_json;
use sandproof_guest_runtime::{
    file_download::LinuxErrno,
    file_protocol::{FaultStage, FileFault},
};
use tokio::time::Instant;

/// Send an actual bounded `Stat`/`MakeDir`/`Move` observation, using identity JSON.
/// Path/errno data is captured by the worker before releasing authorization.
/// # Errors
/// Missing observations, invalid/insufficient response budget or original
/// expiry. Failure retains the same slots and never permits retrying file I/O.
pub async fn io_failure(
    fault: FileFault,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Failure> {
    let (status, _) = classification(&fault);
    send_encoded(owner, limits, deadline, status, true, move || {
        payload(&fault, limits.wire_bytes)
    })
    .await
}

fn classification(fault: &FileFault) -> (StatusCode, &'static [u8]) {
    match fault.stage {
        FaultStage::EntryStat | FaultStage::Rename if fault.errno == Some(LinuxErrno(2)) => {
            (StatusCode::NOT_FOUND, b"not_found")
        }
        FaultStage::MakeDirExists => (StatusCode::CONFLICT, b"already_exists"),
        FaultStage::MakeDirNotDirectory => (StatusCode::BAD_REQUEST, b"invalid_argument"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, b"internal"),
    }
}

fn payload(fault: &FileFault, limit: usize) -> Result<Vec<u8>, Error> {
    let (prefix, operation): (&[u8], &[u8]) = match fault.stage {
        FaultStage::EntryStat => (
            if fault.errno == Some(LinuxErrno(2)) {
                b"file not found: "
            } else {
                b"error getting file info: "
            },
            b"lstat",
        ),
        FaultStage::MakeDirStat => (b"error getting file info: ", b"stat"),
        FaultStage::MakeDirExists => (b"directory already exists: ", b""),
        FaultStage::MakeDirNotDirectory => {
            (b"path already exists but it is not a directory: ", b"")
        }
        FaultStage::Rename => (
            if fault.errno == Some(LinuxErrno(2)) {
                b"source file not found: "
            } else {
                b"error renaming: "
            },
            b"rename",
        ),
        FaultStage::ParentStat => (b"failed to stat directory: ", b"stat"),
        FaultStage::ParentNotDirectory => (b"path is a file: ", b""),
        FaultStage::ParentCreate => (b"failed to create directory: ", b"mkdir"),
        FaultStage::ParentChown => (b"failed to chown directory: ", b"chown"),
    };
    let errno = if operation.is_empty() {
        None
    } else {
        Some(fault.errno.ok_or(Error::UnclassifiedFault)?.message())
    };
    let destination = if fault.stage == FaultStage::Rename {
        Some(fault.destination.as_ref().ok_or(Error::UnclassifiedFault)?)
    } else {
        None
    };
    go_json::bounded(limit as u64, |put| {
        put(br#"{"code":""#)?;
        put(classification(fault).1)?;
        put(br#"","message":""#)?;
        let mut message = |bytes: &[u8]| go_json::string_content(bytes, put);
        message(prefix)?;
        if !operation.is_empty() {
            message(operation)?;
            message(b" ")?;
        }
        message(&fault.path)?;
        if let Some(destination) = destination {
            message(b" ")?;
            message(destination)?;
        }
        if let Some(errno) = &errno {
            message(b": ")?;
            message(errno.as_bytes())?;
        }
        put(br#""}"#)
    })
    .map_err(|_| Error::ResponseLimit)
}

#[cfg(test)]
mod tests;
