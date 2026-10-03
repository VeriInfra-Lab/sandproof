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

use sandproof_controller_service::physical_storage::{PhysicalPool, PhysicalStorageProfile};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

const MAX_PROFILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Activation {
    version: u32,
    device: u64,
    inode: u64,
    profile: PhysicalStorageProfile,
}

/// An immutable shared profile, not a mutable cross-pool quota controller.
/// A shared lock prevents another compliant initializer changing its contents.
/// Host roots and logical quota leases remain the existing single writers.
pub(crate) struct PhysicalProfileLease {
    file: File,
    path: PathBuf,
    marker: Vec<u8>,
    device: u64,
    inode: u64,
    runtime: PathBuf,
    kernel: PathBuf,
    profile: PhysicalStorageProfile,
    namespace: String,
}

impl Drop for PhysicalProfileLease {
    fn drop(&mut self) {
        // This lease is controller-only, never a child ownership capability.
        // End transient fork-before-exec references just like the host lease.
        let _ = self.file.unlock();
    }
}

fn protected_file(file: &File, path: &Path, owner: u32) -> Result<(), Box<dyn Error>> {
    let open = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    if !open.is_file()
        || current.file_type().is_symlink()
        || open.uid() != owner
        || open.mode() & 0o077 != 0
        || open.nlink() != 1
        || open.len() > MAX_PROFILE_BYTES
        || (open.dev(), open.ino()) != (current.dev(), current.ino())
    {
        return Err("untrusted physical profile coordination file".into());
    }
    Ok(())
}

fn read_marker(file: &mut File) -> Result<Vec<u8>, Box<dyn Error>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err("physical profile is too large".into());
    }
    Ok(bytes)
}

fn empty(directory: &Path) -> Result<(), Box<dyn Error>> {
    if !fs::metadata(directory)?.is_dir() || fs::read_dir(directory)?.next().is_some() {
        return Err("physical profile activation requires empty managed roots".into());
    }
    Ok(())
}

