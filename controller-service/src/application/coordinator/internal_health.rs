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
        let (host, bounded, namespace) = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .lifecycle_pending_creates(node)
                .map_err(map_journal)?;
            let bounded = inner
                .journal
                .bounded_host_config()
                .map_err(map_journal)?
                .ok_or(ApplicationError::Busy)?;
            (
                std::sync::Arc::clone(&inner.host),
                bounded,
                inner.journal.deployment_namespace().to_owned(),
            )
        };
        let profile = bounded
            .physical_storage
            .as_ref()
            .ok_or(ApplicationError::Busy)?;
        if profile.template_cache(&namespace).map_err(map_journal)? != cache {
            return Err(ApplicationError::Invalid);
        }
        let pool = profile.pool(&namespace).map_err(map_journal)?;
        host.validate_physical_storage(profile, pool)?;
        if host.available_disk_bytes()? < profile.peak_bytes().map_err(map_journal)? {
            return Err(ApplicationError::QuotaExceeded);
        }
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        inner
            .journal
            .lifecycle_pending_creates(node)
            .map_err(map_journal)?;
        if inner
            .journal
            .bounded_host_config()
            .map_err(map_journal)?
            .as_ref()
            != Some(&bounded)
            || inner.journal.deployment_namespace() != namespace
        {
            return Err(ApplicationError::OutcomeUnknown);
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
        let (host, bounded, namespace) = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .lifecycle_available_instances(node)
                .map_err(map_journal)?;
            let bounded = inner
                .journal
                .bounded_host_config()
                .map_err(map_journal)?
                .ok_or(ApplicationError::Busy)?;
            (
                std::sync::Arc::clone(&inner.host),
                bounded,
                inner.journal.deployment_namespace().to_owned(),
            )
        };
        let profile = bounded
            .physical_storage
            .as_ref()
            .ok_or(ApplicationError::Busy)?;
        let pool = profile.pool(&namespace).map_err(map_journal)?;
        host.validate_physical_storage(profile, pool)?;
        let disk = host
            .available_disk_bytes()?
            .saturating_sub(profile.peak_bytes().map_err(map_journal)?);
        let (cpu, memory) = host.available_compute()?;
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let slots =
            inner
                .journal
                .lifecycle_available_instances(node)
                .map_err(|error| match error {
                    JournalError::Conflict => ApplicationError::Invalid,
                    other => map_journal(other),
                })?;
        if inner
            .journal
            .bounded_host_config()
            .map_err(map_journal)?
            .as_ref()
            != Some(&bounded)
            || inner.journal.deployment_namespace() != namespace
        {
            return Err(ApplicationError::OutcomeUnknown);
        }
        let usage = inner
            .journal
            .retained_resource_usage()
            .map_err(map_admission)?;
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
            .lifecycle_pending_creates(node)
            .map(|_| ())
    }

    /// Check the complete protected current configuration without initializing
    /// or repairing the journal, observing hardware or granting execution.
    /// # Errors
    /// Missing, mismatched, fenced or corrupt authority refuses.
    pub fn check_lifecycle_authority(
        &self,
        expected: &crate::LifecycleInboxConfig,
    ) -> Result<(), JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        inner.journal.world()?;
        if inner.journal.lifecycle_inbox_configuration()?.as_ref() != Some(expected) {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }

    /// # Errors
    /// Stored source limits must exactly match the protected deployment input.
    pub fn check_lifecycle_source_observations(
        &self,
        expected: &crate::capture::SourceObservationConfig,
    ) -> Result<(), JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .check_lifecycle_source_observations(expected)
    }
}
