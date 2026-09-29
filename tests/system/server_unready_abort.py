"""Server-level KVM regression: a live but unready VMM is exactly reclaimed.

The debug server exits after the physical AbortPrepare succeeds but before its
receipt commits. Restart must recover the two unknown receipts without a new
PrepareVm or a guessed PID kill. Run only as root on an isolated KVM host.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import sys
import tempfile
import time
import urllib.request

from server_process_crash import (
    fetch, free_loopback_port, process_start_ticks, start_server, stop_server,
    wait_ready,
)


def sha256_file(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def wait_for(predicate, seconds: float, description: str) -> object:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise AssertionError(f"timed out waiting for {description}")


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    parent = args.scratch_parent.resolve(strict=True)
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 2 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient room for private VM disk and cleanup")
    root = Path(tempfile.mkdtemp(prefix="s11-", dir=parent))
    namespace = secrets.token_hex(8)
    vm_name = f"vm-{namespace}-0"
    if len(os.fsencode(root / "jails" / args.firecracker.name / vm_name / "root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("dedicated test path is too long for Firecracker API")
    key = secrets.token_hex(32)
    port = free_loopback_port()
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
    base_digest = sha256_file(rootfs)
    # Only the private test copy is corrupted. The VMM launches, but its boot
    # API cannot establish Running from these invalid image bytes.
    with kernel.open("r+b") as image:
        image.write(bytes(4096))
        image.flush()
        os.fsync(image.fileno())
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
        "credentials": [{"tenant": 1, "secret": key}],
        "max_active_global": 2,
        "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 3,
        "max_disk_bytes_per_tenant": disk_bytes * 3,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 60,
        "default_timeout_secs": 5,
        "expiry_scan_seconds": 1,
    }))
    config.chmod(0o600)
    log = root / "server.log"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{port}"
    process = None
    completed = False
    try:
        process = start_server(binary, config, log, "abort_prepare")
        wait_ready(opener, base_url, key, process)
        status, _ = fetch(opener, base_url, "POST", "/v2/sandboxes", key, {
            "templateID": "base", "timeout": 5, "allow_internet_access": False,
        })
        assert status >= 500, f"invalid kernel was published: HTTP {status}"
        claim = root / "jails/.claims" / vm_name
        record = json.loads(wait_for(
            lambda: (claim / "record.json").read_text() if (claim / "record.json").exists() else None,
            3, "durable VMM process record",
        ))
        pid, ticks = record["pid"], record["start_time_ticks"]
        assert record["resource_id"] == vm_name
        assert process_start_ticks(pid) == ticks, "test did not produce a live unready VMM"
        assert process.wait(timeout=30) == 86, "debug exit after exact abort did not fire"
        assert process_start_ticks(pid) != ticks, "exact VMM survived durable abort"
        marker = claim / "failed-prepare.json"
        assert marker.is_file(), "physical reclaim marker missing before receipt"
        assert not (root / "jails" / args.firecracker.name / vm_name).exists()
        assert not Path(f"/run/netns/{vm_name}").exists()
        with sqlite3.connect(root / "db/controller.db") as database:
            inputs = [row[0] for row in database.execute("SELECT input_json FROM controller_inputs")]
            actions = [state for state, action in database.execute(
                "SELECT dispatch_state, action_json FROM outbox"
            ) if b'"AbortPrepare"' in action]
        assert sum(b'"Compensate"' in row for row in inputs) == 1
        assert actions == ["dispatched"], actions

        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, key, process)

        def terminal() -> bool:
            with sqlite3.connect(root / "db/controller.db") as database:
                states = [json.loads(row[0]) for row in database.execute(
                    "SELECT state_blob FROM sandboxes"
                )]
                unresolved = database.execute(
                    "SELECT count(*) FROM outbox WHERE dispatch_state='dispatched'"
                ).fetchone()[0]
            return len(states) == 1 and states[0]["phase"] == "Destroyed" and unresolved == 0

        wait_for(terminal, 20, "failed terminal publication and receipt reconciliation")
        status, body = fetch(opener, base_url, "GET", "/v2/sandboxes", key)
        assert status == 200 and json.loads(body) == [], (status, body)
        assert process_start_ticks(pid) != ticks
        assert not (root / "jails" / args.firecracker.name / vm_name).exists()
        assert not Path(f"/run/netns/{vm_name}").exists()
        assert sha256_file(rootfs) == base_digest
        completed = True
        print(json.dumps({
            "scope": "real VMM, unready boot, durable deadline abort, receipt-loss restart",
            "binary_sha256": sha256_file(binary),
            "prepare_count": 1,
            "abort_receipt": "lost then observed on restart",
            "terminal": "Destroyed with no dispatched outbox actions",
            "residual": "no exact process, jail or network namespace; base disk unchanged",
        }))
    finally:
        stop_server(process)
        if completed:
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", required=True, type=Path)
    parser.add_argument("--jailer", required=True, type=Path)
    parser.add_argument("--firecracker", required=True, type=Path)
    parser.add_argument("--kernel", required=True, type=Path)
    parser.add_argument("--rootfs", required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    parser.add_argument("--scratch-parent", required=True, type=Path)
    exercise(parser.parse_args())