fn quiescent(profile: &PhysicalStorageProfile) -> Result<(), Box<dyn Error>> {
    for pool in &profile.pools {
        for suffix in ["", "-wal", "-shm", ".controller.lock"] {
            let mut path = pool.database.as_os_str().to_os_string();
            path.push(suffix);
            match fs::symlink_metadata(Path::new(&path)) {
                Ok(_) => {
                    return Err(
                        "physical profile activation cannot adopt an existing controller".into(),
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        empty(&pool.node_state_dir)?;
        empty(&pool.chroot_base)?;
    }
    if let Some(public) = &profile.public_build {
        empty(&public.root)?;
        // Registry owns exactly these two initially empty directories.
        for entry in fs::read_dir(&public.registry_root)? {
            if !matches!(entry?.file_name().to_str(), Some("inbox" | "artifacts")) {
                return Err("physical profile activation cannot adopt registry history".into());
            }
        }
        empty(&public.registry_root.join("inbox"))?;
        empty(&public.registry_root.join("artifacts"))?;
    }
    Ok(())
}

fn allocation_paths(profile: &PhysicalStorageProfile) -> Vec<&Path> {
    let mut paths = vec![profile.coordination_file.as_path()];
    for pool in &profile.pools {
        paths.extend([
            pool.database
                .parent()
                .expect("validated absolute database path"),
            pool.node_state_dir.as_path(),
            pool.chroot_base.as_path(),
        ]);
    }
    if let Some(public) = &profile.public_build {
        paths.extend([public.root.as_path(), public.registry_root.as_path()]);
    }
    paths
}

fn validate_placement(
    profile: &PhysicalStorageProfile,
    owner: u32,
    trusted: bool,
) -> Result<(), Box<dyn Error>> {
    profile.validate()?;
    let device = fs::metadata(&profile.coordination_file)?.dev();
    let mut paths = allocation_paths(profile)
        .into_iter()
        .map(Path::to_owned)
        .collect::<Vec<_>>();
    if let Some(public) = &profile.public_build {
        paths.extend([
            public.registry_root.join("inbox"),
            public.registry_root.join("artifacts"),
        ]);
    }
    for path in &paths {
        if trusted {
            sandbox_runtime::check_trusted_path(path)?;
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink()
            || metadata.dev() != device
            || metadata.uid() != owner
            || metadata.mode() & 0o077 != 0
        {
            return Err("physical profile paths must be private and share one filesystem".into());
        }
    }
    let mut roots = profile
        .pools
        .iter()
        .flat_map(|pool| [&pool.node_state_dir, &pool.chroot_base])
        .collect::<Vec<_>>();
    if let Some(public) = &profile.public_build {
        roots.extend([&public.root, &public.registry_root]);
    }
    let identities = roots
        .iter()
        .map(|path| fs::metadata(path).map(|meta| (meta.dev(), meta.ino())))
        .collect::<Result<Vec<_>, _>>()?;
    for (index, path) in roots.iter().enumerate() {
        for ancestor in path.ancestors() {
            let meta = fs::metadata(ancestor)?;
            if identities
                .iter()
                .enumerate()
                .any(|(other, identity)| other != index && *identity == (meta.dev(), meta.ino()))
            {
                return Err("physical allocation roots alias or overlap".into());
            }
        }
    }
    for file in std::iter::once(&profile.coordination_file)
        .chain(profile.pools.iter().map(|pool| &pool.database))
    {
        for ancestor in file.ancestors().skip(1) {
            let meta = fs::metadata(ancestor)?;
            if identities.contains(&(meta.dev(), meta.ino())) {
                return Err("physical file lies inside an aliased allocation root".into());
            }
        }
    }
    for pool in &profile.pools {
        if pool.database.try_exists()? && fs::metadata(&pool.database)?.dev() != device {
            return Err("physical database moved off the shared filesystem".into());
        }
    }
    if let Some(public) = &profile.public_build {
        let registry = fs::metadata(&public.registry_root)?;
        let own_registry = (registry.dev(), registry.ino());
        for child in [
            public.registry_root.join("inbox"),
            public.registry_root.join("artifacts"),
        ] {
            for ancestor in child.ancestors() {
                let meta = fs::metadata(ancestor)?;
                let identity = (meta.dev(), meta.ino());
                if identity != own_registry && identities.contains(&identity) {
                    return Err("physical registry child aliases another allocation root".into());
                }
            }
        }
        let inbox = fs::metadata(public.registry_root.join("inbox"))?;
        let artifacts = fs::metadata(public.registry_root.join("artifacts"))?;
        if (inbox.dev(), inbox.ino()) == (artifacts.dev(), artifacts.ino()) {
            return Err("physical registry inbox/artifact roots alias".into());
        }
    }
    Ok(())
}

impl PhysicalProfileLease {
    /// Only an explicit profile can initialize its already-provisioned file.
    /// No legacy state is adopted. Exact restarts never demand new-work space.
    pub(crate) fn activate(
        profile: &PhysicalStorageProfile,
        namespace: &str,
        runtime: &Path,
        kernel: &Path,
    ) -> Result<Self, Box<dyn Error>> {
        Self::activate_owned(profile, namespace, runtime, kernel, 0, true)
    }

    fn activate_owned(
        profile: &PhysicalStorageProfile,
        namespace: &str,
        runtime: &Path,
        kernel: &Path,
        owner: u32,
        trusted: bool,
    ) -> Result<Self, Box<dyn Error>> {
        Self::activate_observed(
            profile,
            namespace,
            runtime,
            kernel,
            owner,
            trusted,
            &sandproof_node::disk_space,
        )
    }

    fn activate_observed(
        profile: &PhysicalStorageProfile,
        namespace: &str,
        runtime: &Path,
        kernel: &Path,
        owner: u32,
        trusted: bool,
        observe: &impl Fn(&Path) -> Result<sandproof_node::DiskSpace, Box<dyn Error>>,
    ) -> Result<Self, Box<dyn Error>> {
        validate_placement(profile, owner, trusted)?;
        profile.pool(namespace)?;
        let path = &profile.coordination_file;
        // The operator supplies this exact empty private file. Never silently
        // create a new coordination identity if the existing one is lost.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        protected_file(&file, path, owner)?;
        file.try_lock_shared()?;
        let metadata = file.metadata()?;
        // Value maps use sorted keys; fixed UTF-8 and no trailing newline are
        // also checked against the public supervisor's JSON implementation.
        let marker = serde_json::to_vec(&serde_json::to_value(Activation {
            version: 1,
            device: metadata.dev(),
            inode: metadata.ino(),
            profile: profile.clone(),
        })?)?;
        let geometry = observe(path)?;
        let known_overhead = profile
            .filesystem_overhead_inodes
            .checked_mul(geometry.fragment_bytes)
            .and_then(|bytes| bytes.checked_add(marker.len() as u64))
            .ok_or("physical profile overhead overflow")?;
        if marker.len() as u64 > MAX_PROFILE_BYTES
            || known_overhead > profile.filesystem_overhead_bytes
        {
            return Err(
                "physical profile exceeds its metadata and block-rounding provision".into(),
            );
        }
        if read_marker(&mut file)? != marker {
            file.unlock()?;
            file.try_lock()?;
            protected_file(&file, path, owner)?;
            let old = read_marker(&mut file)?;
            if old != marker {
                if !marker.starts_with(&old) {
                    return Err("immutable physical profile changed".into());
                }
                quiescent(profile)?;
                if !old.is_empty() {
                    // This exact unpublished prefix is the only owned object
                    // allowed before activation. Clear it under the exclusive
                    // lock before measuring fresh headroom: crediting its old
                    // blocks would race, and counting them again strands a
                    // crash/retry at exactly twice the required peak.
                    file.set_len(0)?;
                    file.sync_all()?;
                }
                let space = observe(path)?;
                let needed_inodes = profile
                    .filesystem_overhead_inodes
                    .checked_mul(2)
                    .ok_or("physical inode overflow")?;
                if space.available_bytes < profile.activation_bytes()?
                    || space.available_inodes < needed_inodes
                {
                    return Err(
                        "fresh physical activation requires twice the complete byte/inode envelope"
                            .into(),
                    );
                }
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&marker)?;
                file.sync_all()?;
                File::open(path.parent().ok_or("physical profile parent missing")?)?.sync_all()?;
            }
            file.unlock()?;
            file.try_lock_shared()?;
        }
        // Recheck exact content while holding the lifetime shared lease.
        if read_marker(&mut file)? != marker {
            return Err("physical profile changed during activation".into());
        }
        // A previous writer may have completed the bytes but received an
        // unknown fsync result. Exact reopen re-establishes durability too.
        file.sync_all()?;
        File::open(path.parent().ok_or("physical profile parent missing")?)?.sync_all()?;
        Ok(Self {
            file,
            path: path.clone(),
            marker,
            device: metadata.dev(),
            inode: metadata.ino(),
            runtime: runtime.to_owned(),
            kernel: kernel.to_owned(),
            profile: profile.clone(),
            namespace: namespace.into(),
        })
    }

    pub(crate) fn validate_admission(
        &self,
        profile: &PhysicalStorageProfile,
        pool: &PhysicalPool,
    ) -> Result<(), Box<dyn Error>> {
        if self.profile != *profile
            || pool.namespace != self.namespace
            || profile.pool(&self.namespace)? != pool
        {
            return Err("physical admission profile mismatch".into());
        }
        protected_file(&self.file, &self.path, 0)?;
        let metadata = self.file.metadata()?;
        if metadata.dev() != self.device
            || metadata.ino() != self.inode
            || read_marker(&mut self.file.try_clone()?)? != self.marker
        {
            return Err("physical admission activation identity changed".into());
        }
        sandbox_runtime::check_trusted_path(&self.runtime)?;
        let runtime = fs::metadata(&self.runtime)?;
        if !runtime.is_file() || runtime.len() == 0 || runtime.len() > pool.runtime_copy_bytes {
            return Err("runtime executable exceeds immutable physical copy bound".into());
        }
        sandbox_runtime::check_trusted_path(&self.kernel)?;
        let kernel = fs::metadata(&self.kernel)?;
        if !kernel.is_file() || kernel.len() == 0 || kernel.len() > pool.kernel_copy_bytes {
            return Err("base kernel exceeds immutable physical copy bound".into());
        }
        let space = sandproof_node::disk_space(&self.path)?;
        if space.device != self.device
            || space.available_inodes < profile.filesystem_overhead_inodes
        {
            return Err("physical inode reserve unavailable".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod physical_tests {
    use super::*;
    use sandproof_controller_service::physical_storage::{PhysicalPool, PoolRole};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        profile: PhysicalStorageProfile,
        runtime: PathBuf,
        owner: u32,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sandproof-physical-容量-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let owner = fs::metadata(&root).unwrap().uid();
            for name in ["db", "node", "jails"] {
                fs::create_dir(root.join(name)).unwrap();
                fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700)).unwrap();
            }
            let coordination = root.join("physical-profile");
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&coordination)
                .unwrap();
            let runtime = root.join("firecracker");
            fs::write(&runtime, b"test-executable").unwrap();
            let profile = PhysicalStorageProfile {
                version: 1,
                coordination_file: coordination,
                pools: vec![PhysicalPool {
                    role: PoolRole::Runtime,
                    namespace: "0123456789abcdef".into(),
                    database: root.join("db/controller.db"),
                    node_state_dir: root.join("node"),
                    chroot_base: root.join("jails"),
                    rootfs_budget_bytes: 1000,
                    max_host_slots: 2,
                    kernel_copy_bytes: 50,
                    runtime_copy_bytes: 50,
                    cleanup_reserve_bytes: 200,
                    host_metadata_bytes: 100,
                    host_metadata_inodes: 20,
                    jail_inodes_per_slot: 10,
                }],
                public_build: None,
                filesystem_overhead_bytes: 1024 * 1024,
                filesystem_overhead_inodes: 100,
            };
            Self {
                root,
                profile,
                runtime,
                owner,
            }
        }
        fn activate(&self) -> Result<PhysicalProfileLease, Box<dyn Error>> {
            PhysicalProfileLease::activate_owned(
                &self.profile,
                "0123456789abcdef",
                &self.runtime,
                &self.runtime,
                self.owner,
                false,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn fresh_explicit_profile_has_one_identity_and_exact_restarts() {
        let fixture = Fixture::new();
        let first = fixture.activate().unwrap();
        let second = fixture.activate().unwrap();
        assert_eq!((first.device, first.inode), (second.device, second.inode));
        // Existing state is expected only after activation; restarting does
        // not demand empty roots, repeat provisioning, or consume a new lease.
        fs::write(&fixture.profile.pools[0].database, b"existing database").unwrap();
        fs::write(fixture.root.join("node/owned"), b"pending cleanup").unwrap();
        drop(first);
        drop(second);
        fixture.activate().unwrap();
    }

    #[test]
    fn legacy_state_and_missing_coordination_fail_without_adoption() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("node/unknown"), b"pending").unwrap();
        assert!(fixture.activate().is_err());
        assert_eq!(fs::read(&fixture.profile.coordination_file).unwrap(), b"");
        fs::remove_file(fixture.root.join("node/unknown")).unwrap();
        fs::remove_file(&fixture.profile.coordination_file).unwrap();
        assert!(fixture.activate().is_err());
        assert!(!fixture.profile.coordination_file.exists());
    }

    #[test]
    fn changed_profile_or_replaced_inode_never_resets_provisioning() {
        let mut fixture = Fixture::new();
        let lease = fixture.activate().unwrap();
        drop(lease);
        let saved = fs::read(&fixture.profile.coordination_file).unwrap();
        fixture.profile.pools[0].rootfs_budget_bytes += 1;
        assert!(fixture.activate().is_err());
        fixture.profile.pools[0].rootfs_budget_bytes -= 1;
        fs::rename(
            &fixture.profile.coordination_file,
            fixture.root.join("old-profile"),
        )
        .unwrap();
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&fixture.profile.coordination_file)
            .unwrap()
            .write_all(&saved)
            .unwrap();
        assert!(fixture.activate().is_err());
    }

    #[test]
    fn interrupted_initial_marker_requires_exact_prefix_and_still_empty_roots() {
        let fixture = Fixture::new();
        drop(fixture.activate().unwrap());
        let saved = fs::read(&fixture.profile.coordination_file).unwrap();
        fs::write(
            &fixture.profile.coordination_file,
            &saved[..saved.len() / 2],
        )
        .unwrap();
        drop(fixture.activate().unwrap());
        assert_eq!(fs::read(&fixture.profile.coordination_file).unwrap(), saved);
        fs::write(
            &fixture.profile.coordination_file,
            &saved[..saved.len() / 2],
        )
        .unwrap();
        fs::write(fixture.root.join("jails/unresolved"), b"unknown").unwrap();
        assert!(fixture.activate().is_err());
    }

    #[test]
    fn every_profile_path_must_share_the_verified_filesystem() {
        let mut fixture = Fixture::new();
        fixture.profile.pools[0].node_state_dir = PathBuf::from("/proc");
        assert!(fixture.activate().is_err());
    }
    #[test]
    fn rust_utf8_marker_is_read_by_python_and_every_partial_prefix_recovers() {
        let fixture = Fixture::new();
        drop(fixture.activate().unwrap());
        let saved = fs::read(&fixture.profile.coordination_file).unwrap();
        assert!(!saved.ends_with(b"\n"));
        assert!(String::from_utf8(saved.clone()).unwrap().contains("容量"));
        let script = r"import json,sys
from pathlib import Path
from tools.template_physical_storage import PhysicalProfileLease
marker=json.loads(Path(sys.argv[1]).read_bytes())
lease=PhysicalProfileLease(marker['profile'])
lease.close()
sys.stdout.buffer.write(json.dumps(marker,ensure_ascii=False,sort_keys=True,separators=(',',':')).encode('utf-8'))
";
        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .arg(&fixture.profile.coordination_file)
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, saved);
        for prefix in 0..saved.len() {
            fs::write(&fixture.profile.coordination_file, &saved[..prefix]).unwrap();
            drop(fixture.activate().unwrap());
            assert_eq!(fs::read(&fixture.profile.coordination_file).unwrap(), saved);
        }
    }

    #[test]
    fn python_first_refuses_partial_activation_without_writing_any_state() {
        let fixture = Fixture::new();
        drop(fixture.activate().unwrap());
        let saved = fs::read(&fixture.profile.coordination_file).unwrap();
        let json_profile = serde_json::to_string(&fixture.profile).unwrap();
        for prefix in [0, saved.len() / 2] {
            fs::write(&fixture.profile.coordination_file, &saved[..prefix]).unwrap();
            let script = r"import json,sys
from tools.template_physical_storage import PhysicalProfileLease
try:
    PhysicalProfileLease(json.loads(sys.argv[1]))
except ValueError:
    sys.exit(0)
raise SystemExit('partial profile unexpectedly accepted')
";
            let output = std::process::Command::new("python3")
                .arg("-c")
                .arg(script)
                .arg(&json_profile)
                .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read(&fixture.profile.coordination_file).unwrap(),
                saved[..prefix]
            );
            quiescent(&fixture.profile).unwrap();
        }
    }
    #[test]
    fn partial_activation_at_exact_initial_headroom_recovers_without_block_credit() {
        let fixture = Fixture::new();
        let available = fixture.profile.activation_bytes().unwrap();
        let observe = |path: &Path| -> Result<sandproof_node::DiskSpace, Box<dyn Error>> {
            let metadata = fs::metadata(path)?;
            let mut space = sandproof_node::disk_space(path)?;
            space.available_bytes = available.checked_sub(metadata.blocks() * 512).unwrap();
            Ok(space)
        };
        let activate = || {
            PhysicalProfileLease::activate_observed(
                &fixture.profile,
                "0123456789abcdef",
                &fixture.runtime,
                &fixture.runtime,
                fixture.owner,
                false,
                &observe,
            )
        };
        drop(activate().unwrap());
        let marker = fs::read(&fixture.profile.coordination_file).unwrap();
        fs::write(
            &fixture.profile.coordination_file,
            &marker[..marker.len() / 2],
        )
        .unwrap();
        assert!(
            observe(&fixture.profile.coordination_file)
                .unwrap()
                .available_bytes
                < available
        );
        drop(activate().unwrap());
        assert_eq!(
            fs::read(&fixture.profile.coordination_file).unwrap(),
            marker
        );
        // Complete marker restart is exempt from the fresh 2x check.
        drop(activate().unwrap());
    }
}
