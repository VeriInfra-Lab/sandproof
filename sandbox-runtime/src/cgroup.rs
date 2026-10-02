//! Per-VM cgroup v2 limits, configured before the recorded launch gate opens.
//! The operator delegates a private, root-owned parent with CPU/memory enabled.
//! This module never enables controllers in shared ancestors or kills a group.

use super::{
    Runtime, RuntimeError, check_trusted_path, path_present, read_trusted_record, validate_id,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const CPU_PERIOD_US: u64 = 100_000;
const CGROUP2_MAGIC: i64 = 0x6367_7270;

/// Trusted deployment settings, never supplied by a sandbox API request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostResources {
    pub cgroup_parent: PathBuf,
    /// Entire Firecracker process group, including guest RAM and VMM overhead.
    /// This is not the guest's advertised memory size or a host reservation.
    pub memory_max_bytes: u64,
}

impl HostResources {
    /// Validate limits and the already-delegated cgroup parent without mutation.
    /// # Errors
    /// Rejects untrusted paths, non-v2 filesystems, unavailable controllers, or
    /// a host memory ceiling that cannot accommodate the configured guest RAM.
    pub fn validate(&self, guest_memory_mib: u32) -> Result<(), RuntimeError> {
        self.validate_values(guest_memory_mib)?;
        check_trusted_path(&self.cgroup_parent)?;
        let metadata = fs::symlink_metadata(&self.cgroup_parent)?;
        if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
            return Err(RuntimeError::InvalidConfiguration(
                "cgroup parent must be root-only",
            ));
        }
        let status = rustix::fs::statfs(&self.cgroup_parent).map_err(std::io::Error::from)?;
        if status.f_type != CGROUP2_MAGIC {
            return Err(RuntimeError::InvalidConfiguration(
                "host limits require cgroup v2",
            ));
        }
        let controllers = fs::read_to_string(self.cgroup_parent.join("cgroup.subtree_control"))?;
        if !["cpu", "memory"]
            .iter()
            .all(|name| controllers.split_whitespace().any(|item| item == *name))
            || !fs::read_to_string(self.cgroup_parent.join("cgroup.procs"))?
                .trim()
                .is_empty()
            || fs::read_to_string(self.cgroup_parent.join("cgroup.type"))?.trim() != "domain"
        {
            return Err(RuntimeError::InvalidConfiguration(
                "cgroup parent must be an empty domain with CPU/memory delegated",
            ));
        }
        Ok(())
    }

    fn validate_values(&self, guest_memory_mib: u32) -> Result<(), RuntimeError> {
        let root = Path::new(CGROUP_ROOT);
        if self.cgroup_parent == root
            || !self.cgroup_parent.starts_with(root)
            || self
                .cgroup_parent
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            || self.cgroup_parent.to_str().is_none()
            || self.memory_max_bytes <= u64::from(guest_memory_mib) * 1024 * 1024
            || self.memory_max_bytes > i64::MAX as u64
            || !self.memory_max_bytes.is_multiple_of(4096)
        {
            return Err(RuntimeError::InvalidConfiguration(
                "invalid private cgroup path or VMM memory budget",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    resource_id: String,
    parent: PathBuf,
    parent_device: u64,
    parent_inode: u64,
    boot_id: String,
    cpu_quota_us: u64,
    memory_max_bytes: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    intent: Intent,
    device: u64,
    inode: u64,
}

impl Runtime {
    fn cgroup_claim(&self, id: &str) -> PathBuf {
        self.config.chroot_base.join(".claims").join(id)
    }

    fn cgroup_settings(&self, id: &str) -> Result<Option<&HostResources>, RuntimeError> {
        validate_id(id)?;
        if let Some(settings) = &self.config.host_resources {
            let claim = self.cgroup_claim(id);
            if path_present(&claim.join("cgroup-intent.json"))? {
                let intent: Intent = read_trusted_record(&claim.join("cgroup-intent.json"))?;
                validate_intent_settings(&intent, id, settings, self.config.vcpu_count)?;
            } else if path_present(&claim.join("cgroup.json"))? {
                return Err(RuntimeError::IdentityMismatch(
                    "cgroup record lost its durable intent",
                ));
            }
        }
        if self.config.host_resources.is_none()
            && (path_present(&self.cgroup_claim(id).join("cgroup-intent.json"))?
                || path_present(&self.cgroup_claim(id).join("cgroup.json"))?)
        {
            return Err(RuntimeError::IdentityMismatch(
                "recorded host limits cannot be disabled during recovery",
            ));
        }
        Ok(self.config.host_resources.as_ref())
    }

    fn cgroup_intent(&self, id: &str, settings: &HostResources) -> Result<Intent, RuntimeError> {
        let parent = fs::symlink_metadata(&settings.cgroup_parent)?;
        Ok(Intent {
            resource_id: id.to_owned(),
            parent: settings.cgroup_parent.clone(),
            parent_device: parent.dev(),
            parent_inode: parent.ino(),
            boot_id: fs::read_to_string("/proc/sys/kernel/random/boot_id")?
                .trim()
                .to_owned(),
            cpu_quota_us: u64::from(self.config.vcpu_count) * CPU_PERIOD_US,
            memory_max_bytes: settings.memory_max_bytes,
        })
    }

    pub(super) fn prepare_cgroup(&self, id: &str) -> Result<(), RuntimeError> {
        let Some(settings) = self.cgroup_settings(id)? else {
            return Ok(());
        };
        settings.validate(self.config.memory_mib)?;
        let path = settings.cgroup_parent.join(id);
        // Check before publishing intent: a pre-existing group is not ours.
        if path_present(&path)? {
            return Err(RuntimeError::AlreadyClaimed);
        }
        let intent = self.cgroup_intent(id, settings)?;
        self.write_new_record(&self.cgroup_claim(id).join("cgroup-intent.json"), &intent)?;
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        let record = Record {
            intent,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        self.write_new_record(&self.cgroup_claim(id).join("cgroup.json"), &record)?;
        for (name, value) in limits(&record.intent) {
            fs::write(path.join(name), value)?;
        }
        self.verify_cgroup(id, None)?;
        Ok(())
    }

    pub(super) fn jailer_cgroup(&self, id: &str) -> Result<Option<PathBuf>, RuntimeError> {
        Ok(self.cgroup_settings(id)?.map(|settings| {
            // validate_values established this prefix; jailer expects a relative path.
            settings
                .cgroup_parent
                .strip_prefix(CGROUP_ROOT)
                .expect("validated cgroup parent")
                .join(id)
        }))
    }

    /// Observe the recorded cgroup identity, even for a stopped VM. Absence is
    /// distinct from a missing/corrupt record or a replaced kernel object.
    /// # Errors
    /// Refuses unrecorded groups, inode reuse, another boot, or changed settings.
    pub fn cgroup_inode(&self, id: &str) -> Result<Option<u64>, RuntimeError> {
        let Some(settings) = self.cgroup_settings(id)? else {
            return Ok(None);
        };
        let path = settings.cgroup_parent.join(id);
        if !path_present(&path)? {
            return Ok(None);
        }
        let record = self.checked_cgroup_record(id, settings)?;
        Ok(Some(record.inode))
    }

    fn checked_cgroup_record(
        &self,
        id: &str,
        settings: &HostResources,
    ) -> Result<Record, RuntimeError> {
        let record: Record = read_trusted_record(&self.cgroup_claim(id).join("cgroup.json"))?;
        let intent: Intent =
            read_trusted_record(&self.cgroup_claim(id).join("cgroup-intent.json"))?;
        if intent != record.intent || record.intent != self.cgroup_intent(id, settings)? {
            return Err(RuntimeError::IdentityMismatch(
                "cgroup intent, parent, boot or limits",
            ));
        }
        let path = settings.cgroup_parent.join(id);
        check_trusted_path(&path)?;
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.dev() != record.device
            || metadata.ino() != record.inode
            || metadata.mode() & 0o077 != 0
        {
            return Err(RuntimeError::IdentityMismatch(
                "cgroup inode or permissions",
            ));
        }
        Ok(record)
    }

    pub(super) fn verify_cgroup(&self, id: &str, pid: Option<u32>) -> Result<(), RuntimeError> {
        let Some(settings) = self.cgroup_settings(id)? else {
            return Ok(());
        };
        let record = self.checked_cgroup_record(id, settings)?;
        let path = settings.cgroup_parent.join(id);
        for (name, expected) in limits(&record.intent) {
            if fs::read_to_string(path.join(name))?.trim() != expected {
                return Err(RuntimeError::IdentityMismatch("host cgroup limits changed"));
            }
        }
        if let Some(pid) = pid {
            let relative = path
                .strip_prefix(CGROUP_ROOT)
                .expect("validated cgroup path");
            let expected = format!("0::/{}", relative.display());
            if fs::read_to_string(format!("/proc/{pid}/cgroup"))?.trim() != expected {
                return Err(RuntimeError::IdentityMismatch(
                    "VMM is outside its recorded cgroup",
                ));
            }
        }
        Ok(())
    }

    /// Refuse cleanup success while a configured cgroup still exists.
    /// # Errors
    /// Rejects extant groups, unavailable paths, or disabled recorded settings.
    pub fn require_cgroup_absent(&self, id: &str) -> Result<(), RuntimeError> {
        if let Some(settings) = self.cgroup_settings(id)?
            && path_present(&settings.cgroup_parent.join(id))?
        {
            return Err(RuntimeError::IdentityMismatch("VM cgroup still exists"));
        }
        Ok(())
    }

    pub(super) fn cleanup_cgroup(&self, id: &str, unlaunched: bool) -> Result<(), RuntimeError> {
        let Some(settings) = self.cgroup_settings(id)? else {
            return Ok(());
        };
        let path = settings.cgroup_parent.join(id);
        if !path_present(&path)? {
            return Ok(());
        }
        let claim = self.cgroup_claim(id);
        let recorded = path_present(&claim.join("cgroup.json"))?;
        if recorded {
            self.checked_cgroup_record(id, settings)?;
        } else if unlaunched && !path_present(&claim.join("record.json"))? {
            // A private parent has one allocator, and resource IDs are never
            // reused. Reclaim only an empty intent-bound creation interrupted
            // before recording its inode, with no launch gate ever released.
            let intent: Intent = read_trusted_record(&claim.join("cgroup-intent.json"))?;
            if intent != self.cgroup_intent(id, settings)? {
                return Err(RuntimeError::IdentityMismatch("unlaunched cgroup intent"));
            }
            check_trusted_path(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
                return Err(RuntimeError::IdentityMismatch(
                    "unlaunched cgroup permissions",
                ));
            }
        } else {
            return Err(RuntimeError::IdentityMismatch(
                "cgroup has no identity record",
            ));
        }
        if !fs::read_to_string(path.join("cgroup.procs"))?
            .trim()
            .is_empty()
            || !fs::read_to_string(path.join("cgroup.events"))?
                .lines()
                .any(|line| line == "populated 0")
        {
            return Err(RuntimeError::IdentityMismatch(
                "cgroup still contains tasks",
            ));
        }
        if !recorded {
            // Persist the identity before removing an interrupted creation.
            // A repeated cleanup can then reject a replacement inode even if
            // the outer abort marker was not committed before a crash.
            let metadata = fs::symlink_metadata(&path)?;
            self.write_new_record(
                &claim.join("cgroup.json"),
                &Record {
                    intent: self.cgroup_intent(id, settings)?,
                    device: metadata.dev(),
                    inode: metadata.ino(),
                },
            )?;
        }
        // rmdir also refuses child groups; never recursively remove a subtree.
        fs::remove_dir(path)?;
        Ok(())
    }
}

fn limits(intent: &Intent) -> [(&'static str, String); 5] {
    [
        (
            "cpu.max",
            format!("{} {CPU_PERIOD_US}", intent.cpu_quota_us),
        ),
        ("cpu.max.burst", "0".into()),
        ("memory.max", intent.memory_max_bytes.to_string()),
        ("memory.swap.max", "0".into()),
        ("memory.oom.group", "1".into()),
    ]
}

fn validate_intent_settings(
    intent: &Intent,
    id: &str,
    settings: &HostResources,
    vcpu_count: u8,
) -> Result<(), RuntimeError> {
    // Check even when the configured path is absent. Otherwise changing the
    // parent could make an old, still-populated group disappear from recovery.
    if intent.resource_id != id
        || intent.parent != settings.cgroup_parent
        || intent.memory_max_bytes != settings.memory_max_bytes
        || intent.cpu_quota_us != u64::from(vcpu_count) * CPU_PERIOD_US
    {
        return Err(RuntimeError::IdentityMismatch(
            "configured host resources differ from durable intent",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::write_new_record;
    use super::*;
    use crate::RuntimeConfig;
    use std::process::{Child, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn settings() -> HostResources {
        HostResources {
            cgroup_parent: PathBuf::from("/sys/fs/cgroup/sandproof-test"),
            memory_max_bytes: 256 * 1024 * 1024,
        }
    }

    #[test]
    fn budgets_require_private_paths_and_explicit_memory_headroom() {
        assert!(settings().validate_values(128).is_ok());
        for path in [
            "/sys/fs/cgroup",
            "/tmp/cgroup",
            "/sys/fs/cgroup/a/../b",
            "relative",
        ] {
            assert!(
                HostResources {
                    cgroup_parent: path.into(),
                    ..settings()
                }
                .validate_values(128)
                .is_err()
            );
        }
        for memory_max_bytes in [0, 128 * 1024 * 1024, 256 * 1024 * 1024 + 1, u64::MAX] {
            assert!(
                HostResources {
                    memory_max_bytes,
                    ..settings()
                }
                .validate_values(128)
                .is_err()
            );
        }
    }

    #[test]
    fn changed_configuration_cannot_hide_old_groups_from_recovery() {
        let config = settings();
        let intent = Intent {
            resource_id: "vm-test-1".into(),
            parent: config.cgroup_parent.clone(),
            parent_device: 1,
            parent_inode: 2,
            boot_id: "boot".into(),
            cpu_quota_us: CPU_PERIOD_US,
            memory_max_bytes: config.memory_max_bytes,
        };
        assert!(validate_intent_settings(&intent, "vm-test-1", &config, 1).is_ok());
        assert!(validate_intent_settings(&intent, "vm-test-2", &config, 1).is_err());
        assert!(validate_intent_settings(&intent, "vm-test-1", &config, 2).is_err());
        let moved = HostResources {
            cgroup_parent: PathBuf::from("/sys/fs/cgroup/empty-new-parent"),
            ..config.clone()
        };
        assert!(validate_intent_settings(&intent, "vm-test-1", &moved, 1).is_err());
        let increased = HostResources {
            memory_max_bytes: config.memory_max_bytes * 2,
            ..config
        };
        assert!(validate_intent_settings(&intent, "vm-test-1", &increased, 1).is_err());
    }

    fn fixture() -> (Runtime, PathBuf, String) {
        let parent = PathBuf::from(
            std::env::var("SANDPROOF_TEST_CGROUP_PARENT").expect("dedicated cgroup parent"),
        );
        let scratch = PathBuf::from(
            std::env::var("SANDPROOF_TEST_SCRATCH").expect("root-only scratch parent"),
        );
        check_trusted_path(&scratch).unwrap();
        let work = tempfile::tempdir_in(scratch).unwrap().keep();
        let id = format!(
            "sp-cgroup-{}-{:x}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let runtime = Runtime {
            config: RuntimeConfig {
                console_policy: None,
                host_slot: None,
                jailer: "/unused/jailer".into(),
                firecracker: "/unused/firecracker".into(),
                chroot_base: work.clone(),
                kernel: "/unused/kernel".into(),
                rootfs: "/unused/rootfs".into(),
                vm_uid: 60001,
                vm_gid: 60001,
                vcpu_count: 1,
                memory_mib: 128,
                host_resources: Some(HostResources {
                    cgroup_parent: parent,
                    ..settings()
                }),
                private_netns: None,
                access_token_hash: None,
            },
        };
        runtime.claim_resource(&id).unwrap();
        eprintln!("cgroup test resource {id}; evidence {}", work.display());
        (runtime, work, id)
    }

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn gated_child(path: &Path, program: &str) -> ChildGuard {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = ChildGuard(
            Command::new("/usr/bin/python3")
                .arg("-c")
                .arg(format!("import sys\nsys.stdin.buffer.read(1)\n{program}"))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        fs::write(path.join("cgroup.procs"), child.0.id().to_string()).unwrap();
        child.0.stdin.take().unwrap().write_all(b"x").unwrap();
        child
    }

    fn counter(path: &Path, name: &str) -> u64 {
        let text = fs::read_to_string(path).unwrap();
        text.lines()
            .find_map(|line| {
                let (key, value) = line.split_once(' ')?;
                (key == name).then(|| value.parse::<u64>().unwrap())
            })
            .expect("required kernel counter")
    }

    #[test]
    #[ignore = "requires root, at least two available CPUs and a dedicated cgroup v2 parent"]
    fn real_cgroup_throttles_cpu_and_contains_memory_oom() {
        use std::os::unix::process::ExitStatusExt;
        use std::time::{Duration, Instant};
        assert!(
            std::thread::available_parallelism().unwrap().get() >= 2,
            "CPU enforcement probe requires at least two available host CPUs"
        );
        let (runtime, work, id) = fixture();
        runtime.prepare_cgroup(&id).unwrap();
        let path = runtime
            .config
            .host_resources
            .as_ref()
            .unwrap()
            .cgroup_parent
            .join(&id);
        let before = counter(&path.join("cpu.stat"), "nr_throttled");
        {
            // Both children wait for membership before starting load.
            let _first = gated_child(&path, "while True: pass");
            let _second = gated_child(&path, "while True: pass");
            let deadline = Instant::now() + Duration::from_secs(5);
            while counter(&path.join("cpu.stat"), "nr_throttled") == before {
                assert!(
                    Instant::now() < deadline,
                    "host CPU limit did not throttle competing workers"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let before = counter(&path.join("memory.events"), "oom_kill");
        {
            let mut child = gated_child(&path, "data = bytearray(512 * 1024 * 1024)");
            let deadline = Instant::now() + Duration::from_secs(10);
            let status = loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    Instant::now() < deadline,
                    "memory-limited child did not finish"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(status.signal(), Some(9));
            assert!(counter(&path.join("memory.events"), "oom_kill") > before);
        }
        runtime.cleanup_cgroup(&id, false).unwrap();
        runtime.require_cgroup_absent(&id).unwrap();
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    #[ignore = "requires root and an exclusively delegated cgroup v2 parent"]
    fn real_cgroup_rejects_tampering_live_tasks_and_replacement() {
        let (runtime, work, id) = fixture();
        runtime.prepare_cgroup(&id).unwrap();
        let config = runtime.config.host_resources.as_ref().unwrap();
        let path = config.cgroup_parent.join(&id);
        let inode = runtime.cgroup_inode(&id).unwrap().unwrap();
        runtime.verify_cgroup(&id, None).unwrap();
        fs::write(path.join("cpu.max"), "200000 100000").unwrap();
        assert!(runtime.verify_cgroup(&id, None).is_err());
        fs::write(path.join("cpu.max"), "100000 100000").unwrap();
        {
            let child = ChildGuard(Command::new("/usr/bin/sleep").arg("60").spawn().unwrap());
            fs::write(path.join("cgroup.procs"), child.0.id().to_string()).unwrap();
            runtime.verify_cgroup(&id, Some(child.0.id())).unwrap();
            assert!(runtime.cleanup_cgroup(&id, false).is_err());
            assert_eq!(runtime.cgroup_inode(&id).unwrap(), Some(inode));
        }
        let disabled = Runtime {
            config: RuntimeConfig {
                console_policy: None,
                host_resources: None,
                ..runtime.config.clone()
            },
        };
        assert!(disabled.require_cgroup_absent(&id).is_err());
        runtime.cleanup_cgroup(&id, false).unwrap();
        runtime.require_cgroup_absent(&id).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
        assert!(runtime.cgroup_inode(&id).is_err());
        assert!(runtime.cleanup_cgroup(&id, false).is_err());
        // This replacement was created by this test; production refused it.
        fs::remove_dir(&path).unwrap();
        runtime.cleanup_cgroup(&id, false).unwrap();
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    #[ignore = "requires root and an exclusively delegated cgroup v2 parent"]
    fn real_interrupted_creation_is_reclaimed_without_adopting_replacement() {
        let (runtime, work, id) = fixture();
        let config = runtime.config.host_resources.as_ref().unwrap();
        config.validate(runtime.config.memory_mib).unwrap();
        let claim = runtime.cgroup_claim(&id);
        let path = config.cgroup_parent.join(&id);
        write_new_record(
            &claim.join("cgroup-intent.json"),
            &runtime.cgroup_intent(&id, config).unwrap(),
        )
        .unwrap();
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        runtime.reclaim_unlaunched(&id).unwrap();
        assert!(runtime.was_reclaimed_unlaunched(&id).unwrap());
        runtime.reclaim_unlaunched(&id).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        assert!(runtime.was_reclaimed_unlaunched(&id).is_err());
        assert!(runtime.reclaim_unlaunched(&id).is_err());
        fs::remove_dir(path).unwrap();
        fs::remove_dir_all(work).unwrap();
    }
}
