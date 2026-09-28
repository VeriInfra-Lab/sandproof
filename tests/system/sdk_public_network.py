"""Fixed E2B SDK + real Firecracker public/static IPv4 smoke on isolated Linux.

The outside peer is a test-only network namespace. No packet is sent to the
Internet. The caller must temporarily enable and later restore host IPv4
forwarding; this script refuses to change the global setting itself.
"""

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile

from e2b import Sandbox

from server_process_crash import (free_loopback_port, process_start_ticks,
                                  start_server, stop_server, wait_ready)
import urllib.request


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
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, _kind, _error, _traceback):
        if exists("/usr/bin/ip", "link", "show", "dev", self.root_link):
            ip("link", "delete", self.root_link)
        if Path(f"/run/netns/{self.namespace}").exists():
            ip("netns", "delete", self.namespace)


def guest_ping(sandbox: Sandbox, address: str) -> bool:
    result = sandbox.commands.run(
        f"ping -n -c 1 -W 1 {address} >/dev/null 2>&1; printf '%s' \"$?\""
    )
    return result.stdout == "0"


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
        "default_timeout_secs": 180,
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
            public = Sandbox.create(template="base", timeout=180, **options)
            sandboxes.append(public)
            assert guest_ping(public, "11.22.33.1"), "public default egress failed"
            assert not guest_ping(public, "11.22.33.2"), "public mode reached host-local address"
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
            assert Sandbox.kill(public.sandbox_id, **options)
            sandboxes.clear()
            clean_vm(root, args.firecracker, namespace, 1, identities[1])
        completed = True
        with rootfs.open("rb") as stream:
            image_hash = hashlib.file_digest(stream, "sha256").hexdigest()
        print(json.dumps({
            "fixed_sdk": "2.51.0", "base_image_sha256": image_hash,
            "static_allow_and_deny": True, "public_default": True,
            "host_local_denied": True, "cross_vm_denied": True,
            "restart_keeps_exact_vms_and_policies": True,
            "independent_cleanup": True, "exact_cleanup": True,
            "scope": "real Firecracker and SDK, test-only outside namespace; no real Internet or DNS claim",
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
    exercise(parser.parse_args())
