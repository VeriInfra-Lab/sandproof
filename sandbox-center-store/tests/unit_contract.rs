#![cfg(unix)]

use sandproof_center_store::{RedisEndpoint, StoreConfig, StoreError, WriterLease};
use sandproof_protocol::DeploymentId;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    time::Duration,
};

fn deployment() -> DeploymentId {
    "component-deployment".try_into().unwrap()
}

fn private_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

#[test]
fn configuration_has_checked_bytes_time_identity_and_credentials() {
    for (block, memory, timeout) in [
        (0, 64 << 20, 1),
        (1, 64 << 20, 1),
        (usize::MAX, 64 << 20, 1),
        (1024, 1, 1),
        (1024, 64 << 20, 0),
        (1024, 64 << 20, 61),
    ] {
        assert!(matches!(
            StoreConfig::new(deployment(), block, memory, Duration::from_secs(timeout)),
            Err(StoreError::Configuration)
        ));
    }
    assert!(StoreConfig::new(deployment(), 1024, 64 << 20, Duration::from_secs(1)).is_ok());
    for (address, user, password) in [
        ("192.0.2.1:6379", "user", "fixture"),
        ("127.0.0.1:0", "user", "fixture"),
        ("127.0.0.1:6379", "", "fixture"),
        ("127.0.0.1:6379", "user", ""),
    ] {
        assert!(matches!(
            RedisEndpoint::new(address.parse().unwrap(), user.into(), password.into()),
            Err(StoreError::Configuration)
        ));
    }
}

#[test]
fn writer_is_exclusive_immediate_and_released_by_drop() {
    let root = private_directory();
    let first = WriterLease::acquire(root.path(), deployment()).unwrap();
    assert!(matches!(
        WriterLease::acquire(root.path(), deployment()),
        Err(StoreError::WriterUnavailable)
    ));
    drop(first);
    WriterLease::acquire(root.path(), deployment()).unwrap();
}

#[test]
fn writer_rejects_exposed_or_linked_directory_and_preserves_target() {
    let root = private_directory();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        WriterLease::acquire(root.path(), deployment()),
        Err(StoreError::Configuration)
    ));
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let parent = private_directory();
    let link = parent.path().join("link");
    symlink(root.path(), &link).unwrap();
    assert!(matches!(
        WriterLease::acquire(&link, deployment()),
        Err(StoreError::Configuration)
    ));
    assert!(fs::read_dir(root.path()).unwrap().next().is_none());
}

#[test]
fn writer_never_follows_a_final_symlink_or_accepts_multiple_names() {
    let root = private_directory();
    let target = root.path().join("target");
    fs::write(&target, b"unchanged fixture").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    let lock = root.path().join("center-component-deployment.lock");
    symlink(&target, &lock).unwrap();
    assert!(matches!(
        WriterLease::acquire(root.path(), deployment()),
        Err(StoreError::Configuration)
    ));
    fs::remove_file(&lock).unwrap();
    fs::hard_link(&target, &lock).unwrap();
    assert!(matches!(
        WriterLease::acquire(root.path(), deployment()),
        Err(StoreError::Configuration)
    ));
    assert_eq!(fs::read(&target).unwrap(), b"unchanged fixture");
}

#[test]
fn writer_refuses_fifo_and_does_not_wait_on_it() {
    let root = private_directory();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.path().join("center-component-deployment.lock"))
            .status()
            .unwrap()
            .success()
    );
    assert!(matches!(
        WriterLease::acquire(root.path(), deployment()),
        Err(StoreError::Configuration)
    ));
}

#[test]
fn diagnostics_have_no_arbitrary_payload_credentials_or_path_details() {
    for error in [
        StoreError::Configuration,
        StoreError::InvalidRecord,
        StoreError::OutcomeUnknown,
        StoreError::Unavailable,
        StoreError::WriteFenced,
    ] {
        let text = format!("{error} {error:?}");
        for secret in [
            "fixture-password",
            "/protected/private",
            "RedisError",
            "payload=",
            "tenant=",
        ] {
            assert!(!text.contains(secret));
        }
        assert!(text.len() < 128);
    }
}
