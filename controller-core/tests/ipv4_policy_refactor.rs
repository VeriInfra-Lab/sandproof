//! Focused regressions for moving the existing destination policy into
//! `NetworkPolicy` and owning the former `slice::Iter::any` traversal.
//! The frozen old decision below is a test oracle, not a second specification
//! and not a substitute for the implementation refinement proof.

use std::net::Ipv4Addr;

use sandbox_controller_core::{
    Actor, AuxiliaryReject, CreateRequest, Endpoint, FilePolicyRef, FsId, Ipv4Cidr, NetSlotId,
    NetworkPolicy, OperationId, PLATFORM_DENY_IPV4, PolicyDigest, Protocol, SandboxId, TenantId,
    VmId, World, handle_create, handle_egress, platform_protected_ipv4,
};

const SID: SandboxId = SandboxId(2);
const VM: VmId = VmId(3);
const SLOT: NetSlotId = NetSlotId(4);

fn endpoint(address: &str) -> Endpoint {
    Endpoint {
        address: address.into(),
        prefix_length: 32,
        protocol: Protocol::Tcp,
        port: 443,
    }
}

fn range(network: u32, prefix_length: u8) -> Ipv4Cidr {
    Ipv4Cidr {
        network,
        prefix_length,
    }
}

fn policy(
    internet_access: bool,
    allow_out: Vec<Ipv4Cidr>,
    deny_out: Vec<Ipv4Cidr>,
) -> NetworkPolicy {
    NetworkPolicy::Ipv4 {
        internet_access,
        allow_out,
        deny_out,
    }
}

// Original auxiliary::destination_allowed at c7d1afd, with only its policy
// argument factored from the SandboxRecord and its original platform any query
// kept here. The library parser and CIDR callback are unchanged production code.
fn original_destination_allowed(policy: &NetworkPolicy, destination: &Endpoint) -> bool {
    match policy {
        NetworkPolicy::DenyAll => false,
        NetworkPolicy::StaticAllowlist(endpoints) => {
            let mut cursor = 0;
            let mut allowed = false;
            while cursor < endpoints.len() && !allowed {
                if &endpoints[cursor] == destination {
                    allowed = true;
                }
                cursor += 1;
            }
            allowed
        }
        NetworkPolicy::Ipv4 {
            internet_access,
            allow_out,
            deny_out,
        } => {
            let Ok(address) = destination.address.parse::<Ipv4Addr>() else {
                return false;
            };
            if destination.prefix_length != 32 {
                return false;
            }
            let address = u32::from(address);
            if PLATFORM_DENY_IPV4
                .iter()
                .any(|range| range.contains(address))
            {
                return false;
            }
            if allow_out.iter().any(|range| range.contains(address)) {
                return true;
            }
            if deny_out.iter().any(|range| range.contains(address)) {
                return false;
            }
            *internet_access
        }
    }
}

#[test]
fn standard_library_ipv4_format_and_integer_boundaries_remain_strict() {
    for (input, bits) in [
        ("0.0.0.0", 0),
        ("255.255.255.255", u32::MAX),
        ("1.2.3.4", 0x0102_0304),
        ("128.0.0.1", 0x8000_0001),
        ("192.168.1.255", 0xc0a8_01ff),
    ] {
        assert_eq!(
            input.parse::<Ipv4Addr>().map(u32::from),
            Ok(bits),
            "{input}"
        );
        let policy = policy(true, vec![], vec![]);
        let destination = endpoint(input);
        assert_eq!(
            policy.destination_allowed(&destination),
            original_destination_allowed(&policy, &destination),
            "{input}"
        );
    }

    for input in [
        "",
        "1.2.3",
        "1.2.3.4.5",
        ".1.2.3",
        "1..2.3",
        "1.2.3.",
        "256.0.0.1",
        "999.0.0.1",
        "0000.0.0.1",
        "01.2.3.4",
        "00.0.0.0",
        "1.2.3.004",
        " 1.2.3.4",
        "1.2.3.4 ",
        "1.2.3.4\t",
        "1.2.3.4\n",
        "1.2.3.4\0",
        "+1.2.3.4",
        "-1.2.3.4",
        "0x7f.0.0.1",
        "127.1",
        "2130706433",
        "1.2.3.4/32",
        "1.2.3.4:80",
        "1.2.3.4%eth0",
        "１.2.3.4",
        "١.2.3.4",
        "1。2.3.4",
        "::1",
        "::ffff:1.2.3.4",
        "example.com",
    ] {
        assert!(input.parse::<Ipv4Addr>().is_err(), "{input:?}");
        let policy = policy(true, vec![range(0, 0)], vec![]);
        assert!(!policy.destination_allowed(&endpoint(input)), "{input:?}");
    }
}

