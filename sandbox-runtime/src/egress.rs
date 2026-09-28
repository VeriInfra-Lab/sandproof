//! Compile guest-side IPv4 egress rules. The running controller still installs
//! only `DenyAll`; permissive rules also require an identity-bound host uplink,
//! host INPUT/FORWARD isolation, observation and crash-safe cleanup.

use std::collections::BTreeSet;
use std::fmt::{self, Display, Write};
use std::net::Ipv4Addr;

const MAX_RULES_PER_LIST: usize = 128;
const CHAIN_PREFIX: &str = "add table inet sandproof_guard\n\
add chain inet sandproof_guard guest_out { type filter hook prerouting priority -10; policy accept; }\n\
add rule inet sandproof_guard guest_out iifname \"tap0\" ct state established,related accept\n";
const CHAIN_SUFFIX: &str = "add rule inet sandproof_guard guest_out iifname \"tap0\" drop\n";

// Platform exclusions are independent of the user's lists. Actual host IPv4
// addresses must also be supplied; a static list cannot discover them.
const PLATFORM_DENY: &[(Ipv4Addr, u8)] = &[
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
pub struct Ipv4Cidr {
    network: Ipv4Addr,
    prefix_length: u8,
}

impl Ipv4Cidr {
    fn contains(self, address: Ipv4Addr) -> bool {
        (u32::from(address) & prefix_mask(self.prefix_length)) == u32::from(self.network)
    }
}

impl Display for Ipv4Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix_length)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ipv4EgressRules {
    internet_access: bool,
    allow_out: Vec<Ipv4Cidr>,
    deny_out: Vec<Ipv4Cidr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EgressPolicy {
    /// The only policy currently installed by the runtime.
    DenyAll,
    /// Normalized fixed-SDK IPv4 address rules, not yet wired to a VM uplink.
    Ipv4(Ipv4EgressRules),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    TooManyRules,
    InvalidIpv4Address,
    UnsupportedAddressKind,
    MissingHostBoundary,
}

fn prefix_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}

fn parse_cidr(input: &str) -> Result<Ipv4Cidr, PolicyError> {
    let (address, prefix) = if let Some((address, prefix)) = input.split_once('/') {
        let prefix = prefix
            .parse::<u8>()
            .map_err(|_| PolicyError::InvalidIpv4Address)?;
        if prefix > 32 {
            return Err(PolicyError::InvalidIpv4Address);
        }
        (address, prefix)
    } else {
        (input, 32)
    };
    let address = address.parse::<Ipv4Addr>().map_err(|_| {
        if input.contains(':') || input.chars().any(char::is_alphabetic) {
            PolicyError::UnsupportedAddressKind
        } else {
            PolicyError::InvalidIpv4Address
        }
    })?;
    // The fixed API accepts this sentinel, not an unspecified host address.
    if address.is_unspecified() && input != "0.0.0.0/0" {
        return Err(PolicyError::InvalidIpv4Address);
    }
    Ok(Ipv4Cidr {
        network: Ipv4Addr::from(u32::from(address) & prefix_mask(prefix)),
        prefix_length: prefix,
    })
}

fn parse_list(entries: &[String]) -> Result<Vec<Ipv4Cidr>, PolicyError> {
    if entries.len() > MAX_RULES_PER_LIST {
        return Err(PolicyError::TooManyRules);
    }
    // The SDK accepts duplicates and CIDRs with host bits. Rule order within
    // one list has no meaning; normalize and deduplicate deterministically.
    entries
        .iter()
        .map(|entry| parse_cidr(entry))
        .collect::<Result<BTreeSet<_>, _>>()
        .map(|rules| rules.into_iter().collect())
}

fn protected_destination(address: Ipv4Addr, host_addresses: &[Ipv4Addr]) -> bool {
    host_addresses.contains(&address)
        || PLATFORM_DENY.iter().any(|(network, prefix)| {
            Ipv4Cidr {
                network: *network,
                prefix_length: *prefix,
            }
            .contains(address)
        })
}

