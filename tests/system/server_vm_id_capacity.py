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
from tools.capture_acceptance import original_chroot
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from _complete_runtime import complete_runtime, terminal_cleanup_confirmed
from _runtime_services import RuntimeServices, add_arguments
from server_process_crash import (
    assert_idle_reopen, fetch, free_loopback_port, idle_snapshot, process_start_ticks,
)
from server_unready_abort import wait_for


def check_rejection(opener, url, key, database, root):
    before = idle_snapshot(database)
    slots = sorted((root / "state/slots").iterdir())
    claims = sorted((original_chroot(root) / ".claims").iterdir())
    status, _ = fetch(opener, url, "POST", "/v2/sandboxes", key,
                      {"templateID": "base", "allow_internet_access": False})
    assert status == 429, f"capacity refusal must be 429, got {status}"
    after = idle_snapshot(database)
    assert_idle_reopen(before, after)
    assert after["states"] == before["states"] and after["actions"] == before["actions"], (
        "capacity refusal changed durable state/outbox")
    assert sorted((root / "state/slots").iterdir()) == slots, "refusal added a slot binding"
    assert sorted((original_chroot(root) / ".claims").iterdir()) == claims, "refusal claimed a new VM"


def check_no_resources(root, namespace, firecracker, uid_base, identities=(), *, native):
    """Audit the original physical pool using its explicit current native config.

    A public/global sandbox ID is deliberately not used to infer this pool's
    slot budget. Callers must supply the native configuration associated with
    the exact host audit mapping; there is no old flat configuration reader.
    """
    if (not isinstance(native, dict) or native.get("namespace") != namespace
            or native.get("uid_base") != uid_base):
        raise ValueError("resource audit requires the exact native pool configuration")
    slots = native["bounded_host"]["maintenance"]["max_host_slots"]
    if type(slots) is not int or slots < 1 or uid_base + slots > 2**32:
        raise ValueError("invalid native physical UID range")
    chroot=Path(native['chroot_base'])
    remaining = [json.loads((claim / "record.json").read_text())
                 for claim in (chroot / '.claims').iterdir() if (claim / "record.json").exists()]
    for record in [*identities, *remaining]:
        assert record["resource_id"].startswith(f"vm-{namespace}-")
        assert process_start_ticks(record["pid"]) != record["start_time_ticks"]
    for parent in (chroot / firecracker.name, Path("/run/netns"), Path("/sys/fs/cgroup")):
        assert not list(parent.rglob(f"vm-{namespace}-*")), f"resource remains under {parent}"
    assert not processes_in_uid_range(uid_base, uid_base + slots - 1)


