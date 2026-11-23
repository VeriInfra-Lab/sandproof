//! Wire types for same-VM training fan-out.
use crate::{
    ResourceId, SandboxId,
    lifecycle::{IncarnationId, OperationKey, RequestSequence, RequestWindowId, SessionId},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, num::NonZeroU16};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicBranchBatchRequest {
    pub window: RequestWindowId,
    pub sequence: RequestSequence,
    pub expected_version: u64,
    pub checkpoint: ResourceId,
    pub count: NonZeroU16,
}

/// Complete guest ownership identity. The checkpoint lives inside this execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchBatchIdentity {
    pub parent: SessionId,
    pub incarnation: IncarnationId,
    pub checkpoint: ResourceId,
    pub batch: ResourceId,
}

/// Center response to the original authorized operation. An active or failed
/// batch exposes no SDK sandbox IDs. Failed means precise cleanup is confirmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicBranchBatch {
    pub operation: OperationKey,
    pub parent: SessionId,
    pub result: PublicBranchBatchResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublicBranchBatchResult {
    Pending {},
    CleaningUp {},
    Ready { branches: Vec<SandboxId> },
    Failed { reason: BranchStartFailure },
    Retired {},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStartFailure {
    CheckpointUnavailable,
    GuestSetupFailed,
    ParentEnded,
}

/// The host plans these endpoints before startup. A guest cannot supply a different
/// address for data forwarding. Ordinals are zero-based positions in this list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchTarget {
    pub sandbox: SandboxId,
    pub port: NonZeroU16,
}

/// Match the complete observed target set against the original host plan.
/// This checks the receipt's shape; it does not prove an endpoint is reachable
/// or authorize forwarding.
#[must_use]
pub fn valid_ready_targets(
    planned: &[BranchTarget],
    observed: &[BranchTarget],
    count: NonZeroU16,
) -> bool {
    if planned.len() != usize::from(count.get()) || planned != observed {
        return false;
    }
    let mut ids = HashSet::with_capacity(planned.len());
    let mut ports = HashSet::with_capacity(planned.len());
    planned
        .iter()
        .all(|target| ids.insert(&target.sandbox) && ports.insert(target.port))
}
