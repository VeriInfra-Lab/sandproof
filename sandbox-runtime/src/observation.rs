//! Physical observation does not need, replace, or grant a guest credential.

use super::{
    ConsolePolicy, HostResources, JailObservation, Observation, Runtime, RuntimeConfig,
    RuntimeError, RuntimeHostSlot,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Original admitted physical configuration, with no token or MMDS fields.
/// The caller must bind it to the durable sandbox, generation and host position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeObservationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_policy: Option<ConsolePolicy>,
    #[serde(default)]
    pub host_slot: Option<RuntimeHostSlot>,
    pub jailer: PathBuf,
    pub firecracker: PathBuf,
    pub chroot_base: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    pub vm_uid: u32,
    pub vm_gid: u32,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    pub host_resources: Option<HostResources>,
    pub private_netns: Option<PathBuf>,
}

impl From<RuntimeConfig> for RuntimeObservationConfig {
    fn from(config: RuntimeConfig) -> Self {
        Self {
            console_policy: config.console_policy,
            host_slot: config.host_slot,
            jailer: config.jailer,
            firecracker: config.firecracker,
            chroot_base: config.chroot_base,
            kernel: config.kernel,
            rootfs: config.rootfs,
            vm_uid: config.vm_uid,
            vm_gid: config.vm_gid,
            vcpu_count: config.vcpu_count,
            memory_mib: config.memory_mib,
            host_resources: config.host_resources,
            private_netns: config.private_netns,
        }
    }
}

impl RuntimeObservationConfig {
    // Private conversion keeps the credential-less Runtime confined to the
    // observer. It cannot be obtained by an executor or used to launch a VM.
    fn into_runtime_config(self) -> RuntimeConfig {
        RuntimeConfig {
            console_policy: self.console_policy,
            host_slot: self.host_slot,
            jailer: self.jailer,
            firecracker: self.firecracker,
            chroot_base: self.chroot_base,
            kernel: self.kernel,
            rootfs: self.rootfs,
            vm_uid: self.vm_uid,
            vm_gid: self.vm_gid,
            vcpu_count: self.vcpu_count,
            memory_mib: self.memory_mib,
            host_resources: self.host_resources,
            private_netns: self.private_netns,
            access_token_hash: None,
        }
    }
}

/// Narrow access to existing physical witnesses, never execution authority.
/// Its private runtime is not exposed or convertible to an executable Runtime.
pub struct RuntimePhysicalObserver {
    runtime: Runtime,
}

impl RuntimePhysicalObserver {
    /// Construct an observer using the same physical validation as an executor.
    /// No source image or guest credential is needed for existing resources.
    /// # Errors
    /// Rejects untrusted executor paths, machine limits and host resources.
    pub fn for_existing(config: RuntimeObservationConfig) -> Result<Self, RuntimeError> {
        let config = config.into_runtime_config();
        Runtime::validate_existing_physical(&config)?;
        Ok(Self {
            runtime: Runtime { config },
        })
    }

    /// Observe only the process bound to the exact recorded resource.
    /// # Errors
    /// Rejects corrupted records, jail witnesses or changed physical resources.
    pub fn observe(&self, id: &str) -> Result<Observation, RuntimeError> {
        self.runtime.observe(id)
    }

    /// Read-only terminal evidence for an exactly reclaimed unlaunched VM.
    /// # Errors
    /// Rejects mismatched markers or surviving physical resources.
    pub fn was_reclaimed_unlaunched(&self, id: &str) -> Result<bool, RuntimeError> {
        self.runtime.was_reclaimed_unlaunched(id)
    }

    /// Read-only terminal evidence for an exactly reclaimed stopped prepare.
    /// # Errors
    /// Rejects live processes, mismatched markers or surviving resources.
    pub fn was_reclaimed_stopped_prepare(&self, id: &str) -> Result<bool, RuntimeError> {
        self.runtime.was_reclaimed_stopped_prepare(id)
    }

    /// Observe the recorded namespace and its private gate or public policy.
    /// # Errors
    /// Rejects changed namespace, uplink or firewall witnesses.
    pub fn observe_recorded_network(&self, id: &str) -> Result<u64, RuntimeError> {
        self.runtime.observe_recorded_network(id)
    }

    /// Read-only jail identity evidence, without adopting a new directory.
    /// # Errors
    /// Rejects corrupt witnesses, replacement directories and changed context.
    pub fn observe_jail(&self, id: &str) -> Result<JailObservation, RuntimeError> {
        self.runtime.observe_jail(id)
    }

