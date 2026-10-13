#![cfg(target_os = "linux")]
use sandproof_guest_runtime::{
    accounts::{AccountError, ownership_from_files},
    file_protocol::{Entry, FileType, Request},
    filesystem::{FileError, FileLimits, FileResponse, Ownership, execute},
};
use std::fs;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{MetadataExt, PermissionsExt, symlink},
};
use std::path::Path;

fn limits() -> FileLimits {
    FileLimits {
        path_bytes: 4096,
        entries: 4096,
        retained_bytes: 1_048_576,
        visited: 16384,
        xattr_bytes: 65536,
        account_bytes: 1_048_576,
    }
}
fn owner() -> Ownership {
    Ownership {
        uid: rustix::process::geteuid().as_raw(),
        gid: rustix::process::getegid().as_raw(),
    }
}
fn p(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}
#[expect(
    clippy::needless_pass_by_value,
    reason = "test requests are one-use owned fixtures"
)]
fn run(request: Request) -> Result<FileResponse, FileError> {
    execute(&request, owner(), limits())
}
fn stat(path: &Path) -> Entry {
    let FileResponse::Entry(entry) = run(Request::Stat(p(path))).unwrap() else {
        panic!()
    };
    entry
}

#[test]
fn real_metadata_preserves_link_attributes_and_target_type() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("snow 雪 ");
    let link = dir.path().join("link");
    fs::write(&file, b"\x00\xffabc").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o6750)).unwrap();
    symlink(file.file_name().unwrap(), &link).unwrap();
    let normal = stat(&file);
    let linked = stat(&link);
    assert_eq!(normal.size, 5);
    assert_eq!(normal.name, "snow 雪 ");
    assert_eq!(normal.permissions, "ugrwxr-x---");
    assert_eq!(linked.kind, FileType::File);
    assert_eq!(linked.mode, 0o750);
    assert_eq!(linked.permissions, "Lrwxrwxrwx");
    assert_eq!(
        linked.size,
        i64::try_from(file.file_name().unwrap().len()).unwrap()
    );
    assert_eq!(linked.symlink_target, Some(p(&file)));
    fs::remove_file(&file).unwrap();
    let broken = stat(&link);
    assert_eq!(broken.kind, FileType::Unspecified);
    assert_eq!(broken.mode, 0);
    assert_eq!(broken.symlink_target, Some(p(&link)));
    assert_eq!(run(Request::Stat(p(&file))), Err(FileError::NotFound));
}

#[test]
fn file_kind_timestamp_and_invalid_utf8_are_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    fs::write(&file, b"").unwrap();
    let times = rustix::fs::Timestamps {
        last_access: rustix::fs::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        last_modification: rustix::fs::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
    };
    rustix::fs::utimensat(rustix::fs::CWD, &file, &times, rustix::fs::AtFlags::empty()).unwrap();
    assert_eq!(stat(&file).modified_time, None);
    let fifo = dir.path().join("fifo");
    rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, rustix::fs::Mode::RUSR).unwrap();
    assert_eq!(stat(&fifo).kind, FileType::Unspecified);
    assert_eq!(stat(&fifo).permissions, "pr--------");
    let invalid = dir.path().join(std::ffi::OsStr::from_bytes(b"bad\xff"));
    fs::write(invalid, b"").unwrap();
    assert_eq!(
        run(Request::ListDir {
            path: p(dir.path()),
            depth: 1
        }),
        Err(FileError::Internal)
    );
}

#[test]
fn metadata_filters_namespace_and_bad_utf8_and_limits_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    fs::write(&file, b"").unwrap();
    for (key, value) in [
        ("user.e2b.good", b"snow \xe9\x9b\xaa".as_slice()),
        ("user.e2b.bad", b"\xff"),
        ("user.other", b"hidden"),
    ] {
        rustix::fs::setxattr(&file, key, value, rustix::fs::XattrFlags::empty()).unwrap();
    }
    let entry = stat(&file);
    assert_eq!(entry.metadata.len(), 1);
    assert_eq!(entry.metadata["good"], "snow 雪");
    let link = dir.path().join("link");
    symlink(&file, &link).unwrap();
    assert_eq!(stat(&link).metadata, entry.metadata);
    let mut bounded = limits();
    bounded.xattr_bytes = 1;
    assert_eq!(
        execute(&Request::Stat(p(&file)), owner(), bounded),
        Err(FileError::ResourceExhausted)
    );
}

