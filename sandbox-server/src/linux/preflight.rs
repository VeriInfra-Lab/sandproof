//! Filesystem, journal and physical-profile checks before host activation.

use super::config::{ConfigFile, DeploymentContract, managed_config};
use sandproof_controller_service::ControllerJournal;
use sandproof_controller_service::application::ApplicationConfig;
use sandproof_node::NodeConfig;
use std::{
    error::Error,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

pub(super) struct ValidatedDeployment {
    pub(super) settings: ConfigFile,
    pub(super) contract: DeploymentContract,
    pub(super) managed: ApplicationConfig,
}

impl ValidatedDeployment {
    pub(super) fn validate(settings: ConfigFile) -> Result<Self, Box<dyn Error>> {
        // All static obligations precede filesystem inspection and activation.
        let contract = DeploymentContract::validate(&settings)?;
        let disk_bytes = validate_config(&settings)?;
        let managed = managed_config(&settings, disk_bytes)?;
        Ok(Self {
            settings,
            contract,
            managed,
        })
    }
}

pub(super) fn validate_physical_storage(
    config: &ConfigFile,
    node: &NodeConfig,
) -> Result<crate::storage::PhysicalProfileLease, Box<dyn Error>> {
    use sandproof_controller_service::physical_storage::PoolRole;
    let profile = node.bounded_host.physical_storage.as_ref();
    let capacity = config
        .maintenance
        .as_ref()
        .and_then(|value| value.capacity.as_ref());
    let profile =
        profile.ok_or("full bounded capacity requires an explicit physical_storage profile")?;
    let capacity = capacity.ok_or("physical_storage requires full controller capacity")?;
    let pool = profile.pool(&config.namespace)?;
    let expected_role = if config.internal_build_vms {
        PoolRole::Builder
    } else {
        PoolRole::Runtime
    };
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
        .max(fs::metadata(&config.kernel)?.len())
        .max(
            config
                .template_registry
                .as_ref()
                .map_or(0, |registry| registry.max_artifact_bytes_per_tenant),
        );
    let envelope = sandproof_node::HostMetadataEnvelope::derive(node, capacity.max_live_sandboxes)?;
    if pool.role != expected_role
        || pool.database != config.database
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
    match (&config.template_registry, &profile.public_build) {
        (Some(registry), Some(public)) if !config.internal_build_vms => {
            profile.validate_registry(&config.namespace, registry)?;
            if public.registry_root != registry.root {
                return Err("physical profile registry root mismatch".into());
            }
        }
        (None, None) if !config.internal_build_vms => {}
        (None, Some(_)) if config.internal_build_vms => {}
        _ => return Err("unsupported physical profile registry/pool composition".into()),
    }
    let space = sandproof_node::disk_space(&profile.coordination_file)?;
    if profile
        .filesystem_overhead_inodes
        .checked_mul(space.fragment_bytes)
        .is_none_or(|bytes| bytes > profile.filesystem_overhead_bytes)
    {
        return Err("physical filesystem overhead must cover per-inode block rounding".into());
    }
    crate::storage::PhysicalProfileLease::activate(
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

fn validate_config(config: &ConfigFile) -> Result<u64, Box<dyn Error>> {
    if let Some(registry) = &config.template_registry {
        if !registry.is_valid() || config.templates.is_none() || config.host_resources.is_none() {
            return Err(
                "template registry requires valid limits, templates and host_resources".into(),
            );
        }
        for path in [
            &registry.root,
            &registry.root.join("inbox"),
            &registry.root.join("artifacts"),
        ] {
            private_root_path(path)?;
            if !fs::metadata(path)?.is_dir() {
                return Err("registry path is not a directory".into());
            }
        }
        crate::storage::validate_filesystems(
            &config.database,
            &registry.root,
            &config.chroot_base,
        )?;
    }
    if let Some(catalog) = &config.templates {
        catalog
            .validate()
            .map_err(|_| "invalid prebuilt template catalog")?;
        if config.host_resources.is_none() {
            return Err("templates require explicit host_resources".into());
        }
    }
    if !config.bind.ip().is_loopback() {
        return Err(
            "HTTP management listener must bind loopback behind a trusted TLS front end".into(),
        );
    }
    if config.credentials.is_empty()
        || config
            .credentials
            .iter()
            .any(|credential| credential.tenant == 0)
    {
        return Err("at least one nonzero tenant credential is required".into());
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
    config: &ConfigFile,
    contract: &DeploymentContract,
) -> Result<(), Box<dyn Error>> {
    ControllerJournal::preflight_build_retry_windows(
        &config.database,
        &config.namespace,
        config.build_retry_windows,
    )?;
    ControllerJournal::preflight_complete(
        &config.database,
        &config.namespace,
        &contract.bounded,
        &contract.maintenance,
    )?;
    Ok(())
}
