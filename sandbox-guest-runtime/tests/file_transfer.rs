//! Real descriptor/body checks. No HTTP, mounted VM, or whole-service evidence.
#![cfg(target_os = "linux")]
use sandproof_guest_runtime::filesystem::{
    FileLimits, Ownership,
    transfer::{
        DownloadFile, DownloadPath, DownloadSource, TransferIoError, TransferLimits, UploadFile,
        validate_metadata,
    },
};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};

fn limits() -> TransferLimits {
    TransferLimits {
        file: FileLimits {
            path_bytes: 4096,
            entries: 32,
            retained_bytes: 65536,
            visited: 1024,
            xattr_bytes: 65536,
            account_bytes: 65536,
        },
        chunk_bytes: 3,
        total_bytes: 1024,
    }
}
fn owner() -> Ownership {
    Ownership {
        uid: rustix::process::geteuid().as_raw(),
        gid: rustix::process::getegid().as_raw(),
    }
}
fn upload(path: &std::path::Path, expected: Option<u64>) -> UploadFile {
    UploadFile::open(
        path.to_str().unwrap().into(),
        owner(),
        BTreeMap::new(),
        expected,
        limits(),
    )
    .unwrap()
}

#[test]
fn binary_empty_nested_and_symlink_bodies_use_real_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/file");
    let mut write = upload(&path, Some(5));
    write.write_chunk(b"\0\xffa").unwrap();
    write.write_chunk(b"bc").unwrap();
    assert_eq!(write.finish().unwrap().written, 5);
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let mut read = DownloadFile::open(link.to_str().unwrap(), limits()).unwrap();
    assert_eq!(read.length(), 5);
    assert_eq!(read.read_chunk().unwrap().unwrap(), b"\0\xffa");
    assert_eq!(read.read_chunk().unwrap().unwrap(), b"bc");
    assert_eq!(read.read_chunk().unwrap(), None);
    upload(&link, Some(0)).finish().unwrap();
    assert!(fs::read(&path).unwrap().is_empty());
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}

#[test]
fn incomplete_and_failed_uploads_leave_prefix_without_metadata_success() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"previous").unwrap();
    let mut write = upload(&path, Some(4));
    write.write_chunk(b"ab").unwrap();
    assert_eq!(write.finish().unwrap_err(), TransferIoError::Incomplete);
    assert_eq!(fs::read(&path).unwrap(), b"ab");
    let mut write = upload(&path, Some(2));
    write.write_chunk(b"a").unwrap();
    assert_eq!(
        write.write_chunk(b"bc"),
        Err(TransferIoError::ResourceExhausted)
    );
    assert_eq!(write.write_chunk(b"b"), Err(TransferIoError::Incomplete));
    assert_eq!(write.finish().unwrap_err(), TransferIoError::Incomplete);
    assert_eq!(fs::read(&path).unwrap(), b"a");
    let mut write = upload(&path, None);
    write.write_chunk(b"xyz").unwrap();
    drop(write); // No EOF/success, no rollback.
    assert_eq!(fs::read(path).unwrap(), b"xyz");
}

#[test]
fn invalid_metadata_and_known_oversize_do_not_truncate() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"previous").unwrap();
    let invalid = BTreeMap::from([("bad".into(), "\n".into())]);
    assert!(matches!(
        UploadFile::open(
            path.to_str().unwrap().into(),
            owner(),
            invalid,
            None,
            limits()
        ),
        Err(TransferIoError::InvalidArgument)
    ));
    assert!(matches!(
        UploadFile::open(
            path.to_str().unwrap().into(),
            owner(),
            BTreeMap::new(),
            Some(1025),
            limits()
        ),
        Err(TransferIoError::ResourceExhausted)
    ));
    assert_eq!(fs::read(path).unwrap(), b"previous");
    assert!(validate_metadata(&BTreeMap::from([("k".into(), "x".repeat(4086))])).is_ok());
    assert_eq!(
        validate_metadata(&BTreeMap::from([("k".into(), "x".repeat(4087))])),
        Err(TransferIoError::InvalidArgument)
    );
}

#[test]
fn metadata_replacement_clears_old_keys_but_keeps_unrelated_xattrs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let values = BTreeMap::from([
        ("first".into(), "one".into()),
        ("second".into(), "two".into()),
    ]);
    let write = UploadFile::open(
        path.to_str().unwrap().into(),
        owner(),
        values.clone(),
        Some(0),
        limits(),
    )
    .unwrap();
    assert_eq!(write.finish().unwrap().metadata, values);
    rustix::fs::setxattr(
        &path,
        "user.other",
        b"keep",
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    let values = BTreeMap::from([("second".into(), "changed".into())]);
    let write = UploadFile::open(
        path.to_str().unwrap().into(),
        owner(),
        values.clone(),
        Some(0),
        limits(),
    )
    .unwrap();
    assert_eq!(write.finish().unwrap().metadata, values);
    assert!(upload(&path, Some(0)).finish().unwrap().metadata.is_empty());
    let mut out = [0; 4];
    assert_eq!(
        rustix::fs::getxattr(&path, "user.other", &mut out).unwrap(),
        4
    );
    assert_eq!(&out, b"keep");
}

