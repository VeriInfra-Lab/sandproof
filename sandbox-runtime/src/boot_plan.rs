//! One source for actual startup configuration and source-profile checks.
//! Mutable credentials and `InstanceStart` are deliberately separate effects.
use super::{RuntimeConfig, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "boot_configuration.rs"]
mod configuration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BootPlan {
    machine: Machine,
    boot: BootSource,
    root_drive: RootDrive,
    network: Option<Network>,
    mmds: Option<Mmds>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Machine {
    vcpu_count: u8,
    mem_size_mib: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootSource {
    kernel_image_path: String,
    boot_args: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootDrive {
    drive_id: String,
    path_on_host: String,
    is_root_device: bool,
    is_read_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Network {
    iface_id: String,
    host_dev_name: String,
    guest_mac: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mmds {
    version: String,
    network_interfaces: Vec<String>,
}

impl BootPlan {
    pub(super) fn for_runtime(config: &RuntimeConfig) -> Self {
        let networked = config.access_token_hash.is_some();
        Self {
            machine: Machine {
                vcpu_count: config.vcpu_count,
                mem_size_mib: config.memory_mib,
            },
            boot: BootSource {
                kernel_image_path: "/vmlinux".into(),
                boot_args: "console=ttyS0 reboot=k panic=1 pci=off".into(),
            },
            root_drive: RootDrive {
                drive_id: "rootfs".into(),
                path_on_host: "/rootfs.ext4".into(),
                is_root_device: true,
                is_read_only: false,
            },
            network: networked.then(|| Network {
                iface_id: "eth0".into(),
                host_dev_name: "tap0".into(),
                guest_mac: "06:00:ac:10:00:02".into(),
            }),
            mmds: networked.then(|| Mmds {
                version: "V2".into(),
                network_interfaces: vec!["eth0".into()],
            }),
        }
    }

    pub(super) fn digest(&self) -> Result<[u8;32], RuntimeError> {
        use sha2::{Digest,Sha256};
        Ok(Sha256::digest(serde_json::to_vec(self)?).into())
    }

    /// Sends the same ordered requests the original launch used. A failed PUT
    /// never advances to later configuration, credentials or `InstanceStart`.
    pub(super) fn configure(
        &self,
        mut put: impl FnMut(&str, &Value) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        put("/machine-config", &serde_json::to_value(&self.machine)?)?;
        put("/boot-source", &serde_json::to_value(&self.boot)?)?;
        put("/drives/rootfs", &serde_json::to_value(&self.root_drive)?)?;
        if let Some(network) = &self.network {
            put("/network-interfaces/eth0", &serde_json::to_value(network)?)?;
        }
        if let Some(mmds) = &self.mmds {
            put("/mmds/config", &serde_json::to_value(mmds)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "boot_plan_tests.rs"]
mod tests;
