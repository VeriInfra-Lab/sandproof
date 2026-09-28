"""Real SDK/Firecracker recovery when network or stop receipts are lost.

Run only as root on an isolated Linux/KVM host with the approved debug server.
The outside destination is a test-only namespace, never the public Internet.
"""

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile
import urllib.request

from e2b import Sandbox

from sdk_public_network import Outside, clean_vm, guest_ping, vm_identity
from server_process_crash import (fetch, free_loopback_port, interrupted_request,
                                  process_start_ticks, start_server, stop_server, wait_ready)


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    if Path("/proc/sys/net/ipv4/ip_forward").read_text().strip() != "1":
        raise RuntimeError("test runner must temporarily enable host IPv4 forwarding")
    assert importlib.metadata.version("e2b") == "2.51.0"
    parent = args.scratch_parent.resolve(strict=True)
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient room for base and private VM disk")
    root = Path(tempfile.mkdtemp(prefix="sp-netcrash-", dir=parent))
    namespace = secrets.token_hex(8)
    api_key = secrets.token_hex(32)
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    binary = root / "sandproof-server"
    kernel = root / "vmlinux"
    rootfs = root / "rootfs.ext4"
    shutil.copyfile(args.server, binary)
    shutil.copyfile(args.kernel, kernel)
    shutil.copyfile(args.rootfs, rootfs)
    binary.chmod(0o700)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    port = free_loopback_port()
    config = root / "config.json"
    config.write_text(json.dumps({
        "namespace": namespace,
        "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)),
        "firecracker": str(args.firecracker.resolve(strict=True)),
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
        "max_active_global": 1,
        "max_active_per_tenant": 1,
        "max_disk_bytes_global": disk_bytes * 3,
        "max_disk_bytes_per_tenant": disk_bytes * 3,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "default_timeout_secs": 300,
        "expiry_scan_seconds": 1,
        "public_ipv4_enabled": True,
    }))
    config.chmod(0o600)
    api_url = f"http://127.0.0.1:{port}"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    options = dict(api_url=api_url, sandbox_url=api_url, api_key=api_key,
                   domain="example.invalid", retries=0, request_timeout=45)
    process = None
    completed = False
    try:
        with Outside(namespace[:6]):
            process = start_server(binary, config, root / "server.log", "configure_network")
            wait_ready(opener, api_url, api_key, process)
            interrupted_request(
                lambda: fetch(opener, api_url, "POST", "/v2/sandboxes", api_key, {
                    "templateID": "base", "timeout": 180,
                    "allow_internet_access": False,
                    "network": {"allowOut": ["11.22.33.1"],
                                "denyOut": ["0.0.0.0/0"]},
                }),
                process,
            )
            identity = vm_identity(root, namespace, 0)
            assert process_start_ticks(identity[0]) == identity[1]
            claim = root / "jails/.claims" / f"vm-{namespace}-0"
            assert (claim / "uplink-prepared.json").is_file()

            process = start_server(binary, config, root / "server.log", None)
            wait_ready(opener, api_url, api_key, process)
            status, body = fetch(opener, api_url, "GET", "/v2/sandboxes", api_key)
            items = json.loads(body)
            assert status == 200 and len(items) == 1 and items[0]["state"] == "running", items
            assert vm_identity(root, namespace, 0) == identity, "recovery created a second VM"
            sandbox = Sandbox.connect(items[0]["sandboxID"], **options)
            assert guest_ping(sandbox, "11.22.33.1"), "allowed route lost on recovery"
            assert not guest_ping(sandbox, "11.22.34.1"), "static restriction widened"
            assert not guest_ping(sandbox, "11.22.33.2"), "host became reachable"
            stop_server(process)
            process = start_server(binary, config, root / "server.log", "stop_vm")
            wait_ready(opener, api_url, api_key, process)
            interrupted_request(
                lambda: fetch(opener, api_url, "DELETE",
                              f"/sandboxes/{sandbox.sandbox_id}", api_key),
                process,
            )
            assert process_start_ticks(identity[0]) != identity[1], "exact VM survived StopVm"
            process = start_server(binary, config, root / "server.log", None)
            wait_ready(opener, api_url, api_key, process)
            status, body = fetch(opener, api_url, "GET", "/v2/sandboxes", api_key)
            assert status == 200 and json.loads(body) == [], "stopped VM was republished"
            clean_vm(root, args.firecracker, namespace, 0, identity)
        completed = True
        with rootfs.open("rb") as stream:
            image_hash = hashlib.file_digest(stream, "sha256").hexdigest()
        print(json.dumps({
            "fixed_sdk": "2.51.0", "base_image_sha256": image_hash,
            "network_receipt_lost": "recovered same PID and exact static policy",
            "stop_receipt_lost": "recovered exact stopped VM and uplink cleanup",
            "exact_cleanup": True,
            "scope": "debug-only service exits after real ConfigureNetwork and StopVm; one VM",
        }))
    finally:
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