#[test]
fn directory_creation_existing_objects_and_lexical_parent_failures() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a/b");
    let FileResponse::Entry(entry) = run(Request::MakeDir(p(&nested))).unwrap() else {
        panic!()
    };
    assert_eq!(entry.kind, FileType::Directory);
    assert_eq!(fs::metadata(&nested).unwrap().uid(), owner().uid);
    assert_eq!(
        run(Request::MakeDir(p(&nested))),
        Err(FileError::AlreadyExists)
    );
    let file = dir.path().join("file");
    fs::write(&file, b"").unwrap();
    assert_eq!(
        run(Request::MakeDir(p(&file))),
        Err(FileError::InvalidArgument)
    );
    assert_eq!(
        run(Request::MakeDir(p(&dir.path().join("absent/../new")))),
        Err(FileError::Internal)
    );
    assert!(!dir.path().join("absent").exists());
    let link = dir.path().join("link");
    symlink(&nested, &link).unwrap();
    assert_eq!(
        run(Request::MakeDir(p(&link))),
        Err(FileError::AlreadyExists)
    );
}

#[test]
fn move_creates_parents_overwrites_and_keeps_partial_effects() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let destination = dir.path().join("a/b/dest");
    fs::write(&source, b"original").unwrap();
    run(Request::Move {
        source: p(&source),
        destination: p(&destination),
    })
    .unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"original");
    fs::write(&source, b"replacement").unwrap();
    run(Request::Move {
        source: p(&source),
        destination: p(&destination),
    })
    .unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"replacement");
    let missing_dest = dir.path().join("left/by/failure");
    assert_eq!(
        run(Request::Move {
            source: p(&source),
            destination: p(&missing_dest)
        }),
        Err(FileError::NotFound)
    );
    assert!(missing_dest.parent().unwrap().is_dir());
    let mut bounded = limits();
    bounded.retained_bytes = 1;
    assert_eq!(
        execute(
            &Request::Move {
                source: p(&destination),
                destination: p(&source)
            },
            owner(),
            bounded
        ),
        Err(FileError::ResourceExhausted)
    );
    assert_eq!(fs::read(&source).unwrap(), b"replacement");
    assert!(!destination.exists());
}

#[test]
fn list_is_lexical_depth_first_and_preserves_requested_alias() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir_all(tree.join("a/sub")).unwrap();
    fs::write(tree.join("a/x"), b"x").unwrap();
    fs::write(tree.join("z"), b"z").unwrap();
    symlink("a", tree.join("b")).unwrap();
    let alias = dir.path().join("alias");
    symlink(&tree, &alias).unwrap();
    let list = |depth| {
        let FileResponse::Entries(entries) = run(Request::ListDir {
            path: p(&alias),
            depth,
        })
        .unwrap() else {
            panic!()
        };
        entries
    };
    assert_eq!(
        list(0).iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["a", "b", "z"]
    );
    let entries = list(2);
    assert_eq!(
        entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["a", "sub", "x", "b", "z"]
    );
    assert!(
        entries
            .iter()
            .all(|e| e.path.starts_with(alias.to_str().unwrap()))
    );
    assert_eq!(entries[3].symlink_target, Some(p(&tree.join("a"))));
    assert_eq!(
        run(Request::ListDir {
            path: p(&tree.join("z")),
            depth: 1
        }),
        Err(FileError::InvalidArgument)
    );
    let mut bounded = limits();
    bounded.entries = 1;
    assert_eq!(
        execute(
            &Request::ListDir {
                path: p(&tree),
                depth: 1
            },
            owner(),
            bounded
        ),
        Err(FileError::ResourceExhausted)
    );
}

