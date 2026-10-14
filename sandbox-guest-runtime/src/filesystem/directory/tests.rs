use super::*;
use crate::{file_download::LinuxErrno, file_protocol::FaultContext, filesystem::FileLimits};
use std::os::unix::{ffi::OsStrExt, fs::symlink};

fn budget() -> Budget {
    Budget {
        limits: FileLimits {
            path_bytes: 4096,
            entries: 64,
            retained_bytes: 65536,
            visited: 128,
            xattr_bytes: 65536,
            account_bytes: 65536,
        },
        visited: 0,
        entries: 0,
        bytes: 0,
    }
}
#[test]
fn root_removal_after_resolution_keeps_directory_stat_observation() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("dir");
    fs::create_dir(&dir).unwrap();
    let failure = list_with(&dir, 1, &mut budget(), |resolved| {
        fs::remove_dir(resolved).unwrap();
    })
    .unwrap_err();
    assert_eq!(failure.error, FileError::NotFound);
    let fault = failure.fault.unwrap();
    assert_eq!(fault.stage, FaultStage::DirectoryStat);
    assert_eq!(fault.path, dir.as_os_str().as_bytes());
    assert_eq!(fault.errno, Some(LinuxErrno(2)));
}
#[test]
fn walk_root_lstat_does_not_follow_a_replacement_link() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("checked");
    let outside = root.path().join("outside");
    fs::create_dir(&dir).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"x").unwrap();
    assert!(fs::metadata(&dir).unwrap().is_dir());
    fs::remove_dir(&dir).unwrap();
    symlink(&outside, &dir).unwrap();
    assert!(walk(&dir, &dir, 1, &mut budget()).unwrap().is_empty());
    fs::remove_file(&dir).unwrap();
    let fault = walk(&dir, &dir, 1, &mut budget())
        .unwrap_err()
        .fault
        .unwrap();
    assert_eq!(fault.stage, FaultStage::WalkStat);
    assert_eq!(fault.errno, Some(LinuxErrno(2)));
    assert!(
        matches!(fault.context, Some(FaultContext::WalkRoot(path)) if path == dir.as_os_str().as_bytes())
    );
}
#[test]
fn traversal_failure_discards_partial_entries_and_retains_original_root() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("walk");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("a"), b"a").unwrap();
    fs::write(dir.join("b"), b"b").unwrap();
    let mut work = budget();
    let failure = walk_with(&dir, &dir, 1, &mut work, |path| {
        if path.ends_with("b") {
            fs::rename(&dir, root.path().join("original")).unwrap();
            fs::write(&dir, b"replacement").unwrap();
        }
    })
    .unwrap_err();
    assert_eq!(work.entries, 1);
    let fault = failure.fault.unwrap();
    assert_eq!(fault.stage, FaultStage::WalkEntryStat);
    assert_eq!(fault.errno, Some(LinuxErrno(20)));
    assert_eq!(fault.path, dir.join("b").as_os_str().as_bytes());
    assert!(
        matches!(fault.context, Some(FaultContext::WalkRoot(path)) if path == dir.as_os_str().as_bytes())
    );
    let fault = match children(&dir, Path::new(""), 1, &mut budget(), root.path()) {
        Err(failure) => failure.fault.unwrap(),
        Ok(_) => panic!("non-directory was read"),
    };
    assert_eq!(fault.stage, FaultStage::WalkOpen);
    assert_eq!(fault.errno, Some(LinuxErrno(20)));
}