impl EgressPolicy {
    /// Normalize the fixed SDK's selected IPv4 address subset. Omitted
    /// `allow_internet_access` means public-by-default; false adds a final
    /// all-deny while preserving explicit `allow_out` exceptions. Rules apply
    /// to all IPv4 protocols and ports, including DNS, without implicit DNS.
    ///
    /// # Errors
    /// Rejects unsupported domains/IPv6, invalid IPv4 and excessive lists.
    pub fn from_sdk_addresses(
        allow_internet_access: Option<bool>,
        allow_out: &[String],
        deny_out: &[String],
    ) -> Result<Self, PolicyError> {
        Ok(Self::Ipv4(Ipv4EgressRules {
            internet_access: allow_internet_access.unwrap_or(true),
            allow_out: parse_list(allow_out)?,
            deny_out: parse_list(deny_out)?,
        }))
    }

    /// Evaluate the intended policy for a new guest-initiated IPv4 connection.
    /// The host address snapshot is not a substitute for host-side guards.
    ///
    /// # Errors
    /// Permissive policies require a nonempty host-boundary snapshot.
    pub fn allows_ipv4(
        &self,
        destination: Ipv4Addr,
        host_addresses: &[Ipv4Addr],
    ) -> Result<bool, PolicyError> {
        let Self::Ipv4(rules) = self else {
            return Ok(false);
        };
        if host_addresses.is_empty() {
            return Err(PolicyError::MissingHostBoundary);
        }
        if protected_destination(destination, host_addresses) {
            return Ok(false);
        }
        if rules
            .allow_out
            .iter()
            .any(|range| range.contains(destination))
        {
            return Ok(true);
        }
        if rules
            .deny_out
            .iter()
            .any(|range| range.contains(destination))
        {
            return Ok(false);
        }
        Ok(rules.internet_access)
    }

