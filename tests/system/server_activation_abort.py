"""Exact KVM activation-abort windows; root on an exclusive test host only.

Freeze only the recorded test VMM through its pidfd after ActivateVm is claimed
but before initialization. The guest becomes genuinely unavailable. After the
durable startup deadline, restart the controller and interrupt its terminal
stop/cleanup once more. No production pause/resume API is introduced.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request

from server_process_crash import (
    fetch, free_loopback_port, interrupted_request, process_start_ticks,
    start_server, stop_server, wait_ready,
)
from server_unready_abort import wait_for
from sdk_c02_acceptance import processes_in_uid_range


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def database_snapshot(database: Path) -> dict:
    with sqlite3.connect(database) as connection:
        return {
            "inputs": [json.loads(row[0])["input"] for row in connection.execute(
                "SELECT input_json FROM controller_inputs ORDER BY sequence")],
            "actions": [(json.loads(action), state, json.loads(receipt) if receipt else None)
                        for action, state, receipt in connection.execute(
                            "SELECT action_json, dispatch_state, receipt_json FROM outbox ORDER BY id")],
            "states": [json.loads(row[0]) for row in connection.execute(
                "SELECT state_blob FROM sandboxes")],
        }


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    parent = args.scratch_parent.resolve(strict=True)
    if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
        raise RuntimeError("scratch parent must be root-owned and private")
    if args.uid_base < 1000 or args.uid_base + 1000 >= 2**32:
        raise RuntimeError("invalid exclusive test UID range")
    if processes_in_uid_range(args.uid_base, args.uid_base + 1000):
        raise RuntimeError("test UID range already occupied")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("source checkout must be clean")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    base_digest = digest(args.rootfs)
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient private disk and cleanup space")
    root = Path(tempfile.mkdtemp(prefix="sa-", dir=parent))
    namespace = secrets.token_hex(8)
    vm_name = f"vm-{namespace}-0"
    if len(os.fsencode(root / "jails" / args.firecracker.name / vm_name / "root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("test path too long for Firecracker socket")
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    binary, kernel, rootfs = (root / name for name in ("sandproof-server", "vmlinux", "rootfs.ext4"))
    for source, destination in ((args.server, binary), (args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source, destination)
    binary.chmod(0o700)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    key = secrets.token_hex(32)
    port = free_loopback_port()
    config = root / "config.json"
    database = root / "db/controller.db"
    values = {
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)), "firecracker": str(args.firecracker.resolve(strict=True)),
        "chroot_base": str(root / "jails"), "kernel": str(kernel), "rootfs": str(rootfs),
        "uid_base": args.uid_base, "max_vm_id": 1000, "vcpu_count": 1, "memory_mib": 512,
        "file_policy_digest": 1, "bind": f"127.0.0.1:{port}", "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": key}],
        "max_active_global": 1, "max_active_per_tenant": 1,
        "max_disk_bytes_global": disk_bytes * 3, "max_disk_bytes_per_tenant": disk_bytes * 3,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600, "default_timeout_secs": 300,
        "startup_timeout_secs": 10, "expiry_scan_seconds": 1,
    }
    if args.cgroup_parent is not None:
        values["host_resources"] = {"cgroup_parent": str(args.cgroup_parent.resolve(strict=True)),
                                   "memory_max_bytes": 768 * 1024 * 1024}
    config.write_text(json.dumps(values))
    config.chmod(0o600)
    log = root / "server.log"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{port}"
    process = None
    completed = False
    try:
        process = start_server(binary, config, log, None, before_target="activate_vm")
        wait_ready(opener, base_url, key, process)
        interrupted_request(lambda: fetch(opener, base_url, "POST", "/v2/sandboxes", key, {
            "templateID": "base", "allow_internet_access": False, "envVars": {"ABORT_TEST": "original"},
        }), process, 87)
        claim = root / "jails/.claims" / vm_name
        record = json.loads((claim / "record.json").read_text())
        pid, ticks = record["pid"], record["start_time_ticks"]
        assert record["resource_id"] == vm_name
        network_inode = json.loads((claim / "network.json").read_text())["namespace_inode"]
        assert process_start_ticks(pid) == ticks
        with os.fdopen(os.pidfd_open(pid), "rb") as pidfd:
            assert process_start_ticks(pid) == ticks
            assert vm_name.encode() in Path(f"/proc/{pid}/cmdline").read_bytes()
            signal.pidfd_send_signal(pidfd.fileno(), signal.SIGSTOP)
        # No firewall or guest configuration is changed. Prove the original
        # process remains present while a real guest health RPC cannot finish.
        health = subprocess.run(["ip", "netns", "exec", vm_name, "curl", "-q", "--noproxy", "*",
                                 "--max-time", "1", "-s", "-o", "/dev/null",
                                 "http://172.16.0.2:49983/health"], check=False)
        assert health.returncode == 28, "guest did not become unavailable"
        assert process_start_ticks(pid) == ticks
        with sqlite3.connect(database) as connection:
            deadline = connection.execute("SELECT startup_deadline_ms FROM sandbox_lifetimes").fetchone()[0]
        wait_for(lambda: time.time_ns() // 1_000_000 >= deadline, 15, "persisted startup deadline")
        target = {"before-stop": None, "after-stop": "stop_vm", "after-cleanup": "cleanup_vm"}[args.window]
        process = start_server(binary, config, log, target,
                               before_target="stop_vm" if args.window == "before-stop" else None)
        expected_exit = 87 if args.window == "before-stop" else 86
        assert process.wait(timeout=40) == expected_exit
        first = database_snapshot(database)
        assert sum("Compensate" in item.get("Request", {}) for item in first["inputs"]) == 1
        activation = [entry for entry in first["actions"] if "ActivateVm" in entry[0]["command"]["logical"]["command"]]
        assert len(activation) == 1 and activation[0][1:] == ("dispatched", None)
        assert (process_start_ticks(pid) == ticks) == (args.window == "before-stop")
        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, key, process)
        wait_for(lambda: database_snapshot(database)["states"][0]["phase"] == "Destroyed", 15, "failed terminal state")
        final = database_snapshot(database)
        retirements = [item["Effect"]["RetireActivation"] for item in final["inputs"]
                       if isinstance(item.get("Effect"), dict) and "RetireActivation" in item["Effect"]]
        assert len(retirements) == 1
        assert final["states"][0]["current_operation"]["status"] == {"Failed": "DeadlineExceeded"}
        assert not any("Running" in item.get("Publication", {}) for item in final["inputs"])
        actions = final["actions"]
        assert sum("PrepareVm" in a["command"]["logical"]["command"] for a, _, _ in actions) == 1
        # Retired activation retains its missing receipt; stop/cleanup are real
        # successful receipts. The outbox column alone is not an unresolved count.
        assert all(state == "receipted" for a, state, _ in actions
                   if any(name in a["command"]["logical"]["command"] for name in ("StopVm", "CleanupVm", "CleanupSlot")))
        assert process_start_ticks(pid) != ticks
        assert not (root / "jails" / args.firecracker.name / vm_name).exists()
        assert not Path("/run/netns", vm_name).exists()
        if args.cgroup_parent is not None:
            assert not (args.cgroup_parent / vm_name).exists()
        stop_server(process)
        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, key, process)
        assert database_snapshot(database) == final, "idle restart changed the retired operation"
        # With max_active=1, a new successful create proves the aborted VM no
        # longer occupies quota. Never resubmit the original unknown create.
        status, body = fetch(opener, base_url, "POST", "/v2/sandboxes", key,
                             {"templateID": "base", "allow_internet_access": False})
        assert status == 201, (status, body)
        sandbox_id = json.loads(body)["sandboxID"]
        status, body = fetch(opener, base_url, "DELETE", f"/sandboxes/{sandbox_id}", key)
        assert status == 204, (status, body)
        wait_for(lambda: not processes_in_uid_range(args.uid_base, args.uid_base + 1000), 15, "all test VM exits")
        for path in (root / "jails/.claims").iterdir():
            saved = json.loads((path / "record.json").read_text())
            assert process_start_ticks(saved["pid"]) != saved["start_time_ticks"]
            assert not (root / "jails" / args.firecracker.name / path.name).exists()
            assert not Path("/run/netns", path.name).exists()
            if args.cgroup_parent is not None:
                assert not (args.cgroup_parent / path.name).exists()
        holders = []
        for path in Path("/proc").iterdir():
            if path.name.isdecimal():
                try:
                    if (path / "ns/net").stat().st_ino == network_inode:
                        holders.append(path.name)
                except FileNotFoundError:
                    pass
        assert not holders
        assert digest(args.rootfs) == digest(rootfs) == base_digest
        completed = True
        print(json.dumps({"source_commit": commit, "server_sha256": digest(binary), "window": args.window,
                          "scope": "unknown activation; exact frozen VMM; lost terminal receipt; idle restart and quota reuse",
                          "vm": vm_name, "pid": pid, "start_time_ticks": ticks, "guest_health": "timed out",
                          "retirement_count": len(retirements), "activation_receipt": "still absent; not fabricated",
                          "base_sha256": base_digest, "cgroup_enabled": args.cgroup_parent is not None,
                          "residual": "no exact processes, jails, network namespaces or enabled cgroups"}))
    finally:
        stop_server(process)
        if completed and not args.keep_artifacts:
            shutil.rmtree(root)
        else:
            print(f"Exact-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    parser.add_argument("--window", required=True, choices=("before-stop", "after-stop", "after-cleanup"))
    parser.add_argument("--cgroup-parent", type=Path)
    parser.add_argument("--keep-artifacts", action="store_true")
    exercise(parser.parse_args())
