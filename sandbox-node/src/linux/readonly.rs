//! Observe an existing initialized node without granting execution capability.
use super::{
    BTreeMap, CreateOptions, HostInventory, HostSlotContext, LinuxWorker, NODE_STATE_DIRECTORIES,
    NodeConfig, NodeResult, check_trusted_path, core, fs, host_lease, host_slots,
    validate_worker_configuration,
};
use std::{os::unix::fs::MetadataExt, sync::Mutex};

/// Holds the existing host's exclusive observation lease. This wrapper has no
/// initialize, execute, stop, release, credential write or recovery methods.
pub struct LinuxObservation {
    worker: LinuxWorker,
}

impl LinuxWorker {
    /// Validate and exclusively inspect an existing Ready layout. Missing or
    /// interrupted initialization fails; no directories or markers are created
    /// and no staged record or lifecycle action is recovered.
    /// # Errors
    /// Rejects unsafe paths, missing/replaced markers, a live owner or bad layout.
    pub fn inspect_existing(config: NodeConfig) -> NodeResult<LinuxObservation> {
        validate_worker_configuration(&config)?;
        let identity = host_slots::stored_lease_identity(&config)?
            .ok_or("missing initialized host mapping")?;
        let host_lease = host_lease::HostLease::inspect_existing(&config, &identity)?;
        for subdir in NODE_STATE_DIRECTORIES
            .iter()
            .copied()
            .chain(host_slots::BOUNDED_STATE_DIRECTORIES)
        {
            let path = config.state_dir.join(subdir);
            check_trusted_path(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
                return Err("node state subdirectory must be root-only".into());
            }
        }
        let worker = Self {
            config,
            host_lease,
            validated_artifacts: Mutex::new(BTreeMap::new()),
        };
        // The SAME original reader rejects Initializing and validates every
        // fixed position/gate and recognized stage, without completing it.
        worker.host_slot_inventory()?;
        Ok(LinuxObservation { worker })
    }
}

impl LinuxObservation {
    /// # Errors
    /// Original physical observation errors remain errors, never absence.
    pub fn inventory(&self) -> NodeResult<HostInventory> {
        self.worker.inventory()
    }

    /// Use original launch bindings, not aliases or unreadable credentials.
    /// # Errors
    /// Rejects any mismatch with the original protected slot or launch options.
    pub fn inventory_with_launch_options(
        &self,
        launches: &BTreeMap<core::SandboxId, (HostSlotContext, CreateOptions)>,
    ) -> NodeResult<HostInventory> {
        self.worker.inventory_with_launch_options(launches)
    }

    /// # Errors
    /// Refuses missing or corrupt original fixed cells; never recreates them.
    pub fn host_slot_inventory(
        &self,
    ) -> NodeResult<Vec<(HostSlotContext, core::maintenance::slots::HostSlotPhase)>> {
        self.worker.host_slot_inventory()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_controller_core::maintenance::{MaintenanceLimits, windows::WindowLimits};
    use sandproof_controller_service::{BoundedHostConfig, ConsolePolicy};
    use std::{
        fs::{DirBuilder, OpenOptions},
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        path::{Path, PathBuf},
    };

    fn fixture() -> (tempfile::TempDir, NodeConfig) {
        assert_eq!(
            rustix::process::geteuid().as_raw(),
            0,
            "explicit Linux layout test requires root, not KVM"
        );
        let parent = std::env::var_os("SANDPROOF_NODE_AUDIT_TEST_PARENT")
            .expect("explicit root-only audit test parent");
        check_trusted_path(Path::new(&parent)).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("a-")
            .tempdir_in(parent)
            .unwrap();
        let root = dir.path();
        for name in ["node", "jails"] {
            DirBuilder::new()
                .mode(0o700)
                .create(root.join(name))
                .unwrap();
        }
        for name in ["jailer", "firecracker", "kernel", "rootfs"] {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(root.join(name))
                .unwrap();
            file.write_all(b"readonly-fixture").unwrap();
        }
        let config = NodeConfig {
            namespace: "0123456789abcdef".into(),
            state_dir: root.join("node"),
            jailer: root.join("jailer"),
            firecracker: root.join("firecracker"),
            chroot_base: root.join("jails"),
            kernel: root.join("kernel"),
            rootfs: root.join("rootfs"),
            bounded_host: BoundedHostConfig {
                physical_storage: Some(super::super::test_support::physical_profile(
                    "0123456789abcdef",
                    &root.join("node"),
                    &root.join("jails"),
                    1,
                )),
                console_policy: Some(ConsolePolicy::Discard),
                uid_base: 10_000,
                uplink_base: 100,
                max_data_routes_per_slot: 4,
                max_data_routes_global: 8,
                data_route_control_reserve: 1,
                maintenance: MaintenanceLimits {
                    max_host_slots: 1,
                    max_retained_results: 16,
                    windows: WindowLimits::new(2, 1000).unwrap(),
                },
            },
            vcpu_count: 1,
            memory_mib: 128,
            host_resources: None,
            file_policy_digest: core::PolicyDigest(1),
        };
        (dir, config)
    }

    // Ignore access time: merely reading files may update atime. Record every
    // directory/file identity, ownership, mode and content, so mkdir/repair or
    // replacement remains observable without dumping private fixture payloads.
    #[derive(Debug, PartialEq, Eq)]
    struct FileSnapshot {
        identity: (u64, u64),
        mode: u32,
        owner: u32,
        modified: (i64, i64),
        changed: (i64, i64),
        digest: Option<[u8; 32]>,
    }
    fn files(root: &Path) -> BTreeMap<PathBuf, FileSnapshot> {
        use sha2::{Digest, Sha256};
        fn collect(path: &Path, output: &mut BTreeMap<PathBuf, FileSnapshot>) {
            let metadata = fs::symlink_metadata(path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            output.insert(
                path.to_owned(),
                FileSnapshot {
                    identity: (metadata.dev(), metadata.ino()),
                    mode: metadata.mode(),
                    owner: metadata.uid(),
                    modified: (metadata.mtime(), metadata.mtime_nsec()),
                    changed: (metadata.ctime(), metadata.ctime_nsec()),
                    digest: metadata
                        .is_file()
                        .then(|| Sha256::digest(fs::read(path).unwrap()).into()),
                },
            );
            if metadata.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    collect(&entry.unwrap().path(), output);
                }
            }
        }
        let mut result = BTreeMap::new();
        collect(root, &mut result);
        result
    }