#[test]
fn long_link_chains_and_cycles_match_go_resolution_not_kernel_limit() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("leaf"), b"").unwrap();
    for i in (0..50).rev() {
        symlink(
            if i == 49 {
                "tree".into()
            } else {
                format!("l{}", i + 1)
            },
            dir.path().join(format!("l{i}")),
        )
        .unwrap();
    }
    assert_eq!(stat(&dir.path().join("l0")).kind, FileType::Directory);
    assert!(
        matches!(run(Request::ListDir { path:p(&dir.path().join("l0")),depth:1 }), Ok(FileResponse::Entries(v)) if v.len()==1)
    );
    symlink("loop", dir.path().join("loop")).unwrap();
    assert_eq!(
        run(Request::ListDir {
            path: p(&dir.path().join("loop")),
            depth: 1
        }),
        Err(FileError::FailedPrecondition)
    );
    assert_eq!(stat(&dir.path().join("loop")).kind, FileType::Unspecified);
}

#[test]
fn remove_unlinks_links_and_tolerates_absent_but_not_non_directory_parent() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"safe").unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir_all(tree.join("a/b")).unwrap();
    symlink(&outside, tree.join("link")).unwrap();
    run(Request::Remove(p(&tree))).unwrap();
    assert!(!tree.exists());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"safe");
    run(Request::Remove(p(&tree))).unwrap();
    assert_eq!(
        run(Request::Remove(p(&outside.join("keep/missing")))),
        Err(FileError::Internal)
    );
    assert_eq!(
        run(Request::Remove(format!("{}/.", outside.display()))),
        Err(FileError::Internal)
    );
    assert!(outside.join("keep").exists());
}

#[test]
fn removal_can_delete_an_unreadable_empty_child_directory() {
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    let child = tree.join("unreadable");
    fs::create_dir_all(&child).unwrap();
    fs::set_permissions(&child, fs::Permissions::from_mode(0o000)).unwrap();
    fs::write(tree.join("sibling"), b"content").unwrap();
    run(Request::Remove(p(&tree))).unwrap();
    assert!(!tree.exists());
}

#[test]
fn remove_reopens_after_large_batches_and_budget_failure_is_partial() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir(&tree).unwrap();
    for i in 0..1100 {
        fs::write(tree.join(i.to_string()), b"").unwrap();
    }
    let mut bounded = limits();
    bounded.visited = 2;
    assert_eq!(
        execute(&Request::Remove(p(&tree)), owner(), bounded),
        Err(FileError::ResourceExhausted)
    );
    let left = fs::read_dir(&tree).unwrap().count();
    assert!(left > 0 && left < 1100);
    run(Request::Remove(p(&tree))).unwrap();
    assert!(!tree.exists());
}

#[test]
fn budgets_and_unresolved_paths_reject_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("new");
    let mut bounded = limits();
    bounded.visited = 0;
    assert_eq!(
        execute(&Request::MakeDir(p(&target)), owner(), bounded),
        Err(FileError::InvalidArgument)
    );
    assert!(!target.exists());
    assert_eq!(
        run(Request::MakeDir("relative".into())),
        Err(FileError::InvalidArgument)
    );
    assert_eq!(
        run(Request::Stat(format!("{}\0", dir.path().display()))),
        Err(FileError::Internal)
    );
    let mut bounded = limits();
    bounded.account_bytes = 1;
    assert_eq!(
        execute(&Request::Stat(p(dir.path())), owner(), bounded),
        Err(FileError::ResourceExhausted)
    );
}

#[test]
fn ownership_display_requires_canonical_id_and_falls_back_numerically() {
    let passwd = b"bad:x:0042:3::/:sh\nnis:x:42:bad::/:sh\n+skip:x:42:3::/:sh\nreal:x:42:3::/:sh\n";
    assert_eq!(
        ownership_from_files(42, 3, passwd, b"group:x:3:\n", 4096).unwrap(),
        ("real".into(), "group".into())
    );
    assert_eq!(
        ownership_from_files(43, 4, passwd, b"", 4096).unwrap(),
        ("43".into(), "4".into())
    );
    assert_eq!(
        ownership_from_files(42, 3, passwd, b"", 1),
        Err(AccountError::ResourceExhausted)
    );
    assert_eq!(
        ownership_from_files(42, 3, b"bad\xff:x:42:3::/:sh", b"", 4096),
        Err(AccountError::Unavailable)
    );
}
