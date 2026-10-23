use crate::{CenterState, NodeMode};
use sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION;
use std::collections::BTreeSet;

/// Exact syntax/frontier check without authority to allocate or restore.
#[must_use]
pub fn number(value: &str, prefix: &str, highwater: u64) -> Option<u64> {
    sandproof_protocol::parse_allocated_number(value, prefix, highwater)
}
impl CenterState {
    /// Full recovery validation. History/identity/lifetime checks are performed
    /// by valid_lifecycle on the SINGLE canonical table, not removed.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let b = &self.bounds;
        if !(1..=256).contains(&b.max_nodes)
            || !(1..=1024).contains(&b.max_templates_per_node)
            || !(1..=4096).contains(&b.max_sandboxes)
            || !(3..=128).contains(&b.operations_per_sandbox)
            || !(1..=65536).contains(&b.max_options_bytes)
            || b.result_retention_ms == 0
            || !b.platform.positive() || self.nodes.is_empty()
            || self.nodes.len() > usize::from(b.max_nodes) || self.tenants.is_empty()
            || self.tenants.len() > usize::from(b.max_sandboxes)
            || self.sandboxes.len() > usize::from(b.max_sandboxes)
            || self.required_authority_bytes() > 16 * 1024 * 1024
            || !self.valid_lanes() || !self.valid_nodes() || !self.valid_tenants()
            || self.reserved(None, None).is_none_or(|r| !r.fits(b.platform))
        { return false; }
        let future = self.sandboxes.iter().filter(|s| !s.released() && !s.terminating()).count() as u64;
        self.operation_highwater.checked_add(future).is_some() && self.valid_lifecycle()
    }
    fn valid_nodes(&self) -> bool {
        let mut nodes = BTreeSet::new();
        let mut index = 0;
        let mut valid = true;
        while valid && index < self.nodes.len() {
            let node = &self.nodes[index];
            valid = !(node.identity.deployment_id != self.deployment
                || !nodes.insert(node.identity.node_id.as_str()) || !node.limit.positive()
                || node.retention_slots == 0 || node.retention_slots > self.bounds.max_sandboxes
                || node.templates.len() > usize::from(self.bounds.max_templates_per_node)
                || node.templates.iter().map(|t| t.revision.as_str()).collect::<BTreeSet<_>>().len() != node.templates.len()
                || (node.protocol_version != INTERNAL_PROTOCOL_VERSION && node.mode == NodeMode::Accepting)
                || self.reserved(None, Some(&node.identity)).is_none_or(|r| !r.fits(node.limit)));
            index += 1;
        }
        valid
    }
    fn valid_tenants(&self) -> bool {
        let mut tenants = BTreeSet::new();
        let mut index = 0;
        let mut valid = true;
        while valid && index < self.tenants.len() {
            let budget = &self.tenants[index];
            valid = !(!budget.limit.positive() || !tenants.insert(budget.tenant.as_str())
                || self.reserved(Some(&budget.tenant), None).is_none_or(|r| !r.fits(budget.limit)));
            index += 1;
        }
        valid
    }
}
