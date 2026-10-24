//! The only adapter-held profile is assembled from protected local settings.
//! Copy/read caps are not a reservation, writer bound, or Stop permission.
use super::{LinuxWorker, NodeResult};
use crate::ProtectedHostEpochSource;
use sandbox_runtime::SnapshotLayout;
use sandproof_controller_service::capture::{HostEpochRequirement, SourceObservationRequirement};
use sandproof_protocol::lifecycle::ContentDigest;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

/// Inputs from the same protected server configuration as the original node.
/// This type is never a field accepted from an HTTP/work request. The runtime
/// directory must already be the original v sibling of the configured store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedCaptureSettings {
    pub layout_parent: PathBuf,
    pub layout_binding: ContentDigest,
    pub state_copy_bytes: u64,
    pub memory_copy_bytes: u64,
    pub disk_copy_bytes: u64,
    pub api_timeout_ms: u64,
    /// Whole helper exchange deadline, separate from the Firecracker API call.
    pub worker_timeout_ms: u64,
}
impl ProtectedCaptureSettings {
    pub(super) fn validate(&self) -> NodeResult<()> {
        // The fixed dependency loads at most 10,000,000 state bytes. This is
        // a read/copy acceptance cap, NEVER a bound on the VMM's initial write.
        if !self.layout_parent.is_absolute()
            || self.state_copy_bytes == 0
            || self.state_copy_bytes > 10_000_000
            || self.memory_copy_bytes == 0
            || self.disk_copy_bytes == 0
            || i64::try_from(self.memory_copy_bytes).is_err()
            || i64::try_from(self.disk_copy_bytes).is_err()
            || !(1000..=300_000).contains(&self.api_timeout_ms)
            || !(self.api_timeout_ms..=300_000).contains(&self.worker_timeout_ms)
        {
            return Err("unsupported protected Full copy/read settings".into());
        }
        Ok(())
    }
}

// No Deserialize/public fields or external constructor. A work DTO cannot
// manufacture this profile or choose a different path/epoch for one operation.
pub(crate) struct ProtectedCaptureProfile {
    pub(super) layout: SnapshotLayout,
    pub(super) layout_parent: PathBuf,
    pub(super) layout_binding: ContentDigest,
    pub(super) epoch: HostEpochRequirement,
    pub(super) state_bytes: u64,
    pub(super) memory_max: u64,
    pub(super) disk_bytes: u64,
    pub(super) api_timeout: Duration,
    pub(super) worker_timeout: Duration,
}
impl ProtectedCaptureProfile {
    pub(crate) fn assemble(
        worker: &LinuxWorker,
        provider: &ProtectedHostEpochSource,
        settings: &ProtectedCaptureSettings,
        epoch: &HostEpochRequirement,
    ) -> NodeResult<Self> {
        settings.validate()?;
        worker.host_lease.validate()?;
        provider.observe(epoch, worker.host_bootstrap_inputs())?;
        let layout =
            SnapshotLayout::reopen(&settings.layout_parent, *settings.layout_binding.as_bytes())?;
        if layout.runtime_root() != worker.config.chroot_base {
            return Err("protected Full layout differs from original runtime root".into());
        }
        Ok(Self {
            layout,
            layout_parent: settings.layout_parent.clone(),
            layout_binding: settings.layout_binding,
            epoch: epoch.clone(),
            state_bytes: settings.state_copy_bytes,
            memory_max: settings.memory_copy_bytes,
            disk_bytes: settings.disk_copy_bytes,
            api_timeout: Duration::from_millis(settings.api_timeout_ms),
            worker_timeout: Duration::from_millis(settings.worker_timeout_ms),
        })
    }
    pub(super) fn require_original_source(
        &self,
        source: &SourceObservationRequirement,
    ) -> NodeResult<()> {
        if self.epoch.node() != &source.node || self.epoch.registration() != &source.host_bootstrap
        {
            return Err("Full source differs from adapter's original protected epoch".into());
        }
        Ok(())
    }
}
