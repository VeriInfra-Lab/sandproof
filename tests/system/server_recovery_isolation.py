"""Two real VMs: one failed stop observation must not stall another expiry.

Run as root only on an exclusive Linux/KVM test host. The debug-only hook
refuses to inspect the first sandbox's already stopped VM. It never supplies
a success receipt or changes the host resource itself.
"""

import argparse
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

from server_activation_abort import database_snapshot, digest
from server_process_crash import (
    fetch, free_loopback_port, interrupted_request, process_start_ticks,
    start_server, stop_server, wait_ready,
)
from server_unready_abort import wait_for
from sdk_c02_acceptance import processes_in_uid_range


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an exclusive Linux/KVM host")
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
    if shutil.disk_usage(parent).free < disk_bytes * 4 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient private disk and cleanup space")
    root = Path(tempfile.mkdtemp(prefix="si-", dir=parent))
    namespace = secrets.token_hex(8)
    names = [f"vm-{namespace}-{index}" for index in range(2)]
    if len(os.fsencode(root / "jails" / args.firecracker.name / names[1] / "root/api.socket")) >= 108:
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
        "max_active_global": 2, "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 4, "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600, "default_timeout_secs": 300,
        "expiry_scan_seconds": 3600,
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
        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, key, process)
        sandbox_ids = []
        for _ in names:
            status, body = fetch(opener, base_url, "POST", "/v2/sandboxes", key,
                                 {"templateID": "base", "allow_internet_access": False})
            assert status == 201, (status, body)
            sandbox_ids.append(json.loads(body)["sandboxID"])
        claims = [json.loads((root / "jails/.claims" / name / "record.json").read_text())
                  for name in names]
        network_inodes = [json.loads((root / "jails/.claims" / name / "network.json").read_text())
                          ["namespace_inode"] for name in names]
        assert all(process_start_ticks(saved["pid"]) == saved["start_time_ticks"] for saved in claims)
        stop_server(process)

        process = start_server(binary, config, log, "stop_vm")
        wait_ready(opener, base_url, key, process)
        # The scanner's initial no-op tick occurs at server startup. Its next
        # tick is one hour away, so B cannot expire before A's injected exit.
        wait_for(lambda: process.poll() is None and
                 fetch(opener, base_url, "GET", "/v2/sandboxes", key)[0] == 200,
                 5, "two live instances before fault injection")
        status, body = fetch(opener, base_url, "POST", f"/sandboxes/{sandbox_ids[1]}/timeout",
                             key, {"timeout": 0})
        assert status == 204, (status, body)
        assert process_start_ticks(claims[1]["pid"]) == claims[1]["start_time_ticks"]
        interrupted_request(lambda: fetch(opener, base_url, "DELETE",
                                          f"/sandboxes/{sandbox_ids[0]}", key), process, 86)
        assert process_start_ticks(claims[0]["pid"]) != claims[0]["start_time_ticks"]
        assert process_start_ticks(claims[1]["pid"]) == claims[1]["start_time_ticks"]
        before = database_snapshot(database)
        assert len([action for action in before["actions"]
                    if "StopVm" in action[0]["command"]["logical"]["command"]
                    and action[1:] == ("dispatched", None)]) == 1

        process = start_server(binary, config, log, None, fail_reclaim_stop_sandbox=1)
        wait_ready(opener, base_url, key, process)
        wait_for(lambda: any(state["phase"] == "Destroyed" for state in database_snapshot(database)["states"]),
                 20, "other expired sandbox cleanup")
        with sqlite3.connect(database) as connection:
            states = {int(sandbox_id.rsplit("-", 1)[1]): json.loads(blob)
                      for sandbox_id, blob in connection.execute(
                          "SELECT sandbox_id, state_blob FROM sandboxes")}
        assert states[1]["phase"] == "Revoking" and states[2]["phase"] == "Destroyed"
        assert process_start_ticks(claims[1]["pid"]) != claims[1]["start_time_ticks"]
        assert not (root / "jails" / args.firecracker.name / names[1]).exists()
        assert not Path("/run/netns", names[1]).exists()
        assert "startup recovery left 1 sandboxes unresolved (1 after host or receipt errors)" in log.read_text()
        assert fetch(opener, base_url, "GET", f"/sandboxes/{sandbox_ids[0]}", key)[0] == 404
        stop_server(process)

        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, key, process)
        wait_for(lambda: all(state["phase"] == "Destroyed" for state in database_snapshot(database)["states"]),
                 20, "original stopped VM reconciliation")
        for name, saved in zip(names, claims):
            assert process_start_ticks(saved["pid"]) != saved["start_time_ticks"]
            assert not (root / "jails" / args.firecracker.name / name).exists()
            assert not Path("/run/netns", name).exists()
            if args.cgroup_parent is not None:
                assert not (args.cgroup_parent / name).exists()
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 1000)
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
        final = database_snapshot(database)
        stop_server(process)
        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, key, process)
        assert database_snapshot(database) == final, "idle restart changed terminal history"
        completed = True
        print(json.dumps({"source_commit": commit, "server_sha256": digest(binary),
                          "base_sha256": base_digest, "vm_names": names,
                          "result": "one failed stop observation remained Revoking while the other expired VM was Destroyed; exact original stop later reconciled",
                          "residual": "no exact VMM process, jail, network namespace, cgroup or test UID process"}))
    finally:
        stop_server(process)
        if completed:
            config.unlink()
            for secret in (root / "state/secrets").glob("*"):
                secret.unlink()
            if not args.keep_artifacts:
                shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    parser.add_argument("--cgroup-parent", type=Path)
    parser.add_argument("--keep-artifacts", action="store_true")
    exercise(parser.parse_args())
