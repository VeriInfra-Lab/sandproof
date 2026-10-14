//! Go 1.26.8 os.rename's directory precheck before the actual rename syscall.
//! Go Authors, BSD: ../multipart/GO-LICENSE.
use super::{FaultStage, FileFailure};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

pub(super) fn rename(source: &str, destination: &str) -> Result<(), FileFailure> {
    let failure = |error: &std::io::Error| {
        FileFailure::io(
            FaultStage::Rename,
            Path::new(source),
            Some(Path::new(destination)),
            error,
        )
    };
    if let Ok(target) = fs::symlink_metadata(destination)
        && target.is_dir()
    {
        // Preserve Go's source-error priority and exact string equality. Path
        // equality would incorrectly normalize aliases such as a final '/.'.
        let original = fs::symlink_metadata(source).map_err(|error| failure(&error))?;
        if source == destination || original.dev() != target.dev() || original.ino() != target.ino()
        {
            // This is the original library's observed directory precondition,
            // not an errno from a syscall we attempted or an automatic retry.
            return Err(failure(&std::io::Error::from_raw_os_error(17)));
        }
    }
    fs::rename(source, destination).map_err(|error| failure(&error))
}