    #[test]
    #[ignore = "requires root and SANDPROOF_NODE_AUDIT_TEST_PARENT in a short trusted private directory; no VM/network actions"]
    fn readonly_existing_layout_uses_original_inventory_without_any_record_mutation() {
        let (dir, config) = fixture();
        let worker = LinuxWorker::new(config.clone()).unwrap();
        assert!(LinuxWorker::inspect_existing(config.clone()).is_err());
        drop(worker);
        let before = files(dir.path());
        let observation = LinuxWorker::inspect_existing(config).unwrap();
        assert!(observation.host_slot_inventory().unwrap().is_empty());
        drop(observation);
        assert_eq!(files(dir.path()), before);
    }

    #[test]
    #[ignore = "requires root and SANDPROOF_NODE_AUDIT_TEST_PARENT in a short trusted private directory; no VM/network actions"]
    fn readonly_uninitialized_or_missing_fixed_cell_never_creates_or_repairs() {
        let (dir, config) = fixture();
        let empty = files(dir.path());
        assert!(LinuxWorker::inspect_existing(config.clone()).is_err());
        assert_eq!(files(dir.path()), empty);
        drop(LinuxWorker::new(config.clone()).unwrap());
        let position = config.state_dir.join("host-slots/0.json");
        fs::remove_file(position).unwrap();
        let missing = files(dir.path());
        assert!(LinuxWorker::inspect_existing(config).is_err());
        assert_eq!(files(dir.path()), missing);
    }

    #[test]
    #[ignore = "requires root and SANDPROOF_NODE_AUDIT_TEST_PARENT in a short trusted private directory; no VM/network actions"]
    fn readonly_initializing_mapping_and_known_stage_never_trigger_recovery() {
        let (dir, config) = fixture();
        drop(LinuxWorker::new(config.clone()).unwrap());
        let target = config.state_dir.join("host-slots/0.json");
        let stage = super::super::node_staging::path(&target).unwrap();
        // A legitimately interrupted empty deterministic stage is observable
        // but must not be cleaned by inspect_existing, unlike activating new.
        DirBuilder::new().mode(0o700).create(&stage).unwrap();
        let before = files(dir.path());
        drop(LinuxWorker::inspect_existing(config.clone()).unwrap());
        assert_eq!(files(dir.path()), before);
        let mapping = config.state_dir.join("host-slot-config.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&mapping).unwrap()).unwrap();
        json["phase"] = "Initializing".into();
        fs::write(&mapping, serde_json::to_vec(&json).unwrap()).unwrap();
        let interrupted = files(dir.path());
        assert!(LinuxWorker::inspect_existing(config).is_err());
        assert_eq!(files(dir.path()), interrupted);
    }
}
