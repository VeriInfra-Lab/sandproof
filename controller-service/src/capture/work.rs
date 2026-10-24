//! Values for original, durable Pause and Full work. These values carry no
//! execution, reservation, publication, Stop or Release authority on their own.
use super::SourceProcessIdentity;
use sandproof_protocol::{
    ResourceKind, ResourceRef,
    internal::NodeIdentity,
    lifecycle::{
        CaptureBinding, ContentDigest, ExpectedSession, PROTOCOL_VERSION, VmCompatibility,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseWorkRequest {
    pub binding: CaptureBinding,
    pub source_work_binding: ContentDigest,
    pub node: NodeIdentity,
    pub source_resource: ResourceRef,
    pub source: SourceProcessIdentity,
}
impl PauseWorkRequest {
    /// Representation only. The original claim must independently check the
    /// full saved request, source work, mapping, lifetime, plan and action.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.binding.operation.protocol_version == PROTOCOL_VERSION
            && self.binding.operation.node == self.node
            && self.binding.operation.key.deployment == self.node.deployment_id
            && matches!(self.binding.operation.expected, ExpectedSession::Version(_))
            && self.source_resource.kind == ResourceKind::Vm
            && self.source.pid > 0
            && self.source.start_time_ticks > 0
            && !self.source.boot_id.is_empty()
            && self.source.boot_id.len() <= 128
            && self.source.boot_id.bytes().all(|b| b.is_ascii_graphic())
    }
}

/// Expected profile and limits are protected local inputs saved before Pause.
/// Numeric lengths retain existing post-write/copy/read semantics; they are
/// neither a Firecracker pre-write bound nor a physical reservation receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullCaptureWorkRequest {
    pub pause: PauseWorkRequest,
    pub compatibility: VmCompatibility,
    pub state_bytes: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub api_timeout_ms: u64,
}
impl FullCaptureWorkRequest {
    /// Bind the complete saved input, without attesting its physical truth.
    /// # Errors
    /// Canonical encoding failure cannot yield an invented work identity.
    pub fn binding_digest(&self) -> serde_json::Result<ContentDigest> {
        Ok(ContentDigest::from_sha256(
            Sha256::digest(serde_json::to_vec(self)?).into(),
        ))
    }
    #[must_use]
    pub fn valid(&self) -> bool {
        self.pause.valid()
            && self.compatibility.validate().is_ok()
            && self.compatibility.writable_drives.len() == 1
            && self.compatibility.writable_drives[0].drive_id.as_str() == "rootfs"
            && self.compatibility.writable_drives[0].jail_relative_path == "rootfs.ext4"
            && self.state_bytes > 0
            && i64::try_from(self.state_bytes).is_ok()
            && self.memory_bytes > 0
            && i64::try_from(self.memory_bytes).is_ok()
            && self.disk_bytes > 0
            && i64::try_from(self.disk_bytes).is_ok()
            && (1000..=300_000).contains(&self.api_timeout_ms)
    }
}
