"""Two real VMs: one failed stop observation must not stall another expiry.

Run as root only on an exclusive Linux/KVM test host. The debug-only hook
refuses to inspect the first sandbox's already stopped VM. It never supplies
a success receipt or changes the host resource itself.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
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
import time
import urllib.request

from server_activation_abort import database_snapshot, digest
from server_process_crash import (
    assert_idle_reopen, fetch, free_loopback_port, idle_snapshot,
    interrupted_host_request, prepare_fault_services,
    process_start_ticks, terminal_cleanup_confirmed,
)
from _runtime_services import add_arguments
from server_unready_abort import wait_for
from sdk_c02_acceptance import processes_in_uid_range


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an exclusive Linux/KVM host")
    parent = args.scratch_parent.resolve(strict=True)
    if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
        raise RuntimeError("scratch parent must be root-owned and private")
    if args.uid_base < 1000 or args.uid_base + 1 >= 2**32 - 1:
        raise RuntimeError("invalid exclusive test UID range")
    if processes_in_uid_range(args.uid_base, args.uid_base + 1):
        raise RuntimeError("test UID range already occupied")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("source checkout must be clean")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    base_digest = digest(args.rootfs)
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 4 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient private disk and cleanup space")
    root = Path(tempfile.mkdtemp(prefix="si-", dir=parent))
    namespace = secrets.token_hex(8)
    names = [f"vm-{namespace}-{index}" for index in range(2)]
    if len(os.fsencode(root / "p/v" / args.firecracker.name / names[1] / "root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("test path too long for Firecracker socket")
    for name in ("db", "state", "p"):
        (root / name).mkdir(mode=0o700)
    kernel, rootfs = (root / name for name in ("vmlinux", "rootfs.ext4"))
    for source, destination in ((args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source, destination)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    key = secrets.token_hex(32)
    port = free_loopback_port()
    database = root / "db/controller.db"
    values = {
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)), "firecracker": str(args.firecracker.resolve(strict=True)),
        "chroot_base": str(root / "p/v"), "kernel": str(kernel), "rootfs": str(rootfs),
        "uid_base": args.uid_base, "vcpu_count": 1, "memory_mib": 512,
        "file_policy_digest": 1,
        "max_active_global": 2, "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 4, "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600, "default_timeout_secs": 300,
        "expiry_scan_seconds": 3600,
    }
    if args.cgroup_parent is not None:
        values["host_resources"] = {"cgroup_parent": str(args.cgroup_parent.resolve(strict=True)),
                                   "memory_max_bytes": 768 * 1024 * 1024}
    services = prepare_fault_services(args, root, values, slots=2, key=key, port=port)
    log = services.layout["logs"] / "host-0.log"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{port}"
    process = None
    completed = False
    try:
        services.start()
        services.wait_ready(for_admission=False)
        process = services.processes.children["host-0"]
        sandbox_ids = []
        for _ in names:
            status, body = fetch(opener, base_url, "POST", "/v2/sandboxes", key,
                                 {"templateID": "base", "allow_internet_access": False})
            assert status == 201, (status, body)
            sandbox_ids.append(json.loads(body)["sandboxID"])
        claims = [json.loads((original_chroot(root) / ".claims" / name / "record.json").read_text())
                  for name in names]
        network_inodes = [json.loads((original_chroot(root) / ".claims" / name / "network.json").read_text())
                          ["namespace_inode"] for name in names]
        assert all(process_start_ticks(saved["pid"]) == saved["start_time_ticks"] for saved in claims)
        services.processes.stop("host-0")
        native_ids = [services.native_id_for(sandbox) for sandbox in sandbox_ids]
        assert native_ids == [f"sb-{namespace}-1", f"sb-{namespace}-2"]

        process = services.processes.start("host-0", env={"SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION": "stop_vm"})
        services.wait_ready(for_admission=False)
        # A positive renewal leaves B running until the original deadline.
        # Zero now means immediate center Destroy and could consume A's exact
        # StopVm crash hook before A's explicit deletion reaches the host.
        wait_for(lambda: process.poll() is None and
                 fetch(opener, base_url, "GET", "/v2/sandboxes", key)[0] == 200,
                 5, "two live instances before fault injection")
        status, body = fetch(opener, base_url, "POST", f"/sandboxes/{sandbox_ids[1]}/timeout",
                             key, {"timeout": 5})
        assert status == 204, (status, body)
        assert process_start_ticks(claims[1]["pid"]) == claims[1]["start_time_ticks"]
        with sqlite3.connect(database) as connection:
            b_deadline = connection.execute(
                "SELECT deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?", (native_ids[1],)
            ).fetchone()[0]
        interrupted_host_request(lambda: fetch(opener, base_url, "DELETE",
                                          f"/sandboxes/{sandbox_ids[0]}", key), process, 86)
        assert process_start_ticks(claims[0]["pid"]) != claims[0]["start_time_ticks"]
        assert process_start_ticks(claims[1]["pid"]) == claims[1]["start_time_ticks"]
        wait_for(lambda: time.time_ns() // 1_000_000 >= b_deadline,
                 10, "original B deadline while the exact host is offline")
        before = database_snapshot(database)
        assert len([action for action in before["actions"]
                    if "StopVm" in action[0]["command"]["logical"]["command"]
                    and action[1:] == ("dispatched", None)]) == 1

        process = services.processes.start("host-0", env={"SANDPROOF_TEST_FAIL_RECLAIM_STOP_SANDBOX": "1"})
        services.wait_ready(for_admission=False)
        wait_for(lambda: terminal_cleanup_confirmed(database, native_ids[1]),
                 20, "other expired sandbox cleanup")
        with sqlite3.connect(database) as connection:
            states = {int(sandbox_id.rsplit("-", 1)[1]): json.loads(blob)
                      for sandbox_id, blob in connection.execute(
                          "SELECT sandbox_id, state_blob FROM sandboxes")}
        assert states[1]["phase"] == "Revoking"
        assert terminal_cleanup_confirmed(database, native_ids[1])
        assert process_start_ticks(claims[1]["pid"]) != claims[1]["start_time_ticks"]
        assert not (original_chroot(root) / args.firecracker.name / names[1]).exists()
        assert not Path("/run/netns", names[1]).exists()
        assert "startup recovery left 1 sandboxes unresolved (1 after host or receipt errors)" in log.read_text()
        assert fetch(opener, base_url, "GET", f"/sandboxes/{sandbox_ids[0]}", key)[0] == 404
        services.processes.stop("host-0")

        process = services.processes.start("host-0")
        services.wait_ready(for_admission=False)
        wait_for(lambda: all(terminal_cleanup_confirmed(database, ident) for ident in native_ids),
                 20, "original stopped VM reconciliation")
        for name, saved in zip(names, claims):
            assert process_start_ticks(saved["pid"]) != saved["start_time_ticks"]
            assert not (original_chroot(root) / args.firecracker.name / name).exists()
            assert not Path("/run/netns", name).exists()
            if args.cgroup_parent is not None:
                assert not (args.cgroup_parent / name).exists()
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 1)
        holders = []
        for path in Path("/proc").iterdir():
            if path.name.isdecimal():
                try:
                    if (path / "ns/net").stat().st_ino in network_inodes:
                        holders.append(path.name)
                except FileNotFoundError:
                    pass
        assert not holders
        assert digest(args.rootfs) == digest(rootfs) == base_digest
        final = idle_snapshot(database)
        services.processes.stop("host-0")
        process = services.processes.start("host-0")
        services.wait_ready(for_admission=False)
        assert_idle_reopen(final, idle_snapshot(database))
        completed = True
        print(json.dumps({"source_commit": commit, "host_sha256": digest(args.host),
                          "center_sha256": digest(args.center),
                          "base_sha256": base_digest, "vm_names": names,
                          "result": "one failed stop observation remained Revoking while the other expired VM was Destroyed; exact original stop later reconciled",
                          "residual": "no exact VMM process, jail, network namespace, cgroup or test UID process"}))
    finally:
        services.stop_all()
        if completed:
            for secret in (root / "state/secrets").glob("*"):
                secret.unlink()
            if not args.keep_artifacts:
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
    parser.add_argument("--cgroup-parent", type=Path)
    parser.add_argument("--keep-artifacts", action="store_true")
    exercise(parser.parse_args())
