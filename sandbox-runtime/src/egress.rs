//! Compile guest-side IPv4 egress rules. Permissive modes additionally require
//! an identity-bound host uplink, host INPUT/FORWARD isolation, observation and
//! crash-safe cleanup.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::net::Ipv4Addr;

use sandbox_controller_core::{
    Ipv4Cidr, NetworkPolicy, PLATFORM_DENY_IPV4, SdkIpv4Error, platform_protected_ipv4,
};

const CHAIN_PREFIX: &str = "add table inet sandproof_guard\n\
add chain inet sandproof_guard guest_out { type filter hook prerouting priority -10; policy accept; }\n\
add rule inet sandproof_guard guest_out iifname \"tap0\" ct state established,related accept\n";
const CHAIN_SUFFIX: &str = "add rule inet sandproof_guard guest_out iifname \"tap0\" drop\n";

// Platform exclusions are shared with the core. Actual host IPv4 addresses
// must also be supplied; static ranges cannot discover them.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ipv4EgressRules {
    internet_access: bool,
    allow_out: Vec<Ipv4Cidr>,
    deny_out: Vec<Ipv4Cidr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EgressPolicy {
    /// The default closed policy.
    DenyAll,
    /// Normalized fixed-SDK IPv4 address rules, installed only with a host uplink.
    Ipv4(Ipv4EgressRules),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    TooManyRules,
    InvalidIpv4Address,
    UnsupportedAddressKind,
    MissingHostBoundary,
}

fn protected_destination(address: Ipv4Addr, host_addresses: &[Ipv4Addr]) -> bool {
    host_addresses.contains(&address) || platform_protected_ipv4(u32::from(address))
}

impl From<SdkIpv4Error> for PolicyError {
    fn from(value: SdkIpv4Error) -> Self {
        match value {
            SdkIpv4Error::TooManyRules => Self::TooManyRules,
            SdkIpv4Error::InvalidAddress => Self::InvalidIpv4Address,
            SdkIpv4Error::UnsupportedAddressKind => Self::UnsupportedAddressKind,
        }
    }
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
        Self::from_core_policy(&NetworkPolicy::from_sdk_ipv4(
            allow_internet_access,
            allow_out,
            deny_out,
        )?)
    }

    /// One canonical core policy becomes the same host firewall policy.
    /// # Errors
    /// Rejects the older endpoint/port allowlist, which is not a V1 SDK mode.
    pub fn from_core_policy(policy: &NetworkPolicy) -> Result<Self, PolicyError> {
        match policy {
            NetworkPolicy::DenyAll => Ok(Self::DenyAll),
            NetworkPolicy::Ipv4 {
                internet_access,
                allow_out,
                deny_out,
            } => Ok(Self::Ipv4(Ipv4EgressRules {
                internet_access: *internet_access,
                allow_out: allow_out.clone(),
                deny_out: deny_out.clone(),
            })),
            NetworkPolicy::StaticAllowlist(_) => Err(PolicyError::UnsupportedAddressKind),
        }
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
            .any(|range| range.contains(u32::from(destination)))
        {
            return Ok(true);
        }
        if rules
            .deny_out
            .iter()
            .any(|range| range.contains(u32::from(destination)))
        {
            return Ok(false);
        }
        Ok(rules.internet_access)
    }

    /// Render one nft transaction for the VM's TAP ingress: platform deny,
    /// user allow, user deny, then default. IPv6 always reaches the final
    /// drop. Installation of permissive policy also requires a host uplink.
    ///
    /// # Errors
    /// Permissive policies require a nonempty host-boundary snapshot.
    pub fn guest_guard_script(&self, host_addresses: &[Ipv4Addr]) -> Result<String, PolicyError> {
        let mut script = String::from(CHAIN_PREFIX);
        self.append_ipv4_rules(&mut script, "guest_out", host_addresses)?;
        script.push_str(CHAIN_SUFFIX);
        Ok(script)
    }

    /// Stage a non-hooked chain while the original `guest_out` gate still drops
    /// every new guest packet. The caller can inspect and durably record the
    /// chain before making one atomic gate change.
    /// # Errors
    /// Requires a supported IPv4 policy and a host-address snapshot.
    pub fn staged_policy_script(&self, host_addresses: &[Ipv4Addr]) -> Result<String, PolicyError> {
        if !matches!(self, Self::Ipv4(_)) {
            return Err(PolicyError::UnsupportedAddressKind);
        }
        let mut script = String::from("add chain inet sandproof_guard guest_policy\n");
        self.append_ipv4_rules(&mut script, "guest_policy", host_addresses)?;
        script.push_str("add rule inet sandproof_guard guest_policy iifname \"tap0\" drop\n");
        Ok(script)
    }

    /// Switch only the hooked gate in one nft transaction. A crash leaves
    /// either the previous deny gate or this exact jump, never an empty gate.
    #[must_use]
    pub fn activate_policy_script() -> &'static str {
        "flush chain inet sandproof_guard guest_out\n\
         add rule inet sandproof_guard guest_out iifname \"tap0\" ct state established,related accept\n\
         add rule inet sandproof_guard guest_out iifname \"tap0\" jump guest_policy\n\
         add rule inet sandproof_guard guest_out iifname \"tap0\" drop\n"
    }

    fn append_ipv4_rules(
        &self,
        script: &mut String,
        chain: &str,
        host_addresses: &[Ipv4Addr],
    ) -> Result<(), PolicyError> {
        let Self::Ipv4(rules) = self else {
            return Ok(());
        };
        if host_addresses.is_empty() {
            return Err(PolicyError::MissingHostBoundary);
        }
        for range in PLATFORM_DENY_IPV4 {
            writeln!(
                script,
                "add rule inet sandproof_guard {chain} iifname \"tap0\" ip daddr {range} drop"
            )
            .expect("string write");
        }
        for address in host_addresses.iter().copied().collect::<BTreeSet<_>>() {
            writeln!(
                script,
                "add rule inet sandproof_guard {chain} iifname \"tap0\" ip daddr {address}/32 drop"
            )
            .expect("string write");
        }
        for range in &rules.allow_out {
            writeln!(
                script,
                "add rule inet sandproof_guard {chain} iifname \"tap0\" ip daddr {range} accept"
            )
            .expect("string write");
        }
        for range in &rules.deny_out {
            writeln!(
                script,
                "add rule inet sandproof_guard {chain} iifname \"tap0\" ip daddr {range} drop"
            )
            .expect("string write");
        }
        if rules.internet_access {
            writeln!(
                script,
                "add rule inet sandproof_guard {chain} iifname \"tap0\" meta nfproto ipv4 accept"
            )
            .expect("string write");
        }
        Ok(())
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
    fn staged_policy_does_not_remove_deny_gate_before_activation() {
        let staged = policy(Some(false), &["1.1.1.1"], &[])
            .staged_policy_script(&host())
            .unwrap();
        assert!(staged.starts_with("add chain inet sandproof_guard guest_policy\n"));
        assert!(staged.contains("guest_policy iifname \"tap0\" ip daddr 1.1.1.1/32 accept"));
        assert!(!staged.contains("flush chain"));
        assert!(EgressPolicy::activate_policy_script().contains("jump guest_policy"));
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
