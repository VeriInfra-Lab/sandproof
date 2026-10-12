"""Real SDK/Firecracker recovery when network or stop receipts are lost.

Run only as root on an isolated Linux/KVM host with the approved debug server.
The outside destination is a test-only namespace, never the public Internet.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


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
from server_process_crash import (fetch, free_loopback_port, interrupted_host_request,
                                  original_create_identity, prepare_fault_services, process_start_ticks)
from _runtime_services import add_arguments


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
        "jailer": str(args.jailer.resolve(strict=True)),
        "firecracker": str(args.firecracker.resolve(strict=True)),
        "chroot_base": str(root / "jails"),
        "kernel": str(kernel),
        "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "vcpu_count": 1,
        "memory_mib": 256,
        "file_policy_digest": 1,
        "max_active_global": 1,
        "max_active_per_tenant": 1,
        "max_disk_bytes_global": disk_bytes * 3,
        "max_disk_bytes_per_tenant": disk_bytes * 3,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "default_timeout_secs": 300,
        "expiry_scan_seconds": 1,
        "public_ipv4_enabled": True,
    }
    services = prepare_fault_services(args, root, settings, slots=1, key=api_key, port=port)
    api_url = f"http://127.0.0.1:{port}"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    options = dict(api_url=api_url, sandbox_url=api_url, api_key=api_key,
                   domain="example.invalid", retries=0, request_timeout=45)
    process = None
    metadata = {"network_receipt_intent": secrets.token_hex(16)}
    completed = False
    try:
        with Outside(namespace[:6]):
            services.start(host_envs={"host-0": {"SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION": "configure_network"}})
            services.wait_ready(for_admission=False)
            process = services.processes.children["host-0"]
            interrupted_host_request(
                lambda: fetch(opener, api_url, "POST", "/v2/sandboxes", api_key, {
                    "templateID": "base", "timeout": 180,
                    "allow_internet_access": False,
                    "network": {"allowOut": ["11.22.33.1"],
                                "denyOut": ["0.0.0.0/0"]},
                    "metadata": metadata,
                }),
                process,
            )
            identity = vm_identity(root, namespace, 0)
            assert process_start_ticks(identity[0]) == identity[1]
            claim = root / "jails/.claims" / f"vm-{namespace}-0"
            assert (claim / "uplink-prepared.json").is_file()
            create_identity = original_create_identity(services, metadata)
            native_id = services.native_id_for(create_identity["sandbox_id"])
            assert native_id == f"sb-{namespace}-1"

            process = services.processes.start("host-0")
            services.wait_ready(for_admission=False)
            status, body = fetch(opener, api_url, "GET", "/v2/sandboxes", api_key)
            items = json.loads(body)
            assert status == 200 and len(items) == 1 and items[0]["state"] == "running", items
            assert items[0]["sandboxID"] == create_identity["sandbox_id"]
            assert vm_identity(root, namespace, 0) == identity, "recovery created a second VM"
            sandbox = Sandbox.connect(items[0]["sandboxID"], **options)
            assert guest_ping(sandbox, "11.22.33.1"), "allowed route lost on recovery"
            assert not guest_ping(sandbox, "11.22.34.1"), "static restriction widened"
            assert not guest_ping(sandbox, "11.22.33.2"), "host became reachable"
            services.processes.stop("host-0")
            process = services.processes.start("host-0", env={"SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION": "stop_vm"})
            services.wait_ready(for_admission=False)
            interrupted_host_request(
                lambda: fetch(opener, api_url, "DELETE",
                              f"/sandboxes/{sandbox.sandbox_id}", api_key),
                process,
            )
            assert process_start_ticks(identity[0]) != identity[1], "exact VM survived StopVm"
            process = services.processes.start("host-0")
            services.wait_ready(for_admission=False)
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
        services.stop_all()
        if completed:
            services.cleanup_staging()
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    for name in ("jailer", "firecracker", "kernel", "rootfs", "scratch-parent"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    exercise(parser.parse_args())
