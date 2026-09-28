"""Fixed E2B SDK + real Firecracker public/static IPv4 smoke on isolated Linux.

The outside peer is a test-only network namespace. No packet is sent to the
Internet. The caller must temporarily enable and later restore host IPv4
forwarding; this script refuses to change the global setting itself.
"""

import argparse
import hashlib
import importlib.metadata
import ipaddress
import json
import os
from pathlib import Path
import select
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile

from e2b import Sandbox

from server_process_crash import (free_loopback_port, process_start_ticks,
                                  start_server, stop_server, wait_ready)
import urllib.request


OUTSIDE_ECHO = r'''
import selectors
import socket

selector = selectors.DefaultSelector()
for address in ("11.22.33.1", "11.22.34.1"):
    for port in (53, 443, 853):
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.bind((address, port))
        listener.listen()
        listener.setblocking(False)
        selector.register(listener, selectors.EVENT_READ, "tcp")
        datagram = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        datagram.bind((address, port))
        datagram.setblocking(False)
        selector.register(datagram, selectors.EVENT_READ, "udp")
print("READY", flush=True)
while True:
    for key, _events in selector.select():
        if key.data == "tcp":
            connection, _peer = key.fileobj.accept()
            connection.settimeout(2)
            try:
                payload = connection.recv(256)
                if payload:
                    connection.sendall(payload)
            finally:
                connection.close()
        else:
            payload, peer = key.fileobj.recvfrom(256)
            key.fileobj.sendto(payload, peer)
'''


def ip(*arguments: str) -> None:
    subprocess.run(["/usr/bin/ip", *arguments], check=True, timeout=10)


def exists(*arguments: str) -> bool:
    return subprocess.run(arguments, stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL, timeout=10).returncode == 0


class Outside:
    def __init__(self, tag: str):
        self.namespace = f"spo{tag}"
        self.root_link = f"spe{tag}"
        self.peer = f"spx{tag}"
        self.echo = None

    def __enter__(self):
        ip("netns", "add", self.namespace)
        try:
            ip("link", "add", self.root_link, "type", "veth", "peer", "name", self.peer)
            ip("link", "set", self.peer, "netns", self.namespace)
            ip("netns", "exec", self.namespace, "/usr/bin/ip", "link", "set", self.peer,
               "name", "outside0")
            ip("addr", "add", "11.22.33.2/30", "dev", self.root_link)
            ip("link", "set", self.root_link, "up")
            ip("netns", "exec", self.namespace, "/usr/bin/ip", "addr", "add",
               "11.22.33.1/30", "dev", "outside0")
            ip("netns", "exec", self.namespace, "/usr/bin/ip", "link", "set", "outside0", "up")
            ip("netns", "exec", self.namespace, "/usr/bin/ip", "link", "set", "lo", "up")
            ip("netns", "exec", self.namespace, "/usr/bin/ip", "addr", "add",
               "11.22.34.1/32", "dev", "lo")
            ip("route", "add", "11.22.34.1/32", "via", "11.22.33.1", "dev", self.root_link)
            assert exists("/usr/bin/ping", "-n", "-c", "1", "-W", "1", "11.22.33.1")
            assert exists("/usr/bin/ping", "-n", "-c", "1", "-W", "1", "11.22.34.1")
            self.echo = subprocess.Popen(
                ["/usr/bin/ip", "netns", "exec", self.namespace, sys.executable,
                 "-u", "-c", OUTSIDE_ECHO], stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True,
            )
            ready, _, _ = select.select([self.echo.stdout], [], [], 5)
            if not ready or self.echo.stdout.readline().strip() != "READY":
                raise RuntimeError("test-only outside echo server did not start")
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, _kind, _error, _traceback):
        if self.echo is not None:
            self.echo.terminate()
            try:
                self.echo.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                self.echo.kill()
                self.echo.communicate(timeout=5)
        if exists("/usr/bin/ip", "link", "show", "dev", self.root_link):
            ip("link", "delete", self.root_link)
        if Path(f"/run/netns/{self.namespace}").exists():
            ip("netns", "delete", self.namespace)


def guest_ping(sandbox: Sandbox, address: str) -> bool:
    result = sandbox.commands.run(
        f"ping -n -c 1 -W 1 {address} >/dev/null 2>&1; printf '%s' \"$?\""
    )
    return result.stdout == "0"


