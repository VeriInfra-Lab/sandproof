#![cfg(target_os = "linux")]

//! Packet test for the #10 guest-side egress compiler. Every veth end
//! stays in a private namespace; this never routes to the real internet or
//! changes the host's global forwarding setting.

use sandbox_runtime::egress::EgressPolicy;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU32 = AtomicU32::new(0);

fn run(program: &str, args: &[&str]) {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// Arguments here are built only from fixed tokens and hex-generated names;
// split_ascii_whitespace is not a shell and cannot interpret metacharacters.
fn ip(command: &str) {
    run("ip", &command.split_ascii_whitespace().collect::<Vec<_>>());
}

fn ping(namespace: &str, address: &str, ipv6: bool) -> bool {
    let mut command = Command::new("ip");
    command.args(["netns", "exec", namespace, "ping"]);
    if ipv6 {
        command.arg("-6");
    }
    let timeout = if ipv6 { "3" } else { "1" };
    command
        .args(["-n", "-c", "1", "-W", timeout, address])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

struct Topology {
    guest: String,
    guard: String,
    outside: String,
    namespaces: Vec<String>,
    root_links: Vec<String>,
}

impl Topology {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let tag = format!(
            "{:x}{:04x}{sequence:x}",
            std::process::id() & 0xffff,
            nanos & 0xffff
        );
        let mut network = Self {
            guest: format!("sp{tag}g"),
            guard: format!("sp{tag}f"),
            outside: format!("sp{tag}e"),
            namespaces: Vec::new(),
            root_links: Vec::new(),
        };
        for name in [&network.guest, &network.guard, &network.outside] {
            ip(&format!("netns add {name}"));
            network.namespaces.push(name.clone());
            ip(&format!("-n {name} link set lo up"));
        }
        let guest_link = format!("vg{tag}");
        let tap_link = format!("vt{tag}");
        let outside_link = format!("ve{tag}");
        let uplink_link = format!("vu{tag}");
        ip(&format!(
            "link add {guest_link} type veth peer name {tap_link}"
        ));
        network.root_links.push(guest_link.clone());
        ip(&format!("link set {guest_link} netns {}", network.guest));
        ip(&format!("link set {tap_link} netns {}", network.guard));
        ip(&format!(
            "-n {} link set {guest_link} name guest0",
            network.guest
        ));
        ip(&format!(
            "-n {} link set {tap_link} name tap0",
            network.guard
        ));
        ip(&format!(
            "link add {outside_link} type veth peer name {uplink_link}"
        ));
        network.root_links.push(outside_link.clone());
        ip(&format!(
            "link set {outside_link} netns {}",
            network.outside
        ));
        ip(&format!("link set {uplink_link} netns {}", network.guard));
        ip(&format!(
            "-n {} link set {outside_link} name outside0",
            network.outside
        ));
        ip(&format!(
            "-n {} link set {uplink_link} name uplink0",
            network.guard
        ));
        Self::configure(&network);
        network
    }

    fn configure(network: &Self) {
        ip(&format!(
            "-n {} addr add 172.16.0.2/30 dev guest0",
            network.guest
        ));
        ip(&format!(
            "-n {} addr add 172.16.0.1/30 dev tap0",
            network.guard
        ));
        ip(&format!(
            "-n {} addr add 11.22.33.1/30 dev uplink0",
            network.guard
        ));
        ip(&format!(
            "-n {} addr add 11.22.33.2/30 dev outside0",
            network.outside
        ));
        for address in ["8.8.8.8/32", "10.10.10.10/32"] {
            ip(&format!("-n {} addr add {address} dev lo", network.outside));
        }
        for (name, address, interface) in [
            (&network.guest, "2001:db8:1::2/64", "guest0"),
            (&network.guard, "2001:db8:1::1/64", "tap0"),
            (&network.guard, "2001:db8:2::1/64", "uplink0"),
            (&network.outside, "2001:db8:2::2/64", "outside0"),
        ] {
            ip(&format!(
                "-n {name} -6 addr add {address} dev {interface} nodad"
            ));
        }
        for (name, interface) in [
            (&network.guest, "guest0"),
            (&network.guard, "tap0"),
            (&network.guard, "uplink0"),
            (&network.outside, "outside0"),
        ] {
            ip(&format!("-n {name} link set {interface} up"));
        }
        for address in ["8.8.8.8/32", "10.10.10.10/32"] {
            ip(&format!(
                "-n {} route add {address} via 11.22.33.2",
                network.guard
            ));
        }
        ip(&format!(
            "-n {} route add default via 172.16.0.1",
            network.guest
        ));
        ip(&format!(
            "-n {} route add 172.16.0.0/30 via 11.22.33.1",
            network.outside
        ));
        ip(&format!(
            "-n {} -6 route add default via 2001:db8:1::1",
            network.guest
        ));
        ip(&format!(
            "-n {} -6 route add 2001:db8:1::/64 via 2001:db8:2::1",
            network.outside
        ));
        ip(&format!(
            "netns exec {} sysctl -w net.ipv4.ip_forward=1",
            network.guard
        ));
        ip(&format!(
            "netns exec {} sysctl -w net.ipv6.conf.all.forwarding=1",
            network.guard
        ));
    }

    fn install(&self, policy: &EgressPolicy) {
        use std::io::Write as _;
        let script = policy
            .guest_guard_script(&[Ipv4Addr::new(11, 22, 33, 1)])
            .unwrap();
        let mut nft = Command::new("ip")
            .args(["netns", "exec", &self.guard, "nft", "-f", "-"])
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
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn remove_root_links(&self) {
        for link in &self.root_links {
            let exists = Command::new("ip")
                .args(["link", "show", "dev", link])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            if exists {
                ip(&format!("link delete {link}"));
            }
        }
    }

    fn cleanup(mut self) {
        while let Some(name) = self.namespaces.last().cloned() {
            ip(&format!("netns delete {name}"));
            self.namespaces.pop();
            assert!(!Path::new(&format!("/run/netns/{name}")).exists());
        }
        self.remove_root_links();
    }
}

impl Drop for Topology {
    fn drop(&mut self) {
        for name in self.namespaces.iter().rev() {
            let _ = Command::new("ip").args(["netns", "delete", name]).output();
        }
        self.remove_root_links();
    }
}

fn policy(internet: Option<bool>, allow: &[&str], deny: &[&str]) -> EgressPolicy {
    EgressPolicy::from_sdk_addresses(
        internet,
        &allow
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect::<Vec<_>>(),
        &deny
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
#[ignore = "requires root on an isolated Linux host with iproute2, nft and ping"]
fn guest_guard_blocks_only_the_declared_packet_paths() {
    // Baseline uses identical routes but a separate namespace/conntrack table:
    // no previously established guest flows can bypass the tested rules.
    let baseline = Topology::new();
    for target in ["11.22.33.2", "8.8.8.8", "10.10.10.10", "11.22.33.1"] {
        assert!(
            ping(&baseline.guest, target, false),
            "baseline route to {target}"
        );
    }
    assert!(
        ping(&baseline.guest, "2001:db8:1::1", true),
        "baseline IPv6 TAP link"
    );
    assert!(
        ping(&baseline.guest, "2001:db8:2::2", true),
        "baseline IPv6 route"
    );
    baseline.cleanup();

    for (name, policy, first, second) in [
        ("public default", policy(None, &[], &[]), true, true),
        (
            "false plus exception",
            policy(Some(false), &["11.22.33.2"], &[]),
            true,
            false,
        ),
        ("full deny", policy(Some(false), &[], &[]), false, false),
        (
            "allow over deny all",
            policy(Some(false), &["0.0.0.0/0"], &["0.0.0.0/0"]),
            true,
            true,
        ),
    ] {
        let network = Topology::new();
        network.install(&policy);
        assert_eq!(
            ping(&network.guest, "11.22.33.2", false),
            first,
            "{name}: first public target"
        );
        assert_eq!(
            ping(&network.guest, "8.8.8.8", false),
            second,
            "{name}: second public target"
        );
        assert!(
            !ping(&network.guest, "10.10.10.10", false),
            "{name}: private range"
        );
        assert!(
            !ping(&network.guest, "11.22.33.1", false),
            "{name}: host address"
        );
        assert!(!ping(&network.guest, "2001:db8:2::2", true), "{name}: IPv6");
        assert!(
            ping(&network.guard, "172.16.0.2", false),
            "{name}: management reply"
        );
        network.cleanup();
    }
}
