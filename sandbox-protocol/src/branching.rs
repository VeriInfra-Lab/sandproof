//! Wire types for same-VM training fan-out.
use crate::{
    ResourceId, SandboxId,
    lifecycle::{
        AuthenticatedCenterContext, IncarnationId, LifecycleAccessIdentity, OperationKey,
        RequestSequence, RequestWindowId, SessionId,
    },
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

/// Internal branch selector. The host independently resolves this identity
/// against its own ready record. An address, port or credential is not a selector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchAccessSelector {
    pub batch: BranchBatchIdentity,
    pub operation: OperationKey,
    pub branch: SandboxId,
}

/// Effective wire identity consists of the existing parent header and the
/// separately bounded branch selector header. Neither one grants authorization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchAccessIdentity {
    pub parent: LifecycleAccessIdentity,
    pub selector: BranchAccessSelector,
}

impl BranchAccessIdentity {
    /// Check the negotiated parent identity and selector consistency. Only
    /// the receiver's exact, current, durable route can authorize a relay.
    #[must_use]
    pub fn valid_for(&self, context: &AuthenticatedCenterContext) -> bool {
        self.parent.validate(context).is_ok() && self.selector.matches_parent(&self.parent)
    }
}

impl BranchAccessSelector {
    #[must_use]
    pub fn matches_parent(&self, parent: &LifecycleAccessIdentity) -> bool {
        self.batch.parent == parent.session
            && self.batch.incarnation == parent.incarnation
            && self.operation.deployment == parent.operation.deployment
            && self.operation.owner == parent.operation.owner
            && self.operation != parent.operation
    }
}

/// Valid local guest ports reserved exclusively for training envd listeners.
/// Runtime dispatch must take a port from the original verified branch plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchRelayPort(NonZeroU16);

impl BranchRelayPort {
    pub const FIRST: u16 = 31_000;
    pub const LAST: u16 = 31_099;

    #[must_use]
    pub const fn new(port: u16) -> Option<Self> {
        if port < Self::FIRST || port > Self::LAST {
            return None;
        }
        match NonZeroU16::new(port) {
            Some(port) => Some(Self(port)),
            None => None,
        }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }

    #[must_use]
    pub const fn nonzero(self) -> NonZeroU16 {
        self.0
    }
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

pub const BRANCH_CONTROL_PATH: &str = "/internal/v2/branch-control";
pub const BRANCH_CONTROL_BODY_BYTES: usize = 24 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchControlRequest {
    pub parent: LifecycleAccessIdentity,
    pub operation: OperationKey,
    pub batch: BranchBatchIdentity,
    pub targets: Vec<BranchTarget>,
    pub action: BranchControlAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BranchControlAction {
    Start {},
    Observe {},
    Release { branch: Option<SandboxId> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchControlReply {
    pub original: BranchControlRequest,
    pub status: BranchControlStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum BranchControlStatus {
    Pending {},
    Ready {},
    CleaningUp {},
    Released {},
    Failed { reason: BranchStartFailure },
}

impl BranchControlRequest {
    #[must_use]
    pub fn valid_for(&self, context: &AuthenticatedCenterContext) -> bool {
        self.parent.validate(context).is_ok() && self.matches_node(context.expected_node())
    }

    #[must_use]
    pub fn matches_node(&self, node: &crate::internal::NodeIdentity) -> bool {
        let count = u16::try_from(self.targets.len())
            .ok()
            .and_then(NonZeroU16::new);
        self.parent.node == *node
            && self.parent.protocol_version == crate::lifecycle::PROTOCOL_VERSION
            && self.parent.accepted_version != 0
            && node.deployment_id == self.parent.center.deployment
            && node.deployment_id == self.parent.operation.deployment
            && node.deployment_id == self.operation.deployment
            && self.batch.parent == self.parent.session
            && self.batch.incarnation == self.parent.incarnation
            && self.operation.owner == self.parent.operation.owner
            && self.operation != self.parent.operation
            && self.targets.len() <= usize::from(BranchRelayPort::LAST - BranchRelayPort::FIRST + 1)
            && count.is_some_and(|count| valid_ready_targets(&self.targets, &self.targets, count))
            && self
                .targets
                .iter()
                .all(|target| BranchRelayPort::new(target.port.get()).is_some())
            && match &self.action {
                BranchControlAction::Release {
                    branch: Some(branch),
                } => self.targets.iter().any(|target| &target.sandbox == branch),
                _ => true,
            }
    }
}

impl BranchControlReply {
    #[must_use]
    pub fn matches(&self, request: &BranchControlRequest) -> bool {
        self.original == *request
            && !matches!(
                (&request.action, self.status),
                (
                    BranchControlAction::Release { .. },
                    BranchControlStatus::Ready {} | BranchControlStatus::Failed { .. }
                )
            )
    }
}
