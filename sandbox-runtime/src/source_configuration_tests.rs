//! Ordinary byte/file fixtures. These do not pretend to be a real VMM or a
//! protected epoch; the full public observation still requires the original jail.
use super::*;
use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
};

fn limits() -> SourceConfigurationLimits {
    SourceConfigurationLimits {
        executable_bytes: 64 * 1024 * 1024,
        kernel_bytes: 256 * 1024 * 1024,
        disk_bytes: i64::MAX as u64,
        configuration_bytes: 1024 * 1024,
    }
}

#[test]
fn zero_or_excessive_read_budgets_are_refused() {
    let valid = limits();
    assert!(limits_valid(valid));
    for invalid in [
        SourceConfigurationLimits {
            executable_bytes: 0,
            ..valid
        },
        SourceConfigurationLimits {
            executable_bytes: valid.executable_bytes + 1,
            ..valid
        },
        SourceConfigurationLimits {
            kernel_bytes: 0,
            ..valid
        },
        SourceConfigurationLimits {
            kernel_bytes: valid.kernel_bytes + 1,
            ..valid
        },
        SourceConfigurationLimits {
            disk_bytes: 0,
            ..valid
        },
        SourceConfigurationLimits {
            disk_bytes: u64::MAX,
            ..valid
        },
        SourceConfigurationLimits {
            configuration_bytes: 0,
            ..valid
        },
        SourceConfigurationLimits {
            configuration_bytes: valid.configuration_bytes + 1,
            ..valid
        },
    ] {
        assert!(!limits_valid(invalid));
    }
}

fn valid_instance() -> serde_json::Value {
    serde_json::json!({"app_name":"Firecracker", "id":"source", "state":"Running", "vmm_version":"1.16.1"})
}

#[test]
fn replies_must_describe_the_same_fixed_running_or_paused_instance() {
    let original = valid_instance();
    let bytes = serde_json::to_vec(&original).unwrap();
    assert!(instance(&bytes, "source").is_ok());
    assert!(instance(&bytes, "replacement").is_err());
    for (field, value) in [
        ("app_name", "Other"),
        ("id", "replacement"),
        ("state", "Not started"),
        ("state", "Exited"),
        ("vmm_version", "1.16.0"),
    ] {
        let mut reply = original.clone();
        reply[field] = value.into();
        assert!(instance(&serde_json::to_vec(&reply).unwrap(), "source").is_err());
    }
    let mut paused = original.clone();
    paused["state"] = "Paused".into();
    assert!(instance(&serde_json::to_vec(&paused).unwrap(), "source").is_ok());
    for field in ["app_name", "id", "state", "vmm_version"] {
        let mut reply = original.clone();
        reply.as_object_mut().unwrap().remove(field);
        assert!(instance(&serde_json::to_vec(&reply).unwrap(), "source").is_err());
    }
    let mut unknown = original;
    unknown["extra"] = true.into();
    assert!(instance(&serde_json::to_vec(&unknown).unwrap(), "source").is_err());
    assert!(instance(br#"{"id":"source","id":"replacement"}"#, "source").is_err());
    assert!(instance(b"{", "source").is_err());
}

fn write_file(root: &std::path::Path, bytes: &[u8]) -> std::path::PathBuf {
    let path = root.join("kernel");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&path)
        .unwrap();
    file.write_all(bytes).unwrap();
    path
}

#[test]
fn digest_reads_actual_readonly_bytes_at_the_exact_length_limit() {
    let root = tempfile::tempdir().unwrap();
    let bytes = b"actual original bytes";
    let path = write_file(root.path(), bytes);
    let before = FileStamp::from_metadata(&path.metadata().unwrap());
    let mut file = File::open(&path).unwrap();
    let (digest, stamp) = measure(&mut file, bytes.len() as u64).unwrap();
    assert_eq!(digest, <[u8; 32]>::from(Sha256::digest(bytes)));
    assert_eq!(stamp, before);
    assert_eq!(FileStamp::from_metadata(&path.metadata().unwrap()), before);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(measure(&mut File::open(&path).unwrap(), bytes.len() as u64 - 1).is_err());
    let directory = File::open(root.path()).unwrap();
    assert!(measure(&mut directory.try_clone().unwrap(), 1024).is_err());
}

#[test]
fn empty_linked_or_unreadable_files_are_not_profile_measurements() {
    let root = tempfile::tempdir().unwrap();
    let path = write_file(root.path(), b"");
    assert!(measure(&mut File::open(&path).unwrap(), 1).is_err());
    fs::remove_file(&path).unwrap();
    let path = write_file(root.path(), b"kernel");
    fs::hard_link(&path, root.path().join("alias")).unwrap();
    assert!(measure(&mut File::open(&path).unwrap(), 100).is_err());
    fs::remove_file(root.path().join("alias")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut write_only = OpenOptions::new().write(true).open(&path).unwrap();
    assert!(measure(&mut write_only, 100).is_err());
}

#[test]
fn same_length_content_or_mode_changes_during_digest_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = write_file(root.path(), b"before");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut other = OpenOptions::new().write(true).open(&path).unwrap();
    assert!(
        measure_with(&mut File::open(&path).unwrap(), 100, || {
            other.seek(SeekFrom::Start(0))?;
            other.write_all(b"after!")?;
            other.sync_all()?;
            other.set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1),
            )?;
            Ok(())
        })
        .is_err()
    );
    assert!(
        measure_with(&mut File::open(&path).unwrap(), 100, || {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400))?;
            Ok(())
        })
        .is_err()
    );
    assert!(
        measure_with(&mut File::open(&path).unwrap(), 100, || {
            Err(error("injected original file observation failure"))
        })
        .is_err()
    );
}

#[test]
fn kernel_and_disk_metadata_require_exact_owner_group_mode_and_length() {
    let root = tempfile::tempdir().unwrap();
    let path = write_file(root.path(), b"kernel");
    let metadata = path.metadata().unwrap();
    let owner = metadata.uid();
    let group = metadata.gid();
    assert!(private_metadata(&metadata, owner, group, 0o400, 6).is_ok());
    assert!(private_metadata(&metadata, owner.wrapping_add(1), group, 0o400, 6).is_err());
    assert!(private_metadata(&metadata, owner, group.wrapping_add(1), 0o400, 6).is_err());
    assert!(private_metadata(&metadata, owner, group, 0o600, 6).is_err());
    assert!(private_metadata(&metadata, owner, group, 0o400, 5).is_err());
    fs::hard_link(&path, root.path().join("alias")).unwrap();
    assert!(private_metadata(&path.metadata().unwrap(), owner, group, 0o400, 6).is_err());
}

#[test]
fn held_kernel_descriptor_cannot_certify_a_replacement_or_symlink() {
    let root = tempfile::tempdir().unwrap();
    let path = write_file(root.path(), b"kernel");
    let held = File::open(&path).unwrap();
    let directory = File::open(root.path()).unwrap();
    assert!(matches_entry(&directory, OsStr::new("kernel"), &held).is_ok());
    fs::rename(&path, root.path().join("old")).unwrap();
    write_file(root.path(), b"kernel");
    assert!(matches_entry(&directory, OsStr::new("kernel"), &held).is_err());
    fs::remove_file(&path).unwrap();
    symlink("old", &path).unwrap();
    assert!(matches_entry(&directory, OsStr::new("kernel"), &held).is_err());
}