#[test]
fn moved_policy_matches_old_ordered_decision_across_bounded_cases() {
    let public = range(0x0101_0101, 32);
    let invalid = range(1, 0);
    let invalid_prefix = range(0, 33);
    let policies = [
        NetworkPolicy::DenyAll,
        NetworkPolicy::StaticAllowlist(vec![]),
        NetworkPolicy::StaticAllowlist(vec![endpoint("1.1.1.1"), endpoint("1.1.1.1")]),
        policy(false, vec![], vec![]),
        policy(true, vec![], vec![]),
        policy(false, vec![public, public], vec![range(0, 0)]),
        policy(true, vec![], vec![public, public]),
        policy(false, vec![invalid, invalid_prefix, public], vec![public]),
        policy(
            true,
            vec![invalid, invalid_prefix],
            vec![invalid, invalid_prefix],
        ),
        policy(false, vec![range(0, 0)], vec![range(0, 0)]),
    ];
    for policy in policies {
        let before = policy.clone();
        for address in [
            "1.1.1.1",
            "1.1.1.2",
            "8.8.8.8",
            "0.0.0.0",
            "127.0.0.1",
            "169.254.169.254",
            "255.255.255.255",
            "01.1.1.1",
            "example.com",
            "::1",
        ] {
            for prefix_length in [0, 31, 32, 33, u64::MAX] {
                for protocol in [Protocol::Tcp, Protocol::Udp] {
                    for port in [0, 443, u64::MAX] {
                        let destination = Endpoint {
                            address: address.into(),
                            prefix_length,
                            protocol,
                            port,
                        };
                        assert_eq!(
                            policy.destination_allowed(&destination),
                            original_destination_allowed(&policy, &destination),
                            "{policy:?}: {destination:?}"
                        );
                    }
                }
            }
        }
        assert_eq!(policy, before);
    }
}

#[test]
fn owned_platform_traversal_preserves_edges_and_cannot_be_overridden() {
    let policy = policy(true, vec![range(0, 0)], vec![]);
    for range in PLATFORM_DENY_IPV4 {
        let last = range.network | (u32::MAX >> range.prefix_length);
        for address in [range.network, last] {
            assert!(platform_protected_ipv4(address));
            assert!(!policy.destination_allowed(&endpoint(&Ipv4Addr::from(address).to_string())));
        }
        for address in [
            range.network.saturating_sub(1),
            range.network,
            last,
            last.saturating_add(1),
        ] {
            assert_eq!(
                platform_protected_ipv4(address),
                PLATFORM_DENY_IPV4
                    .iter()
                    .any(|entry| entry.contains(address))
            );
        }
    }
    assert!(policy.destination_allowed(&endpoint("1.1.1.1")));
}

#[test]
fn ipv4_allow_precedes_deny_and_default_without_port_or_protocol_filtering() {
    let public = range(0x0101_0101, 32);
    let cases = [
        (policy(false, vec![public], vec![public]), true),
        (policy(true, vec![], vec![public]), false),
        (policy(false, vec![], vec![]), false),
        (policy(true, vec![], vec![]), true),
    ];
    for (policy, expected) in cases {
        for protocol in [Protocol::Tcp, Protocol::Udp] {
            for port in [0, 53, 443, 65_535, u64::MAX] {
                let mut destination = endpoint("1.1.1.1");
                destination.protocol = protocol;
                destination.port = port;
                assert_eq!(policy.destination_allowed(&destination), expected);
                for prefix in [0, 24, 31, 33, u64::MAX] {
                    destination.prefix_length = prefix;
                    assert!(!policy.destination_allowed(&destination));
                }
            }
        }
    }
}

