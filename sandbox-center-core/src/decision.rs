use crate::{CenterState, DecisionError, NodeMode, Operation, Sandbox};
use sandproof_protocol::internal::lifecycle::{
    CreateBinding, Intent, LifecycleFact, LifecycleRequest, LifecycleResponse, Resources,
};
use sandproof_protocol::internal::{ControlIdentity, INTERNAL_PROTOCOL_VERSION};
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
fn fact_rank(fact: LifecycleFact) -> u8 {
    match fact {
        LifecycleFact::Recorded => 0,
        LifecycleFact::Stopped => 1,
        _ => 2,
    }
}
impl CenterState {
    fn select_node(
        &self,
        create: &CreateBinding,
        now_ms: u64,
    ) -> Result<sandproof_protocol::internal::NodeIdentity, DecisionError> {
        let mut candidates = self
            .nodes
            .iter()
            .filter_map(|n| {
                let used = self.reserved(None, Some(&n.identity))?;
                (n.mode == NodeMode::Accepting
                    && n.healthy_until_ms > now_ms
                    && n.protocol_version == INTERNAL_PROTOCOL_VERSION
                    && n.templates.iter().any(|t| {
                        t.revision == create.template_revision && t.digest == create.template_digest
                    })
                    && create.resources.fits(n.free)
                    && used.checked_add(create.resources)?.fits(n.limit))
                .then_some((n, ratio(used, n.limit)))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|(a, (an, ad)), (b, (bn, bd))| {
            (u128::from(*an) * u128::from(*bd))
                .cmp(&(u128::from(*bn) * u128::from(*ad)))
                .then_with(|| a.identity.node_id.as_str().cmp(b.identity.node_id.as_str()))
        });
        let node = candidates
            .first()
            .ok_or(DecisionError::Capacity)?
            .0
            .identity
            .clone();
        Ok(node)
    }
    /// # Errors
    /// Rejects input, quotas, absent candidates or exhausted IDs without consuming IDs.
    pub fn create(
        &self,
        tenant: &TenantId,
        create: CreateBinding,
        now_ms: u64,
    ) -> Result<(Self, LifecycleRequest), DecisionError> {
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
            .find(|b| &b.tenant == tenant)
            .ok_or(DecisionError::NotFound)?;
        let shape = create.resources;
        if self
            .reserved(None, None)
            .and_then(|r| r.checked_add(shape))
            .is_none_or(|r| !r.fits(self.bounds.platform))
            || self
                .reserved(Some(tenant), None)
                .and_then(|r| r.checked_add(shape))
                .is_none_or(|r| !r.fits(budget.limit))
        {
            return Err(DecisionError::Capacity);
        }
        let node = self.select_node(&create, now_ms)?;
        let sandbox_number = self
            .sandbox_highwater
            .checked_add(1)
            .ok_or(DecisionError::Exhausted)?;
        let operation_number = self
            .operation_highwater
            .checked_add(1)
            .ok_or(DecisionError::Exhausted)?;
        // Every accepted nonterminal sandbox owns one FUTURE destroy identity.
        let future = self
            .sandboxes
            .iter()
            .filter(|s| !s.released && !s.terminating)
            .count() as u64
            + 1;
        if operation_number.checked_add(future).is_none() {
            return Err(DecisionError::Exhausted);
        }
        let request = LifecycleRequest {
            identity: ControlIdentity {
                protocol_version: INTERNAL_PROTOCOL_VERSION,
                node,
                tenant_id: tenant.clone(),
                sandbox_id: format!("sandbox-{sandbox_number}")
                    .try_into()
                    .map_err(|_| DecisionError::Exhausted)?,
                operation_id: format!("operation-{operation_number}")
                    .try_into()
                    .map_err(|_| DecisionError::Exhausted)?,
                intent_version: 1.try_into().map_err(|_| DecisionError::Exhausted)?,
                request_window: 1.try_into().map_err(|_| DecisionError::Exhausted)?,
            },
            create,
            intent: Intent::Create,
        };
        let mut next = self.clone();
        next.sandbox_highwater = sandbox_number;
        next.operation_highwater = operation_number;
        next.sandboxes.push(Sandbox {
            operations: vec![Operation {
                request: request.clone(),
                response: None,
            }],
            terminating: false,
            released: false,
            started_at_ms: None,
            end_at_ms: None,
            max_end_at_ms: None,
            observation_version: None,
        });
        Ok((next, request))
    }
    /// # Errors
    /// Requires original owner, live confirmed lifetime and one free ordinary slot.
    pub fn renew(
        &self,
        tenant: &TenantId,
        sandbox: &SandboxId,
        end_at_ms: u64,
        now_ms: u64,
    ) -> Result<(Self, LifecycleRequest), DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.index(tenant, sandbox)?;
        let record = &self.sandboxes[index];
        if record.terminating
            || record.released
            || record.end_at_ms.is_none_or(|end| end <= now_ms)
            || end_at_ms <= now_ms
            || record.max_end_at_ms.is_none_or(|end| end_at_ms > end)
            || record.operations.last().is_none_or(|op| {
                op.response.as_ref().is_none_or(|r| {
                    !matches!(
                        r.fact,
                        LifecycleFact::Ready { .. } | LifecycleFact::Renewed { .. }
                    )
                })
            })
        {
            return Err(DecisionError::Conflict);
        }
        // Keep the last slot for termination even when ordinary history is full.
        if record.operations.len() + 1 >= usize::from(self.bounds.operations_per_sandbox) {
            return Err(DecisionError::Capacity);
        }
        self.append(index, Intent::Renew { end_at_ms })
    }
    /// # Errors
    /// Only the original owner can terminate; unknown creates retain reservations.
    pub fn destroy(
        &self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<(Self, LifecycleRequest), DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.index(tenant, sandbox)?;
        let record = &self.sandboxes[index];
        if record.terminating {
            return Ok((
                self.clone(),
                record
                    .operations
                    .last()
                    .ok_or(DecisionError::Invalid)?
                    .request
                    .clone(),
            ));
        }
        if record.released {
            return Err(DecisionError::Conflict);
        }
        self.append(index, Intent::Destroy)
    }
    fn index(&self, tenant: &TenantId, sandbox: &SandboxId) -> Result<usize, DecisionError> {
        self.sandboxes
            .iter()
            .position(|s| {
                s.operations.first().is_some_and(|op| {
                    &op.request.identity.tenant_id == tenant
                        && &op.request.identity.sandbox_id == sandbox
                })
            })
            .ok_or(DecisionError::NotFound)
    }
    fn append(
        &self,
        index: usize,
        intent: Intent,
    ) -> Result<(Self, LifecycleRequest), DecisionError> {
        let number = self
            .operation_highwater
            .checked_add(1)
            .ok_or(DecisionError::Exhausted)?;
        let mut request = self.sandboxes[index]
            .operations
            .last()
            .ok_or(DecisionError::Invalid)?
            .request
            .clone();
        request.identity.operation_id = format!("operation-{number}")
            .try_into()
            .map_err(|_| DecisionError::Exhausted)?;
        request.identity.intent_version = request
            .identity
            .intent_version
            .checked_next()
            .map_err(|_| DecisionError::Exhausted)?;
        request.intent = intent;
        let mut next = self.clone();
        next.operation_highwater = number;
        let record = &mut next.sandboxes[index];
        record.terminating = intent == Intent::Destroy;
        record.operations.push(Operation {
            request: request.clone(),
            response: None,
        });
        if next.is_valid() {
            Ok((next, request))
        } else {
            Err(DecisionError::Exhausted)
        }
    }
    /// # Errors
    /// Rejects wrong full identity, old intent/observation or regressing facts.
    /// Communication failures do not call this method or release reservations.
    pub fn apply_response(&self, response: &LifecycleResponse) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.index(&response.identity.tenant_id, &response.identity.sandbox_id)?;
        let record = &self.sandboxes[index];
        let operation = record.operations.last().ok_or(DecisionError::Invalid)?;
        response
            .validate_for(&operation.request)
            .map_err(|_| DecisionError::Stale)?;
        if operation.response.as_ref() == Some(response) {
            return Ok(self.clone());
        }
        if record
            .observation_version
            .is_some_and(|v| response.observation_version.get() <= v.get())
            || operation
                .response
                .as_ref()
                .is_some_and(|r| fact_rank(response.fact) <= fact_rank(r.fact))
        {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        let record = &mut next.sandboxes[index];
        match response.fact {
            LifecycleFact::Ready {
                started_at_ms,
                end_at_ms,
                max_end_at_ms,
            } => {
                record.started_at_ms = Some(started_at_ms);
                record.end_at_ms = Some(end_at_ms);
                record.max_end_at_ms = Some(max_end_at_ms);
            }
            LifecycleFact::Renewed { end_at_ms } => record.end_at_ms = Some(end_at_ms),
            LifecycleFact::Cleaned | LifecycleFact::NotAccepted => record.released = true,
            LifecycleFact::Recorded | LifecycleFact::Stopped => {}
        }
        record.observation_version = Some(response.observation_version);
        record
            .operations
            .last_mut()
            .ok_or(DecisionError::Invalid)?
            .response = Some(response.clone());
        if next.is_valid() {
            Ok(next)
        } else {
            Err(DecisionError::Invalid)
        }
    }
}