    /// Observe the exact recorded cgroup, including a stopped VM's group.
    /// # Errors
    /// Rejects changed parent, boot, limits, permissions or kernel identity.
    pub fn cgroup_inode(&self, id: &str) -> Result<Option<u64>, RuntimeError> {
        self.runtime.cgroup_inode(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> RuntimeObservationConfig {
        RuntimeObservationConfig {
            console_policy: Some(ConsolePolicy::Discard),
            host_slot: None,
            jailer: "/unused/jailer".into(),
            firecracker: "/unused/firecracker".into(),
            chroot_base: "/unused/jails".into(),
            kernel: "/unused/kernel".into(),
            rootfs: "/unused/rootfs".into(),
            vm_uid: 10_001,
            vm_gid: 10_001,
            vcpu_count: 1,
            memory_mib: 128,
            host_resources: None,
            private_netns: Some("/run/netns/vm-test-1".into()),
        }
    }

    #[test]
    fn observation_config_excludes_credentials_without_disabling_private_network() {
        let physical = config();
        let encoded = serde_json::to_value(&physical).unwrap();
        assert!(encoded.get("access_token_hash").is_none());
        let runtime = physical.into_runtime_config();
        assert!(runtime.access_token_hash.is_none());
        assert_eq!(runtime.private_netns, config().private_netns);
        assert!(Runtime::validate_physical_values(&runtime).is_ok());
        assert!(Runtime::validate_namespace_config(&runtime).is_ok());
        assert!(Runtime::validate_access_token_config(&runtime).is_err());
        let mut executable = runtime;
        executable.access_token_hash = Some("f".repeat(128));
        assert_eq!(RuntimeObservationConfig::from(executable), config());
        let mut unexpected = encoded;
        unexpected["access_token_hash"] = "f".repeat(128).into();
        assert!(serde_json::from_value::<RuntimeObservationConfig>(unexpected).is_err());
    }

    #[test]
    fn observation_reuses_physical_and_namespace_value_checks() {
        let runtime = config().into_runtime_config();
        for field in ["uid", "gid", "cpu", "memory"] {
            let mut invalid = runtime.clone();
            match field {
                "uid" => invalid.vm_uid = 0,
                "gid" => invalid.vm_gid = 0,
                "cpu" => invalid.vcpu_count = 0,
                "memory" => invalid.memory_mib = 127,
                _ => unreachable!(),
            }
            assert!(Runtime::validate_physical_values(&invalid).is_err());
        }
        for path in ["/tmp/vm-test-1", "/run/netns/nested/vm-test-1", "vm-test-1"] {
            let mut invalid = runtime.clone();
            invalid.private_netns = Some(path.into());
            assert!(Runtime::validate_namespace_config(&invalid).is_err());
        }
        let slot = RuntimeHostSlot {
            position: 1_048_575,
            uplink_index: 1_048_575,
            vm: 1,
            helper_gate: super::super::RuntimeHelperGate {
                path: "/unused/helper.lock".into(),
                device: 1,
                inode: 1,
                max_data_routes_per_slot: 1,
            },
        };
        let mut bounded = runtime;
        bounded.host_slot = Some(slot.clone());
        assert!(Runtime::validate_physical_values(&bounded).is_ok());
        for invalid_slot in [
            RuntimeHostSlot {
                position: 1_048_576,
                ..slot.clone()
            },
            RuntimeHostSlot {
                uplink_index: 1_048_576,
                ..slot
            },
        ] {
            bounded.host_slot = Some(invalid_slot);
            assert!(Runtime::validate_physical_values(&bounded).is_err());
        }
    }

    #[test]
    fn executable_runtime_still_requires_exact_hash_and_network_pair() {
        let mut runtime = config().into_runtime_config();
        for hash in [None, Some("f".repeat(127)), Some("z".repeat(128))] {
            runtime.access_token_hash = hash;
            assert!(Runtime::validate_access_token_config(&runtime).is_err());
        }
        runtime.access_token_hash = Some("f".repeat(128));
        assert!(Runtime::validate_access_token_config(&runtime).is_ok());
        runtime.private_netns = None;
        assert!(Runtime::validate_access_token_config(&runtime).is_err());
        runtime.access_token_hash = None;
        assert!(Runtime::validate_access_token_config(&runtime).is_ok());
    }
}
