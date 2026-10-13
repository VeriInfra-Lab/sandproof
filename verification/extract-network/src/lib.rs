//! Real production calls, not a second network algorithm.
use sandbox_controller_core::{Endpoint, Ipv4Cidr, NetworkPolicy};

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