    /// Render one nft transaction for the VM's TAP ingress: platform deny,
    /// user allow, user deny, then default. IPv6 always reaches the final
    /// drop. Only `DenyAll` may be installed by the current runtime.
    ///
    /// # Errors
    /// Permissive policies require a nonempty host-boundary snapshot.
    pub fn guest_guard_script(&self, host_addresses: &[Ipv4Addr]) -> Result<String, PolicyError> {
        let mut script = String::from(CHAIN_PREFIX);
        if let Self::Ipv4(rules) = self {
            if host_addresses.is_empty() {
                return Err(PolicyError::MissingHostBoundary);
            }
            for (network, prefix) in PLATFORM_DENY {
                writeln!(script, "add rule inet sandproof_guard guest_out iifname \"tap0\" ip daddr {network}/{prefix} drop")
                    .expect("writing to a string cannot fail");
            }
            for address in host_addresses.iter().copied().collect::<BTreeSet<_>>() {
                writeln!(script, "add rule inet sandproof_guard guest_out iifname \"tap0\" ip daddr {address}/32 drop")
                    .expect("writing to a string cannot fail");
            }
            for range in &rules.allow_out {
                writeln!(script, "add rule inet sandproof_guard guest_out iifname \"tap0\" ip daddr {range} accept")
                    .expect("writing to a string cannot fail");
            }
            for range in &rules.deny_out {
                writeln!(
                    script,
                    "add rule inet sandproof_guard guest_out iifname \"tap0\" ip daddr {range} drop"
                )
                .expect("writing to a string cannot fail");
            }
            if rules.internet_access {
                script.push_str("add rule inet sandproof_guard guest_out iifname \"tap0\" meta nfproto ipv4 accept\n");
            }
        }
        script.push_str(CHAIN_SUFFIX);
        Ok(script)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| (*entry).to_owned()).collect()
    }

    fn policy(internet: Option<bool>, allow: &[&str], deny: &[&str]) -> EgressPolicy {
        EgressPolicy::from_sdk_addresses(internet, &values(allow), &values(deny)).unwrap()
    }

    fn host() -> [Ipv4Addr; 1] {
        [Ipv4Addr::new(8, 8, 4, 4)]
    }

    fn permitted(policy: &EgressPolicy, address: [u8; 4]) -> bool {
        policy
            .allows_ipv4(Ipv4Addr::from(address), &host())
            .unwrap()
    }

    #[test]
    fn deny_keeps_the_existing_exact_default_drop() {
        let script = EgressPolicy::DenyAll.guest_guard_script(&[]).unwrap();
        assert_eq!(script, format!("{CHAIN_PREFIX}{CHAIN_SUFFIX}"));
        assert!(!permitted(&EgressPolicy::DenyAll, [1, 1, 1, 1]));
    }

    #[test]
    fn sdk_default_is_public_but_false_retains_explicit_exceptions() {
        let public = policy(None, &[], &[]);
        assert!(permitted(&public, [1, 1, 1, 1]));
        assert!(!permitted(&public, [10, 0, 0, 1]));
        assert!(!permitted(&public, [8, 8, 4, 4]));
        let restricted = policy(Some(false), &["1.1.1.1"], &[]);
        assert!(permitted(&restricted, [1, 1, 1, 1]));
        assert!(!permitted(&restricted, [1, 0, 0, 1]));
        assert!(!permitted(&policy(Some(false), &[], &[]), [1, 1, 1, 1]));
    }

    #[test]
    fn allow_precedes_user_deny_but_never_platform_deny() {
        let rules = policy(
            Some(false),
            &["1.1.1.1", "0.0.0.0/0"],
            &["1.1.1.0/24", "0.0.0.0/0"],
        );
        assert!(permitted(&rules, [1, 1, 1, 1]));
        assert!(permitted(&rules, [9, 9, 9, 9]));
        assert!(!permitted(&rules, [10, 0, 0, 1]));
        assert!(!permitted(&rules, [8, 8, 4, 4]));
        let script = rules.guest_guard_script(&host()).unwrap();
        let platform = script.find("ip daddr 10.0.0.0/8 drop").unwrap();
        let host_rule = script.find("ip daddr 8.8.4.4/32 drop").unwrap();
        let allow = script.find("ip daddr 0.0.0.0/0 accept").unwrap();
        let deny = script.rfind("ip daddr 0.0.0.0/0 drop").unwrap();
        assert!(platform < host_rule && host_rule < allow && allow < deny);
        assert!(script.ends_with(CHAIN_SUFFIX));
    }

    #[test]
    fn allow_list_alone_does_not_restrict_public_default() {
        let rules = policy(Some(true), &["1.1.1.1"], &["1.1.1.0/24"]);
        assert!(permitted(&rules, [1, 1, 1, 1]));
        assert!(!permitted(&rules, [1, 1, 1, 2]));
        assert!(permitted(&rules, [9, 9, 9, 9]));
    }

    #[test]
    fn duplicate_and_host_bit_rules_normalize_without_ports() {
        let rules = policy(
            Some(false),
            &["1.1.1.1/24", "1.1.1.0/24", "1.1.1.0/24"],
            &["0.0.0.0/0", "0.0.0.0/0"],
        );
        assert!(permitted(&rules, [1, 1, 1, 200]));
        assert!(!permitted(&rules, [1, 1, 2, 1]));
        let script = rules.guest_guard_script(&host()).unwrap();
        assert_eq!(script.matches("ip daddr 1.1.1.0/24 accept").count(), 1);
        assert_eq!(script.matches("ip daddr 0.0.0.0/0 drop").count(), 1);
        assert!(!script.contains(" dport "));
    }

    #[test]
    fn unsupported_inputs_fail_before_render_or_install() {
        for input in ["example.com", "::1", "0.0.0.0", "bad address"] {
            assert!(EgressPolicy::from_sdk_addresses(None, &values(&[input]), &[]).is_err());
        }
        for input in ["1.1.1.1/33", "1.1.1.1/-1", "1.1.1.1/24/1"] {
            assert_eq!(
                EgressPolicy::from_sdk_addresses(None, &values(&[input]), &[]),
                Err(PolicyError::InvalidIpv4Address)
            );
        }
        assert_eq!(
            EgressPolicy::from_sdk_addresses(None, &values(&vec!["1.1.1.1"; 129]), &[]),
            Err(PolicyError::TooManyRules)
        );
        assert_eq!(
            policy(None, &[], &[]).guest_guard_script(&[]),
            Err(PolicyError::MissingHostBoundary)
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
            policy(None, &[], &[]),
            policy(Some(false), &["1.1.1.1", "0.0.0.0/0"], &["0.0.0.0/0"]),
        ] {
            let script = policy.guest_guard_script(&host()).unwrap();
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
