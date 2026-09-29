"""Exhaust VM identities while a real SDK sandbox remains usable across restart.

Run as root on the isolated KVM host. Uses three identities in a fresh private
deployment; failed runs retain only their exact artifacts for diagnosis.
"""

import argparse
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request

from e2b import Sandbox

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import (
    fetch, free_loopback_port, process_start_ticks, start_server, stop_server, wait_ready,
)


MAX_VM_ID = 2
LEGACY_SERVER_SHA256 = "6c2c5ab4827047a9e1e080aea0717d627266c2fb02beedff7d860a723eb5c10b"
LEGACY_SOURCE_COMMIT = "4c485237d4c94a65abeb34c9676b3e683b353871"


def durable_snapshot(database):
    with sqlite3.connect(database) as connection:
        return tuple(connection.iterdump())


def check_rejection(opener, url, key, database, root):
    before = durable_snapshot(database)
    slots = sorted((root / "state/slots").iterdir())
    claims = sorted((root / "jails/.claims").iterdir())
    status, _ = fetch(opener, url, "POST", "/v2/sandboxes", key,
                      {"templateID": "base", "allow_internet_access": False})
    assert status == 429, f"capacity refusal must be 429, got {status}"
    assert durable_snapshot(database) == before, "refusal changed durable state/outbox"
    assert sorted((root / "state/slots").iterdir()) == slots, "refusal added a slot binding"
    assert sorted((root / "jails/.claims").iterdir()) == claims, "refusal claimed a new VM"


def check_no_resources(root, namespace, firecracker, uid_base):
    for claim in (root / "jails/.claims").iterdir():
        record = json.loads((claim / "record.json").read_text())
        assert record["resource_id"] == claim.name
        assert process_start_ticks(record["pid"]) != record["start_time_ticks"]
    for parent in (root / "jails" / firecracker.name, Path("/run/netns"), Path("/sys/fs/cgroup")):
        assert not list(parent.rglob(f"vm-{namespace}-*")), f"resource remains under {parent}"
    assert not processes_in_uid_range(uid_base, uid_base + MAX_VM_ID)


