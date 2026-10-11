use crate::{CenterState, NodeMode};
use sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION;
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
use std::collections::BTreeSet;

fn number(value: &str, prefix: &str, highwater: u64) -> Option<u64> {
    let suffix = value.strip_prefix(prefix)?;
    let value = suffix.parse::<u64>().ok()?;
    (value > 0 && value <= highwater && value.to_string() == suffix).then_some(value)
}
impl CenterState {
    /// Full recovery validation, not merely successful deserialization.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "one bounded authority image validation"
    )]
    pub fn is_valid(&self) -> bool {
        let b = &self.bounds;
        if !(1..=256).contains(&b.max_nodes)
            || !(1..=1024).contains(&b.max_templates_per_node)
            || !(1..=4096).contains(&b.max_sandboxes)
            || !(2..=64).contains(&b.operations_per_sandbox)
            || !(1..=65536).contains(&b.max_options_bytes)
            || !b.platform.positive()
            || self.nodes.is_empty()
            || self.nodes.len() > usize::from(b.max_nodes)
            || self.tenants.is_empty()
            || self.tenants.len() > usize::from(b.max_sandboxes)
            || self.sandboxes.len() > usize::from(b.max_sandboxes)
            || b.required_block_bytes() > 16 * 1024 * 1024
        {
            return false;
        }
        let mut nodes = BTreeSet::new();
        for node in &self.nodes {
            if node.identity.deployment_id != self.deployment
                || !nodes.insert(node.identity.node_id.as_str())
                || !node.limit.positive()
                || node.templates.len() > usize::from(b.max_templates_per_node)
                || node
                    .templates
                    .iter()
                    .map(|t| t.revision.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    != node.templates.len()
                || (node.protocol_version != INTERNAL_PROTOCOL_VERSION
                    && node.mode == NodeMode::Accepting)
                || self
                    .reserved(None, Some(&node.identity))
                    .is_none_or(|r| !r.fits(node.limit))
            {
                return false;
            }
        }
        let mut tenants = BTreeSet::new();
        for budget in &self.tenants {
            if !budget.limit.positive()
                || !tenants.insert(budget.tenant.as_str())
                || self
                    .reserved(Some(&budget.tenant), None)
                    .is_none_or(|r| !r.fits(budget.limit))
            {
                return false;
            }
        }
        if self
            .reserved(None, None)
            .is_none_or(|r| !r.fits(b.platform))
        {
            return false;
        }
        let future = self
            .sandboxes
            .iter()
            .filter(|s| !s.released && !s.terminating)
            .count() as u64;
        if self.operation_highwater.checked_add(future).is_none() {
            return false;
        }
        let mut sandboxes = BTreeSet::new();
        let mut operations = BTreeSet::new();
        for sandbox in &self.sandboxes {
            let Some(first) = sandbox.operations.first() else {
                return false;
            };
            let Some(last) = sandbox.operations.last() else {
                return false;
            };
            if first.request.intent != Intent::Create
                || !sandboxes.insert(first.request.identity.sandbox_id.as_str())
                || number(
                    first.request.identity.sandbox_id.as_str(),
                    "sandbox-",
                    self.sandbox_highwater,
                )
                .is_none()
                || sandbox.operations.len() > usize::from(b.operations_per_sandbox)
                || (sandbox.operations.len() == usize::from(b.operations_per_sandbox)
                    && last.request.intent != Intent::Destroy)
                || sandbox.terminating != (last.request.intent == Intent::Destroy)
                || sandbox.released
                    != last.response.as_ref().is_some_and(|r| {
                        matches!(r.fact, LifecycleFact::Cleaned | LifecycleFact::NotAccepted)
                    })
            {
                return false;
            }
            let mut observed = None;
            let mut started = None;
            let mut end = None;
            let mut max_end = None;
            let mut prior_operation = 0;
            for (index, op) in sandbox.operations.iter().enumerate() {
                let request = &op.request;
                let id = &request.identity;
                let Some(n) = number(
                    id.operation_id.as_str(),
                    "operation-",
                    self.operation_highwater,
                ) else {
                    return false;
                };
                if !operations.insert(id.operation_id.as_str())
                    || n <= prior_operation
                    || id.intent_version.get() != index as u64 + 1
                    || id.request_window.get() != 1
                    || id.node != first.request.identity.node
                    || id.tenant_id != first.request.identity.tenant_id
                    || id.sandbox_id != first.request.identity.sandbox_id
                    || request.create != first.request.create
                    || !tenants.contains(id.tenant_id.as_str())
                    || !self.nodes.iter().any(|node| node.identity == id.node)
                    || request.validate(&id.node).is_err()
                    || request.create.options.len() > b.max_options_bytes as usize
                    || (index > 0 && request.intent == Intent::Create)
                    || (index + 1 < sandbox.operations.len()
                        && matches!(request.intent, Intent::Destroy))
                {
                    return false;
                }
                prior_operation = n;
                if let Intent::Renew { end_at_ms } = request.intent
                    && (started.is_none() || max_end.is_none_or(|maximum| end_at_ms > maximum))
                {
                    return false;
                }
                if let Some(response) = &op.response {
                    if response.validate_for(request).is_err()
                        || observed.is_some_and(|v| response.observation_version.get() <= v)
                    {
                        return false;
                    }
                    observed = Some(response.observation_version.get());
                    match response.fact {
                        LifecycleFact::Ready {
                            started_at_ms,
                            end_at_ms,
                            max_end_at_ms,
                        } => {
                            started = Some(started_at_ms);
                            end = Some(end_at_ms);
                            max_end = Some(max_end_at_ms);
                        }
                        LifecycleFact::Renewed { end_at_ms } => end = Some(end_at_ms),
                        LifecycleFact::Cleaned | LifecycleFact::NotAccepted
                            if index + 1 < sandbox.operations.len() =>
                        {
                            return false;
                        }
                        _ => {}
                    }
                }
            }
            if sandbox.started_at_ms != started
                || sandbox.end_at_ms != end
                || sandbox.max_end_at_ms != max_end
                || sandbox
                    .observation_version
                    .map(sandproof_protocol::internal::PositiveCounter::get)
                    != observed
            {
                return false;
            }
        }
        true
    }
}
