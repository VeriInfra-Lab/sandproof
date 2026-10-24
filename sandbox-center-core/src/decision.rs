use crate::{CenterState, DecisionError, NodeMode, NodeRole};
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::internal::{INTERNAL_PROTOCOL_VERSION, NodeIdentity};
use sandproof_protocol::lifecycle::{RetentionBinding, SessionId};
use sandproof_protocol::{SandboxId, TenantId};

fn ratio(used: Resources, limit: Resources) -> (u64, u64) {
    used.values()
        .into_iter()
        .zip(limit.values())
        .max_by(|(a, b), (c, d)| {
            (u128::from(*a) * u128::from(*d)).cmp(&(u128::from(*c) * u128::from(*b)))
        })
        .expect("four positive denominators")
}

/// Candidate fields from the ORIGINAL quota/lane/counter decision.
/// No state is allocated until the complete business envelope commits.
pub(crate) struct SessionAllocation {
    pub(crate) node: NodeIdentity,
    pub(crate) session: SessionId,
    pub(crate) retention: RetentionBinding,
    pub(crate) sandbox_number: u64,
    pub(crate) operation_number: u64,
}
impl CenterState {
    fn select_node(
        &self,
        create: &CreateBinding,
        now_ms: u64,
        role: NodeRole,
    ) -> Result<NodeIdentity, DecisionError> {
        let mut available = false;
        let mut candidates = self
            .nodes
            .iter()
            .filter_map(|node| {
                let used = self.reserved(None, Some(&node.identity))?;
                let registered_ready = node.role == role
                    && node.mode == NodeMode::Accepting
                    && node.healthy_until_ms > now_ms
                    && node.protocol_version == INTERNAL_PROTOCOL_VERSION
                    && node.templates.iter().any(|template| {
                        template.revision == create.template_revision
                            && template.digest == create.template_digest
                    });
                available |= registered_ready;
                (registered_ready
                    && self.available_lane(&node.identity).is_some()
                    && create.resources.fits(node.free)
                    && used.checked_add(create.resources)?.fits(node.limit))
                .then_some((node, ratio(used, node.limit)))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|(a, (an, ad)), (b, (bn, bd))| {
            (u128::from(*an) * u128::from(*bd))
                .cmp(&(u128::from(*bn) * u128::from(*ad)))
                .then_with(|| a.identity.node_id.as_str().cmp(b.identity.node_id.as_str()))
        });
        Ok(candidates
            .first()
            .ok_or(if available {
                DecisionError::Capacity
            } else {
                DecisionError::Unavailable
            })?
            .0
            .identity
            .clone())
    }
    pub(crate) fn allocate_session(
        &self,
        tenant: &TenantId,
        create: &CreateBinding,
        now_ms: u64,
        role: NodeRole,
    ) -> Result<SessionAllocation, DecisionError> {
        if !self.is_valid()
            || !create.is_valid()
            || create.options.len() > self.bounds.max_options_bytes as usize
            || create.startup_deadline_ms <= now_ms
        {
            return Err(DecisionError::Invalid);
        }
        if self.sandboxes.len() >= usize::from(self.bounds.max_sandboxes) {
            return Err(DecisionError::Capacity);
        }
        let budget = self
            .tenants
            .iter()
            .find(|budget| budget.tenant == *tenant)
            .ok_or(DecisionError::NotFound)?;
        let shape = create.resources;
        if self
            .reserved(None, None)
            .and_then(|used| used.checked_add(shape))
            .is_none_or(|used| !used.fits(self.bounds.platform))
            || self
                .reserved(Some(tenant), None)
                .and_then(|used| used.checked_add(shape))
                .is_none_or(|used| !used.fits(budget.limit))
        {
            return Err(DecisionError::Capacity);
        }
        let node = self.select_node(create, now_ms, role)?;
        let lane = self.available_lane(&node).ok_or(DecisionError::Capacity)?;
        let retention = RetentionBinding {
            slot: lane.retention_slot(),
            window: lane
                .current()
                .ok_or(DecisionError::Capacity)?
                .get()
                .try_into()
                .map_err(|_| DecisionError::Exhausted)?,
        };
        let sandbox_number = self
            .sandbox_highwater
            .checked_add(1)
            .ok_or(DecisionError::Exhausted)?;
        let operation_number = self
            .operation_highwater
            .checked_add(1)
            .ok_or(DecisionError::Exhausted)?;
        // Each live nonterminal session retains one future destroy identity.
        let future = self
            .sandboxes
            .iter()
            .filter(|record| !record.released() && !record.terminating())
            .count() as u64
            + 1;
        if operation_number.checked_add(future).is_none() {
            return Err(DecisionError::Exhausted);
        }
        let protocol_id = SandboxId::try_from(format!("sandbox-{sandbox_number}"))
            .map_err(|_| DecisionError::Exhausted)?;
        Ok(SessionAllocation {
            node,
            session: SessionId::from_protocol_id(protocol_id),
            retention,
            sandbox_number,
            operation_number,
        })
    }
    pub(crate) fn index(
        &self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<usize, DecisionError> {
        self.sandboxes
            .iter()
            .position(|record| {
                record.operations.first().is_some_and(|operation| {
                    operation.intent().request.binding.key.owner == *tenant
                        && operation.intent().request.binding.session.as_protocol_id() == sandbox
                })
            })
            .ok_or(DecisionError::NotFound)
    }
}
