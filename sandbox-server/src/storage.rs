//! The chroot free-space observation must also cover journal and node writes.

use std::{error::Error, fs, os::unix::fs::MetadataExt, path::Path};

pub(crate) fn validate_filesystems(
    database: &Path,
    node_state_dir: &Path,
    chroot_base: &Path,
) -> Result<(), Box<dyn Error>> {
    let database_parent = database.parent().ok_or("database parent missing")?;
    let disk_device = fs::metadata(chroot_base)?.dev();
    if fs::metadata(database_parent)?.dev() != disk_device
        || fs::metadata(node_state_dir)?.dev() != disk_device
    {
        return Err(
            "database, node state and private VM disk must share a filesystem for cleanup reserve"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_uses_real_filesystem_devices_without_creating_the_database() {
        // Linux procfs provides an independently mounted filesystem in ordinary
        // CI, so this regression needs neither root nor a synthetic device ID.
        let local = std::env::temp_dir();
        let other = Path::new("/proc");
        assert_ne!(
            fs::metadata(&local).unwrap().dev(),
            fs::metadata(other).unwrap().dev()
        );
        let database = local.join("sandproof-unopened-database");
        let other_database = other.join("sandproof-unopened-database");
        assert!(validate_filesystems(&database, &local, &local).is_ok());
        for (db, node, jail) in [
            (&other_database, local.as_path(), local.as_path()),
            (&database, other, local.as_path()),
            (&database, local.as_path(), other),
        ] {
            assert_eq!(
                validate_filesystems(db, node, jail)
                    .unwrap_err()
                    .to_string(),
                "database, node state and private VM disk must share a filesystem for cleanup reserve"
            );
        }
    }

    #[test]
    fn missing_storage_paths_fail_closed() {
        let local = std::env::temp_dir();
        let database = local.join("sandproof-unopened-database");
        // procfs cannot contain this non-numeric directory; no shared scratch
        // path is created or deleted by this test.
        let missing = Path::new("/proc/sandproof-missing-storage");
        assert!(!missing.exists());
        assert!(validate_filesystems(&missing.join("db"), &local, &local).is_err());
        assert!(validate_filesystems(&database, missing, &local).is_err());
        assert!(validate_filesystems(&database, &local, missing).is_err());
        assert!(validate_filesystems(Path::new("/"), &local, &local).is_err());
    }
}
