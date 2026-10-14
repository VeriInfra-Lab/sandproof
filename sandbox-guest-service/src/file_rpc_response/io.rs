//! Fixed envd f455fd58 filesystem/{utils,dir,move,remove}.go and permissions/path.go.
//! Apache-2.0: sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
use super::{Error, Failure, Limits, Owner, Response, StatusCode, send_encoded};
use crate::{go_json, go_quote};
use sandproof_guest_runtime::{
    file_download::LinuxErrno,
    file_protocol::{FaultContext, FaultStage, FileFault},
};
use tokio::time::Instant;

/// Send an actual bounded filesystem observation, using identity JSON.
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
        FaultStage::EntryStat
        | FaultStage::Rename
        | FaultStage::ResolveStat
        | FaultStage::ResolveReadlink
        | FaultStage::DirectoryStat
            if fault.errno == Some(LinuxErrno(2)) =>
        {
            (StatusCode::NOT_FOUND, b"not_found")
        }
        FaultStage::MakeDirExists => (StatusCode::CONFLICT, b"already_exists"),
        FaultStage::MakeDirNotDirectory | FaultStage::DirectoryNotDirectory => {
            (StatusCode::BAD_REQUEST, b"invalid_argument")
        }
        FaultStage::ResolveCycle => (StatusCode::BAD_REQUEST, b"failed_precondition"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, b"internal"),
    }
}

fn message_parts(fault: &FileFault) -> (&'static [u8], &'static [u8]) {
    match fault.stage {
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
        FaultStage::ResolveStat | FaultStage::ResolveReadlink => (
            if fault.errno == Some(LinuxErrno(2)) {
                b"path not found: "
            } else {
                b"error resolving symlink: "
            },
            if fault.stage == FaultStage::ResolveStat {
                b"lstat"
            } else {
                b"readlink"
            },
        ),
        FaultStage::ResolveNotDirectory => (b"error resolving symlink: not a directory", b""),
        FaultStage::ResolveCycle => (b"cyclic symlink or chain >255 links at ", b""),
        FaultStage::DirectoryStat => (
            if fault.errno == Some(LinuxErrno(2)) {
                b"directory not found: "
            } else {
                b"error getting file info: "
            },
            b"stat",
        ),
        FaultStage::DirectoryNotDirectory => (b"path is not a directory: ", b""),
        FaultStage::WalkStat => (b"", b"lstat"),
        FaultStage::WalkOpen => (b"", b"open"),
        FaultStage::WalkRead => (b"", b"readdirent"),
        FaultStage::WalkType => (b"", b"fstatat"),
        FaultStage::WalkEntryStat => (b"internal: error getting file info: ", b"lstat"),
        FaultStage::RemoveAll => (b"error removing file or directory: ", b"RemoveAll"),
        FaultStage::RemoveParentOpen => (b"error removing file or directory: ", b"open"),
        FaultStage::RemoveUnlink => (b"error removing file or directory: ", b"unlinkat"),
        FaultStage::RemoveOpenDir => (b"error removing file or directory: ", b"openfdat"),
        FaultStage::RemoveRead => (b"error removing file or directory: ", b"readdirnames"),
    }
}

fn payload(fault: &FileFault, limit: usize) -> Result<Vec<u8>, Error> {
    let (prefix, operation) = message_parts(fault);
    let errno = if operation.is_empty() {
        None
    } else {
        Some(fault.errno.ok_or(Error::UnclassifiedFault)?.message())
    };
    let context = context(fault)?;
    go_json::bounded(limit as u64, |put| {
        put(br#"{"code":""#)?;
        put(classification(fault).1)?;
        put(br#"","message":""#)?;
        let mut message = |bytes: &[u8]| go_json::string_content(bytes, put);
        if let Some(root) =
            context.filter(|_| matches!(fault.context, Some(FaultContext::WalkRoot(_))))
        {
            message(b"error reading directory ")?;
            message(root)?;
            message(b": ")?;
        }
        message(prefix)?;
        if !operation.is_empty() {
            message(operation)?;
            message(b" ")?;
        }
        match fault.stage {
            FaultStage::ResolveCycle => go_quote::quoted(&fault.path, &mut message)?,
            FaultStage::ResolveNotDirectory => {}
            _ => message(&fault.path)?,
        }
        if let Some(destination) = context.filter(|_| fault.stage == FaultStage::Rename) {
            message(b" ")?;
            message(destination)?;
        }
        if let Some(name) = context.filter(|_| fault.stage == FaultStage::RemoveRead) {
            message(b": readdirent ")?;
            message(name)?;
        }
        if let Some(errno) = &errno {
            message(b": ")?;
            message(errno.as_bytes())?;
        }
        put(br#""}"#)
    })
    .map_err(|_| Error::ResponseLimit)
}

fn context(fault: &FileFault) -> Result<Option<&[u8]>, Error> {
    match (&fault.stage, &fault.context) {
        (FaultStage::Rename, Some(FaultContext::RenameDestination(path)))
        | (FaultStage::RemoveRead, Some(FaultContext::RemoveReadName(path)))
        | (
            FaultStage::WalkStat
            | FaultStage::WalkOpen
            | FaultStage::WalkRead
            | FaultStage::WalkType
            | FaultStage::WalkEntryStat,
            Some(FaultContext::WalkRoot(path)),
        ) => Ok(Some(path)),
        (
            FaultStage::Rename
            | FaultStage::RemoveRead
            | FaultStage::WalkStat
            | FaultStage::WalkOpen
            | FaultStage::WalkRead
            | FaultStage::WalkType
            | FaultStage::WalkEntryStat,
            _,
        )
        | (_, Some(_)) => Err(Error::UnclassifiedFault),
        (_, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests;
