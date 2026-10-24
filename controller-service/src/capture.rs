//! Capture values shared by the original durable controller and host adapter.
//!
//! Physical observations are not database commit receipts or Stop permits.
//! The controller must bind them to its original work and commit that work
//! before a host effect; deserializing these values grants no authority.

use serde::{Deserialize, Serialize};
mod launch;
mod restore;
mod source;
mod work;
pub use launch::{
    CaptureWorkerBinding, FullCaptureLaunchPermit, FullCaptureResult, PreparedFullCapture,
};
pub use restore::{RestoreActivationPermit, RestoreWorkRequest};
pub use source::{RawSourceObservation, SourceObservationConfig, SourceObservationRequirement};
pub use work::{FullCaptureWorkRequest, PauseWorkRequest};

/// Fact supplied only after the exact Full API response was received and its
/// original worker is confirmed exited. This is neither a successful bundle
/// nor permission to continue the VM; the journal separately resolves commit
/// uncertainty, revokes restore eligibility and records the continuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureApiEndObservation {
    pub action: sandproof_protocol::WorkAction<sandbox_controller_core::EffectId>,
    pub full_work_binding: sandproof_protocol::lifecycle::ContentDigest,
    pub worker: CaptureWorkerBinding,
    pub source: SourceProcessIdentity,
    pub http_status: u16,
}
impl CaptureApiEndObservation {
    /// Complete saved input matching, not independent process/API evidence.
    /// # Errors
    /// Full work encoding failure cannot manufacture a matching observation.
    pub fn matches(
        &self,
        action: &sandproof_protocol::WorkAction<sandbox_controller_core::EffectId>,
        full: &FullCaptureWorkRequest,
        source: &SourceObservationRequirement,
        worker: &CaptureWorkerBinding,
    ) -> serde_json::Result<bool> {
        Ok(self.action == *action
            && self.full_work_binding == full.binding_digest()?
            && self.worker == *worker
            && self.source == source.source
            && (200..=599).contains(&self.http_status))
    }
}

/// Original committed material expectation for readonly reopen after the
/// source position has been released. It grants no restore, delete or access
/// permission. The original journal must eventually provide this only after
/// checking the saved full work, source/profile binding and manifest receipt.
/// No constructor is exposed while that durable handoff is not implemented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedCaptureExpectation {
    work: FullCaptureWorkRequest,
    manifest_digest: sandproof_protocol::lifecycle::ContentDigest,
    seal_action: sandproof_protocol::WorkAction<sandbox_controller_core::EffectId>,
}
impl CommittedCaptureExpectation {
    #[cfg(feature = "durable")]
    pub(crate) fn after_commit(
        work: FullCaptureWorkRequest,
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest,
        seal_action: sandproof_protocol::WorkAction<sandbox_controller_core::EffectId>,
    ) -> Self {
        Self {
            work,
            manifest_digest,
            seal_action,
        }
    }
    #[must_use]
    pub const fn seal_action(
        &self,
    ) -> &sandproof_protocol::WorkAction<sandbox_controller_core::EffectId> {
        &self.seal_action
    }
    #[must_use]
    pub const fn work(&self) -> &FullCaptureWorkRequest {
        &self.work
    }
    #[must_use]
    pub const fn manifest_digest(&self) -> &sandproof_protocol::lifecycle::ContentDigest {
        &self.manifest_digest
    }
}

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
