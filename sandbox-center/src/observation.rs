//! Registered physical/time observations are admission vetoes. All state is
//! refreshed in the original business CAS; no status cache or release decision.
use sandproof_api::PortError;
use sandproof_center_service::{
    NodeObservationPolicy, RegisteredHosts, business::SharedBusinessAuthority,
};
use sandproof_controller_service::application::{Clock, SystemClock};
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
    for node in nodes {
        let before = now()?;
        let observation = runtime.block_on(hosts.status(&node.identity));
        let after = now()?;
        let Ok(status) = observation else {
            // A busy original host work slot or lost read-only reply cannot
            // extend the previous finite freshness lease. Let it expire;
            // do not manufacture a physical failure or release responsibility.
            continue;
        };
        let accepted = policy
            .healthy_until(before, after, status.observed_at_ms)
            .filter(|_| status.node == node.identity && status.free.fits(node.limit))
            .map(|healthy_until| (healthy_until, status.free));
        let (healthy_until, free) = accepted.unwrap_or((0, Resources::ZERO));
        // An unavailable host affects its veto only; a shared commit fault
        // aborts the scan so no old observation can authorize later admission.
        authority.refresh_node(&node.identity, healthy_until, free)?;
    }
    Ok(())
}

fn now() -> Result<u64, PortError> {
    u64::try_from(SystemClock.now_ms()?).map_err(|_| PortError::Busy)
}