def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on the isolated Linux/KVM host")
    assert importlib.metadata.version("e2b") == "2.51.0"
    if args.reproduce_legacy and sha256_file(args.server) != LEGACY_SERVER_SHA256:
        raise RuntimeError("legacy reproduction requires the recorded pre-fix binary")
    parent = args.scratch_parent.resolve(strict=True)
    if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
        raise RuntimeError("scratch parent must be root-owned and private")
    if args.uid_base < 1000 or args.uid_base + MAX_VM_ID >= 2**32 - 1:
        raise RuntimeError("invalid exclusive test UID range")
    if processes_in_uid_range(args.uid_base, args.uid_base + MAX_VM_ID):
        raise RuntimeError("test UID range is occupied")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("test checkout must be clean")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient private disk and cleanup space")
    base_digest = sha256_file(args.rootfs)
    root = Path(tempfile.mkdtemp(prefix="vi-", dir=parent))
    namespace = secrets.token_hex(8)
    names = [f"vm-{namespace}-{index}" for index in range(MAX_VM_ID + 1)]
    firecracker = args.firecracker.resolve(strict=True)
    if len(os.fsencode(root / "jails" / firecracker.name / names[-1] / "root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("test path too long for Firecracker socket")
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    binary, kernel, rootfs = (root / name for name in ("sandproof-server", "vmlinux", "rootfs.ext4"))
    for source, destination in ((args.server, binary), (args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source.resolve(strict=True), destination)
    binary.chmod(0o700)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    assert sha256_file(rootfs) == base_digest
    key = secrets.token_hex(32)
    port = free_loopback_port()
    database = root / "db/controller.db"
    config = root / "config.json"
    config.write_text(json.dumps({
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)), "firecracker": str(firecracker),
        "chroot_base": str(root / "jails"), "kernel": str(kernel), "rootfs": str(rootfs),
        "uid_base": args.uid_base, "max_vm_id": 1 if args.reproduce_legacy else MAX_VM_ID,
        "vcpu_count": 1, "memory_mib": 512,
        "file_policy_digest": 1, "bind": f"127.0.0.1:{port}", "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": key}],
        "max_active_global": 2, "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 4, "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600,
        "default_timeout_secs": 300, "expiry_scan_seconds": 3600,
    }))
    config.chmod(0o600)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    url = f"http://127.0.0.1:{port}"
    owner = options(key, url, url)
    process = None
    completed = False
    try:
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        for _ in range(2):
            sandbox = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
            assert Sandbox.kill(sandbox.sandbox_id, **owner)
        if args.reproduce_legacy:
            status, _ = fetch(opener, url, "POST", "/v2/sandboxes", key,
                              {"templateID": "base", "allow_internet_access": False})
            assert status == 500, f"expected legacy unknown create, got {status}"
            with sqlite3.connect(database) as connection:
                phases = [json.loads(row[0])["phase"] for row in connection.execute(
                    "SELECT state_blob FROM sandboxes ORDER BY sandbox_id")]
            assert phases == ["Destroyed", "Destroyed", "Starting"], phases
            assert (root / "state/slots" / f"slot-{namespace}-2.json").exists()
            assert not (root / "jails/.claims" / names[2]).exists()
            stop_server(process)
            process = start_server(binary, config, root / "server.log", None)
            assert process.wait(timeout=10) != 0, "legacy startup unexpectedly accepted invalid binding"
            assert "slot filename or VM identity mismatch" in (root / "server.log").read_text()
            check_no_resources(root, namespace, firecracker, args.uid_base)
            assert sha256_file(rootfs) == sha256_file(args.rootfs) == base_digest
            completed = True
            print(json.dumps({
                "probe_commit": commit, "legacy_source_commit": LEGACY_SOURCE_COMMIT,
                "server_sha256": LEGACY_SERVER_SHA256, "probe_sha256": sha256_file(Path(__file__)),
                "base_sha256": base_digest, "namespace": namespace,
                "result": "legacy failure reproduced: third create became unknown with out-of-range slot; same-config startup refused",
                "residual": "both real VMs already destroyed; rejected third had no VMM claim; no exact process/jail/netns/cgroup/UID remains",
            }))
            return
        survivor = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
        survivor.files.write("/tmp/capacity.txt", "original-vm")
        claims = [json.loads((root / "jails/.claims" / name / "record.json").read_text()) for name in names]
        live = claims[-1]
        assert process_start_ticks(live["pid"]) == live["start_time_ticks"]
        check_rejection(opener, url, key, database, root)
        assert survivor.commands.run("printf capacity-ok", user="ubuntu").stdout == "capacity-ok"
        assert survivor.files.read("/tmp/capacity.txt") == "original-vm"

        stop_server(process)
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        check_rejection(opener, url, key, database, root)
        assert process_start_ticks(live["pid"]) == live["start_time_ticks"], "VM replaced during restart"
        assert survivor.get_info().state == "running"
        assert survivor.commands.run("printf restart-ok", user="ubuntu").stdout == "restart-ok"
        assert survivor.files.read("/tmp/capacity.txt") == "original-vm"
        assert Sandbox.kill(survivor.sandbox_id, **owner)
        check_rejection(opener, url, key, database, root)
        with sqlite3.connect(database) as connection:
            phases = [json.loads(row[0])["phase"] for row in connection.execute("SELECT state_blob FROM sandboxes")]
        assert phases == ["Destroyed"] * 3, phases
        assert len(claims) == 3
        check_no_resources(root, namespace, firecracker, args.uid_base)
        assert sha256_file(rootfs) == sha256_file(args.rootfs) == base_digest
        completed = True
        print(json.dumps({
            "source_commit": commit, "server_sha256": sha256_file(binary),
            "probe_sha256": sha256_file(Path(__file__)), "base_sha256": base_digest,
            "host_kernel": os.uname().release, "sdk": "2.51.0", "namespace": namespace,
            "max_vm_id": MAX_VM_ID, "vms_destroyed": 3,
            "result": "capacity refusal before and after same-config restart changed no durable/host records; original VM kept SDK command/file access and was destroyed",
            "residual": "no exact VMM process, jail, netns, cgroup or test UID process",
        }))
    finally:
        stop_server(process)
        if completed:
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent"):
        parser.add_argument(f"--{argument}", type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--reproduce-legacy", action="store_true",
                        help="reproduce the recorded pre-fix failure; not a passing implementation acceptance")
    exercise(parser.parse_args())
