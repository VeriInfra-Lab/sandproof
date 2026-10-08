"""Reuse one physical position without exhausting permanent VM identities.

Run as root on the isolated KVM host. Uses three identities in a fresh private
deployment, then admits a fourth after restart and exact cleanup. Failed runs
retain only their exact artifacts for diagnosis. True u64 exhaustion is covered
by controller/API component tests, not a removed deployment ID-range setting.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


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
    assert_idle_reopen, complete_runtime, fetch, free_loopback_port, idle_snapshot,
    process_start_ticks, provision_runtime, start_server, stop_server,
    terminal_cleanup_confirmed, wait_ready,
)
from server_unready_abort import wait_for


def check_rejection(opener, url, key, database, root):
    before = idle_snapshot(database)
    slots = sorted((root / "state/slots").iterdir())
    claims = sorted((root / "jails/.claims").iterdir())
    status, _ = fetch(opener, url, "POST", "/v2/sandboxes", key,
                      {"templateID": "base", "allow_internet_access": False})
    assert status == 429, f"capacity refusal must be 429, got {status}"
    after = idle_snapshot(database)
    assert_idle_reopen(before, after)
    assert after["states"] == before["states"] and after["actions"] == before["actions"], (
        "capacity refusal changed durable state/outbox")
    assert sorted((root / "state/slots").iterdir()) == slots, "refusal added a slot binding"
    assert sorted((root / "jails/.claims").iterdir()) == claims, "refusal claimed a new VM"


def check_no_resources(root, namespace, firecracker, uid_base, identities=()):
    remaining = [json.loads((claim / "record.json").read_text())
                 for claim in (root / "jails/.claims").iterdir() if (claim / "record.json").exists()]
    for record in [*identities, *remaining]:
        assert record["resource_id"].startswith(f"vm-{namespace}-")
        assert process_start_ticks(record["pid"]) != record["start_time_ticks"]
    for parent in (root / "jails" / firecracker.name, Path("/run/netns"), Path("/sys/fs/cgroup")):
        assert not list(parent.rglob(f"vm-{namespace}-*")), f"resource remains under {parent}"
    configured = json.loads((root / "config.json").read_text())
    slots = configured["bounded_host"]["maintenance"]["max_host_slots"]
    assert not processes_in_uid_range(uid_base, uid_base + slots - 1)


def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on the isolated Linux/KVM host")
    assert importlib.metadata.version("e2b") == "2.51.0"
    parent = args.scratch_parent.resolve(strict=True)
    if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
        raise RuntimeError("scratch parent must be root-owned and private")
    if args.uid_base < 1000 or args.uid_base >= 2**32 - 1:
        raise RuntimeError("invalid exclusive test UID range")
    if processes_in_uid_range(args.uid_base, args.uid_base):
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
    names = [f"vm-{namespace}-{index}" for index in range(4)]
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
    settings = {
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)), "firecracker": str(firecracker),
        "chroot_base": str(root / "jails"), "kernel": str(kernel), "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "vcpu_count": 1, "memory_mib": 512,
        "file_policy_digest": 1, "bind": f"127.0.0.1:{port}", "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": key}],
        "max_active_global": 1, "max_active_per_tenant": 1,
        "max_disk_bytes_global": disk_bytes * 4, "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600,
        "default_timeout_secs": 300, "expiry_scan_seconds": 1,
    }
    settings = complete_runtime(settings, slots=1)
    provision_runtime(settings)
    config.write_text(json.dumps(settings))
    config.chmod(0o600)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    url = f"http://127.0.0.1:{port}"
    owner = options(key, url, url)
    process = None
    completed = False
    claims = []

    def record_identity(number):
        record = json.loads((root / "jails/.claims" / names[number] / "record.json").read_text())
        assert record["resource_id"] == names[number]
        assert process_start_ticks(record["pid"]) == record["start_time_ticks"]
        uid = next(line.split()[1] for line in Path(f"/proc/{record['pid']}/status").read_text().splitlines()
                   if line.startswith("Uid:"))
        assert int(uid) == args.uid_base, "permanent VM ID changed the single physical UID"
        claims.append(record)
        return record

    def retired(ident):
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            present = connection.execute("SELECT count(*) FROM sandboxes WHERE sandbox_id=?", (ident,)).fetchone()[0]
        return not present and terminal_cleanup_confirmed(database, ident)

    try:
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        for index in range(2):
            sandbox = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
            assert sandbox.sandbox_id == f"sb-{namespace}-{index + 1}"
            record_identity(index)
            _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **owner))
            assert _kill_confirmed
            wait_for(lambda: retired(sandbox.sandbox_id), 20, "exact terminal retirement before slot reuse")
        survivor = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
        assert survivor.sandbox_id == f"sb-{namespace}-3"
        survivor.files.write("/tmp/capacity.txt", "original-vm")
        live = record_identity(2)
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
        _kill_confirmed = (Sandbox.kill(survivor.sandbox_id, **owner))
        assert _kill_confirmed
        wait_for(lambda: retired(survivor.sandbox_id), 20, "exact survivor retirement")
        fourth = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
        assert fourth.sandbox_id == f"sb-{namespace}-4", "permanent identity reset or remained exhausted"
        record_identity(3)
        assert fourth.commands.run("printf slot-reused", user="ubuntu").stdout == "slot-reused"
        _kill_confirmed = Sandbox.kill(fourth.sandbox_id, **owner)
        assert _kill_confirmed
        wait_for(lambda: retired(fourth.sandbox_id), 20, "exact fourth retirement")
        with sqlite3.connect(database) as connection:
            phases = [json.loads(row[0])["phase"] for row in connection.execute("SELECT state_blob FROM sandboxes")]
        assert phases == [], phases
        assert len(claims) == 4
        check_no_resources(root, namespace, firecracker, args.uid_base, claims)
        assert sha256_file(rootfs) == sha256_file(args.rootfs) == base_digest
        completed = True
        print(json.dumps({
            "source_commit": commit, "server_sha256": sha256_file(binary),
            "probe_sha256": sha256_file(Path(__file__)), "base_sha256": base_digest,
            "host_kernel": os.uname().release, "sdk": "2.51.0", "namespace": namespace,
            "physical_slots": 1, "permanent_sandbox_ids": [1, 2, 3, 4], "vms_destroyed": 4,
            "result": "one physical UID reused; live capacity refused before/after restart without allocating identities; original SDK command/file access preserved; fourth admitted after cleanup",
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
    exercise(parser.parse_args())
