use sandproof_protocol::internal::lifecycle::{
    LifecycleRequest, LifecycleResponse, RESULT_CONFIRMATION_BODY_BYTES, Resources,
};
use sandproof_protocol::internal::window::WINDOW_CLOSE_BODY_BYTES;
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use sandproof_protocol::{DeploymentId, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionError {
    Invalid,
    Capacity,
    /// No authenticated fresh host/template is eligible; not a tenant quota.
    Unavailable,
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
                    .saturating_add(8192)
                    // Reserve the ENTIRE future exact terminal acknowledgement.
                    .saturating_add(RESULT_CONFIRMATION_BODY_BYTES),
            )
            .saturating_add(usize::from(self.max_nodes).saturating_mul(
                4096 + WINDOW_CLOSE_BODY_BYTES + usize::from(self.max_templates_per_node) * 512,
            ))
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
/// Protected resource-pool assignment. Public clients cannot select this role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Runtime,
    Builder,
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
    pub role: NodeRole,
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
    /// Exact latest terminal result durably acknowledged by the host. This is
    /// not ordinary renewal compaction and never replaces cleanup evidence.
    pub terminal_confirmation: Option<LifecycleResponse>,
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
    /// Durable authority, not mutable heartbeat/registration input.
    pub(crate) node_windows: Vec<NodeWindow>,
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
        let current = 1.try_into().map_err(|_| DecisionError::Invalid)?;
        let node_windows = nodes
            .iter()
            .map(|node| NodeWindow {
                node: node.identity.clone(),
                current: Some(current),
                retired_through: 0,
                phase: WindowPhase::Open,
            })
            .collect();
        let state = Self {
            deployment,
            bounds,
            nodes,
            node_windows,
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
    pub fn nodes(&self) -> &[NodeRegistration] {
        &self.nodes
    }
    /// Read-only registration check for trusted metadata decisions. It neither
    /// allocates an identity nor substitutes for the original resource admission.
    #[must_use]
    pub fn tenant_is_registered(&self, tenant: &TenantId) -> bool {
        self.tenants
            .iter()
            .any(|registered| registered.tenant == *tenant)
    }
    /// Existing sessions remain bound to this exact node; stale health is a
    /// temporary route veto, never NotFound/Destroyed or quota release.
    #[must_use]
    pub fn node_is_reachable(&self, identity: &NodeIdentity, now_ms: u64) -> bool {
        self.nodes.iter().any(|node| {
            node.identity == *identity
                && matches!(node.mode, NodeMode::Accepting | NodeMode::Draining)
                && node.healthy_until_ms > now_ms
        })
    }
    /// Diagnostic identity frontiers, not an allocation or restoration API.
    #[must_use]
    pub const fn identity_frontiers(&self) -> (u64, u64) {
        (self.sandbox_highwater, self.operation_highwater)
    }
    /// Protected startup registration must agree with the original immutable
    /// deployment shape. Health/cache facts, windows and history are NOT reset
    /// from configuration when the process reattaches its existing authority.
    #[must_use]
    pub fn registration_matches(&self, registration: &Self) -> bool {
        self.deployment == registration.deployment
            && self.bounds == registration.bounds
            && self.tenants == registration.tenants
            && self.nodes.len() == registration.nodes.len()
            && self.nodes.iter().all(|node| {
                registration.nodes.iter().any(|expected| {
                    node.identity == expected.identity
                        && node.role == expected.role
                        && node.limit == expected.limit
                        && node.protocol_version == expected.protocol_version
                        && expected
                            .templates
                            .iter()
                            .all(|template| node.templates.contains(template))
                })
            })
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
        if node.limit != observation.limit
            || node.role != observation.role
            || node.protocol_version != observation.protocol_version
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

    /// Refresh only the authenticated physical/time veto. Registration policy,
    /// template facts, durable windows and accepted responsibility stay intact.
    /// # Errors
    /// Wrong node generations and capacity beyond the protected limit reject.
    pub fn refresh_node(
        &self,
        identity: &NodeIdentity,
        healthy_until_ms: u64,
        free: Resources,
    ) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let mut next = self.clone();
        let node = next
            .nodes
            .iter_mut()
            .find(|node| node.identity == *identity)
            .ok_or(DecisionError::NotFound)?;
        if !free.fits(node.limit) {
            return Err(DecisionError::Invalid);
        }
        node.healthy_until_ms = healthy_until_ms;
        node.free = free;
        Ok(next)
    }
}

/// One finite lifecycle window per registered node; no historical close list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPhase {
    Open,
    Draining,
    ClosePending,
    Exhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeWindow {
    pub(crate) node: NodeIdentity,
    pub(crate) current: Option<PositiveCounter>,
    pub(crate) retired_through: u64,
    pub(crate) phase: WindowPhase,
}
impl NodeWindow {
    #[must_use]
    pub const fn node(&self) -> &NodeIdentity {
        &self.node
    }
    #[must_use]
    pub const fn current(&self) -> Option<PositiveCounter> {
        self.current
    }
    #[must_use]
    pub const fn retired_through(&self) -> u64 {
        self.retired_through
    }
    #[must_use]
    pub const fn phase(&self) -> WindowPhase {
        self.phase
    }
}
