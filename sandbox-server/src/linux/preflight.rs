//! Filesystem, journal and physical-profile checks before host activation.

use super::config::{DeploymentContract, NativeSettings};
use sandproof_controller_service::ControllerJournal;
use sandproof_node::NodeConfig;
use std::{
    error::Error,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub(super) fn open_journal(
    database: PathBuf,
    namespace: &str,
    contract: &DeploymentContract,
) -> Result<ControllerJournal, Box<dyn Error>> {
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?;
    Ok(ControllerJournal::open_complete(
        database,
        namespace,
        contract.bounded.clone(),
        contract.maintenance.clone(),
        now_ms,
    )?)
}

pub(super) fn validate_physical_storage(
    config: &NativeSettings,
    node: &NodeConfig,
) -> Result<sandproof_node::PhysicalProfileLease, Box<dyn Error>> {
    let profile = node.bounded_host.physical_storage.as_ref();
    let capacity = config
        .maintenance
        .as_ref()
        .and_then(|value| value.capacity.as_ref());
    let profile =
        profile.ok_or("full bounded capacity requires an explicit physical_storage profile")?;
    let capacity = capacity.ok_or("physical_storage requires full controller capacity")?;
    let pool = profile.pool(&config.namespace)?;
    let expected_kernel = config
        .templates
        .as_ref()
        .map_or(0, |catalog| {
            catalog
                .templates
                .iter()
                .map(|template| template.kernel.size_bytes)
                .max()
                .unwrap_or(0)
        })
        .max(fs::metadata(&config.kernel)?.len());
    let envelope = sandproof_node::HostMetadataEnvelope::derive(node, capacity.max_live_sandboxes)?;
    if pool.database != config.database
        || pool.node_state_dir != config.node_state_dir
        || pool.chroot_base != config.chroot_base
        || node.bounded_host.maintenance.max_host_slots != pool.max_host_slots
        || config
            .max_disk_bytes_global
            .checked_sub(config.cleanup_reserve_bytes)
            != Some(pool.rootfs_budget_bytes)
        || pool.cleanup_reserve_bytes != config.cleanup_reserve_bytes
        || pool.kernel_copy_bytes < expected_kernel
        || pool.runtime_copy_bytes < fs::metadata(&config.firecracker)?.len()
        || pool.host_metadata_bytes < envelope.payload_bytes
        || pool.host_metadata_inodes < envelope.inode_count
        || pool
            .host_metadata_bytes
            .checked_add(capacity.max_online_storage_bytes)
            .is_none_or(|bytes| bytes > pool.cleanup_reserve_bytes)
    {
        return Err("physical profile does not cover this exact pool configuration".into());
    }
    let space = sandproof_node::disk_space(&profile.coordination_file)?;
    if profile
        .filesystem_overhead_inodes
        .checked_mul(space.fragment_bytes)
        .is_none_or(|bytes| bytes > profile.filesystem_overhead_bytes)
    {
        return Err("physical filesystem overhead must cover per-inode block rounding".into());
    }
    sandproof_node::PhysicalProfileLease::activate(
        profile,
        &config.namespace,
        &config.firecracker,
        &config.kernel,
    )
}

pub(super) fn private_root_path(path: &Path) -> Result<(), Box<dyn Error>> {
    sandbox_runtime::check_trusted_path(path)?;
    if !path.is_absolute() || path.is_symlink() {
        return Err("configuration path must be absolute and not a symlink".into());
    }
    let metadata = fs::metadata(path)?;
    if metadata.uid() != 0 || metadata.permissions().mode() & 0o077 != 0 {
        return Err("configuration and database directory must be root-only".into());
    }
    Ok(())
}

pub(super) fn validate_config(config: &NativeSettings) -> Result<u64, Box<dyn Error>> {
    if let Some(catalog) = &config.templates {
        catalog
            .validate()
            .map_err(|_| "invalid prebuilt template catalog")?;
        if config.host_resources.is_none() {
            return Err("templates require explicit host_resources".into());
        }
    }
    private_root_path(config.database.parent().ok_or("database parent missing")?)?;
    crate::storage::validate_filesystems(
        &config.database,
        &config.node_state_dir,
        &config.chroot_base,
    )?;
    let disk_bytes = fs::metadata(&config.rootfs)?.len();
    if disk_bytes == 0 || disk_bytes > config.max_disk_bytes_per_tenant {
        return Err("root disk exceeds per-tenant budget".into());
    }
    Ok(disk_bytes)
}

pub(super) fn preflight_journal_configuration(
    config: &NativeSettings,
    contract: &DeploymentContract,
) -> Result<(), Box<dyn Error>> {
    ControllerJournal::preflight_build_retry_windows(&config.database, &config.namespace, None)?;
    ControllerJournal::preflight_complete(
        &config.database,
        &config.namespace,
        &contract.bounded,
        &contract.maintenance,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
