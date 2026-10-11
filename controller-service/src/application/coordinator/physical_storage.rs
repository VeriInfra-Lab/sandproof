//! The HTTP executable is not the only admission entry point. Full-capacity
//! library users must supply a host adapter that validates the shared profile.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, CreateOptions, Inner, map_journal,
};
use crate::{
    ControllerJournal,
    physical_storage::{PhysicalStorageProfile, PoolRole},
    template_registry::RegistryConfig,
};

fn full_profile(journal: &ControllerJournal) -> Result<PhysicalStorageProfile, ApplicationError> {
    journal
        .maintenance_config()
        .map_err(map_journal)?
        .and_then(|value| value.capacity)
        .ok_or(ApplicationError::Busy)?;
    journal
        .bounded_host_config()
        .map_err(map_journal)?
        .and_then(|value| value.physical_storage)
        .ok_or(ApplicationError::Busy)
}

pub(super) fn validate_registry_configuration(
    journal: &ControllerJournal,
    internal: bool,
    config: &RegistryConfig,
) -> Result<(), ApplicationError> {
    let profile = full_profile(journal)?;
    if internal {
        return Err(ApplicationError::Invalid);
    }
    profile
        .validate_registry(journal.deployment_namespace(), config)
        .map_err(map_journal)?;
    Ok(())
}

pub(super) fn validate_builder_configuration(
    journal: &ControllerJournal,
) -> Result<(), ApplicationError> {
    if full_profile(journal)?
        .pool(journal.deployment_namespace())
        .map_err(map_journal)?
        .role
        != PoolRole::Builder
    {
        return Err(ApplicationError::Invalid);
    }
    Ok(())
}

impl<H: ControllerHost> ControllerApplication<H> {
    pub(super) fn validate_publication_admission(
        &self,
        inner: &Inner<H>,
    ) -> Result<(), ApplicationError> {
        let required = self.physical_admission_bytes(inner, &CreateOptions::default(), 0)?;
        if inner.host.available_disk_bytes()? < required {
            return Err(ApplicationError::QuotaExceeded);
        }
        Ok(())
    }

    pub(super) fn physical_admission_bytes(
        &self,
        inner: &Inner<H>,
        options: &CreateOptions,
        disk_bytes: u64,
    ) -> Result<u64, ApplicationError> {
        let role = if self.internal_build_vms {
            PoolRole::Builder
        } else {
            PoolRole::Runtime
        };
        if self.internal_build_vms
            && (inner
                .journal
                .build_retry_config()
                .map_err(map_journal)?
                .is_none()
                || !options.metadata.contains_key("_sandproof_build_window"))
        {
            return Err(ApplicationError::Busy);
        }
        self.physical_capacity_bytes(inner, options, disk_bytes, role)
    }

    /// Internal lifecycle uses the original registered pool, not the removed
    /// public/private build product's second retry-window metadata or journal.
    pub(super) fn internal_physical_admission_bytes(
        &self,
        inner: &Inner<H>,
        options: &CreateOptions,
        disk_bytes: u64,
    ) -> Result<u64, ApplicationError> {
        let profile = full_profile(&inner.journal)?;
        let role = profile
            .pool(inner.journal.deployment_namespace())
            .map_err(map_journal)?
            .role;
        self.physical_capacity_bytes(inner, options, disk_bytes, role)
    }

    fn physical_capacity_bytes(
        &self,
        inner: &Inner<H>,
        options: &CreateOptions,
        disk_bytes: u64,
        role: PoolRole,
    ) -> Result<u64, ApplicationError> {
        let maintenance = inner.journal.maintenance_config().map_err(map_journal)?;
        let bounded = inner.journal.bounded_host_config().map_err(map_journal)?;
        let profile = bounded
            .as_ref()
            .and_then(|value| value.physical_storage.as_ref());
        let full = maintenance
            .as_ref()
            .and_then(|value| value.capacity.as_ref());
        let capacity = full.ok_or(ApplicationError::Busy)?;
        let profile = profile.ok_or(ApplicationError::Busy)?;
        let pool = profile
            .pool(inner.journal.deployment_namespace())
            .map_err(map_journal)?;
        pool.validate_limits(&self.config.limits)
            .map_err(map_journal)?;
        if pool.role != role {
            return Err(ApplicationError::Busy);
        }
        if let Some(registry) = &self.registry {
            profile
                .validate_registry(inner.journal.deployment_namespace(), registry.config())
                .map_err(map_journal)?;
        }
        if bounded
            .as_ref()
            .map(|value| value.maintenance.max_host_slots)
            != Some(pool.max_host_slots)
            || pool
                .host_metadata_bytes
                .checked_add(capacity.max_online_storage_bytes)
                .is_none_or(|need| need > pool.cleanup_reserve_bytes)
            || options
                .template
                .as_ref()
                .is_some_and(|template| template.kernel.size_bytes > pool.kernel_copy_bytes)
            || disk_bytes > pool.rootfs_budget_bytes
        {
            return Err(ApplicationError::Busy);
        }
        inner.host.validate_physical_storage(profile, pool)?;
        // Do not add the new disk again: the profile already includes the full
        // rootfs quota of both pools, pending sparse growth and unknown cleanup.
        profile.peak_bytes().map_err(map_journal)
    }
}
