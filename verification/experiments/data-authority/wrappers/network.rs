//! Real production calls, not a second network algorithm.
use sandbox_controller_core::auxiliary::{AuxiliaryDecision, AuxiliaryReject};
use sandbox_controller_core::maintenance::slots::{HostSlotRecord, SlotInput, SlotReject};
use sandbox_controller_core::recovery_monitor::{RuntimeDecision, RuntimeReject};
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

pub fn runtime_crash(world: &World) -> Result<RuntimeDecision, RuntimeReject> {
    sandbox_controller_core::recovery_monitor::handle_controller_crash(world)
}

pub fn runtime_recover(world: &World) -> Result<RuntimeDecision, RuntimeReject> {
    sandbox_controller_core::recovery_monitor::handle_controller_recover(world)
}

pub fn slot_transition(
    previous: Option<HostSlotRecord>,
    input: SlotInput,
) -> Result<HostSlotRecord, SlotReject> {
    sandbox_controller_core::maintenance::slots::reduce_slot(previous, input)
}

/// Preserve the real public input type in the shared current-state extraction.
/// This is a representation boundary, not a second controller dispatcher.
pub fn controller_input(
    input: sandbox_controller_core::ControllerInput,
) -> sandbox_controller_core::ControllerInput {
    input
}

pub fn runtime(
    world: &World,
    input: sandbox_controller_core::recovery_monitor::RuntimeInput,
) -> Result<RuntimeDecision, RuntimeReject> {
    sandbox_controller_core::recovery_monitor::handle_runtime(world, input)
}

/// The production creation handler, including its ordered admission checks.
pub fn create(
    world: &World,
    input: sandbox_controller_core::request::CreateRequest,
) -> Result<sandbox_controller_core::request::Decision, sandbox_controller_core::request::Reject> {
    sandbox_controller_core::request::handle_create(world, input)
}

// Additional roots for the isolated data-authority batch. No substitute algorithm.
pub fn access_exec(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    vm: VmId,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    sandbox_controller_core::auxiliary::handle_exec(world, actor, sandbox, vm)
}

pub fn access_file_read(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    generation: sandbox_controller_core::Generation,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    sandbox_controller_core::auxiliary::handle_file_read(world, actor, sandbox, generation)
}

pub fn access_file_write(
    world: &World,
    actor: Actor,
    sandbox: SandboxId,
    generation: sandbox_controller_core::Generation,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    sandbox_controller_core::auxiliary::handle_file_write(world, actor, sandbox, generation)
}

pub fn access(
    world: &World,
    input: sandbox_controller_core::auxiliary::AccessInput,
) -> Result<AuxiliaryDecision, AuxiliaryReject> {
    sandbox_controller_core::auxiliary::handle_access(world, input)
}
