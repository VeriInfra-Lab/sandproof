//! Real production calls, not a second network algorithm.
use sandbox_controller_core::auxiliary::{AuxiliaryDecision, AuxiliaryReject};
use sandbox_controller_core::{
    Actor, Endpoint, Ipv4Cidr, NetSlotId, NetworkPolicy, SandboxId, VmId, World,
};

pub fn cidr_contains(cidr: &Ipv4Cidr, address: u32) -> bool {
    cidr.contains(address)
}

pub fn protected_ipv4(address: u32) -> bool {
    sandbox_controller_core::platform_protected_ipv4(address)
}

pub fn destination_allowed(policy: &NetworkPolicy, destination: &Endpoint) -> bool {
    policy.destination_allowed(destination)
}

pub fn parse_ipv4_bits(value: &str) -> Option<u32> {
    match value.parse::<std::net::Ipv4Addr>() {
        Ok(address) => Some(u32::from(address)),
        Err(_) => None,
    }
}

pub fn egress(
    world: &World,
    sandbox: SandboxId,
    vm: VmId,
    slot: NetSlotId,
    destination: Endpoint,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    sandbox_controller_core::auxiliary::handle_egress(world, sandbox, vm, slot, destination)
}

pub fn ingress(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    vm: VmId,
    slot: NetSlotId,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    sandbox_controller_core::auxiliary::handle_ingress(world, actor, sandbox, vm, slot)
}

pub fn clone_world(world: &World) -> World {
    world.clone()
}

pub fn initial_world() -> World {
    sandbox_controller_core::initial_world()
}
