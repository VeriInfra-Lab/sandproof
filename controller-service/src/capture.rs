//! Capture values shared by the original durable controller and host adapter.
//!
//! Physical observations are not database commit receipts or Stop permits.
//! The controller must bind them to its original work and commit that work
//! before a host effect; deserializing these values grants no authority.

use serde::{Deserialize, Serialize};
mod source;
pub use source::{RawSourceObservation, SourceObservationConfig, SourceObservationRequirement};

/// Reference to the protected, versioned `HostBootstrap` payload registered before
/// the first V3 VM starts. `policy` hashes that actual immutable registration
/// (node/boot/kernel/CPU set and details/microcode/fixed tool and CPU template),
/// not generic policy text or a value freshly invented during capture.
/// The node resolves and checks that original payload against the actual host;
/// Its strict purpose is startup/bootstrap, not a running VMM's CPU domain,
/// VM-scope XSAVE/KVM capability, snapshot compatibility or resource guarantee.
/// Deserializing this reference does not attest any hardware or tool fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEpochRegistration {
    pub version: u16,
    pub boot_id: String,
    pub policy: sandproof_protocol::lifecycle::ContentDigest,
}

/// Original protected requirement, assembled from persisted configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostEpochRequirement {
    node: sandproof_protocol::internal::NodeIdentity,
    registration: HostEpochRegistration,
}
impl HostEpochRequirement {
    #[cfg(feature = "durable")]
    pub(crate) const fn configured(
        node: sandproof_protocol::internal::NodeIdentity,
        registration: HostEpochRegistration,
    ) -> Self {
        Self { node, registration }
    }
    #[must_use]
    pub const fn node(&self) -> &sandproof_protocol::internal::NodeIdentity {
        &self.node
    }
    #[must_use]
    pub const fn registration(&self) -> &HostEpochRegistration {
        &self.registration
    }
}

/// Boundary `HostBootstrap` observation from the protected provider after it
/// checks the registered payload and its explicitly listed current host facts.
/// Matching before/after samples is not proof of intervening/future invariance;
/// deployment still keeps the registered epoch's tool/kernel/CPU setup fixed.
/// This grants no Source/Capture/Stop/Release capability. Public fields do not make this attested
/// hardware data: the receiver relies on that exact provider contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEpochObservation {
    pub node: sandproof_protocol::internal::NodeIdentity,
    pub registration: HostEpochRegistration,
}
impl HostEpochRegistration {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.version == 1
            && !self.boot_id.is_empty()
            && self.boot_id.len() <= 128
            && self.boot_id.bytes().all(|b| b.is_ascii_graphic())
    }
}

/// Identity observed for the exact original VMM process, including its boot.
/// PID reuse, process-start reuse across boots and manifest self-reporting
/// cannot substitute for a command-specific original host observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProcessIdentity {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub boot_id: String,
}
