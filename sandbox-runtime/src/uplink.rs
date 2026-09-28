//! One VM's root/namespace uplink plan. The 100.64.0.0/10 pool yields a
//! distinct /30 for each never-reused VM number; no address is shared across
//! live links. A conflicting route or interface must fail installation.

use std::{
    collections::BTreeSet,
    fmt::Write,
    fs,
    io::Write as _,
    net::Ipv4Addr,
    os::unix::fs::MetadataExt,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sandbox_controller_core::{NetworkPolicy, PLATFORM_DENY_IPV4};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    RuntimeError,
    egress::EgressPolicy,
    linux::{Runtime, read_trusted_record, run_ip, run_nft_in_namespace, write_new_record},
};

const POOL_START: u32 = 0x6440_0000;
const MAX_VM_NUMBER: u64 = 1_048_575;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct UplinkPlan {
    pub root_interface: String,
    pub filter_table: String,
    pub nat_table: String,
    pub root_address: Ipv4Addr,
    pub namespace_address: Ipv4Addr,
    pub network: Ipv4Addr,
}

impl UplinkPlan {
    /// Derive bounded names and addresses from one deployment-scoped VM ID.
    /// # Errors
    /// Rejects malformed or exhausted IDs before any host effect.
    pub fn new(id: &str) -> Result<Self, RuntimeError> {
        let mut parts = id.split('-');
        let (Some("vm"), Some(namespace), Some(number), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(RuntimeError::InvalidId);
        };
        if namespace.len() != 16 || !namespace.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RuntimeError::InvalidId);
        }
        let parsed = number.parse::<u64>().map_err(|_| RuntimeError::InvalidId)?;
        if parsed > MAX_VM_NUMBER || parsed.to_string() != number {
            return Err(RuntimeError::InvalidId);
        }
        let number = parsed;
        let tag = format!("{}{:06x}", &namespace[..6], number);
        let network = POOL_START + u32::try_from(number * 4).unwrap();
        Ok(Self {
            root_interface: format!("sp{tag}"),
            filter_table: format!("spf{namespace}{number:06x}"),
            nat_table: format!("spn{namespace}{number:06x}"),
            root_address: Ipv4Addr::from(network + 1),
            namespace_address: Ipv4Addr::from(network + 2),
            network: Ipv4Addr::from(network),
        })
    }

    /// Root namespace receives only the matching VM link. The INPUT hook
    /// protects all current and future host-local addresses, while the
    /// FORWARD hook protects other sandboxes and reserved destinations.
    #[must_use]
    pub fn root_firewall_script(&self) -> String {
        let mut script = format!(
            "add table inet {filter}\n\
             add chain inet {filter} input {{ type filter hook input priority -10; policy accept; }}\n\
             add chain inet {filter} forward {{ type filter hook forward priority -10; policy accept; }}\n\
             add rule inet {filter} input iifname \"{iface}\" drop\n\
             add rule inet {filter} forward iifname \"{iface}\" meta nfproto ipv6 drop\n\
             add rule inet {filter} forward iifname \"{iface}\" ip saddr != {ns}/32 drop\n\
             add rule inet {filter} forward iifname \"{iface}\" fib daddr type local drop\n",
            filter = self.filter_table,
            iface = self.root_interface,
            ns = self.namespace_address,
        );
        for range in PLATFORM_DENY_IPV4 {
            writeln!(
                script,
                "add rule inet {} forward iifname \"{}\" ip daddr {range} drop",
                self.filter_table, self.root_interface
            )
            .expect("string write");
        }
        writeln!(
            script,
            "add rule inet {filter} forward iifname \"{iface}\" accept\n\
             add rule inet {filter} forward oifname \"{iface}\" ct state established,related accept\n\
             add rule inet {filter} forward oifname \"{iface}\" drop\n\
             add table ip {nat}\n\
             add chain ip {nat} postrouting {{ type nat hook postrouting priority srcnat; policy accept; }}\n\
             add rule ip {nat} postrouting iifname \"{iface}\" ip saddr {ns}/32 masquerade",
            filter = self.filter_table,
            iface = self.root_interface,
            nat = self.nat_table,
            ns = self.namespace_address,
        )
        .expect("string write");
        script
    }

    #[must_use]
    pub fn namespace_nat_script() -> &'static str {
        "add table ip sandproof_uplink\n\
         add chain ip sandproof_uplink postrouting { type nat hook postrouting priority srcnat; policy accept; }\n\
         add rule ip sandproof_uplink postrouting iifname \"tap0\" oifname \"uplink0\" masquerade\n"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UplinkIntent {
    resource_id: String,
    plan: UplinkPlan,
    policy: NetworkPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UplinkPrepared {
    intent: UplinkIntent,
    namespace_inode: u64,
    root_ifindex: u64,
    host_addresses: Vec<Ipv4Addr>,
    root_filter: Value,
    root_nat: Value,
    namespace_nat: Value,
    staged_policy: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UplinkAborted {
    resource_id: String,
    policy: NetworkPolicy,
    namespace_inode: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UplinkState {
    Active(u64),
    Staged,
}

fn output(operation: &'static str, program: &str, args: &[&str]) -> Result<Vec<u8>, RuntimeError> {
    let result = Command::new(program)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()?;
    if !result.status.success() {
        return Err(RuntimeError::HostCommand {
            operation,
            stderr: String::from_utf8_lossy(&result.stderr[..result.stderr.len().min(4096)])
                .into_owned(),
        });
    }
    Ok(result.stdout)
}

fn root_nft_script(operation: &'static str, script: &str) -> Result<(), RuntimeError> {
    let mut child = Command::new("/usr/sbin/nft")
        .args(["-f", "-"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or(RuntimeError::Protocol("nft stdin"))?
        .write_all(script.as_bytes())?;
    let result = child.wait_with_output()?;
    if !result.status.success() {
        return Err(RuntimeError::HostCommand {
            operation,
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        });
    }
    Ok(())
}

fn nft_table(namespace: Option<&str>, family: &str, table: &str) -> Result<Value, RuntimeError> {
    let bytes = match namespace {
        Some(id) => run_nft_in_namespace(
            id,
            "observe namespace nft table",
            &["-j", "list", "table", family, table],
            None,
        )?,
        None => output(
            "observe root nft table",
            "/usr/sbin/nft",
            &["-j", "list", "table", family, table],
        )?,
    };
    let mut value: Value = serde_json::from_slice(&bytes)?;
    let entries = value
        .get_mut("nftables")
        .and_then(Value::as_array_mut)
        .ok_or(RuntimeError::Protocol("nft table JSON"))?;
    if entries
        .first()
        .is_some_and(|entry| entry.get("metainfo").is_some())
    {
        entries.remove(0);
    }
    if entries.is_empty() {
        return Err(RuntimeError::Protocol("empty nft table"));
    }
    Ok(value)
}

fn nft_chain(namespace: &str, chain: &str) -> Result<Value, RuntimeError> {
    let bytes = run_nft_in_namespace(
        namespace,
        "observe guest gate",
        &["-j", "list", "chain", "inet", "sandproof_guard", chain],
        None,
    )?;
    let mut value: Value = serde_json::from_slice(&bytes)?;
    let entries = value
        .get_mut("nftables")
        .and_then(Value::as_array_mut)
        .ok_or(RuntimeError::Protocol("nft chain JSON"))?;
    if entries
        .first()
        .is_some_and(|entry| entry.get("metainfo").is_some())
    {
        entries.remove(0);
    }
    Ok(value)
}

fn root_ifindex(name: &str) -> Result<u64, RuntimeError> {
    let bytes = output(
        "observe root uplink",
        "/usr/bin/ip",
        &["-j", "link", "show", "dev", name],
    )?;
    let value: Value = serde_json::from_slice(&bytes)?;
    value[0]["ifindex"]
        .as_u64()
        .filter(|index| *index != 0)
        .ok_or(RuntimeError::Protocol("uplink ifindex"))
}

fn exact_interface_address(
    namespace: Option<&str>,
    interface: &str,
    address: Ipv4Addr,
) -> Result<bool, RuntimeError> {
    let bytes = match namespace {
        Some(id) => output(
            "observe namespace uplink address",
            "/usr/bin/ip",
            &[
                "netns",
                "exec",
                id,
                "/usr/bin/ip",
                "-j",
                "-4",
                "addr",
                "show",
                "dev",
                interface,
            ],
        )?,
        None => output(
            "observe root uplink address",
            "/usr/bin/ip",
            &["-j", "-4", "addr", "show", "dev", interface],
        )?,
    };
    let value: Value = serde_json::from_slice(&bytes)?;
    let entries = value
        .as_array()
        .ok_or(RuntimeError::Protocol("uplink address JSON"))?;
    let Some(link) = entries.first() else {
        return Ok(false);
    };
    let Some(addresses) = link["addr_info"].as_array() else {
        return Ok(false);
    };
    Ok(entries.len() == 1
        && link["ifname"] == interface
        && link["flags"]
            .as_array()
            .is_some_and(|flags| flags.iter().any(|flag| flag == "UP"))
        && addresses.len() == 1
        && addresses[0]["local"] == address.to_string()
        && addresses[0]["prefixlen"] == 30)
}

fn exact_namespace_default(id: &str, gateway: Ipv4Addr) -> Result<bool, RuntimeError> {
    let bytes = output(
        "observe namespace default route",
        "/usr/bin/ip",
        &[
            "netns",
            "exec",
            id,
            "/usr/bin/ip",
            "-j",
            "-4",
            "route",
            "show",
            "default",
        ],
    )?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let routes = value
        .as_array()
        .ok_or(RuntimeError::Protocol("uplink route JSON"))?;
    Ok(routes.len() == 1
        && routes[0]["dst"] == "default"
        && routes[0]["gateway"] == gateway.to_string()
        && routes[0]["dev"] == "uplink0")
}

fn root_link_exists(name: &str) -> Result<bool, RuntimeError> {
    let bytes = output("list root links", "/usr/bin/ip", &["-j", "link", "show"])?;
    let links: Value = serde_json::from_slice(&bytes)?;
    Ok(links
        .as_array()
        .ok_or(RuntimeError::Protocol("root link JSON"))?
        .iter()
        .any(|link| link["ifname"] == name))
}

fn root_table_exists(family: &str, table: &str) -> Result<bool, RuntimeError> {
    let bytes = output(
        "list root nft tables",
        "/usr/sbin/nft",
        &["-j", "list", "tables"],
    )?;
    let value: Value = serde_json::from_slice(&bytes)?;
    Ok(value["nftables"]
        .as_array()
        .ok_or(RuntimeError::Protocol("root table JSON"))?
        .iter()
        .any(|entry| entry["table"]["family"] == family && entry["table"]["name"] == table))
}

fn namespace_link_exists(id: &str, name: &str) -> Result<bool, RuntimeError> {
    let bytes = output(
        "list namespace links",
        "/usr/bin/ip",
        &["netns", "exec", id, "/usr/bin/ip", "-j", "link", "show"],
    )?;
    let value: Value = serde_json::from_slice(&bytes)?;
    Ok(value
        .as_array()
        .ok_or(RuntimeError::Protocol("namespace link JSON"))?
        .iter()
        .any(|link| link["ifname"] == name))
}

fn namespace_nft_object_exists(
    id: &str,
    object: &str,
    family: &str,
    table: &str,
    name: Option<&str>,
) -> Result<bool, RuntimeError> {
    let bytes = run_nft_in_namespace(
        id,
        "list namespace nft objects",
        &["-j", "list", object],
        None,
    )?;
    let value: Value = serde_json::from_slice(&bytes)?;
    Ok(value["nftables"]
        .as_array()
        .ok_or(RuntimeError::Protocol("namespace nft JSON"))?
        .iter()
        .any(|entry| {
            let item = &entry[if object == "tables" { "table" } else { "chain" }];
            item["family"] == family
                && item["table"].as_str().is_none_or(|value| value == table)
                && (object == "tables" && item["name"] == table
                    || object == "chains"
                        && item["table"] == table
                        && item["name"] == name.unwrap_or(""))
        }))
}

fn record_exists(path: &std::path::Path) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.uid() == 0 => Ok(true),
        Ok(_) => Err(RuntimeError::IdentityMismatch("uplink record type")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn host_ipv4_addresses() -> Result<Vec<Ipv4Addr>, RuntimeError> {
    let bytes = output(
        "enumerate host IPv4",
        "/usr/bin/ip",
        &["-j", "-4", "addr", "show"],
    )?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let interfaces = value
        .as_array()
        .ok_or(RuntimeError::Protocol("host address JSON"))?;
    let mut addresses = BTreeSet::new();
    for interface in interfaces {
        if let Some(entries) = interface["addr_info"].as_array() {
            for address in entries {
                let address = address["local"]
                    .as_str()
                    .ok_or(RuntimeError::Protocol("host IPv4 address"))?
                    .parse()
                    .map_err(|_| RuntimeError::Protocol("invalid host IPv4 address"))?;
                addresses.insert(address);
            }
        }
    }
    if addresses.is_empty() {
        return Err(RuntimeError::InvalidConfiguration("no host IPv4 addresses"));
    }
    Ok(addresses.into_iter().collect())
}

fn root_forwarding_enabled() -> Result<bool, RuntimeError> {
    Ok(fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() == "1")
}

fn namespace_forwarding_enabled(id: &str) -> Result<bool, RuntimeError> {
    Ok(String::from_utf8(output(
        "observe namespace forwarding",
        "/usr/bin/ip",
        &[
            "netns",
            "exec",
            id,
            "/usr/sbin/sysctl",
            "-n",
            "net.ipv4.ip_forward",
        ],
    )?)
    .map_err(|_| RuntimeError::Protocol("namespace forwarding output"))?
    .trim()
        == "1")
}

fn gate_is_active(id: &str) -> Result<bool, RuntimeError> {
    let value = nft_chain(id, "guest_out")?;
    let entries = value["nftables"]
        .as_array()
        .ok_or(RuntimeError::Protocol("guest gate JSON"))?;
    if entries.len() == 3 {
        // The old exact deny guard remains. The existing strict observer
        // checks its complete contents before a staged failure is returned.
        return Ok(false);
    }
    if entries.len() != 4 {
        return Err(RuntimeError::IdentityMismatch(
            "unexpected guest gate length",
        ));
    }
    let first = &entries[1]["rule"]["expr"];
    let middle = &entries[2]["rule"]["expr"];
    let last = &entries[3]["rule"]["expr"];
    let tap_match =
        serde_json::json!({"match":{"op":"==","left":{"meta":{"key":"iifname"}},"right":"tap0"}});
    if first
        != &serde_json::json!([tap_match.clone(), {"match":{"op":"in","left":{"ct":{"key":"state"}},"right":["established","related"]}}, {"accept":null}])
        || middle != &serde_json::json!([tap_match.clone(), {"jump":{"target":"guest_policy"}}])
        || last != &serde_json::json!([tap_match, {"drop":null}])
    {
        return Err(RuntimeError::IdentityMismatch("guest policy gate"));
    }
    Ok(true)
}

impl Runtime {
    fn uplink_claim(&self, id: &str) -> std::path::PathBuf {
        self.config.chroot_base.join(".claims").join(id)
    }

    pub(crate) fn uplink_intent_exists(&self, id: &str) -> Result<bool, RuntimeError> {
        self.validate_network_id(id)?;
        record_exists(&self.uplink_claim(id).join("uplink-intent.json"))
    }

    /// Observe the recorded public policy or the still-closed private gate.
    /// # Errors
    /// Rejects a missing, changed or foreign network record.
    pub fn observe_recorded_network(&self, id: &str) -> Result<u64, RuntimeError> {
        if self.uplink_intent_exists(id)? {
            let intent: UplinkIntent =
                read_trusted_record(&self.uplink_claim(id).join("uplink-intent.json"))?;
            if intent.resource_id != id || intent.plan != UplinkPlan::new(id)? {
                return Err(RuntimeError::IdentityMismatch("recorded uplink intent"));
            }
            if record_exists(&self.uplink_claim(id).join("uplink-prepared.json"))? {
                return match self.observe_ipv4_uplink(id, &intent.policy)? {
                    UplinkState::Active(inode) => Ok(inode),
                    UplinkState::Staged => self.observe_private_network(id),
                };
            }
        }
        // An interrupted installation has never opened the hooked guest
        // gate. This inventory observation does not claim the policy worked.
        self.observe_private_network(id)
    }

    /// Check deterministic host uplink names before accepting cleanup.
    /// # Errors
    /// A failed host inventory cannot be treated as absence.
    pub fn uplink_resources_absent(&self, id: &str) -> Result<bool, RuntimeError> {
        let plan = match UplinkPlan::new(id) {
            Ok(plan) => plan,
            // A private-only resource may use an ID beyond the finite IPv4
            // /30 pool, but could never have acquired an uplink plan.
            Err(RuntimeError::InvalidId) => return Ok(true),
            Err(error) => return Err(error),
        };
        Ok(!root_link_exists(&plan.root_interface)?
            && !root_table_exists("inet", &plan.filter_table)?
            && !root_table_exists("ip", &plan.nat_table)?)
    }

    /// Called only after the exact VMM is stopped and its namespace is gone.
    /// A persisted intent binds deterministic host names even if installation
    /// failed between effects; absence checks keep repeated cleanup idempotent.
    pub(crate) fn cleanup_uplink_after_namespace(&self, id: &str) -> Result<(), RuntimeError> {
        if !self.uplink_intent_exists(id)? {
            return if self.uplink_resources_absent(id)? {
                Ok(())
            } else {
                Err(RuntimeError::IdentityMismatch("unclaimed uplink resources"))
            };
        }
        let intent: UplinkIntent =
            read_trusted_record(&self.uplink_claim(id).join("uplink-intent.json"))?;
        let plan = UplinkPlan::new(id)?;
        if intent.resource_id != id || intent.plan != plan {
            return Err(RuntimeError::IdentityMismatch("uplink intent identity"));
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while root_link_exists(&plan.root_interface)? {
            if Instant::now() >= deadline {
                return Err(RuntimeError::IdentityMismatch(
                    "uplink link survived namespace removal",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
        Self::delete_root_uplink_tables(&plan)?;
        if !self.uplink_resources_absent(id)? {
            return Err(RuntimeError::IdentityMismatch("uplink cleanup incomplete"));
        }
        Ok(())
    }

    fn delete_root_uplink_tables(plan: &UplinkPlan) -> Result<(), RuntimeError> {
        for (family, table) in [("inet", &plan.filter_table), ("ip", &plan.nat_table)] {
            if root_table_exists(family, table)? {
                root_nft_script(
                    "delete stopped uplink table",
                    &format!("delete table {family} {table}\n"),
                )?;
            }
        }
        Ok(())
    }

    /// Complete only the exact dispatched installation. A prepared record may
    /// be activated after re-observation; an unprepared installation remains
    /// behind the original deny gate and is dismantled before final failure.
    /// # Errors
    /// Refuses to reconcile unbound, changed or live foreign resources.
    #[expect(
        clippy::too_many_lines,
        reason = "each partial uplink effect is checked in installation order before a durable abort marker is accepted"
    )]
    pub fn reconcile_incomplete_ipv4_uplink(
        &self,
        id: &str,
        policy: &NetworkPolicy,
    ) -> Result<(), RuntimeError> {
        self.validate_network_id(id)?;
        let claim = self.uplink_claim(id);
        if record_exists(&claim.join("uplink-prepared.json"))? {
            self.configure_ipv4_uplink(id, policy)?;
            return Ok(());
        }
        let inode = self.observe_private_network(id)?;
        let plan = UplinkPlan::new(id)?;
        if self.uplink_intent_exists(id)? {
            let intent: UplinkIntent = read_trusted_record(&claim.join("uplink-intent.json"))?;
            if intent.resource_id != id || intent.plan != plan || &intent.policy != policy {
                return Err(RuntimeError::IdentityMismatch("incomplete uplink intent"));
            }
        } else if !self.uplink_resources_absent(id)?
            || namespace_link_exists(id, "uplink0")?
            || namespace_nft_object_exists(id, "tables", "ip", "sandproof_uplink", None)?
            || namespace_nft_object_exists(
                id,
                "chains",
                "inet",
                "sandproof_guard",
                Some("guest_policy"),
            )?
        {
            return Err(RuntimeError::IdentityMismatch(
                "uplink exists without intent",
            ));
        }
        if namespace_link_exists(id, "uplink0")? {
            run_ip(
                "reclaim incomplete namespace uplink",
                &[
                    "netns",
                    "exec",
                    id,
                    "/usr/bin/ip",
                    "link",
                    "delete",
                    "uplink0",
                ],
            )?;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while root_link_exists(&plan.root_interface)? {
            if Instant::now() >= deadline {
                return Err(RuntimeError::IdentityMismatch(
                    "incomplete root uplink still exists",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
        if namespace_nft_object_exists(id, "tables", "ip", "sandproof_uplink", None)? {
            run_nft_in_namespace(
                id,
                "reclaim namespace NAT",
                &["-f", "-"],
                Some("delete table ip sandproof_uplink\n"),
            )?;
        }
        if namespace_nft_object_exists(
            id,
            "chains",
            "inet",
            "sandproof_guard",
            Some("guest_policy"),
        )? {
            run_nft_in_namespace(
                id,
                "reclaim staged policy",
                &["-f", "-"],
                Some(
                    "flush chain inet sandproof_guard guest_policy\ndelete chain inet sandproof_guard guest_policy\n",
                ),
            )?;
        }
        Self::delete_root_uplink_tables(&plan)?;
        if !self.uplink_resources_absent(id)?
            || namespace_link_exists(id, "uplink0")?
            || namespace_nft_object_exists(id, "tables", "ip", "sandproof_uplink", None)?
            || namespace_nft_object_exists(
                id,
                "chains",
                "inet",
                "sandproof_guard",
                Some("guest_policy"),
            )?
            || self.observe_private_network(id)? != inode
        {
            return Err(RuntimeError::IdentityMismatch(
                "incomplete uplink reclamation",
            ));
        }
        let aborted = UplinkAborted {
            resource_id: id.to_owned(),
            policy: policy.clone(),
            namespace_inode: inode,
        };
        if record_exists(&claim.join("uplink-aborted.json"))? {
            let stored: UplinkAborted = read_trusted_record(&claim.join("uplink-aborted.json"))?;
            if stored.resource_id != aborted.resource_id
                || stored.policy != aborted.policy
                || stored.namespace_inode != aborted.namespace_inode
            {
                return Err(RuntimeError::IdentityMismatch("uplink abort marker"));
            }
        } else {
            write_new_record(&claim.join("uplink-aborted.json"), &aborted)?;
        }
        Ok(())
    }

    /// Observe the exact failed installation after its cleanup marker.
    /// # Errors
    /// Host observation or recorded identities may be unavailable or invalid.
    pub fn was_reclaimed_ipv4_uplink(
        &self,
        id: &str,
        policy: &NetworkPolicy,
    ) -> Result<bool, RuntimeError> {
        let path = self.uplink_claim(id).join("uplink-aborted.json");
        if !record_exists(&path)? {
            return Ok(false);
        }
        let record: UplinkAborted = read_trusted_record(&path)?;
        Ok(record.resource_id == id
            && &record.policy == policy
            && record.namespace_inode != 0
            && self.observe_private_network(id)? == record.namespace_inode
            && self.uplink_resources_absent(id)?
            && !namespace_link_exists(id, "uplink0")?
            && !namespace_nft_object_exists(id, "tables", "ip", "sandproof_uplink", None)?
            && !namespace_nft_object_exists(
                id,
                "chains",
                "inet",
                "sandproof_guard",
                Some("guest_policy"),
            )?)
    }

    /// Install a non-reusable per-VM uplink. The guest deny gate is the last
    /// change, after every host rule and its observation is durably recorded.
    /// # Errors
    /// Leaves an intent for exact reconciliation if any partial step fails.
    #[expect(
        clippy::too_many_lines,
        reason = "host effects are deliberately ordered behind one durable intent and before one final guest-gate activation"
    )]
    pub fn configure_ipv4_uplink(
        &self,
        id: &str,
        policy: &NetworkPolicy,
    ) -> Result<u64, RuntimeError> {
        self.validate_network_id(id)?;
        let egress = EgressPolicy::from_core_policy(policy)
            .map_err(|_| RuntimeError::InvalidConfiguration("unsupported network policy"))?;
        if matches!(egress, EgressPolicy::DenyAll) {
            return self.observe_private_network(id);
        }
        if self.uplink_intent_exists(id)? {
            return match self.observe_ipv4_uplink(id, policy)? {
                UplinkState::Active(inode) => Ok(inode),
                UplinkState::Staged => {
                    run_nft_in_namespace(
                        id,
                        "activate prepared guest IPv4 policy",
                        &["-f", "-"],
                        Some(EgressPolicy::activate_policy_script()),
                    )?;
                    match self.observe_ipv4_uplink(id, policy)? {
                        UplinkState::Active(inode) => Ok(inode),
                        UplinkState::Staged => {
                            Err(RuntimeError::Protocol("guest policy gate did not open"))
                        }
                    }
                }
            };
        }
        let namespace_inode = self.observe_private_network(id)?;
        if !root_forwarding_enabled()? {
            return Err(RuntimeError::InvalidConfiguration(
                "host IPv4 forwarding is disabled",
            ));
        }
        let plan = UplinkPlan::new(id)?;
        if !self.uplink_resources_absent(id)? {
            return Err(RuntimeError::AlreadyClaimed);
        }
        if namespace_link_exists(id, "uplink0")?
            || namespace_nft_object_exists(id, "tables", "ip", "sandproof_uplink", None)?
            || namespace_nft_object_exists(
                id,
                "chains",
                "inet",
                "sandproof_guard",
                Some("guest_policy"),
            )?
        {
            return Err(RuntimeError::AlreadyClaimed);
        }
        let claim = self.uplink_claim(id);
        let intent = UplinkIntent {
            resource_id: id.to_owned(),
            plan: plan.clone(),
            policy: policy.clone(),
        };
        write_new_record(&claim.join("uplink-intent.json"), &intent)?;
        root_nft_script("install root egress guard", &plan.root_firewall_script())?;
        run_ip(
            "create VM uplink",
            &[
                "link",
                "add",
                &plan.root_interface,
                "type",
                "veth",
                "peer",
                "name",
                "uplink0",
                "netns",
                id,
            ],
        )?;
        run_ip(
            "assign root uplink address",
            &[
                "addr",
                "add",
                &format!("{}/30", plan.root_address),
                "dev",
                &plan.root_interface,
            ],
        )?;
        run_ip(
            "raise root uplink",
            &["link", "set", &plan.root_interface, "up"],
        )?;
        run_ip(
            "assign namespace uplink address",
            &[
                "netns",
                "exec",
                id,
                "/usr/bin/ip",
                "addr",
                "add",
                &format!("{}/30", plan.namespace_address),
                "dev",
                "uplink0",
            ],
        )?;
        run_ip(
            "raise namespace uplink",
            &[
                "netns",
                "exec",
                id,
                "/usr/bin/ip",
                "link",
                "set",
                "uplink0",
                "up",
            ],
        )?;
        run_ip(
            "route namespace egress",
            &[
                "netns",
                "exec",
                id,
                "/usr/bin/ip",
                "route",
                "add",
                "default",
                "via",
                &plan.root_address.to_string(),
                "dev",
                "uplink0",
            ],
        )?;
        run_ip(
            "enable namespace forwarding",
            &[
                "netns",
                "exec",
                id,
                "/usr/sbin/sysctl",
                "-w",
                "net.ipv4.ip_forward=1",
            ],
        )?;
        run_nft_in_namespace(
            id,
            "install namespace NAT",
            &["-f", "-"],
            Some(UplinkPlan::namespace_nat_script()),
        )?;
        let host_addresses = host_ipv4_addresses()?;
        run_nft_in_namespace(
            id,
            "stage guest IPv4 policy",
            &["-f", "-"],
            Some(
                &egress
                    .staged_policy_script(&host_addresses)
                    .map_err(|_| RuntimeError::Protocol("render guest policy"))?,
            ),
        )?;
        let prepared = UplinkPrepared {
            intent,
            namespace_inode,
            root_ifindex: root_ifindex(&plan.root_interface)?,
            host_addresses,
            root_filter: nft_table(None, "inet", &plan.filter_table)?,
            root_nat: nft_table(None, "ip", &plan.nat_table)?,
            namespace_nat: nft_table(Some(id), "ip", "sandproof_uplink")?,
            staged_policy: nft_chain(id, "guest_policy")?,
        };
        write_new_record(&claim.join("uplink-prepared.json"), &prepared)?;
        run_nft_in_namespace(
            id,
            "activate guest IPv4 policy",
            &["-f", "-"],
            Some(EgressPolicy::activate_policy_script()),
        )?;
        match self.observe_ipv4_uplink(id, policy)? {
            UplinkState::Active(inode) => Ok(inode),
            UplinkState::Staged => Err(RuntimeError::Protocol("guest policy gate did not open")),
        }
    }

    /// Read-only inspection; staged but unopened policy is a definitive
    /// failed configuration, not an observed success.
    /// # Errors
    /// Rejects missing, foreign or altered host/namespace resources.
    pub fn observe_ipv4_uplink(
        &self,
        id: &str,
        policy: &NetworkPolicy,
    ) -> Result<UplinkState, RuntimeError> {
        self.validate_network_id(id)?;
        let plan = UplinkPlan::new(id)?;
        let claim = self.uplink_claim(id);
        let prepared: UplinkPrepared = read_trusted_record(&claim.join("uplink-prepared.json"))?;
        if prepared.intent.resource_id != id
            || prepared.intent.plan != plan
            || &prepared.intent.policy != policy
            || prepared.namespace_inode == 0
            || prepared.root_ifindex == 0
            || prepared.host_addresses.is_empty()
        {
            return Err(RuntimeError::IdentityMismatch("uplink prepared record"));
        }
        let namespace =
            self.config
                .private_netns
                .as_ref()
                .ok_or(RuntimeError::InvalidConfiguration(
                    "private network required",
                ))?;
        if fs::symlink_metadata(namespace)?.ino() != prepared.namespace_inode
            || root_ifindex(&plan.root_interface)? != prepared.root_ifindex
            || !root_forwarding_enabled()?
            || !namespace_forwarding_enabled(id)?
            || !exact_interface_address(None, &plan.root_interface, plan.root_address)?
            || !exact_interface_address(Some(id), "uplink0", plan.namespace_address)?
            || !exact_namespace_default(id, plan.root_address)?
            || nft_table(None, "inet", &plan.filter_table)? != prepared.root_filter
            || nft_table(None, "ip", &plan.nat_table)? != prepared.root_nat
            || nft_table(Some(id), "ip", "sandproof_uplink")? != prepared.namespace_nat
            || nft_chain(id, "guest_policy")? != prepared.staged_policy
        {
            return Err(RuntimeError::IdentityMismatch("uplink state or rules"));
        }
        if !gate_is_active(id)? {
            self.observe_private_network(id)?;
            return Ok(UplinkState::Staged);
        }
        Ok(UplinkState::Active(prepared.namespace_inode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, MetadataExt},
        path::PathBuf,
        process::{Command, Stdio},
    };

    struct Cleanup<'a> {
        runtime: &'a Runtime,
        id: &'a str,
        guest: Option<&'a str>,
        outside: Option<&'a str>,
        outside_root: Option<&'a str>,
    }

    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            for name in [self.guest, self.outside].into_iter().flatten() {
                if PathBuf::from(format!("/run/netns/{name}")).exists() {
                    let _ = run_ip("test cleanup peer namespace", &["netns", "delete", name]);
                }
            }
            if let Some(link) = self.outside_root
                && root_link_exists(link).unwrap_or(false)
            {
                let _ = run_ip("test cleanup outside link", &["link", "delete", link]);
            }
            if PathBuf::from(format!("/run/netns/{}", self.id)).exists() {
                let _ = run_ip("test cleanup namespace", &["netns", "delete", self.id]);
            }
            let _ = self.runtime.cleanup_uplink_after_namespace(self.id);
        }
    }

    #[test]
    fn one_vm_has_deterministic_bounded_names_and_distinct_addresses() {
        let first = UplinkPlan::new("vm-0123456789abcdef-0").unwrap();
        let next = UplinkPlan::new("vm-0123456789abcdef-1").unwrap();
        assert_eq!(first.root_interface, "sp012345000000");
        assert!(first.root_interface.len() <= 15);
        assert_eq!(first.root_address, Ipv4Addr::new(100, 64, 0, 1));
        assert_eq!(first.namespace_address, Ipv4Addr::new(100, 64, 0, 2));
        assert_eq!(next.root_address, Ipv4Addr::new(100, 64, 0, 5));
        assert_ne!(first.filter_table, next.filter_table);
        assert!(UplinkPlan::new("vm-0123456789abcdef-1048576").is_err());
        assert!(UplinkPlan::new("vm-0123456789abcdef-00").is_err());
        assert!(UplinkPlan::new("vm-evil-1").is_err());
    }

    #[test]
    #[ignore = "requires root with nft on an isolated Linux host"]
    fn nft_transactions_parse_without_installing_rules() {
        for script in [
            UplinkPlan::new("vm-0123456789abcdef-1")
                .unwrap()
                .root_firewall_script(),
            UplinkPlan::namespace_nat_script().into(),
        ] {
            let mut child = Command::new("/usr/sbin/nft")
                .args(["-c", "-f", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(script.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    #[ignore = "requires root, KVM host namespaces and temporary IPv4 forwarding"]
    #[expect(
        clippy::too_many_lines,
        reason = "one isolated host topology exercises staged activation, real packets, abort and exact cleanup"
    )]
    fn configured_uplink_survives_observation_and_exact_cleanup() {
        assert!(
            root_forwarding_enabled().unwrap(),
            "test host must temporarily enable forwarding"
        );
        let namespace = format!(
            "{:016x}",
            u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )
            .unwrap()
                ^ u64::from(std::process::id())
        );
        let id = format!("vm-{namespace}-1");
        let guest = format!("spg{}", &namespace[..8]);
        let outside = format!("spo{}", &namespace[..8]);
        let outside_root = format!("spe{}", &namespace[..8]);
        let outside_peer = format!("spp{}", &namespace[..8]);
        let guest_end = format!("spx{}", &namespace[..8]);
        let tap_end = format!("spy{}", &namespace[..8]);
        let base = tempfile::tempdir_in("/var/lib").unwrap();
        let claim = base.path().join(".claims").join(&id);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&claim)
            .unwrap();
        let runtime = Runtime {
            config: crate::RuntimeConfig {
                jailer: PathBuf::from("/usr/bin/true"),
                firecracker: PathBuf::from("/usr/bin/true"),
                chroot_base: base.path().to_path_buf(),
                kernel: PathBuf::new(),
                rootfs: PathBuf::new(),
                vm_uid: 10001,
                vm_gid: 10001,
                vcpu_count: 1,
                memory_mib: 128,
                private_netns: Some(PathBuf::from(format!("/run/netns/{id}"))),
                access_token_hash: None,
                host_resources: None,
            },
        };
        run_ip("test create namespace", &["netns", "add", &id]).unwrap();
        let _cleanup = Cleanup {
            runtime: &runtime,
            id: &id,
            guest: Some(&guest),
            outside: Some(&outside),
            outside_root: Some(&outside_root),
        };
        let inode = fs::symlink_metadata(format!("/run/netns/{id}"))
            .unwrap()
            .ino();
        write_new_record(
            &claim.join("network.json"),
            &serde_json::json!({"resource_id": id, "namespace_inode": inode}),
        )
        .unwrap();
        let guard = EgressPolicy::DenyAll.guest_guard_script(&[]).unwrap();
        run_nft_in_namespace(&id, "test install guest guard", &["-f", "-"], Some(&guard)).unwrap();
        run_ip("test guest namespace", &["netns", "add", &guest]).unwrap();
        run_ip("test outside namespace", &["netns", "add", &outside]).unwrap();
        run_ip(
            "test guest veth",
            &[
                "link", "add", &guest_end, "type", "veth", "peer", "name", &tap_end,
            ],
        )
        .unwrap();
        run_ip(
            "test guest veth move",
            &["link", "set", &guest_end, "netns", &guest],
        )
        .unwrap();
        run_ip(
            "test tap veth move",
            &["link", "set", &tap_end, "netns", &id],
        )
        .unwrap();
        run_ip(
            "test guest rename",
            &[
                "netns",
                "exec",
                &guest,
                "/usr/bin/ip",
                "link",
                "set",
                &guest_end,
                "name",
                "eth0",
            ],
        )
        .unwrap();
        run_ip(
            "test tap rename",
            &[
                "netns",
                "exec",
                &id,
                "/usr/bin/ip",
                "link",
                "set",
                &tap_end,
                "name",
                "tap0",
            ],
        )
        .unwrap();
        for (netns, address, dev) in [
            (&guest, "172.16.0.2/30", "eth0"),
            (&id, "172.16.0.1/30", "tap0"),
        ] {
            run_ip(
                "test guest address",
                &[
                    "netns",
                    "exec",
                    netns,
                    "/usr/bin/ip",
                    "addr",
                    "add",
                    address,
                    "dev",
                    dev,
                ],
            )
            .unwrap();
            run_ip(
                "test guest link up",
                &[
                    "netns",
                    "exec",
                    netns,
                    "/usr/bin/ip",
                    "link",
                    "set",
                    dev,
                    "up",
                ],
            )
            .unwrap();
        }
        run_ip(
            "test guest default route",
            &[
                "netns",
                "exec",
                &guest,
                "/usr/bin/ip",
                "route",
                "add",
                "default",
                "via",
                "172.16.0.1",
            ],
        )
        .unwrap();
        run_ip(
            "test outside veth",
            &[
                "link",
                "add",
                &outside_root,
                "type",
                "veth",
                "peer",
                "name",
                &outside_peer,
            ],
        )
        .unwrap();
        run_ip(
            "test outside veth move",
            &["link", "set", &outside_peer, "netns", &outside],
        )
        .unwrap();
        run_ip(
            "test outside rename",
            &[
                "netns",
                "exec",
                &outside,
                "/usr/bin/ip",
                "link",
                "set",
                &outside_peer,
                "name",
                "outside0",
            ],
        )
        .unwrap();
        run_ip(
            "test outside root address",
            &["addr", "add", "11.22.33.2/30", "dev", &outside_root],
        )
        .unwrap();
        run_ip(
            "test outside root link up",
            &["link", "set", &outside_root, "up"],
        )
        .unwrap();
        run_ip(
            "test outside address",
            &[
                "netns",
                "exec",
                &outside,
                "/usr/bin/ip",
                "addr",
                "add",
                "11.22.33.1/30",
                "dev",
                "outside0",
            ],
        )
        .unwrap();
        run_ip(
            "test outside link up",
            &[
                "netns",
                "exec",
                &outside,
                "/usr/bin/ip",
                "link",
                "set",
                "outside0",
                "up",
            ],
        )
        .unwrap();
        run_ip(
            "test outside loopback",
            &[
                "netns",
                "exec",
                &outside,
                "/usr/bin/ip",
                "link",
                "set",
                "lo",
                "up",
            ],
        )
        .unwrap();
        run_ip(
            "test denied outside address",
            &[
                "netns",
                "exec",
                &outside,
                "/usr/bin/ip",
                "addr",
                "add",
                "11.22.34.1/32",
                "dev",
                "lo",
            ],
        )
        .unwrap();
        run_ip(
            "test denied outside route",
            &[
                "route",
                "add",
                "11.22.34.1/32",
                "via",
                "11.22.33.1",
                "dev",
                &outside_root,
            ],
        )
        .unwrap();
        let ping = |destination: &str| {
            Command::new("/usr/bin/ip")
                .args([
                    "netns",
                    "exec",
                    &guest,
                    "/usr/bin/ping",
                    "-n",
                    "-c",
                    "1",
                    "-W",
                    "1",
                    destination,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        assert!(!ping("11.22.33.1"), "unconfigured guest must remain denied");
        let policy = NetworkPolicy::from_sdk_ipv4(
            Some(false),
            &["11.22.33.1".into()],
            &["0.0.0.0/0".into()],
        )
        .unwrap();
        assert_eq!(runtime.configure_ipv4_uplink(&id, &policy).unwrap(), inode);
        assert_eq!(
            runtime.observe_ipv4_uplink(&id, &policy).unwrap(),
            UplinkState::Active(inode)
        );
        assert_eq!(runtime.configure_ipv4_uplink(&id, &policy).unwrap(), inode);
        run_nft_in_namespace(&id, "test interrupt before gate activation", &["-f", "-"], Some(
            "flush chain inet sandproof_guard guest_out\n\
             add rule inet sandproof_guard guest_out iifname \"tap0\" ct state established,related accept\n\
             add rule inet sandproof_guard guest_out iifname \"tap0\" drop\n"
        )).unwrap();
        assert_eq!(
            runtime.observe_ipv4_uplink(&id, &policy).unwrap(),
            UplinkState::Staged
        );
        assert!(!ping("11.22.33.1"), "prepared but unopened gate must deny");
        runtime
            .reconcile_incomplete_ipv4_uplink(&id, &policy)
            .unwrap();
        assert_eq!(
            runtime.observe_ipv4_uplink(&id, &policy).unwrap(),
            UplinkState::Active(inode)
        );
        assert!(
            ping("11.22.33.1"),
            "explicitly allowed outside packet must return"
        );
        assert!(
            !ping("11.22.34.1"),
            "reachable but unlisted outside address must be denied"
        );
        assert!(!ping("11.22.33.2"), "host-local address must remain denied");
        run_ip("test delete guest namespace", &["netns", "delete", &guest]).unwrap();
        run_ip(
            "test delete outside namespace",
            &["netns", "delete", &outside],
        )
        .unwrap();
        run_ip(
            "test delete outside root link",
            &["link", "delete", &outside_root],
        )
        .unwrap();
        run_ip("test delete namespace", &["netns", "delete", &id]).unwrap();
        runtime.cleanup_uplink_after_namespace(&id).unwrap();
        assert!(runtime.uplink_resources_absent(&id).unwrap());

        // Crash after the durable intent and atomic root firewall batch, but
        // before the veth or prepared receipt: cleanup must not require a VM.
        let partial_id = format!("vm-{namespace}-2");
        let partial_claim = base.path().join(".claims").join(&partial_id);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&partial_claim)
            .unwrap();
        let mut partial_config = runtime.config.clone();
        partial_config.private_netns = Some(PathBuf::from(format!("/run/netns/{partial_id}")));
        let partial_runtime = Runtime {
            config: partial_config,
        };
        let _partial_cleanup = Cleanup {
            runtime: &partial_runtime,
            id: &partial_id,
            guest: None,
            outside: None,
            outside_root: None,
        };
        let plan = UplinkPlan::new(&partial_id).unwrap();
        run_ip("test partial namespace", &["netns", "add", &partial_id]).unwrap();
        let partial_inode = fs::symlink_metadata(format!("/run/netns/{partial_id}"))
            .unwrap()
            .ino();
        write_new_record(
            &partial_claim.join("network.json"),
            &serde_json::json!({"resource_id": partial_id, "namespace_inode": partial_inode}),
        )
        .unwrap();
        run_nft_in_namespace(
            &partial_id,
            "test partial deny guard",
            &["-f", "-"],
            Some(&guard),
        )
        .unwrap();
        write_new_record(
            &partial_claim.join("uplink-intent.json"),
            &UplinkIntent {
                resource_id: partial_id.clone(),
                plan: plan.clone(),
                policy: policy.clone(),
            },
        )
        .unwrap();
        root_nft_script("test partial root rules", &plan.root_firewall_script()).unwrap();
        run_ip(
            "test partial uplink",
            &[
                "link",
                "add",
                &plan.root_interface,
                "type",
                "veth",
                "peer",
                "name",
                "uplink0",
                "netns",
                &partial_id,
            ],
        )
        .unwrap();
        assert!(
            !partial_runtime
                .uplink_resources_absent(&partial_id)
                .unwrap()
        );
        assert_eq!(
            partial_runtime
                .observe_recorded_network(&partial_id)
                .unwrap(),
            partial_inode
        );
        partial_runtime
            .reconcile_incomplete_ipv4_uplink(&partial_id, &policy)
            .unwrap();
        assert!(
            partial_runtime
                .was_reclaimed_ipv4_uplink(&partial_id, &policy)
                .unwrap()
        );
        run_ip(
            "test delete partial namespace",
            &["netns", "delete", &partial_id],
        )
        .unwrap();
        partial_runtime
            .cleanup_uplink_after_namespace(&partial_id)
            .unwrap();
        assert!(
            partial_runtime
                .uplink_resources_absent(&partial_id)
                .unwrap()
        );
    }
}
