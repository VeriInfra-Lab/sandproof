//! Fixed Firecracker 1.16.1 GET projection, checked against the actual BootPlan.
//! This is a visible configuration check, not a CPU or resource certificate.
//! VmmConfig::from omits custom CPU/logger/metrics state even when configured;
//! their null fields cannot prove absence. The original protected launch path
//! and exclusive API writer condition remain necessary.
use super::BootPlan;
use crate::RuntimeError;
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct FullConfiguration {
    balloon: (),
    drives: Vec<Drive>,
    boot_source: Boot,
    cpu_config: (),
    logger: (),
    machine_config: Machine,
    metrics: (),
    mmds_config: Option<Mmds>,
    network_interfaces: Vec<Network>,
    vsock: (),
    entropy: (),
    pmem: Vec<()>,
    memory_hotplug: (),
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Machine {
    vcpu_count: u8,
    mem_size_mib: u32,
    smt: bool,
    cpu_template: Option<String>,
    track_dirty_pages: bool,
    huge_pages: String,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Boot {
    kernel_image_path: String,
    initrd_path: (),
    boot_args: String,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Drive {
    drive_id: String,
    partuuid: (),
    is_root_device: bool,
    cache_type: String,
    is_read_only: bool,
    path_on_host: String,
    rate_limiter: (),
    io_engine: String,
    socket: (),
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Network {
    iface_id: String,
    host_dev_name: String,
    guest_mac: String,
    mtu: (),
    rx_rate_limiter: (),
    tx_rate_limiter: (),
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mmds {
    version: String,
    network_interfaces: Vec<String>,
    ipv4_address: String,
    imds_compat: bool,
}

impl BootPlan {
    /// Compare visible GET fields against the same launch configuration, not
    /// against another copy of the GET or a caller-supplied manifest profile.
    /// Uses only the cold-launch profile currently sent by configure; this is
    /// not a general validator for arbitrary restored or external VMMs.
    pub(crate) fn check_full_configuration(&self, bytes: &[u8]) -> Result<(), RuntimeError> {
        if bytes.is_empty() || bytes.len() > 1_048_576 {
            return Err(RuntimeError::Protocol(
                "source configuration response budget",
            ));
        }
        let actual: FullConfiguration = serde_json::from_slice(bytes)?;
        let expected = FullConfiguration {
            balloon: (),
            drives: vec![Drive {
                drive_id: self.root_drive.drive_id.clone(),
                partuuid: (),
                is_root_device: self.root_drive.is_root_device,
                cache_type: "Unsafe".into(),
                is_read_only: self.root_drive.is_read_only,
                path_on_host: self.root_drive.path_on_host.clone(),
                rate_limiter: (),
                io_engine: "Sync".into(),
                socket: (),
            }],
            boot_source: Boot {
                kernel_image_path: self.boot.kernel_image_path.clone(),
                initrd_path: (),
                boot_args: self.boot.boot_args.clone(),
            },
            cpu_config: (),
            logger: (),
            machine_config: Machine {
                vcpu_count: self.machine.vcpu_count,
                mem_size_mib: self.machine.mem_size_mib,
                smt: false,
                cpu_template: None,
                track_dirty_pages: false,
                huge_pages: "None".into(),
            },
            metrics: (),
            mmds_config: self.mmds.as_ref().map(|mmds| Mmds {
                version: mmds.version.clone(),
                network_interfaces: mmds.network_interfaces.clone(),
                ipv4_address: "169.254.169.254".into(),
                imds_compat: false,
            }),
            network_interfaces: self
                .network
                .iter()
                .map(|net| Network {
                    iface_id: net.iface_id.clone(),
                    host_dev_name: net.host_dev_name.clone(),
                    guest_mac: net.guest_mac.clone(),
                    mtu: (),
                    rx_rate_limiter: (),
                    tx_rate_limiter: (),
                })
                .collect(),
            vsock: (),
            entropy: (),
            pmem: Vec::new(),
            memory_hotplug: (),
        };
        if actual != expected {
            return Err(RuntimeError::IdentityMismatch(
                "source configuration differs from original boot plan",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "boot_configuration_tests.rs"]
mod tests;