def guest_echo(sandbox: Sandbox, address: str, port: int, protocol: str) -> bool:
    """A returned nonce proves both directions of one real packet path."""
    probe = f'''python3 - <<'PY'
import socket
address = {address!r}
port = {port}
payload = b'sandproof-network-probe'
try:
    with socket.socket(socket.AF_INET, socket.SOCK_{'STREAM' if protocol == 'tcp' else 'DGRAM'}) as connection:
        connection.settimeout(1)
        if {protocol!r} == 'tcp':
            connection.connect((address, port))
            connection.sendall(payload)
            answer = connection.recv(len(payload))
        else:
            connection.sendto(payload, (address, port))
            answer, _peer = connection.recvfrom(len(payload))
    print('allowed' if answer == payload else 'wrong-response')
except (OSError, TimeoutError):
    print('denied')
PY'''
    result = sandbox.commands.run(probe)
    assert result.stdout.strip() in ("allowed", "denied"), result.stdout
    return result.stdout.strip() == "allowed"


def guest_tcp_connect(sandbox: Sandbox, address: str, port: int) -> bool:
    """A completed TCP handshake tests an optional real public route."""
    probe = f'''python3 - <<'PY'
import socket
try:
    with socket.create_connection(({address!r}, {port}), timeout=3):
        print('connected')
except OSError:
    print('denied')
PY'''
    result = sandbox.commands.run(probe)
    assert result.stdout.strip() in ("connected", "denied"), result.stdout
    return result.stdout.strip() == "connected"


def probe_ports(sandbox: Sandbox, address: str, allowed: bool) -> None:
    for port in (53, 853, 443):
        for protocol in ("udp", "tcp"):
            assert guest_echo(sandbox, address, port, protocol) == allowed, (
                f"unexpected {protocol}/{port} reachability for {address}"
            )


def probe_guest_ipv6(sandbox: Sandbox, namespace: str, number: int) -> None:
    """Guest root may add an IPv6 address, but not bypass its TAP guard."""
    vm = f"vm-{namespace}-{number}"
    gateway = "2001:db8:10::1"
    guest_address = "2001:db8:10::2"
    ip("netns", "exec", vm, "/usr/bin/ip", "-6", "addr", "add",
       f"{gateway}/64", "dev", "tap0")
    guest_configured = False
    try:
        sandbox.commands.run(f"ip -6 addr add {guest_address}/64 dev eth0")
        guest_configured = True
        route = sandbox.commands.run(f"ip -6 route get {gateway}").stdout
        assert "eth0" in route and gateway in route, route
        result = sandbox.commands.run(
            f"ping -6 -n -c 1 -W 1 {gateway} >/dev/null 2>&1; printf '%s' \"$?\""
        )
        assert result.stdout != "0", "guest root reached IPv6 host namespace"
    finally:
        try:
            if guest_configured:
                sandbox.commands.run(f"ip -6 addr del {guest_address}/64 dev eth0")
        finally:
            ip("netns", "exec", vm, "/usr/bin/ip", "-6", "addr", "del",
               f"{gateway}/64", "dev", "tap0")


def clean_vm(root: Path, firecracker: Path, namespace: str,
             number: int, identity: tuple[int, int]) -> None:
    vm = f"vm-{namespace}-{number}"
    iface = f"sp{namespace[:6]}{number:06x}"
    assert process_start_ticks(identity[0]) != identity[1], f"VM process survived: {vm}"
    assert not (root / "jails" / firecracker.resolve(strict=True).name / vm).exists(), vm
    for path in [f"/run/netns/{vm}"]:
        assert not Path(path).exists(), f"network namespace survived: {path}"
    assert not exists("/usr/bin/ip", "link", "show", "dev", iface), iface
    for family, prefix in [("inet", "spf"), ("ip", "spn")]:
        table = f"{prefix}{namespace}{number:06x}"
        assert not exists("/usr/sbin/nft", "list", "table", family, table), table


