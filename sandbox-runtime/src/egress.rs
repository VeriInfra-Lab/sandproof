//! Compiles a guest-facing nftables guard. This is only one half of egress:
//! callers must also install an identity-bound host uplink, host INPUT and
//! FORWARD restrictions, and exact cleanup before applying a permissive plan.
//! Currently the runtime applies only `DenyAll`; the other plans are not yet
//! exposed to the controller or a real uplink.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::net::Ipv4Addr;

const MAX_STATIC_RULES: usize = 128;
const CHAIN_PREFIX: &str = "add table inet sandproof_guard\n\
add chain inet sandproof_guard guest_out { type filter hook prerouting priority -10; policy accept; }\n\
add rule inet sandproof_guard guest_out iifname \"tap0\" ct state established,related accept\n";
const CHAIN_SUFFIX: &str = "add rule inet sandproof_guard guest_out iifname \"tap0\" drop\n";

// Conservative exclusions: no public mode packet is deliberately routed to
// loopback, link-local, RFC1918, CGNAT, documentation, benchmark, multicast,
// or future-use IPv4 space. Host-local *public* addresses need a separate
// host INPUT guard; this list alone is never sufficient for host isolation.
const NON_PUBLIC: &[(Ipv4Addr, u8)] = &[
    (Ipv4Addr::UNSPECIFIED, 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 88, 99, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Transport {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StaticIpv4Rule {
    pub network: Ipv4Addr,
    pub prefix_length: u8,
    pub transport: Transport,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EgressPolicy {
    DenyAll,
    PublicIpv4,
    StaticIpv4(Vec<StaticIpv4Rule>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    EmptyStaticRules,
    TooManyStaticRules,
    InvalidPrefix,
    NonCanonicalNetwork,
    NonPublicDestination,
    UnsupportedDnsPort,
    InvalidPort,
    DuplicateRule,
}

fn mask(prefix: u8) -> Option<u32> {
    if prefix > 32 {
        return None;
    }
    Some(if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    })
}

fn overlaps(left: (Ipv4Addr, u8), right: (Ipv4Addr, u8)) -> bool {
    let prefix = left.1.min(right.1);
    let prefix_mask = mask(prefix).expect("fixed or validated prefix");
    (u32::from(left.0) & prefix_mask) == (u32::from(right.0) & prefix_mask)
}

fn validate_static(rules: &[StaticIpv4Rule]) -> Result<(), PolicyError> {
    if rules.is_empty() {
        return Err(PolicyError::EmptyStaticRules);
    }
    if rules.len() > MAX_STATIC_RULES {
        return Err(PolicyError::TooManyStaticRules);
    }
    let mut unique = BTreeSet::new();
    for rule in rules {
        let network_mask = mask(rule.prefix_length).ok_or(PolicyError::InvalidPrefix)?;
        if u32::from(rule.network) & network_mask != u32::from(rule.network) {
            return Err(PolicyError::NonCanonicalNetwork);
        }
        if NON_PUBLIC
            .iter()
            .any(|blocked| overlaps((rule.network, rule.prefix_length), *blocked))
        {
            return Err(PolicyError::NonPublicDestination);
        }
        if rule.port == 0 {
            return Err(PolicyError::InvalidPort);
        }
        // V1 has no controlled resolver. Explicit raw DNS/DoT endpoints must
        // not silently turn the static-IP mode into a general DNS tunnel.
        if rule.port == 53 || rule.port == 853 {
            return Err(PolicyError::UnsupportedDnsPort);
        }
        if !unique.insert(*rule) {
            return Err(PolicyError::DuplicateRule);
        }
    }
    Ok(())
}

impl EgressPolicy {
    /// Render a single nft transaction for the VM namespace's TAP ingress.
    /// This does not create a route, NAT, or the required host-side firewall.
    /// The caller may apply only `DenyAll` until that separate path is ready.
    /// # Errors
    /// Rejects invalid, overbroad, non-public, duplicate or unsupported
    /// static destination rules before any host-side effect occurs.
    pub fn guest_guard_script(&self) -> Result<String, PolicyError> {
        let mut script = String::from(CHAIN_PREFIX);
        match self {
            Self::DenyAll => {}
            Self::PublicIpv4 => {
                for (network, prefix) in NON_PUBLIC {
                    writeln!(
                        script,
                        "add rule inet sandproof_guard guest_out iifname \"tap0\" ip daddr {network}/{prefix} drop"
                    )
                    .expect("writing to a string cannot fail");
                }
                script.push_str(
                    "add rule inet sandproof_guard guest_out iifname \"tap0\" meta nfproto ipv4 accept\n",
                );
            }
            Self::StaticIpv4(rules) => {
                validate_static(rules)?;
                for rule in rules {
                    let protocol = match rule.transport {
                        Transport::Tcp => "tcp",
                        Transport::Udp => "udp",
                    };
                    writeln!(
                        script,
                        "add rule inet sandproof_guard guest_out iifname \"tap0\" ip daddr {}/{} {protocol} dport {} accept",
                        rule.network, rule.prefix_length, rule.port
                    )
                    .expect("writing to a string cannot fail");
                }
            }
        }
        script.push_str(CHAIN_SUFFIX);
        Ok(script)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn static_rule(address: [u8; 4], prefix: u8, port: u16) -> StaticIpv4Rule {
        StaticIpv4Rule {
            network: Ipv4Addr::from(address),
            prefix_length: prefix,
            transport: Transport::Tcp,
            port,
        }
    }

    #[test]
    fn deny_keeps_the_current_exact_default_drop() {
        let script = EgressPolicy::DenyAll.guest_guard_script().unwrap();
        assert_eq!(script, format!("{CHAIN_PREFIX}{CHAIN_SUFFIX}"));
        assert_eq!(script.matches(" accept\n").count(), 1);
        assert_eq!(script.matches(" drop\n").count(), 1);
    }

    #[test]
    fn static_rules_reject_broad_or_ambiguous_destinations() {
        for rule in [
            static_rule([0, 0, 0, 0], 0, 443),
            static_rule([10, 0, 0, 0], 8, 443),
            static_rule([100, 64, 0, 0], 10, 443),
            static_rule([192, 0, 2, 1], 32, 443),
            static_rule([203, 0, 112, 0], 23, 443),
            static_rule([224, 0, 0, 0], 4, 443),
        ] {
            assert_eq!(
                EgressPolicy::StaticIpv4(vec![rule]).guest_guard_script(),
                Err(PolicyError::NonPublicDestination)
            );
        }
        assert_eq!(
            EgressPolicy::StaticIpv4(vec![static_rule([1, 1, 1, 1], 33, 443)]).guest_guard_script(),
            Err(PolicyError::InvalidPrefix)
        );
        assert_eq!(
            EgressPolicy::StaticIpv4(vec![static_rule([1, 1, 1, 1], 24, 443)]).guest_guard_script(),
            Err(PolicyError::NonCanonicalNetwork)
        );
    }

    #[test]
    fn static_rules_have_no_implicit_dns_or_ipv6_accept() {
        let rule = static_rule([1, 1, 1, 1], 32, 443);
        let script = EgressPolicy::StaticIpv4(vec![rule])
            .guest_guard_script()
            .unwrap();
        assert!(script.contains("ip daddr 1.1.1.1/32 tcp dport 443 accept"));
        assert!(script.ends_with(CHAIN_SUFFIX));
        assert!(!script.contains("ip6"));
        for port in [53, 853] {
            assert_eq!(
                EgressPolicy::StaticIpv4(vec![static_rule([1, 1, 1, 1], 32, port)])
                    .guest_guard_script(),
                Err(PolicyError::UnsupportedDnsPort)
            );
        }
    }

    #[test]
    fn public_ipv4_still_excludes_private_and_ipv6() {
        let script = EgressPolicy::PublicIpv4.guest_guard_script().unwrap();
        assert!(script.contains("ip daddr 100.64.0.0/10 drop"));
        assert!(script.contains("ip daddr 169.254.0.0/16 drop"));
        assert!(script.contains("meta nfproto ipv4 accept"));
        assert!(script.ends_with(CHAIN_SUFFIX));
    }

    #[test]
    fn static_rule_input_is_bounded_and_unambiguous() {
        assert_eq!(
            EgressPolicy::StaticIpv4(vec![]).guest_guard_script(),
            Err(PolicyError::EmptyStaticRules)
        );
        assert_eq!(
            EgressPolicy::StaticIpv4(vec![static_rule([1, 1, 1, 1], 32, 443); 129])
                .guest_guard_script(),
            Err(PolicyError::TooManyStaticRules)
        );
        let rule = static_rule([1, 1, 1, 1], 32, 443);
        assert_eq!(
            EgressPolicy::StaticIpv4(vec![rule, rule]).guest_guard_script(),
            Err(PolicyError::DuplicateRule)
        );
        assert_eq!(
            EgressPolicy::StaticIpv4(vec![static_rule([1, 1, 1, 1], 32, 0)]).guest_guard_script(),
            Err(PolicyError::InvalidPort)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires an isolated Linux test host with nft and CAP_NET_ADMIN"]
    fn generated_transactions_pass_nft_parser() {
        use std::io::Write as _;
        use std::process::{Command, Stdio};

        for policy in [
            EgressPolicy::DenyAll,
            EgressPolicy::PublicIpv4,
            EgressPolicy::StaticIpv4(vec![static_rule([1, 1, 1, 1], 32, 443)]),
        ] {
            let script = policy.guest_guard_script().unwrap();
            let mut nft = Command::new("nft")
                .args(["-c", "-f", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            nft.stdin
                .take()
                .unwrap()
                .write_all(script.as_bytes())
                .unwrap();
            let output = nft.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{policy:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