#[test]
fn static_allowlist_preserves_all_endpoint_fields_and_first_match() {
    let destination = endpoint("203.0.113.1");
    let policy = NetworkPolicy::StaticAllowlist(vec![
        endpoint("1.1.1.1"),
        destination.clone(),
        destination.clone(),
    ]);
    assert!(policy.destination_allowed(&destination));
    let mut changed = destination.clone();
    changed.address = "203.0.113.2".into();
    assert!(!policy.destination_allowed(&changed));
    changed = destination.clone();
    changed.prefix_length = 31;
    assert!(!policy.destination_allowed(&changed));
    changed = destination.clone();
    changed.protocol = Protocol::Udp;
    assert!(!policy.destination_allowed(&changed));
    changed = destination.clone();
    changed.port = 80;
    assert!(!policy.destination_allowed(&changed));
    assert!(!NetworkPolicy::StaticAllowlist(vec![]).destination_allowed(&destination));
    assert!(!NetworkPolicy::DenyAll.destination_allowed(&destination));
}

#[test]
fn cidr_contains_matches_arithmetic_blocks_including_invalid_prefixes() {
    for prefix_length in 0..=u8::MAX {
        for network in [0, 1, 0x0101_0100, 0x0101_0101, 0x8000_0000, u32::MAX] {
            for address in [
                0,
                1,
                0x0101_0100,
                0x0101_0101,
                0x0101_01ff,
                0x8000_0000,
                u32::MAX,
            ] {
                let expected = if prefix_length > 32 {
                    false
                } else {
                    let block = 1_u64 << (32 - prefix_length);
                    u64::from(network) % block == 0
                        && u64::from(address) / block == u64::from(network) / block
                };
                assert_eq!(range(network, prefix_length).contains(address), expected);
            }
        }
    }
}

fn created_world(policy: NetworkPolicy) -> World {
    let owner = TenantId(1);
    let workspace = FsId(6);
    handle_create(
        &World::default(),
        CreateRequest {
            actor: Actor::Tenant(owner),
            owner,
            sandbox: SID,
            workspace,
            file_policy: FilePolicyRef {
                digest: PolicyDigest(7),
            },
            policy,
            operation: OperationId(5),
        },
    )
    .expect("create fixture")
    .next
}

#[test]
fn policy_does_not_bypass_egress_authority_or_change_first_rejection() {
    let world = created_world(policy(false, vec![range(0x0101_0101, 32)], vec![]));
    let before = world.clone();
    let accepted = endpoint("1.1.1.1");
    assert!(
        world
            .sandbox_record(SID)
            .expect("sandbox")
            .policy
            .destination_allowed(&accepted)
    );
    assert_eq!(
        handle_egress(&world, SID, VM, SLOT, accepted),
        Err(AuxiliaryReject::CapabilityDenied)
    );
    let malformed = endpoint("01.1.1.1");
    assert_eq!(
        handle_egress(
            &world,
            SandboxId(99),
            VmId(99),
            NetSlotId(99),
            malformed.clone()
        ),
        Err(AuxiliaryReject::SandboxNotFound)
    );
    for (vm, slot) in [(VmId(99), SLOT), (VM, NetSlotId(99))] {
        assert_eq!(
            handle_egress(&world, SID, vm, slot, malformed.clone()),
            Err(AuxiliaryReject::CapabilityDenied)
        );
    }
    assert_eq!(
        handle_egress(&world, SID, VM, SLOT, malformed.clone()),
        Err(AuxiliaryReject::CapabilityDenied)
    );
    assert_eq!(world, before);
}