def vm_identity(root: Path, namespace: str, number: int) -> tuple[int, int]:
    vm = f"vm-{namespace}-{number}"
    record = json.loads((root / "jails/.claims" / vm / "record.json").read_text())
    assert record["resource_id"] == vm
    return record["pid"], record["start_time_ticks"]


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    if Path("/proc/sys/net/ipv4/ip_forward").read_text().strip() != "1":
        raise RuntimeError("host IPv4 forwarding must be temporarily enabled by the test runner")
    if args.public_probe_ip is not None:
        address = ipaddress.IPv4Address(args.public_probe_ip)
        if not address.is_global or not 1 <= args.public_probe_port <= 65535:
            raise ValueError("public probe requires a global IPv4 and a valid TCP port")
        with socket.create_connection((str(address), args.public_probe_port), timeout=3):
            pass  # Do not judge a guest by a public endpoint the host cannot reach.
    assert importlib.metadata.version("e2b") == "2.51.0"
    parent = Path(args.scratch_parent).resolve(strict=True)
    disk_bytes = Path(args.rootfs).stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 4 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient room for template and private VM disks")
    root = Path(tempfile.mkdtemp(prefix="sp-ipv4-", dir=parent))
    namespace = secrets.token_hex(8)
    api_key = secrets.token_hex(32)
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    binary = root / "sandproof-server"
    shutil.copyfile(args.server, binary)
    binary.chmod(0o700)
    kernel = root / "vmlinux"
    rootfs = root / "rootfs.ext4"
    shutil.copyfile(args.kernel, kernel)
    shutil.copyfile(args.rootfs, rootfs)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    port = free_loopback_port()
    settings = {
        "namespace": namespace,
        "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"),
        "jailer": str(Path(args.jailer).resolve(strict=True)),
        "firecracker": str(Path(args.firecracker).resolve(strict=True)),
        "chroot_base": str(root / "jails"),
        "kernel": str(kernel),
        "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "max_vm_id": 1000,
        "vcpu_count": 1,
        "memory_mib": 256,
        "file_policy_digest": 1,
        "bind": f"127.0.0.1:{port}",
        "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": api_key}],
        "max_active_global": 2,
        "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 4,
        "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "default_timeout_secs": 300,
        "expiry_scan_seconds": 1,
        "public_ipv4_enabled": True,
    }
    config = root / "config.json"
    config.write_text(json.dumps(settings))
    config.chmod(0o600)
    api_url = f"http://127.0.0.1:{port}"
    options = dict(api_url=api_url, sandbox_url=api_url, api_key=api_key,
                   domain="example.invalid", retries=0, request_timeout=45)
    process = None
    completed = False
    sandboxes = []
    try:
        with Outside(namespace[:6]):
            process = start_server(binary, config, root / "server.log", None)
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            wait_ready(opener, api_url, api_key, process)
            static = Sandbox.create(template="base", timeout=180,
                                    allow_internet_access=False,
                                    network={"allow_out": ["11.22.33.1"],
                                             "deny_out": ["0.0.0.0/0"]}, **options)
            sandboxes.append(static)
            assert "via 172.16.0.1" in static.commands.run("ip -4 route get 11.22.33.1").stdout
            assert guest_ping(static, "11.22.33.1"), "static allow failed"
            assert not guest_ping(static, "11.22.34.1"), "unlisted address escaped"
            assert not guest_ping(static, "11.22.33.2"), "host-local address escaped"
            probe_ports(static, "11.22.33.1", True)
            probe_ports(static, "11.22.34.1", False)
            if args.public_probe_ip is not None:
                assert not guest_tcp_connect(static, args.public_probe_ip,
                                             args.public_probe_port), "static mode reached Internet"
            # The guest controls resolv.conf, but this cannot create a host
            # firewall exception for an otherwise denied resolver address.
            static.commands.run("printf 'nameserver 11.22.34.1\\n' > /etc/resolv.conf")
            probe_ports(static, "11.22.34.1", False)
            probe_guest_ipv6(static, namespace, 0)
            public = Sandbox.create(template="base", timeout=180, **options)
            sandboxes.append(public)
            assert guest_ping(public, "11.22.33.1"), "public default egress failed"
            assert not guest_ping(public, "11.22.33.2"), "public mode reached host-local address"
            probe_ports(public, "11.22.34.1", True)
            if args.public_probe_ip is not None:
                assert guest_tcp_connect(public, args.public_probe_ip,
                                         args.public_probe_port), "public Internet path failed"
            probe_guest_ipv6(public, namespace, 1)
            # This address did not exist when either policy was compiled. The
            # root INPUT/FORWARD guard must still block a newly local address.
            dynamic_host = "11.22.35.7"
            local_routes = subprocess.check_output(
                ["/usr/bin/ip", "-j", "route", "show", "table", "local"], text=True)
            assert dynamic_host not in local_routes, "test host address already in use"
            ip("addr", "add", f"{dynamic_host}/32", "dev", "lo")
            try:
                assert not guest_ping(public, dynamic_host), "dynamic host address escaped"
                assert not guest_ping(static, dynamic_host), "static mode reached dynamic host"
            finally:
                ip("addr", "del", f"{dynamic_host}/32", "dev", "lo")
            assert exists("/usr/bin/ip", "route", "get", "100.64.0.2")
            assert exists("/usr/bin/ip", "route", "get", "100.64.0.6")
            assert not guest_ping(static, "100.64.0.6"), "static mode reached another VM link"
            assert not guest_ping(public, "100.64.0.2"), "public mode reached another VM link"
            identities = [vm_identity(root, namespace, number) for number in (0, 1)]
            stop_server(process)
            process = start_server(binary, config, root / "server.log", None)
            wait_ready(opener, api_url, api_key, process)
            assert [vm_identity(root, namespace, number) for number in (0, 1)] == identities
            assert all(process_start_ticks(pid) == ticks for pid, ticks in identities)
            assert guest_ping(static, "11.22.33.1"), "static policy lost after restart"
            assert not guest_ping(static, "11.22.34.1"), "static policy widened after restart"
            assert guest_ping(public, "11.22.33.1"), "public policy lost after restart"
            assert not guest_ping(public, "11.22.33.2"), "host isolation lost after restart"
            assert Sandbox.kill(static.sandbox_id, **options)
            sandboxes.remove(static)
            clean_vm(root, args.firecracker, namespace, 0, identities[0])
            assert guest_ping(public, "11.22.33.1"), "first VM cleanup broke the second VM"
            closed = Sandbox.create(template="base", timeout=180,
                                    allow_internet_access=False, **options)
            sandboxes.append(closed)
            closed_identity = vm_identity(root, namespace, 2)
            assert not guest_ping(closed, "11.22.33.1"), "closed mode escaped"
            probe_ports(closed, "11.22.33.1", False)
            if args.public_probe_ip is not None:
                assert not guest_tcp_connect(closed, args.public_probe_ip,
                                             args.public_probe_port), "closed mode reached Internet"
            closed.commands.run("printf 'nameserver 11.22.33.1\\n' > /etc/resolv.conf")
            probe_ports(closed, "11.22.33.1", False)
            probe_guest_ipv6(closed, namespace, 2)
            assert Sandbox.kill(closed.sandbox_id, **options)
            sandboxes.remove(closed)
            clean_vm(root, args.firecracker, namespace, 2, closed_identity)
            assert guest_ping(public, "11.22.33.1"), "closed VM cleanup broke public VM"
            explicit = Sandbox.create(template="base", timeout=180,
                                      allow_internet_access=True,
                                      network={"deny_out": ["11.22.34.1"]}, **options)
            sandboxes.append(explicit)
            explicit_identity = vm_identity(root, namespace, 3)
            probe_ports(explicit, "11.22.33.1", True)
            probe_ports(explicit, "11.22.34.1", False)
            if args.public_probe_ip is not None:
                assert guest_tcp_connect(explicit, args.public_probe_ip,
                                         args.public_probe_port), "explicit public Internet path failed"
            assert not guest_ping(explicit, "11.22.33.2"), "explicit public reached host"
            assert Sandbox.kill(explicit.sandbox_id, **options)
            sandboxes.remove(explicit)
            clean_vm(root, args.firecracker, namespace, 3, explicit_identity)
            assert Sandbox.kill(public.sandbox_id, **options)
            sandboxes.clear()
            clean_vm(root, args.firecracker, namespace, 1, identities[1])
        completed = True
        with rootfs.open("rb") as stream:
            image_hash = hashlib.file_digest(stream, "sha256").hexdigest()
        print(json.dumps({
            "fixed_sdk": "2.51.0", "base_image_sha256": image_hash,
            "static_allow_and_deny": True, "public_default": True,
            "public_explicit_with_deny": True,
            "host_local_denied": True, "cross_vm_denied": True,
            "udp_tcp_dns_port_matrix": True, "guest_resolver_rewrite_denied": True,
            "new_host_address_denied": True, "closed_default_denied": True,
            "guest_root_ipv6_route_denied": True,
            "real_public_ipv4_tcp": args.public_probe_ip is not None,
            "restart_keeps_exact_vms_and_policies": True,
            "independent_cleanup": True, "exact_cleanup": True,
            "scope": "real Firecracker and SDK, isolated outside namespace"
                     + (" plus one optional public TCP handshake" if args.public_probe_ip else "")
                     + "; no DNS query-content claim",
        }))
    finally:
        for sandbox in sandboxes:
            try:
                Sandbox.kill(sandbox.sandbox_id, **options)
            except Exception:
                pass
        stop_server(process)
        if completed:
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    parser.add_argument("--public-probe-ip", help="optional preflighted real public IPv4 target")
    parser.add_argument("--public-probe-port", type=int, default=443)
    exercise(parser.parse_args())