#[test]
fn metadata_mutation_uses_original_descriptor_after_path_replacement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let old = root.path().join("old");
    let values = BTreeMap::from([("owned".into(), "original".into())]);
    let mut write = UploadFile::open(
        path.to_str().unwrap().into(),
        owner(),
        values,
        Some(3),
        limits(),
    )
    .unwrap();
    fs::rename(&path, &old).unwrap();
    fs::write(&path, b"replacement").unwrap();
    write.write_chunk(b"abc").unwrap();
    // Response lookup follows the original path, as in fixed envd; mutation
    // itself remains on the held inode and must not touch its replacement.
    assert!(write.finish().unwrap().metadata.is_empty());
    assert_eq!(fs::read(&old).unwrap(), b"abc");
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    let mut value = [0; 16];
    assert_eq!(
        rustix::fs::getxattr(&old, "user.e2b.owned", &mut value).unwrap(),
        8
    );
    assert_eq!(
        rustix::fs::getxattr(&path, "user.e2b.owned", &mut value),
        Err(rustix::io::Errno::NODATA)
    );
}

#[test]
fn exhausted_result_budget_fails_after_effect_without_truncating_metadata() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let mut config = limits();
    config.file.retained_bytes = path.to_str().unwrap().len();
    let mut write = UploadFile::open(
        path.to_str().unwrap().into(),
        owner(),
        BTreeMap::from([("key".into(), "value".into())]),
        Some(3),
        config,
    )
    .unwrap();
    write.write_chunk(b"abc").unwrap();
    assert_eq!(
        write.finish().unwrap_err(),
        TransferIoError::ResourceExhausted
    );
    assert_eq!(fs::read(&path).unwrap(), b"abc");
    let mut value = [0; 5];
    assert_eq!(
        rustix::fs::getxattr(&path, "user.e2b.key", &mut value).unwrap(),
        5
    );
    assert_eq!(&value, b"value");
}

#[test]
fn download_shrink_is_incomplete_growth_is_not_appended_and_replacement_is_not_adopted() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"abcdef").unwrap();
    let mut read = DownloadFile::open(path.to_str().unwrap(), limits()).unwrap();
    assert_eq!(read.read_chunk().unwrap().unwrap(), b"abc");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(3)
        .unwrap();
    assert_eq!(read.read_chunk(), Err(TransferIoError::Incomplete));
    fs::write(&path, b"abcdef").unwrap();
    let mut read = DownloadFile::open(path.to_str().unwrap(), limits()).unwrap();
    fs::rename(&path, root.path().join("old")).unwrap();
    fs::write(&path, b"new").unwrap();
    assert_eq!(read.read_chunk().unwrap().unwrap(), b"abc");
    assert_eq!(read.read_chunk().unwrap().unwrap(), b"def");
    assert_eq!(read.read_chunk().unwrap(), None);
    let mut read = DownloadFile::open(path.to_str().unwrap(), limits()).unwrap();
    fs::write(&path, b"new-tail").unwrap();
    assert_eq!(read.read_chunk().unwrap().unwrap(), b"new");
    assert_eq!(read.read_chunk().unwrap(), None);
}

#[test]
fn gzip_source_observes_growth_shrink_and_original_descriptor_without_length_seek() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"abc").unwrap();
    let mut source = DownloadSource::open(path.to_str().unwrap(), limits()).unwrap();
    let mut first = [0; 3];
    source.read_exact(&mut first).unwrap();
    assert_eq!(&first, b"abc");
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"tail")
        .unwrap();
    let mut rest = Vec::new();
    source.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, b"tail");

    let mut source = DownloadSource::open(path.to_str().unwrap(), limits()).unwrap();
    source.read_exact(&mut first).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(3)
        .unwrap();
    assert_eq!(source.read(&mut first).unwrap(), 0);

    let link = root.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let mut source = DownloadSource::open(link.to_str().unwrap(), limits()).unwrap();
    fs::rename(&path, root.path().join("old")).unwrap();
    fs::write(&path, b"replacement").unwrap();
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abc");

    // procfs advertises zero bytes yet has a readable body; gzip does not seek
    // or pre-reject by stat length. The service codec owns the body budget.
    let mut source = DownloadSource::open("/proc/self/status", limits()).unwrap();
    assert!(source.read(&mut first).unwrap() > 0);
    assert!(matches!(
        DownloadSource::open(root.path().to_str().unwrap(), limits()),
        Err(TransferIoError::InvalidArgument)
    ));
    assert!(matches!(
        DownloadSource::open(root.path().join("missing").to_str().unwrap(), limits()),
        Err(TransferIoError::NotFound)
    ));
}

#[test]
fn deferred_open_preserves_preopen_metadata_without_restat_or_inode_pinning() {
    use std::os::unix::fs::MetadataExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source");
    fs::write(&path, b"old").unwrap();
    let pending = DownloadPath::inspect(path.to_str().unwrap().into(), limits()).unwrap();
    let old_inode = pending.metadata().ino();
    fs::rename(&path, root.path().join("old")).unwrap();
    fs::write(&path, b"replacement").unwrap();
    assert_ne!(fs::metadata(&path).unwrap().ino(), old_inode);
    let (mut source, metadata) = pending.open().unwrap();
    assert_eq!(metadata.ino(), old_inode);
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"replacement");
    let pending = DownloadPath::inspect(path.to_str().unwrap().into(), limits()).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(matches!(pending.open(), Err(TransferIoError::Internal)));
}
