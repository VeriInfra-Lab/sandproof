use sandproof_protocol::internal::lifecycle::{LifecycleRequest, LifecycleResponse, Resources};
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use sandproof_protocol::{DeploymentId, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionError {
    Invalid,
    Capacity,
    NotFound,
    Conflict,
    Exhausted,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub max_nodes: u16,
    pub max_templates_per_node: u16,
    pub max_sandboxes: u16,
    /// Includes one operation slot reserved exclusively for destroy.
    pub operations_per_sandbox: u16,
    pub max_options_bytes: u32,
    pub platform: Resources,
}
impl Bounds {
    /// Conservative JSON envelope: options encode as at most four bytes per
    /// byte, plus fixed bounded IDs/results/config. Reserved at construction,
    /// including EVERY future operation and receipt, not after admission.
    #[must_use]
    pub fn required_block_bytes(&self) -> usize {
        usize::from(self.max_sandboxes)
            .saturating_mul(
                usize::from(self.operations_per_sandbox)
                    .saturating_mul(
                        (self.max_options_bytes as usize)
                            .saturating_mul(4)
                            .saturating_add(8192),
                    )
                    .saturating_add(8192),
            )
            .saturating_add(
                usize::from(self.max_nodes)
                    .saturating_mul(4096 + usize::from(self.max_templates_per_node) * 512),
            )
            .saturating_add(8192)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TenantBudget {
    pub tenant: TenantId,
    pub limit: Resources,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeMode {
    Accepting,
    Draining,
    Unreachable,
    Disabled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailableTemplate {
    pub revision: sandproof_protocol::ResourceId,
    pub digest: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRegistration {
    pub identity: NodeIdentity,
    pub limit: Resources,
    pub mode: NodeMode,
    pub healthy_until_ms: u64,
    /// Physical free is a veto; never a replacement for reservations.
    pub free: Resources,
    /// Available immutable template revisions; finite registration, not aliases.
    pub templates: Vec<AvailableTemplate>,
    pub protocol_version: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub request: LifecycleRequest,
    /// None includes an unknown response: it stays eligible for SAME-request replay.
    pub response: Option<LifecycleResponse>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sandbox {
    pub operations: Vec<Operation>,
    pub terminating: bool,
    pub released: bool,
    pub started_at_ms: Option<u64>,
    pub end_at_ms: Option<u64>,
    pub max_end_at_ms: Option<u64>,
    pub observation_version: Option<PositiveCounter>,
}
impl Sandbox {
    pub(crate) fn renewal_anchor(&self) -> Option<usize> {
        (self.operations.last()?.request.intent
            == sandproof_protocol::internal::lifecycle::Intent::Destroy)
            .then(|| {
                self.operations.iter().rposition(|op| {
                    op.response.as_ref().is_some_and(|r| {
                        matches!(
                            r.fact,
                            sandproof_protocol::internal::lifecycle::LifecycleFact::Renewed { .. }
                        )
                    })
                })
            })
            .flatten()
    }
    /// Detailed ordinary results can be dropped, but origin/current intent and
    /// the last confirmed lifetime before destruction remain authoritative.
    #[must_use]
    pub fn has_compaction_work(&self) -> bool {
        self.operations.len() > 2 + usize::from(self.renewal_anchor().is_some())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CenterState {
    pub(crate) deployment: DeploymentId,
    pub(crate) bounds: Bounds,
    pub(crate) nodes: Vec<NodeRegistration>,
    pub(crate) tenants: Vec<TenantBudget>,
    pub(crate) sandbox_highwater: u64,
    pub(crate) operation_highwater: u64,
    pub(crate) sandboxes: Vec<Sandbox>,
}
impl CenterState {
    /// # Errors
    /// Rejects contradictory registration or an unbounded future envelope.
    pub fn new(
        deployment: DeploymentId,
        bounds: Bounds,
        nodes: Vec<NodeRegistration>,
        tenants: Vec<TenantBudget>,
    ) -> Result<Self, DecisionError> {
        let state = Self {
            deployment,
            bounds,
            nodes,
            tenants,
            sandbox_highwater: 0,
            operation_highwater: 0,
            sandboxes: vec![],
        };
        if state.is_valid() {
            Ok(state)
        } else {
            Err(DecisionError::Invalid)
        }
    }
    #[must_use]
    pub const fn bounds(&self) -> &Bounds {
        &self.bounds
    }
    #[must_use]
    pub const fn deployment(&self) -> &DeploymentId {
        &self.deployment
    }
    #[must_use]
    pub fn sandboxes(&self) -> &[Sandbox] {
        &self.sandboxes
    }
    #[must_use]
    pub fn pending(&self) -> Vec<LifecycleRequest> {
        self.sandboxes
            .iter()
            .filter(|s| !s.released)
            .filter_map(|s| s.operations.last())
            .filter(|op| {
                op.response.as_ref().is_none_or(|r| {
                    matches!(
                        r.fact,
                        sandproof_protocol::internal::lifecycle::LifecycleFact::Recorded
                            | sandproof_protocol::internal::lifecycle::LifecycleFact::Stopped
                    )
                })
            })
            .map(|op| op.request.clone())
            .collect()
    }
    #[must_use]
    pub fn reserved(
        &self,
        tenant: Option<&TenantId>,
        node: Option<&NodeIdentity>,
    ) -> Option<Resources> {
        self.sandboxes
            .iter()
            .filter(|s| !s.released)
            .filter_map(|s| s.operations.first())
            .filter(|op| {
                tenant.is_none_or(|t| &op.request.identity.tenant_id == t)
                    && node.is_none_or(|n| &op.request.identity.node == n)
            })
            .try_fold(Resources::ZERO, |sum, op| {
                sum.checked_add(op.request.create.resources)
            })
    }
    /// Explicit physical/time/config input; no heartbeat-based release or rebind.
    /// # Errors
    /// Existing permanent identity/limit and available-template count remain bounded.
    pub fn observe_node(&self, observation: NodeRegistration) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let mut next = self.clone();
        let node = next
            .nodes
            .iter_mut()
            .find(|n| n.identity == observation.identity)
            .ok_or(DecisionError::NotFound)?;
        if node.limit != observation.limit || node.protocol_version != observation.protocol_version
        {
            return Err(DecisionError::Conflict);
        }
        *node = observation;
        if next.is_valid() {
            Ok(next)
        } else {
            Err(DecisionError::Invalid)
        }
    }
}
