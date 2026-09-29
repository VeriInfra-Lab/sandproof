"""Run the existing C02 SDK probes against one isolated real KVM service.

Run as root on an exclusive Linux/KVM host. This wrapper adds no new SDK
expectations: it reuses sdk_server_smoke.py and sdk_data_contract.py, then
checks the durable terminal state and this run's exact host resources.
Failed runs retain only their own private directory for diagnosis.
"""

import argparse
import hashlib
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
import time
import urllib.request

from server_process_crash import (
    free_loopback_port,
    process_start_ticks,
    start_server,
    stop_server,
    wait_ready,
)


MAX_VM_ID = 1000
EXPECTED_SANDBOXES = 4


def sha256_file(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def processes_in_uid_range(first: int, last: int) -> list[int]:
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            status = (entry / "status").read_text()
        except (FileNotFoundError, ProcessLookupError):
            continue
        for line in status.splitlines():
            if line.startswith("Uid:"):
                if any(first <= int(uid) <= last for uid in line.split()[1:]):
                    found.append(int(entry.name))
                break
    return sorted(found)


def wait_for_destroyed(database: Path) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        with sqlite3.connect(database) as connection:
            rows = connection.execute("SELECT state_blob FROM sandboxes").fetchall()
        phases = [json.loads(row[0])["phase"] for row in rows]
        if len(phases) == EXPECTED_SANDBOXES and all(phase == "Destroyed" for phase in phases):
            return
        time.sleep(0.2)
    raise AssertionError(f"expected {EXPECTED_SANDBOXES} durable Destroyed rows; observed {phases}")


def assert_no_exact_resources(root: Path, namespace: str, first_uid: int, firecracker_name: str) -> None:
    prefix = f"vm-{namespace}-"
    claims = root / "jails/.claims"
    tombstones = sorted(claims.glob(f"{prefix}*"))
    assert len(tombstones) == EXPECTED_SANDBOXES, "expected one non-reusable claim per VM"
    for claim in tombstones:
        record = json.loads((claim / "record.json").read_text())
        assert record["resource_id"] == claim.name, "claim identity changed"
        assert process_start_ticks(record["pid"]) != record["start_time_ticks"], (
            "recorded VM process survived cleanup"
        )
    # Claims are intentionally retained as non-reusable tombstones, not live VMs.
    for parent in (root / "jails" / firecracker_name, Path("/run/netns")):
        assert not [path for path in parent.glob(f"{prefix}*")], f"VM resource remains under {parent}"
    cgroup_root = Path("/sys/fs/cgroup")
    assert not list(cgroup_root.rglob(f"{prefix}*")), "test cgroup remains"
    assert not processes_in_uid_range(first_uid, first_uid + MAX_VM_ID), "test UID process remains"


def run_probe(root: Path, script: Path, arguments: list[str], environment: dict[str, str]) -> None:
    stdout = root / f"{script.stem}.stdout"
    stderr = root / f"{script.stem}.stderr"
    with stdout.open("wb") as out, stderr.open("wb") as err:
        result = subprocess.run(
            [sys.executable, str(script), *arguments],
            stdout=out,
            stderr=err,
            env=environment,
            check=False,
            timeout=240,
        )
    if result.returncode != 0:
        raise AssertionError(f"{script.name} failed with exit {result.returncode}; private logs retained")


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    if args.uid_base < 1000 or args.uid_base + MAX_VM_ID >= 2**32:
        raise RuntimeError("invalid dedicated test UID range")
    assert importlib.metadata.version("e2b") == "2.51.0"
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("test checkout must be clean")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    parent = args.scratch_parent.resolve(strict=True)
    parent_mode = parent.stat()
    if parent_mode.st_uid != 0 or parent_mode.st_mode & 0o077:
        raise RuntimeError("scratch parent must be root-owned and private")
    if processes_in_uid_range(args.uid_base, args.uid_base + MAX_VM_ID):
        raise RuntimeError("test UID range is already occupied")
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient disk space for private VM copies and cleanup")
    trusted_base_digest = sha256_file(args.rootfs)

    firecracker = args.firecracker.resolve(strict=True)
    root = Path(tempfile.mkdtemp(prefix="sp12-", dir=parent))
    namespace = secrets.token_hex(8)
    longest_socket = root / "jails" / firecracker.name / f"vm-{namespace}-{MAX_VM_ID}" / "root/api.socket"
    if len(os.fsencode(longest_socket)) >= 108:
        root.rmdir()
        raise RuntimeError("dedicated test root is too long for Firecracker API socket")
    owner_key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    port = free_loopback_port()
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    binary = root / "sandproof-server"
    kernel = root / "vmlinux"
    rootfs = root / "rootfs.ext4"
    for source, destination in ((args.server, binary), (args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source.resolve(strict=True), destination)
    binary.chmod(0o700)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    base_digest = sha256_file(rootfs)
    config = root / "config.json"
    config.write_text(json.dumps({
        "namespace": namespace,
        "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)),
        "firecracker": str(firecracker),
        "chroot_base": str(root / "jails"),
        "kernel": str(kernel),
        "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "max_vm_id": MAX_VM_ID,
        "vcpu_count": 1,
        "memory_mib": 512,
        "file_policy_digest": 1,
        "bind": f"127.0.0.1:{port}",
        "domain": "example.invalid",
        "credentials": [
            {"tenant": 1, "secret": owner_key},
            {"tenant": 2, "secret": foreign_key},
        ],
        "max_active_global": 2,
        "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 6,
        "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "max_lifetime_secs": 600,
        "default_timeout_secs": 300,
        "expiry_scan_seconds": 1,
    }))
    config.chmod(0o600)
    process = None
    completed = False
    try:
        if base_digest != trusted_base_digest:
            raise RuntimeError("private base copy does not match the selected source")
        process = start_server(binary, config, root / "server.log", None)
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        base_url = f"http://127.0.0.1:{port}"
        wait_ready(opener, base_url, owner_key, process)
        environment = os.environ.copy()
        environment["SANDPROOF_TEST_API_KEY"] = owner_key
        environment["SANDPROOF_TEST_FOREIGN_API_KEY"] = foreign_key
        scripts = ("sdk_server_smoke.py", "sdk_data_contract.py")
        for name in scripts:
            run_probe(
                root,
                checkout / "tests/system" / name,
                ["--api-url", base_url, "--sandbox-url", base_url],
                environment,
            )
            assert process.poll() is None, f"server exited during {name}"
        wait_for_destroyed(root / "db/controller.db")
        assert_no_exact_resources(root, namespace, args.uid_base, firecracker.name)
        assert sha256_file(rootfs) == base_digest, "private base copy changed"
        assert sha256_file(args.rootfs) == trusted_base_digest, "source base changed"
        completed = True
        print(json.dumps({
            "commit": commit,
            "host_kernel": os.uname().release,
            "sdk": "e2b 2.51.0",
            "server_sha256": sha256_file(binary),
            "base_sha256": base_digest,
            "namespace": namespace,
            "probes": list(scripts),
            "probe_sha256": {
                name: sha256_file(checkout / "tests/system" / name) for name in scripts
            },
            "vms_destroyed": EXPECTED_SANDBOXES,
            "residual": "no exact VM process, jail, netns, cgroup or test UID process; non-reusable claims retained until private-root removal",
            "scope": "selected C02 SDK/auth/data gateway chain; not full #8/#9 or V1",
        }))
    finally:
        stop_server(process)
        if completed:
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--jailer", type=Path, required=True)
    parser.add_argument("--firecracker", type=Path, required=True)
    parser.add_argument("--kernel", type=Path, required=True)
    parser.add_argument("--rootfs", type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--scratch-parent", type=Path, required=True)
    exercise(parser.parse_args())