def exercise(args):
    from e2b import Sandbox
    from sdk_server_smoke import options
    from server_templates import descriptor
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
    if len(os.fsencode(root / "p/v" / firecracker.name / names[-1] / "root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("test path too long for Firecracker socket")
    for name in ("db", "state", "p"):
        (root / name).mkdir(mode=0o700)
    kernel, rootfs = (root / name for name in ("vmlinux", "rootfs.ext4"))
    for source, destination in ((args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source.resolve(strict=True), destination)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    assert sha256_file(rootfs) == base_digest
    key = secrets.token_hex(32)
    port = free_loopback_port()
    database = root / "db/controller.db"
    template = descriptor("runtime-base", kernel, rootfs, 1, 512)
    catalog = {"templates": [template], "aliases": {"base": template["template_id"]},
        "compute_limits": {"max_vcpus_global": 1, "max_vcpus_per_tenant": 1,
            "max_memory_bytes_global": 1024 * 1024**2,
            "max_memory_bytes_per_tenant": 1024 * 1024**2}}
    settings = {
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)), "firecracker": str(firecracker),
        "chroot_base": str(root / "p/v"), "kernel": str(kernel), "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "vcpu_count": 1, "memory_mib": 512,
        "file_policy_digest": 1, "templates": catalog,
        "max_active_global": 1, "max_active_per_tenant": 1,
        "max_disk_bytes_global": disk_bytes * 4, "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600, "max_lifetime_secs": 600,
        "startup_timeout_secs": 300, "public_ipv4_enabled": False,
        "default_timeout_secs": 300, "expiry_scan_seconds": 1,
    }
    settings = complete_runtime(settings, slots=1, inbox_sessions=8+settings["max_active_global"])
    registry = dict(root=str(root / "registry"), max_templates_global=1, max_templates_per_tenant=1,
        max_artifact_bytes_global=disk_bytes + kernel.stat().st_size,
        max_artifact_bytes_per_tenant=disk_bytes + kernel.stat().st_size, validation_timeout_secs=60)
    services = RuntimeServices.prepare(args, root, [settings], [catalog], key,
        f"127.0.0.1:{port}", args.service_cgroup_record, retained_sessions=8 + settings['max_active_global'], registry_config=registry)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    url = f"http://127.0.0.1:{port}"
    owner = options(key, url, url)
    completed = False
    claims = []
    native_ids = []

    def map_identity(sandbox, number):
        # The global ID is opaque. Read the original host inbox under its
        # exclusive OFFLINE audit lease; never infer a local ID from its suffix.
        services.processes.stop("host-0")
        native_id = services.native_id_for(sandbox.sandbox_id)
        assert native_id == f"sb-{namespace}-{number}"
        native_ids.append(native_id)
        services.processes.start("host-0")
        services.wait_ready(for_admission=False)
        return native_id

    def record_identity(number):
        record = json.loads((original_chroot(root) / ".claims" / names[number] / "record.json").read_text())
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
        services.start()
        services.wait_ready()
        for index in range(2):
            sandbox = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
            assert sandbox.sandbox_id == f"sandbox-{index + 1}"
            record_identity(index)
            native_id = map_identity(sandbox, index + 1)
            _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **owner))
            assert _kill_confirmed
            wait_for(lambda: retired(native_id), 20, "exact terminal retirement before slot reuse")
        survivor = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
        assert survivor.sandbox_id == "sandbox-3"
        survivor_native = map_identity(survivor, 3)
        survivor.files.write("/tmp/capacity.txt", "original-vm")
        live = record_identity(2)
        assert process_start_ticks(live["pid"]) == live["start_time_ticks"]
        check_rejection(opener, url, key, database, root)
        assert survivor.commands.run("printf capacity-ok", user="ubuntu").stdout == "capacity-ok"
        assert survivor.files.read("/tmp/capacity.txt") == "original-vm"

        services.processes.stop("center")
        services.processes.stop("host-0")
        services.processes.start("host-0")
        services.processes.start("center")
        services.wait_ready(for_admission=False)
        check_rejection(opener, url, key, database, root)
        assert process_start_ticks(live["pid"]) == live["start_time_ticks"], "VM replaced during restart"
        assert survivor.get_info().state == "running"
        assert survivor.commands.run("printf restart-ok", user="ubuntu").stdout == "restart-ok"
        assert survivor.files.read("/tmp/capacity.txt") == "original-vm"
        _kill_confirmed = (Sandbox.kill(survivor.sandbox_id, **owner))
        assert _kill_confirmed
        wait_for(lambda: retired(survivor_native), 20, "exact survivor retirement")
        fourth = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
        assert fourth.sandbox_id == "sandbox-4", "permanent identity reset or remained exhausted"
        fourth_native = map_identity(fourth, 4)
        record_identity(3)
        assert fourth.commands.run("printf slot-reused", user="ubuntu").stdout == "slot-reused"
        _kill_confirmed = Sandbox.kill(fourth.sandbox_id, **owner)
        assert _kill_confirmed
        wait_for(lambda: retired(fourth_native), 20, "exact fourth retirement")
        with sqlite3.connect(database) as connection:
            phases = [json.loads(row[0])["phase"] for row in connection.execute("SELECT state_blob FROM sandboxes")]
        assert phases == [], phases
        assert len(claims) == 4
        check_no_resources(root, namespace, firecracker, args.uid_base, claims, native=services.native_pools[0])
        assert sha256_file(rootfs) == sha256_file(args.rootfs) == base_digest
        completed = True
        print(json.dumps({
            "source_commit": commit, "center_sha256": sha256_file(args.center), "host_sha256": sha256_file(args.host),
            "probe_sha256": sha256_file(Path(__file__)), "base_sha256": base_digest,
            "host_kernel": os.uname().release, "sdk": "2.51.0", "namespace": namespace,
            "physical_slots": 1, "permanent_sandbox_ids": [1, 2, 3, 4], "native_ids": native_ids, "vms_destroyed": 4,
            "result": "one physical UID reused; live capacity refused before/after restart without allocating identities; original SDK command/file access preserved; fourth admitted after cleanup",
            "residual": "no exact VMM process, jail, netns, cgroup or test UID process",
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
    for argument in ("jailer", "firecracker", "kernel", "rootfs", "scratch-parent"):
        parser.add_argument(f"--{argument}", type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    exercise(parser.parse_args())
