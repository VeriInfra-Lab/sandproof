//! Raw, bounded source observations. None of these values authorize capture.
use super::{HostEpochRegistration, SourceProcessIdentity};
use sandproof_protocol::{ResourceRef, internal::NodeIdentity, lifecycle::ContentDigest};
use serde::{Deserialize, Serialize};

/// Protected local limits, explicitly provisioned with `SQLite` feature 18.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceObservationConfig {
    pub max_work_bytes: u32,
    pub max_configuration_bytes: u32,
    pub max_executable_bytes: u64,
    pub max_kernel_bytes: u64,
    pub max_disk_bytes: u64,
    /// Persistent host-wide material reservation, including unknown work.
    pub max_material_bytes: u64,
}
impl SourceObservationConfig {
    #[must_use]
    pub fn valid(&self) -> bool {
        (1024..=1_048_576).contains(&self.max_work_bytes)
            && (1..=1_048_576).contains(&self.max_configuration_bytes)
            && (1..=67_108_864).contains(&self.max_executable_bytes)
            && (1..=268_435_456).contains(&self.max_kernel_bytes)
            && (1..=i64::MAX as u64).contains(&self.max_disk_bytes)
            && (1..=i64::MAX as u64).contains(&self.max_material_bytes)
    }
    #[cfg(feature = "durable")]
    pub(crate) fn required_bytes(&self, records: u64) -> u64 {
        // Each byte in the raw JSON array takes at most four characters.
        // Fixed result envelopes and table metadata are separately reserved.
        records
            * (u64::from(self.max_work_bytes)
                + 4 * u64::from(self.max_configuration_bytes)
                + 8192
                // Original Full/action/worker record cap is reserved before
                // source reception; later authorization cannot add a debt.
                + 131_072)
            + 8192
    }
}

/// Question addressed to the exact original process. The digest binds the full
/// stored work, not a caller's assertion that the VM is compatible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceObservationRequirement {
    pub work_binding: ContentDigest,
    pub node: NodeIdentity,
    pub host_bootstrap: HostEpochRegistration,
    pub resource: ResourceRef,
    pub source: SourceProcessIdentity,
    pub limits: SourceObservationConfig,
}

/// Actual GET /vm/config bytes and bounded file observations from a readonly
/// provider. Configuration has not been interpreted as device compatibility.
/// Matching PID samples are not CPU/VM-scope XSAVE or future continuity,
/// disk/RAM reservation, bundle publication, or Stop/Release permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSourceObservation {
    pub work_binding: ContentDigest,
    pub resource: ResourceRef,
    pub source: SourceProcessIdentity,
    pub configuration: Vec<u8>,
    pub executable_sha256: [u8; 32],
    pub kernel_sha256: [u8; 32],
    pub kernel_bytes: u64,
    pub disk_bytes: u64,
}
impl RawSourceObservation {
    #[must_use]
    pub fn matches(&self, requirement: &SourceObservationRequirement) -> bool {
        self.work_binding == requirement.work_binding
            && self.resource == requirement.resource
            && self.source == requirement.source
            && !self.configuration.is_empty()
            && self.configuration.len() as u64
                <= u64::from(requirement.limits.max_configuration_bytes)
            && self.kernel_bytes > 0
            && self.kernel_bytes <= requirement.limits.max_kernel_bytes
            && self.disk_bytes > 0
            && self.disk_bytes <= requirement.limits.max_disk_bytes
    }
}
