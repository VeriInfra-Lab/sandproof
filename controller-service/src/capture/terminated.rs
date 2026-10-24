//! Terminal-only original writer-drain question. Reading this never authorizes
//! a VM command, deletes material, confirms an API response or releases quota.
use super::{
    CaptureWorkerBinding, FullCaptureWorkRequest, SourceObservationRequirement,
    SourceProcessIdentity,
};
use crate::HostSlotContext;
use sandbox_controller_core::{self as core, capture::OriginalContinueIdentity};
use sandproof_protocol::{WorkAction, lifecycle::ContentDigest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminatedCaptureWorkRequest {
    identity: OriginalContinueIdentity,
    seal_action: WorkAction<core::EffectId>,
    full: FullCaptureWorkRequest,
    source: SourceObservationRequirement,
    worker: Option<CaptureWorkerBinding>,
    stop_action: WorkAction<core::EffectId>,
    stop: core::StopCoverage,
    host_slot: HostSlotContext,
}
impl TerminatedCaptureWorkRequest {
    #[must_use]
    pub const fn identity(&self) -> OriginalContinueIdentity {
        self.identity
    }
    #[must_use]
    pub const fn seal_action(&self) -> &WorkAction<core::EffectId> {
        &self.seal_action
    }
    #[must_use]
    pub const fn full(&self) -> &FullCaptureWorkRequest {
        &self.full
    }
    #[must_use]
    pub const fn source(&self) -> &SourceObservationRequirement {
        &self.source
    }
    #[must_use]
    pub const fn worker(&self) -> Option<&CaptureWorkerBinding> {
        self.worker.as_ref()
    }
    #[must_use]
    pub const fn stop_action(&self) -> &WorkAction<core::EffectId> {
        &self.stop_action
    }
    #[must_use]
    pub const fn stop(&self) -> core::StopCoverage {
        self.stop
    }
    #[must_use]
    pub const fn host_slot(&self) -> HostSlotContext {
        self.host_slot
    }
    /// Bind the original complete question, including its actual occupied slot.
    /// # Errors
    /// Encoding failure produces no question binding or native drain fact.
    pub fn binding_digest(&self) -> serde_json::Result<ContentDigest> {
        Ok(ContentDigest::from_sha256(
            Sha256::digest(serde_json::to_vec(&(
                self.identity,
                &self.seal_action,
                &self.full,
                &self.source,
                &self.worker,
                &self.stop_action,
                self.stop,
                self.host_slot,
            ))?)
            .into(),
        ))
    }
}

/// Produced only while the exact original VMM is observed stopped, any
/// authorized worker is confirmed exited, and the original exclusive helper
/// gate is held with its fd identity checked before and after. `worker=None`
/// must match the journal's never-authorized state and still requires that
/// actual gate drain; absence of a process/marker alone supplies no fact.
/// The gate protocol may reclaim a dead-helper marker, but sends no VM/API
/// command and deletes no runtime or capture material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminatedCaptureObservation {
    StoppedAndDrained {
        work_binding: ContentDigest,
        full_work_binding: ContentDigest,
        seal_action: WorkAction<core::EffectId>,
        stop_action: WorkAction<core::EffectId>,
        source: SourceProcessIdentity,
        worker: Option<CaptureWorkerBinding>,
        host_slot: HostSlotContext,
        helper_gate_device: u64,
        helper_gate_inode: u64,
    },
}
impl TerminatedCaptureObservation {
    /// Complete saved-input matching. Physical truth is supplied by the
    /// protected provider; matching bytes cannot attest process or fd exit.
    /// # Errors
    /// Work encoding failure cannot create a matching observation.
    pub fn matches(&self, work: &TerminatedCaptureWorkRequest) -> serde_json::Result<bool> {
        let Self::StoppedAndDrained {
            work_binding,
            full_work_binding,
            seal_action,
            stop_action,
            source,
            worker,
            host_slot,
            ..
        } = self;
        Ok(*work_binding == work.binding_digest()?
            && *full_work_binding == work.full.binding_digest()?
            && *seal_action == work.seal_action
            && *stop_action == work.stop_action
            && *source == work.source.source
            && *worker == work.worker
            && *host_slot == work.host_slot)
    }
}
