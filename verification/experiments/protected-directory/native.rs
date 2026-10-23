use super::{ProtectedFileError, directory};
use std::{fs, os::unix::fs::PermissionsExt};

fn metadata(mode: u32, file: bool) -> (tempfile::TempDir, fs::Metadata) {
    use std::os::unix::fs::MetadataExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("object");
    if file {
        fs::write(&path, b"fixture").unwrap();
    } else {
        fs::create_dir(&path).unwrap();
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    let metadata = fs::metadata(path).unwrap();
    assert_ne!(
        metadata.uid(),
        0,
        "ordinary fixtures must use a non-root owner"
    );
    (root, metadata)
}

fn allowed(metadata: &fs::Metadata, owner: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.is_dir()
        && (metadata.uid() == 0 || metadata.uid() == owner)
        && (metadata.mode() & 0o022 == 0 || (metadata.uid() == 0 && metadata.mode() & 0o1000 != 0))
}

fn assert_safe(metadata: &fs::Metadata, owner: u32) {
    assert!(
        directory(metadata, owner).is_err() || allowed(metadata, owner),
        "directory accepted a snapshot that violates the shared acceptance property"
    );
}

#[test]
fn own_directory_with_no_untrusted_writes_is_accepted() {
    use std::os::unix::fs::MetadataExt;
    for mode in [0o700, 0o750, 0o755, 0o711] {
        let (_root, metadata) = metadata(mode, false);
        let owner = metadata.uid();
        assert_eq!(directory(&metadata, owner), Ok(()));
        assert_safe(&metadata, owner);
    }
}

#[test]
fn different_non_root_owner_is_rejected() {
    use std::os::unix::fs::MetadataExt;
    let (_root, metadata) = metadata(0o700, false);
    let owner = metadata.uid().checked_add(1).unwrap();
    assert_eq!(
        directory(&metadata, owner),
        Err(ProtectedFileError::Untrusted)
    );
    assert_safe(&metadata, owner);
}

#[test]
fn regular_file_is_rejected_even_with_private_permissions() {
    use std::os::unix::fs::MetadataExt;
    let (_root, metadata) = metadata(0o600, true);
    let owner = metadata.uid();
    assert_eq!(
        directory(&metadata, owner),
        Err(ProtectedFileError::Untrusted)
    );
    assert_safe(&metadata, owner);
}

#[test]
fn writable_directory_cannot_be_accepted() {
    use std::os::unix::fs::MetadataExt;
    let (_root, metadata) = metadata(0o770, false);
    assert_safe(&metadata, metadata.uid());
}

#[test]
fn other_write_bit_is_rejected() {
    use std::os::unix::fs::MetadataExt;
    let (_root, metadata) = metadata(0o702, false);
    assert_eq!(
        directory(&metadata, metadata.uid()),
        Err(ProtectedFileError::Untrusted)
    );
    assert_safe(&metadata, metadata.uid());
}

#[test]
fn non_root_sticky_bit_does_not_allow_untrusted_writes() {
    use std::os::unix::fs::MetadataExt;
    let (_root, metadata) = metadata(0o1777, false);
    assert_eq!(
        directory(&metadata, metadata.uid()),
        Err(ProtectedFileError::Untrusted)
    );
    assert_safe(&metadata, metadata.uid());
}

#[test]
fn old_metadata_is_a_snapshot_not_a_future_path_promise() {
    use std::os::unix::fs::MetadataExt;
    let (root, old) = metadata(0o700, false);
    let path = root.path().join("object");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(old.mode() & 0o022, 0);
    assert_eq!(directory(&old, old.uid()), Ok(()));
    let current = fs::metadata(path).unwrap();
    assert_eq!(
        directory(&current, current.uid()),
        Err(ProtectedFileError::Untrusted)
    );
    assert_safe(&old, old.uid());
    assert_safe(&current, current.uid());
}
