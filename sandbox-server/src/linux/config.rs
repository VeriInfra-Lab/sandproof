//! Parsed deployment settings and pure configuration obligations.

use crate::gateway;
use sandbox_controller_core as core;
use sandproof_api::managed::ManagedConfig;
use sandproof_controller_service::{
    AdmissionLimits, BoundedHostConfig, MaintenanceConfig, templates::TemplateCatalog,
};
use sandproof_node::NodeConfig;
use serde::Deserialize;
use std::{error::Error, net::SocketAddr, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CredentialFile {
    pub(super) tenant: u64,
    pub(super) secret: String,
}

/// User settings exclude derived durable limits to avoid two competing
/// sources of truth for the gateway's physical route-cell capacity.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BoundedHostSettings {
    #[serde(default)]
    pub(super) physical_storage:
        Option<sandproof_controller_service::physical_storage::PhysicalStorageProfile>,
    #[serde(default)]
    pub(super) console_policy: Option<sandproof_controller_service::ConsolePolicy>,
    pub(super) maintenance: core::maintenance::MaintenanceLimits,
    pub(super) uid_base: u32,
    pub(super) uplink_base: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigFile {
    pub(super) namespace: String,
    pub(super) database: PathBuf,
    pub(super) node_state_dir: PathBuf,
    pub(super) jailer: PathBuf,
    pub(super) firecracker: PathBuf,
    pub(super) chroot_base: PathBuf,
    pub(super) kernel: PathBuf,
    pub(super) rootfs: PathBuf,
    pub(super) uid_base: u32,
    /// Explicit physical-position maintenance limits; no production defaults.
    #[serde(default)]
    pub(super) bounded_host: Option<BoundedHostSettings>,
    /// Complete formal deployment policy; absence is rejected before activation.
    #[serde(default)]
    pub(super) maintenance: Option<MaintenanceConfig>,
    /// Explicit private-pool retry-window policy; cannot be disabled once persisted.
    #[serde(default)]
    pub(super) build_retry_windows: Option<sandproof_controller_service::BuildRetryConfig>,
    pub(super) vcpu_count: u8,
    pub(super) memory_mib: u32,
    #[serde(default)]
    pub(super) host_resources: Option<sandbox_runtime::HostResources>,
    pub(super) file_policy_digest: u64,
    pub(super) bind: SocketAddr,
    pub(super) domain: String,
    pub(super) credentials: Vec<CredentialFile>,
    pub(super) max_active_global: u64,
    pub(super) max_active_per_tenant: u64,
    pub(super) max_disk_bytes_global: u64,
    pub(super) max_disk_bytes_per_tenant: u64,
    pub(super) cleanup_reserve_bytes: u64,
    pub(super) max_timeout_secs: u64,
    #[serde(default)]
    pub(super) max_lifetime_secs: Option<u64>,
    #[serde(default = "default_startup_timeout_secs")]
    pub(super) startup_timeout_secs: u64,
    #[serde(default = "default_timeout_secs")]
    pub(super) default_timeout_secs: u64,
    #[serde(default)]
    public_ipv4_enabled: bool,
    pub(super) expiry_scan_seconds: u64,
    /// Explicit deployment budgets, never SDK request parameters or inferred defaults.
    #[serde(default)]
    pub(super) data_transfer: Option<gateway::TransferLimits>,
    /// Explicit opt-in: trusted prebuilt descriptors, not a build service.
    #[serde(default)]
    pub(super) templates: Option<TemplateCatalog>,
    #[serde(default)]
    pub(super) template_registry:
        Option<sandproof_controller_service::template_registry::RegistryConfig>,
    /// Private, separate build pool only; never enable on the runtime endpoint.
    #[serde(default)]
    pub(super) internal_build_vms: bool,
}

fn default_timeout_secs() -> u64 {
    sandproof_controller_service::DEFAULT_TIMEOUT_SECS
}

fn default_startup_timeout_secs() -> u64 {
    sandproof_controller_service::DEFAULT_STARTUP_TIMEOUT_SECS
}

/// Parsed settings are not a deployment capability. Only this constructor
/// can produce the complete configuration consumed by the formal startup.
/// Static validation does not acquire a physical lease or open a journal.
pub(super) struct DeploymentContract {
    pub(super) bounded: BoundedHostConfig,
    pub(super) maintenance: MaintenanceConfig,
    pub(super) transfers: gateway::TransferLimits,
}

impl DeploymentContract {
    pub(super) fn validate(config: &ConfigFile) -> Result<Self, Box<dyn Error>> {
        let bounded = resolve_bounded_host_config(
            config.uid_base,
            config.bounded_host.as_ref(),
            config.data_transfer.as_ref(),
        )?
        .ok_or("formal service requires explicit bounded_host")?;
        let maintenance = config
            .maintenance
            .clone()
            .ok_or("formal service requires explicit maintenance")?;
        let transfers = config
            .data_transfer
            .clone()
            .ok_or("formal service requires explicit data_transfer")?;
        if bounded.physical_storage.is_none() {
            return Err("formal service requires explicit physical_storage".into());
        }
        validate_maintenance_config(Some(&bounded), Some(&maintenance))?;
        validate_build_retry_config(config, Some(&bounded))?;
        validate_controller_storage_reserve(config)?;
        validate_lifetime_settings(config)?;
        if [
            config.max_active_global,
            config.max_active_per_tenant,
            config.max_disk_bytes_global,
            config.max_disk_bytes_per_tenant,
            config.cleanup_reserve_bytes,
        ]
        .contains(&0)
            || config.cleanup_reserve_bytes >= config.max_disk_bytes_global
        {
            return Err("invalid deployment admission limits".into());
        }
        if config.internal_build_vms && config.template_registry.is_some() {
            return Err("internal build pool cannot also publish templates".into());
        }
        Ok(Self {
            bounded,
            maintenance,
            transfers,
        })
    }
}

pub(super) fn node_configuration(
    config: &ConfigFile,
    bounded_host: BoundedHostConfig,
) -> NodeConfig {
    NodeConfig {
        namespace: config.namespace.clone(),
        state_dir: config.node_state_dir.clone(),
        jailer: config.jailer.clone(),
        firecracker: config.firecracker.clone(),
        chroot_base: config.chroot_base.clone(),
        kernel: config.kernel.clone(),
        rootfs: config.rootfs.clone(),
        bounded_host,
        vcpu_count: config.vcpu_count,
        memory_mib: config.memory_mib,
        host_resources: config.host_resources.clone(),
        file_policy_digest: core::PolicyDigest(config.file_policy_digest),
    }
}

fn validate_lifetime_settings(config: &ConfigFile) -> Result<(), Box<dyn Error>> {
    if config.expiry_scan_seconds == 0
        || config.startup_timeout_secs == 0
        || config.default_timeout_secs != default_timeout_secs()
        || config.default_timeout_secs > config.max_timeout_secs
        || config.max_lifetime_secs == Some(0)
    {
        return Err("invalid lifetime scanner/limit; E2B default must be 300 seconds".into());
    }
    Ok(())
}

pub(super) fn resolve_bounded_host_config(
    uid_base: u32,
    bounded: Option<&BoundedHostSettings>,
    data_transfer: Option<&gateway::TransferLimits>,
) -> Result<Option<BoundedHostConfig>, Box<dyn Error>> {
    let Some(settings) = bounded else {
        return Ok(None);
    };
    let transfers = data_transfer.ok_or("bounded host requires explicit data_transfer limits")?;
    transfers.validate()?;
    let bounded = BoundedHostConfig {
        physical_storage: settings.physical_storage.clone(),
        console_policy: settings.console_policy,
        maintenance: settings.maintenance.clone(),
        uid_base: settings.uid_base,
        uplink_base: settings.uplink_base,
        max_data_routes_per_slot: u64::try_from(transfers.max_transfers_per_vm)?,
        max_data_routes_global: u64::try_from(transfers.max_transfers)?,
        data_route_control_reserve: u64::try_from(transfers.control_reserve)?,
    };
    bounded.validate()?;
    if bounded.uid_base != uid_base {
        return Err("bounded host UID base differs from node UID base".into());
    }
    Ok(Some(bounded))
}

fn validate_maintenance_config(
    bounded: Option<&BoundedHostConfig>,
    maintenance: Option<&MaintenanceConfig>,
) -> Result<(), Box<dyn Error>> {
    if let Some(maintenance) = maintenance {
        if bounded.is_none() {
            return Err("maintenance requires explicit bounded_host configuration".into());
        }
        maintenance.validate()?;
        if maintenance.capacity.is_none() {
            return Err(
                "production maintenance requires explicit cleanup capacity reservations".into(),
            );
        }
        if maintenance.capacity.is_some()
            && bounded.and_then(|config| config.console_policy)
                != Some(sandproof_controller_service::ConsolePolicy::Discard)
        {
            return Err(
                "full bounded capacity requires explicit bounded_host.console_policy=discard"
                    .into(),
            );
        }
    }
    Ok(())
}

fn validate_controller_storage_reserve(config: &ConfigFile) -> Result<(), Box<dyn Error>> {
    if let Some(capacity) = config
        .maintenance
        .as_ref()
        .and_then(|policy| policy.capacity.as_ref())
        && config.cleanup_reserve_bytes < capacity.max_online_storage_bytes
    {
        return Err(
            "cleanup_reserve_bytes must cover the full controller DB/WAL/SHM/checkpoint envelope"
                .into(),
        );
    }
    Ok(())
}

fn validate_build_retry_config(
    config: &ConfigFile,
    bounded: Option<&BoundedHostConfig>,
) -> Result<(), Box<dyn Error>> {
    if config.internal_build_vms
        && config
            .maintenance
            .as_ref()
            .is_some_and(|policy| policy.capacity.is_some())
        && config.build_retry_windows.is_none()
    {
        return Err("full private capacity requires explicit build_retry_windows".into());
    }
    if let Some(retry) = config.build_retry_windows {
        if !config.internal_build_vms
            || config.maintenance.is_none()
            || config.template_registry.is_some()
        {
            return Err(
                "build_retry_windows requires an explicitly maintained private build pool".into(),
            );
        }
        let bounded = bounded.ok_or("build_retry_windows requires bounded_host")?;
        retry.validate(bounded.maintenance.windows.max_duration_ms())?;
    }
    Ok(())
}

pub(super) fn managed_config(
    config: &ConfigFile,
    disk_bytes: u64,
) -> Result<ManagedConfig, Box<dyn Error>> {
    Ok(ManagedConfig {
        public_ipv4_enabled: config.public_ipv4_enabled,
        limits: AdmissionLimits {
            max_active_global: config.max_active_global,
            max_active_per_tenant: config.max_active_per_tenant,
            max_disk_bytes_global: config.max_disk_bytes_global,
            max_disk_bytes_per_tenant: config.max_disk_bytes_per_tenant,
            cleanup_reserve_bytes: config.cleanup_reserve_bytes,
            max_timeout_secs: config.max_timeout_secs,
            max_lifetime_secs: config.max_lifetime_secs.unwrap_or(config.max_timeout_secs),
            startup_timeout_secs: config.startup_timeout_secs,
        },
        disk_bytes,
        file_policy_digest: core::PolicyDigest(config.file_policy_digest),
        cpu_count: u32::from(config.vcpu_count),
        memory_mb: config.memory_mib,
        disk_size_mb: u32::try_from(disk_bytes.div_ceil(1024 * 1024))?,
        envd_version: "0.9.0".into(),
        domain: config.domain.clone(),
    })
}

#[cfg(test)]
mod tests;
