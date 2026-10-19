//! Registered physical/time observations are admission vetoes. All state is
//! refreshed in the original business CAS; no status cache or release decision.
mod diagnostics;

use sandproof_api::PortError;
use sandproof_center_core::NodeRegistration;
use sandproof_center_service::{
    NodeObservationPolicy, RegisteredHosts, business::SharedBusinessAuthority,
};
use sandproof_controller_service::application::{Clock, SystemClock};
use sandproof_internal::status::HostStatus;
use sandproof_protocol::internal::lifecycle::Resources;
use tokio::runtime::Handle;

pub fn refresh(
    authority: &SharedBusinessAuthority,
    hosts: &RegisteredHosts,
    policy: NodeObservationPolicy,
    runtime: &Handle,
) -> Result<(), PortError> {
    // The fresh bounded authority supplies the exact registered generations;
    // HTTP never supplies a destination or a replacement node list.
    let nodes = authority.snapshot()?.center.nodes().to_vec();
    for (index, node) in nodes.into_iter().enumerate() {
        let before = now()?;
        let observation = runtime.block_on(hosts.status(&node.identity));
        let after = now()?;
        let Ok(status) = observation else {
            // A busy original host work slot or lost read-only reply cannot
            // extend the previous finite freshness lease. Let it expire;
            // do not manufacture a physical failure or release responsibility.
            diagnostics::emit(index, &node, None, policy, before, after, false);
            continue;
        };
        let accepted = accepted_observation(&node, &status, policy, before, after);
        diagnostics::emit(
            index,
            &node,
            Some(&status),
            policy,
            before,
            after,
            accepted.is_some(),
        );
        let (healthy_until, free) = accepted.unwrap_or((0, Resources::ZERO));
        // An unavailable host affects its veto only; a shared commit fault
        // aborts the scan so no old observation can authorize later admission.
        authority.refresh_node(&node.identity, healthy_until, free)?;
    }
    Ok(())
}

fn accepted_observation(
    node: &NodeRegistration,
    status: &HostStatus,
    policy: NodeObservationPolicy,
    before: u64,
    after: u64,
) -> Option<(u64, Resources)> {
    policy
        .healthy_until(before, after, status.observed_at_ms)
        .filter(|_| {
            status.node == node.identity
                && status.retention_slots == node.retention_slots
                && status.free.fits(node.limit)
        })
        .map(|healthy_until| (healthy_until, status.free))
}

fn now() -> Result<u64, PortError> {
    u64::try_from(SystemClock.now_ms()?).map_err(|_| PortError::Busy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_center_core::{NodeMode, NodeRole};
    use sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION;

    #[test]
    fn authenticated_status_cannot_change_protected_inbox_capacity() {
        let identity = sandproof_protocol::internal::NodeIdentity {
            deployment_id: "fixture-deployment".try_into().unwrap(),
            node_id: "fixture-host".try_into().unwrap(),
            node_generation: 1.try_into().unwrap(),
        };
        let limit = Resources {
            instances: 2,
            cpu: 2,
            memory_bytes: 4096,
            disk_bytes: 8192,
        };
        let node = NodeRegistration {
            identity: identity.clone(),
            retention_slots: 2,
            role: NodeRole::Runtime,
            limit,
            mode: NodeMode::Accepting,
            healthy_until_ms: 0,
            free: Resources::ZERO,
            templates: vec![],
            protocol_version: INTERNAL_PROTOCOL_VERSION,
        };
        let mut status = HostStatus {
            node: identity,
            retention_slots: 2,
            observed_at_ms: 150,
            free: limit,
        };
        let policy = NodeObservationPolicy {
            health_window_ms: 1000,
            max_clock_skew_ms: 10,
        };
        assert_eq!(
            accepted_observation(&node, &status, policy, 100, 200),
            Some((1100, limit))
        );
        for slots in [0, 1, 3] {
            status.retention_slots = slots;
            assert_eq!(accepted_observation(&node, &status, policy, 100, 200), None);
        }
    }
}
