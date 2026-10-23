use sandproof_protocol::internal::lifecycle::{RESULT_CONFIRMATION_BODY_BYTES, Resources};
use sandproof_protocol::internal::window::RETENTION_LANE_BYTES;
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use sandproof_protocol::lifecycle::{
    CenterIdentity, CurrentReadyFacts, LifecycleOperation, TerminalLifecycleObservation,
    TerminalStatus,
};
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
    /// Public results remain in the same original row for this explicit period.
    /// Independent of host native/build/Guest result retention; no default.
    pub result_retention_ms: u64,
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
                4096 + usize::from(self.max_templates_per_node) * 512
                    // Each node has at most max_sandboxes protected positions.
                    // Recent closed stamps are future state, not free old slack.
                    + usize::from(self.max_sandboxes).saturating_mul(RETENTION_LANE_BYTES),
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
    /// Must equal this protected host's original inbox `max_sandboxes`.
    pub retention_slots: u16,
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
pub struct Sandbox {
    /// The only operation history and lifetime authority for this session.
    pub operations: Vec<crate::LifecycleOperationRecord>,
    /// Exact latest terminal result durably acknowledged by the host. This is
    /// not ordinary renewal compaction and never replaces cleanup evidence.
    pub terminal_confirmation: Option<TerminalLifecycleObservation>,
}
impl Sandbox {
    #[must_use]
    pub fn terminating(&self) -> bool {
        self.operations.last().is_some_and(|operation| {
            matches!(
                operation.intent().request.operation,
                LifecycleOperation::Destroy {}
            )
        })
    }
    #[must_use]
    pub fn released(&self) -> bool {
        self.operations
            .last()
            .and_then(|operation| operation.terminal())
            .is_some_and(|result| result.status == TerminalStatus::Cleaned)
    }
    /// Historical lifetime, never sufficient by itself to authorize access.
    #[must_use]
    pub fn last_running(&self) -> Option<&CurrentReadyFacts> {
        self.operations
            .iter()
            .rev()
            .find_map(|operation| operation.running.as_ref())
    }
    /// Only a result committed for the actual current Create/Renew can route.
    #[must_use]
    pub fn current_running(&self) -> Option<&CurrentReadyFacts> {
        self.operations.last()?.running.as_ref()
    }
    #[must_use]
    pub fn created_at_ms(&self) -> Option<u64> {
        self.operations
            .first()
            .map(|operation| operation.accepted_at_ms)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CenterState {
    pub(crate) deployment: DeploymentId,
    pub(crate) bounds: Bounds,
    pub(crate) nodes: Vec<NodeRegistration>,
    /// Durable authority, not mutable heartbeat/registration input.
    pub(crate) retention_lanes: Vec<RetentionLane>,
    pub(crate) tenants: Vec<TenantBudget>,
    pub(crate) sandbox_highwater: u64,
    pub(crate) operation_highwater: u64,
    pub(crate) sandboxes: Vec<Sandbox>,
    pub(crate) lifecycle: crate::LifecycleProfile,
}
impl CenterState {
    /// # Errors
    /// Rejects contradictory registration or an unbounded future envelope.
    pub fn new(
        deployment: DeploymentId,
        bounds: Bounds,
        nodes: Vec<NodeRegistration>,
        tenants: Vec<TenantBudget>,
        center: CenterIdentity,
        windows: Vec<crate::LifecycleWindow>,
    ) -> Result<Self, DecisionError> {
        // Validate the fixed future envelope BEFORE allocating the lane vector.
        if !(1..=256).contains(&bounds.max_nodes)
            || !(1..=4096).contains(&bounds.max_sandboxes)
            || nodes.len() > usize::from(bounds.max_nodes)
            || bounds.required_block_bytes() > 16 * 1024 * 1024
            || nodes.iter().any(|node| {
                node.retention_slots == 0 || node.retention_slots > bounds.max_sandboxes
            })
        {
            return Err(DecisionError::Invalid);
        }
        let retention_lanes = nodes
            .iter()
            .flat_map(|node| {
                (1..=node.retention_slots).map(move |retention_slot| RetentionLane {
                    node: node.identity.clone(),
                    retention_slot,
                    retired_through: 0,
                    phase: LanePhase::Open,
                    last_retired: None,
                })
            })
            .collect();
        let lifecycle = crate::LifecycleProfile::new(center, windows, &tenants)?;
        let state = Self {
            deployment,
            bounds,
            nodes,
            retention_lanes,
            tenants,
            sandbox_highwater: 0,
            operation_highwater: 0,
            sandboxes: vec![],
            lifecycle,
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
            && self.lifecycle_registration_matches(registration)
            && self.bounds == registration.bounds
            && self.tenants == registration.tenants
            && self.nodes.len() == registration.nodes.len()
            && self.nodes.iter().all(|node| {
                registration.nodes.iter().any(|expected| {
                    node.identity == expected.identity
                        && node.retention_slots == expected.retention_slots
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
    pub fn reserved(
        &self,
        tenant: Option<&TenantId>,
        node: Option<&NodeIdentity>,
    ) -> Option<Resources> {
        let mut sum = Some(Resources::ZERO);
        let mut index = 0;
        while sum.is_some() && index < self.sandboxes.len() {
            let sandbox = &self.sandboxes[index];
            if !sandbox.released()
                && let Some(op) = sandbox.operations.first()
                && tenant.is_none_or(|t| &op.intent().request.binding.key.owner == t)
                && node.is_none_or(|n| &op.intent().request.binding.node == n)
            {
                sum = match (&op.intent().request.operation, sum) {
                    (LifecycleOperation::Create { create, .. }, Some(total)) => {
                        total.checked_add(create.resources)
                    }
                    _ => None,
                };
            }
            index += 1;
        }
        sum
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
            || node.retention_slots != observation.retention_slots
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

/// One bounded retention incarnation per protected slot, never a whole-node drain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanePhase {
    Open,
    ClosePending,
    Exhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionLane {
    pub(crate) node: NodeIdentity,
    pub(crate) retention_slot: u16,
    pub(crate) retired_through: u64,
    pub(crate) phase: LanePhase,
    pub(crate) last_retired: Option<TerminalLifecycleObservation>,
}
impl RetentionLane {
    #[must_use]
    pub const fn node(&self) -> &NodeIdentity {
        &self.node
    }
    #[must_use]
    pub const fn retention_slot(&self) -> u16 {
        self.retention_slot
    }
    #[must_use]
    pub fn current(&self) -> Option<PositiveCounter> {
        self.retired_through.checked_add(1)?.try_into().ok()
    }
    #[must_use]
    pub const fn retired_through(&self) -> u64 {
        self.retired_through
    }
    #[must_use]
    pub const fn phase(&self) -> LanePhase {
        self.phase
    }
    #[must_use]
    pub const fn last_retired(&self) -> Option<&TerminalLifecycleObservation> {
        self.last_retired.as_ref()
    }
}
