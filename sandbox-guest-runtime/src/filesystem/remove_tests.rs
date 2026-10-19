use super::*;
use crate::filesystem::FileLimits;

#[test]
fn replacement_link_never_redirects_an_open_removal_walk() {
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    let displaced = root.path().join("displaced");
    let outside = root.path().join("outside");
    std::fs::create_dir(&tree).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(tree.join("old"), b"old").unwrap();
    std::fs::write(outside.join("keep"), b"safe").unwrap();
    let parent = rustix::fs::open(root.path(), OPEN_DIR, Mode::empty()).unwrap();
    let opened = enter(parent.as_fd(), OsStr::new("tree")).unwrap().unwrap();
    // Deterministic interleaving immediately after the real no-follow open.
    std::fs::rename(&tree, &displaced).unwrap();
    std::os::unix::fs::symlink(&outside, &tree).unwrap();
    let mut budget = Budget {
        limits: FileLimits {
            path_bytes: 4096,
            entries: 10,
            retained_bytes: 4096,
            visited: 100,
            xattr_bytes: 4096,
            account_bytes: 4096,
        },
        visited: 0,
        entries: 0,
        bytes: 0,
    };
    assert_eq!(
        walk(parent.as_fd(), opened, &mut budget).unwrap_err().error,
        FileError::Internal
    );
    assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"safe");
    assert!(!displaced.join("old").exists());
    assert!(
        std::fs::symlink_metadata(&tree)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // An unopened child link is removed itself, without traversing target.
    assert!(enter(parent.as_fd(), OsStr::new("tree")).unwrap().is_none());
    assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"safe");
}
fn budget() -> Budget {
    Budget {
        limits: FileLimits {
            path_bytes: 4096,
            entries: 10,
            retained_bytes: 4096,
            visited: 100,
            xattr_bytes: 4096,
            account_bytes: 4096,
        },
        visited: 0,
        entries: 0,
        bytes: 0,
    }
}

#[test]
fn errors_keep_the_actual_operation_and_raw_split_path() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"keep").unwrap();
    std::fs::create_dir(root.path().join("dir")).unwrap();
    for (suffix, stage, errno, diagnostic) in [
        ("dir/.", FaultStage::RemoveAll, 22, "dir/."),
        ("file/child", FaultStage::RemoveUnlink, 20, "file/child"),
        (
            "file/child/deeper",
            FaultStage::RemoveParentOpen,
            20,
            "file/child",
        ),
        ("file//child///", FaultStage::RemoveParentOpen, 20, "file/"),
        ("nul\0", FaultStage::RemoveUnlink, 22, "nul\0"),
    ] {
        let path = format!("{}/{suffix}", root.path().display());
        let failure = remove(Path::new(&path), &mut budget()).unwrap_err();
        assert_eq!(failure.error, FileError::Internal);
        let fault = failure.fault.unwrap();
        assert_eq!(fault.stage, stage, "{path:?}");
        assert_eq!(fault.errno, Some(LinuxErrno(errno)));
        assert_eq!(
            fault.path,
            format!("{}/{diagnostic}", root.path().display()).as_bytes()
        );
    }
    let path = format!("///{}/file/child///", root.path().display());
    let fault = remove(Path::new(&path), &mut budget())
        .unwrap_err()
        .fault
        .unwrap();
    assert_eq!(
        fault.path,
        format!("{}/file/child", root.path().display()).as_bytes()
    );
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"keep");
    assert!(root.path().join("dir").is_dir());
    // Root is only split, never removed by this test.
    for (raw, parent, name) in [
        ("////", ".", "/"),
        ("//x///", "/", "x"),
        ("/a//b/", "/a/", "b"),
    ] {
        assert_eq!(
            split_path(raw.as_bytes()),
            (OsStr::new(parent), OsStr::new(name))
        );
    }
}

