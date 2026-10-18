//! Preserve shared journal failure classes at the autonomous host boundary.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, JournalError, map_admission,
    map_journal,
};
use crate::{physical_storage::PhysicalTemplateCache, templates::ComputeLimits};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::internal::lifecycle::Resources;
use sandproof_protocol::numeric::available_resource;

impl<H: ControllerHost> ControllerApplication<H> {
    /// Original profile/lease and complete peak check before managed cache I/O.
    /// # Errors
    /// Unregistered cache, corrupt authority or low physical reserve refuses.
    pub fn validate_internal_cache_storage(
        &self,
        node: &NodeIdentity,
        cache: &PhysicalTemplateCache,
    ) -> Result<(), ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        inner.journal.internal_pending(node).map_err(map_journal)?;
        let bounded = inner
            .journal
            .bounded_host_config()
            .map_err(map_journal)?
            .ok_or(ApplicationError::Busy)?;
        let profile = bounded.physical_storage.ok_or(ApplicationError::Busy)?;
        if profile
            .template_cache(inner.journal.deployment_namespace())
            .map_err(map_journal)?
            != cache
        {
            return Err(ApplicationError::Invalid);
        }
        let pool = profile
            .pool(inner.journal.deployment_namespace())
            .map_err(map_journal)?;
        inner.host.validate_physical_storage(&profile, pool)?;
        if inner.host.available_disk_bytes()? < profile.peak_bytes().map_err(map_journal)? {
            return Err(ApplicationError::QuotaExceeded);
        }
        Ok(())
    }

    /// A current physical veto, never permission to release center reservations.
    /// Uses the SAME checked authority, original resource accounting and lease.
    /// # Errors
    /// Unknown host state or shared corruption cannot produce a healthy report.
    pub fn internal_free_resources(
        &self,
        node: &NodeIdentity,
        compute: ComputeLimits,
    ) -> Result<(u64, Resources), ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let slots =
            inner
                .journal
                .internal_available_instances(node)
                .map_err(|error| match error {
                    JournalError::Conflict => ApplicationError::Invalid,
                    other => map_journal(other),
                })?;
        let usage = inner
            .journal
            .retained_resource_usage()
            .map_err(map_admission)?;
        let bounded = inner
            .journal
            .bounded_host_config()
            .map_err(map_journal)?
            .ok_or(ApplicationError::Busy)?;
        let profile = bounded.physical_storage.ok_or(ApplicationError::Busy)?;
        let pool = profile
            .pool(inner.journal.deployment_namespace())
            .map_err(map_journal)?;
        inner.host.validate_physical_storage(&profile, pool)?;
        let disk = inner
            .host
            .available_disk_bytes()?
            .saturating_sub(profile.peak_bytes().map_err(map_journal)?);
        let (cpu, memory) = inner.host.available_compute()?;
        let free = Resources {
            instances: available_resource(
                self.config.limits.max_active_global,
                usage.instances,
                slots,
            ),
            cpu: available_resource(compute.max_vcpus_global, usage.cpu, cpu),
            memory_bytes: available_resource(
                compute.max_memory_bytes_global,
                usage.memory_bytes,
                memory,
            ),
            disk_bytes: available_resource(pool.rootfs_budget_bytes, usage.disk_bytes, disk),
        };
        Ok((
            u64::try_from(self.clock.now_ms()?).map_err(|_| ApplicationError::Invalid)?,
            free,
        ))
    }
    /// Inspect the ORIGINAL authority under its application lock. This is not
    /// physical health or a check of every guest credential, and grants no
    /// permission. Unlike public error mapping it retains shared failure class.
    /// # Errors
    /// Poisoned lock, wrong registered identity or invalid original authority.
    pub fn check_internal_authority(&self, node: &NodeIdentity) -> Result<(), JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .internal_pending(node)
            .map(|_| ())
    }
}
