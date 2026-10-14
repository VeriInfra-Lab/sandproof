use super::*;
use crate::file_download::LinuxErrno;
use std::os::unix::fs::symlink;

#[test]
fn resolution_preserves_observed_path_bytes_without_cleaning_missing_components() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file"), b"x").unwrap();
    symlink("missing", root.path().join("broken")).unwrap();
    symlink("loop", root.path().join("loop")).unwrap();
    let raw = format!("/../{}/missing", root.path().display());
    for (requested, stage, observed, errno) in [
        (
            root.path().join("missing/../file"),
            FaultStage::ResolveStat,
            root.path().join("missing"),
            Some(LinuxErrno(2)),
        ),
        (
            root.path().join("broken"),
            FaultStage::ResolveStat,
            root.path().join("missing"),
            Some(LinuxErrno(2)),
        ),
        (
            root.path().join("file/"),
            FaultStage::ResolveNotDirectory,
            root.path().join("file"),
            None,
        ),
        (
            root.path().join("loop"),
            FaultStage::ResolveCycle,
            root.path().join("loop"),
            None,
        ),
        (
            PathBuf::from(&raw),
            FaultStage::ResolveStat,
            PathBuf::from(raw.replace("//", "/")),
            Some(LinuxErrno(2)),
        ),
    ] {
        let failure = resolve_detailed(&requested, 4096).unwrap_err();
        let fault = failure.fault.unwrap();
        assert_eq!(fault.stage, stage);
        assert_eq!(fault.path, observed.as_os_str().as_bytes());
        assert_eq!(fault.errno, errno);
    }
    assert_eq!(
        resolve_detailed(root.path(), 1).unwrap_err().error,
        FileError::ResourceExhausted
    );
}

#[test]
fn link_replaced_after_lstat_reports_original_readlink_stage() {
    let root = tempfile::tempdir().unwrap();
    let link = root.path().join("link");
    symlink("target", &link).unwrap();
    let failure = resolve_with(&link, 4096, |path| {
        assert_eq!(path, link);
        fs::remove_file(path).unwrap();
        fs::write(path, b"replaced").unwrap();
    })
    .unwrap_err();
    let fault = failure.fault.unwrap();
    assert_eq!(fault.stage, FaultStage::ResolveReadlink);
    assert_eq!(fault.path, link.as_os_str().as_bytes());
    assert_eq!(fault.errno, Some(LinuxErrno(22)));
    assert_eq!(fs::read(link).unwrap(), b"replaced");
}

#[test]
fn original_cycle_phrase_checks_not_found_before_path_text() {
    let requested = Path::new("/requested");
    let observed = Path::new("/too many links");
    for (errno, expected) in [(2, FaultStage::ResolveStat), (13, FaultStage::ResolveCycle)] {
        let failure = resolve_error(
            requested,
            FaultStage::ResolveStat,
            observed,
            &std::io::Error::from_raw_os_error(errno),
        );
        assert_eq!(failure.fault.unwrap().stage, expected);
    }
    let failure = resolve_error(
        requested,
        FaultStage::ResolveReadlink,
        Path::new("/link"),
        &std::io::Error::from_raw_os_error(31),
    );
    assert_eq!(failure.fault.unwrap().stage, FaultStage::ResolveCycle);
}