#[test]
fn first_recursive_error_survives_siblings_but_successful_removal_wins() {
    for delete_failed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let tree = root.path().join("tree");
        std::fs::create_dir_all(tree.join("nested")).unwrap();
        for name in ["first", "second", "ok"] {
            std::fs::write(tree.join("nested").join(name), b"data").unwrap();
        }
        let parent = rustix::fs::open(root.path(), OPEN_DIR, Mode::empty()).unwrap();
        let opened = enter(parent.as_fd(), OsStr::new("tree")).unwrap().unwrap();
        let mut first = None;
        let mut faults = 0;
        // Controlled failures in the real traversal; no directory-order assumption.
        let result = walk_with(
            parent.as_fd(),
            opened,
            &mut budget(),
            &mut |fd, name| {
                if matches!(name.to_str(), Some("first" | "second")) {
                    first.get_or_insert_with(|| name.to_owned());
                    faults += 1;
                    if delete_failed {
                        assert!(enter(fd, name)?.is_none());
                    }
                    Err(observed(FaultStage::RemoveUnlink, name, Errno::ACCESS))
                } else {
                    enter(fd, name)
                }
            },
            &mut Dir::next,
        );
        assert_eq!(faults, 2);
        assert!(!tree.join("nested/ok").exists());
        if delete_failed {
            assert!(result.is_ok());
            assert!(!tree.exists());
        } else {
            let fault = result.unwrap_err().fault.unwrap();
            assert_eq!(fault.stage, FaultStage::RemoveUnlink);
            assert_eq!(fault.errno, Some(LinuxErrno(13)));
            assert_eq!(
                fault.path,
                format!("tree/nested/{}", first.unwrap().to_str().unwrap()).as_bytes()
            );
            assert!(tree.join("nested/first").exists());
            assert!(tree.join("nested/second").exists());
        }
    }
}

#[test]
fn read_failure_overrides_old_child_error_and_missing_read_skips_rmdir() {
    for errno in [Errno::IO, Errno::NOENT] {
        let root = tempfile::tempdir().unwrap();
        let tree = root.path().join("tree");
        std::fs::create_dir(&tree).unwrap();
        let parent = rustix::fs::open(root.path(), OPEN_DIR, Mode::empty()).unwrap();
        let mut opened = enter(parent.as_fd(), OsStr::new("tree")).unwrap().unwrap();
        opened.failed_child(
            observed(FaultStage::RemoveUnlink, OsStr::new("old"), Errno::ACCESS),
            &budget(),
        );
        let result = walk_with(
            parent.as_fd(),
            opened,
            &mut budget(),
            &mut enter,
            &mut |_| Some(Err(errno)),
        );
        assert!(tree.exists()); // The empty directory would disappear if rmdir ran.
        if errno == Errno::NOENT {
            assert!(result.is_ok());
        } else {
            let error = prepend(result.unwrap_err(), OsStr::new("/parent"), &budget());
            let fault = error.fault.unwrap();
            assert_eq!(fault.stage, FaultStage::RemoveRead);
            assert_eq!(fault.path, b"/parent/tree");
            assert_eq!(fault.errno, Some(LinuxErrno(5)));
            assert!(
                matches!(fault.context, Some(FaultContext::RemoveReadName(name)) if name == b"tree")
            );
        }
    }
}

#[test]
fn diagnostic_budget_exhaustion_keeps_partial_deletion() {
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    std::fs::create_dir(&tree).unwrap();
    for name in ["bad", "ok"] {
        std::fs::write(tree.join(name), b"data").unwrap();
    }
    let parent = rustix::fs::open(root.path(), OPEN_DIR, Mode::empty()).unwrap();
    let opened = enter(parent.as_fd(), OsStr::new("tree")).unwrap().unwrap();
    let mut budget = budget();
    budget.limits.path_bytes = 4;
    let error = walk_with(
        parent.as_fd(),
        opened,
        &mut budget,
        &mut |fd, name| {
            if name == OsStr::new("bad") {
                Err(observed(FaultStage::RemoveUnlink, name, Errno::ACCESS))
            } else {
                enter(fd, name)
            }
        },
        &mut Dir::next,
    )
    .unwrap_err();
    assert_eq!(error.error, FileError::ResourceExhausted);
    assert!(error.fault.is_none());
    assert!(tree.join("bad").exists());
    assert!(!tree.join("ok").exists());
}
