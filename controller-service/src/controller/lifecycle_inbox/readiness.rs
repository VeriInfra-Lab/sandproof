//! Readonly connection between the original native Ready result and a current
//! V3 Create. Host observations are collected outside every SQL transaction.
pub(in crate::controller) mod snapshot;
#[cfg(test)]
mod tests;

use crate::{InternalLocalBinding, Lifetime, capture::SourceProcessIdentity};
use sandbox_controller_core as core;
use sandproof_protocol::{ActionIdentity, lifecycle::AcceptedLifecycleIntent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateReadinessPending {
    NoLocalMapping,
    StartupIncomplete,
    WorkUnknown,
    OriginalEvidenceMissing,
    HostUnobserved,
    CredentialUnobserved,
    SnapshotChanged,
}

/// Current eligibility is separate from the original Create's historical result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateReadiness {
    NotCurrent,
    NotRunning,
    Expired,
    Pending(CreateReadinessPending),
    Refused {
        reason: sandproof_protocol::lifecycle::CreateRefusalReason,
    },
    Observed(LifecycleReadyObservation),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCreateSnapshot {
    pub owner: core::TenantId,
    pub phase: core::Phase,
    pub terminal: bool,
    pub epoch: core::Epoch,
    pub generation: core::Generation,
    pub current_vm: Option<core::VmId>,
    pub current_operation: Option<core::Operation>,
    pub lifetime: Option<Lifetime>,
    pub uncertain_actions: Vec<core::EffectId>,
}

impl NativeCreateSnapshot {
    /// Pure original-operation check used by the query. This alone is not
    /// Ready: the canonical request, lifetime and fresh host facts are separate.
    #[must_use]
    pub fn has_completed_original_create(&self, local: InternalLocalBinding) -> bool {
        if self.owner != local.owner
            || self.phase != core::Phase::Running
            || self.terminal
            || self.generation != 1
            || self.current_vm.is_none()
            || !self.uncertain_actions.is_empty()
        {
            return false;
        }
        matches!(self.current_operation, Some(operation)
            if operation.id == local.operation && operation.epoch == self.epoch
                && operation.kind == core::OperationKind::Create
                && operation.status == core::OperationStatus::Succeeded)
    }
}

/// Original committed input position plus exact startup identities. This is a
/// native snapshot basis, not a newly allocated physical observation counter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleReadyBasis {
    pub native_input_high_water: u64,
    pub original_startup_actions: Vec<ActionIdentity>,
    pub original_startup_receipts: Vec<sandproof_protocol::lifecycle::ContentDigest>,
}

/// A successful readonly observation in this call, not a promise of future VM
/// life, snapshot compatibility, an Unloaded result or a release permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleReadyObservation {
    pub started_at_ms: u64,
    pub end_at_ms: u64,
    pub max_end_at_ms: u64,
    pub startup_deadline_ms: u64,
    pub observed_at_ms: u64,
    pub source: SourceProcessIdentity,
    pub basis: LifecycleReadyBasis,
}

/// Internal checked data. Public HTTP must explicitly project allowed fields;
/// native IDs and process identity are not public sandbox identifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleCreateObservation {
    pub request: AcceptedLifecycleIntent,
    pub current_request: AcceptedLifecycleIntent,
    pub local: Option<InternalLocalBinding>,
    pub historical_result: Option<core::OperationResult>,
    pub native_input_high_water: u64,
    pub native: Option<NativeCreateSnapshot>,
    pub readiness: CreateReadiness,
}
