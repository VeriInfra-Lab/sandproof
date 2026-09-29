"""Run a small preselected Harbor oracle chain on fresh real KVM sandboxes.

Requires root on an isolated Linux/KVM host, a 10 GiB Ubuntu 24.04 base,
public IPv4 forwarding already enabled for the entire run, and the locked
Harbor/E2B Python environment. No cloud API key or paid model is used.
"""

import argparse
import hashlib
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

from e2b import Sandbox

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import (
    free_loopback_port, process_start_ticks, start_server, stop_server, wait_ready,
)


MAX_VM_ID = 1000
GIB = 1024 ** 3


def selected_inputs(checkout: Path, source: Path, names: list[str]) -> list[tuple[dict, dict]]:
    if str(checkout) not in sys.path:
        sys.path.insert(0, str(checkout))
    from benchmarks.check_harbor import check_runtime
    from harbor.models.task.task import Task

    check_runtime()
    lock = json.loads((checkout / "benchmarks/terminal-bench-2.0.lock.json").read_text())
    selected = json.loads((checkout / "benchmarks/diagnostic-tasks.json").read_text())
    if (lock["task_count"] != 89 or selected["schema_version"] != 1
            or selected["source_commit"] != lock["source"]["commit"]
            or selected["dataset"] != lock["dataset"]):
        raise ValueError("diagnostic selection differs from the 89-task lock")
    if subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip() != selected["source_commit"]:
        raise ValueError("task source is not at the pinned commit")
    if subprocess.check_output(["git", "-C", str(source), "status", "--porcelain", "--untracked-files=all"]):
        raise ValueError("pinned task source is dirty")
    if len(set(names)) != len(names):
        raise ValueError("duplicate selected task")
    selected_by_name = {row["name"]: row for row in selected["tasks"]}
    lock_by_name = {row["name"]: row for row in lock["tasks"]}
    if len(selected_by_name) != len(selected["tasks"]) or not names or any(name not in selected_by_name for name in names):
        raise ValueError("requested task not in the preselected list")
    result = []
    for name in names:
        entry, pin = lock_by_name[name], selected_by_name[name]
        if entry["files_sha256"] != pin["files_sha256"] or entry["requirements"]["cpus"] != 1:
            raise ValueError(f"{name}: selected identity or CPU differs")
        if entry["requirements"]["memory_mib"] != 2048 or entry["requirements"]["disk_mib"] != 10240:
            raise ValueError(f"{name}: unsupported resource profile")
        directory = source / entry["source_path"]
        for item in entry["files"]:
            path = directory / item["path"]
            contents = path.read_bytes()
            mode = "100755" if path.stat().st_mode & 0o111 else "100644"
            if (path.is_symlink() or len(contents) != item["bytes"] or mode != item["mode"]
                    or hashlib.sha256(contents).hexdigest() != item["sha256"]):
                raise ValueError(f"{name}: locked file differs: {item['path']}")
        task = Task(directory)
        actual = task.config.environment
        if task.name != name or (actual.cpus, actual.memory_mb, actual.storage_mb) != (1, 2048, 10240):
            raise ValueError(f"{name}: original Harbor parser differs")
        dockerfile = "\n".join(line for line in (directory / "environment/Dockerfile").read_text().splitlines()
                               if not line.startswith("#"))
        expected = "FROM ubuntu:24.04\n\nWORKDIR /app\n"
        if name == "regex-log" and dockerfile.strip() != expected.strip():
            raise ValueError("regex-log Dockerfile needs a new reviewed initializer")
        if name == "polyglot-c-py" and (dockerfile.strip() != (expected + "\nRUN apt update -y && apt install -y gcc").strip()):
            raise ValueError("polyglot-c-py Dockerfile needs a new reviewed initializer")
        result.append((entry, pin))
    return result


def no_exact_residual(root: Path, namespace: str, first_uid: int, count: int, firecracker_name: str) -> None:
    prefix = f"vm-{namespace}-"
    claims = sorted((root / "jails/.claims").glob(f"{prefix}*"))
    assert len(claims) == count, f"expected {count} VM claim tombstones, got {len(claims)}"
    for claim in claims:
        record = json.loads((claim / "record.json").read_text())
        assert record["resource_id"] == claim.name
        assert process_start_ticks(record["pid"]) != record["start_time_ticks"]
    for parent in (root / "jails" / firecracker_name, Path("/run/netns")):
        assert not list(parent.glob(f"{prefix}*")), f"VM residue under {parent}"
    assert not list(Path("/sys/fs/cgroup").rglob(f"{prefix}*")), "VM cgroup residue"
    assert not processes_in_uid_range(first_uid, first_uid + MAX_VM_ID), "VM UID process residue"


def wait_destroyed(database: Path, count: int) -> None:
    deadline = time.monotonic() + 40
    phases = []
    while time.monotonic() < deadline:
        with sqlite3.connect(database) as connection:
            phases = [json.loads(row[0])["phase"] for row in connection.execute("SELECT state_blob FROM sandboxes")]
        if len(phases) == count and all(phase == "Destroyed" for phase in phases):
            return
        time.sleep(0.2)
    raise AssertionError(f"durable VM states are not all Destroyed: {phases}")


def run_original_task(entry: dict, pin: dict, source: Path, base_url: str, api_key: str,
                      root: Path) -> dict:
    name = entry["name"]
    directory = source / entry["source_path"]
    sdk_options = options(api_key, base_url, base_url)
    sandbox = None
    try:
        sandbox = Sandbox.create(template="base", timeout=86400, allow_internet_access=True, **sdk_options)
        assert Sandbox.connect(sandbox.sandbox_id, **sdk_options).sandbox_id == sandbox.sandbox_id
        assert sandbox.get_info().state == "running"
        assert any(item.sandbox_id == sandbox.sandbox_id for item in Sandbox.list(**sdk_options).next_items())
        Sandbox.set_timeout(sandbox.sandbox_id, 86400, **sdk_options)
        prepared = sandbox.commands.run("mkdir -p /app /tests /logs/verifier && test \"$(id -u)\" = 0 && grep -qx 'VERSION_ID=\"24.04\"' /etc/os-release && test ! -e " + pin["initial_absent"], cwd="/app", timeout=30)
        assert prepared.exit_code == 0
        profile = sandbox.commands.run("nproc; awk '/MemTotal/ {print $2}' /proc/meminfo; df -B1 --output=size / | tail -1", cwd="/app", timeout=30).stdout.splitlines()
        assert int(profile[0]) == 1 and int(profile[1]) >= 2_000_000 and int(profile[2]) >= 9 * GIB, profile
        version = sandbox.commands.run("python3 --version; (gcc --version | head -1) 2>/dev/null || true", cwd="/app", timeout=30).stdout.strip()
        if name == "polyglot-c-py":
            # Exactly the only task-specific Dockerfile RUN step, inside the guest.
            sandbox.commands.run("apt update -y && apt install -y gcc", cwd="/app", timeout=600)
            assert sandbox.commands.run("gcc --version >/dev/null", cwd="/app", timeout=30).exit_code == 0
            version = sandbox.commands.run("python3 --version; gcc --version | head -1", cwd="/app", timeout=30).stdout.strip()
        assert sandbox.commands.run("test ! -e /tests/test.sh && test ! -e /tmp/sandproof-oracle.sh", cwd="/app", timeout=30).exit_code == 0
        # Oracle and verifier are transferred only in their own phases, never in base/init.
        sandbox.files.write("/tmp/sandproof-oracle.sh", (directory / "solution/solve.sh").read_bytes())
        sandbox.commands.run("bash /tmp/sandproof-oracle.sh", cwd="/app", timeout=int(entry["requirements"]["agent_timeout_sec"]))
        sandbox.commands.run("rm /tmp/sandproof-oracle.sh", cwd="/app", timeout=30)
        assert sandbox.commands.run("test ! -e /tmp/sandproof-oracle.sh", cwd="/app", timeout=30).exit_code == 0
        sandbox.files.write("/tests/test.sh", (directory / "tests/test.sh").read_bytes())
        sandbox.files.write("/tests/test_outputs.py", (directory / "tests/test_outputs.py").read_bytes())
        scoring = sandbox.commands.run("bash /tests/test.sh", cwd="/app", timeout=int(entry["requirements"]["verifier_timeout_sec"]))
        reward = sandbox.files.read("/logs/verifier/reward.txt").strip()
        assert reward == "1", f"{name}: original scoring gave {reward!r}"
        assert sandbox.files.read("/logs/verifier/ctrf.json"), "original CTRF report missing"
        (root / f"{name}.score.stdout").write_text(scoring.stdout)
        (root / f"{name}.score.stderr").write_text(scoring.stderr)
        return {"name": name, "source_files_sha256": entry["files_sha256"], "reward": reward,
                "guest_versions": version, "guest_profile": profile, "sandbox_id": sandbox.sandbox_id,
                "original_verifier_sha256": next(item["sha256"] for item in entry["files"] if item["path"] == "tests/test.sh")}
    finally:
        if sandbox is not None:
            assert Sandbox.kill(sandbox.sandbox_id, **sdk_options), f"{name}: kill failed"


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on isolated Linux/KVM host")
    if args.uid_base < 1000 or args.uid_base + MAX_VM_ID >= 2**32:
        raise RuntimeError("invalid dedicated test UID range")
    if Path("/proc/sys/net/ipv4/ip_forward").read_text().strip() != "1":
        raise RuntimeError("public verifier needs pre-authorized IPv4 forwarding enabled for the whole run")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("test checkout must be clean")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    selection = selected_inputs(checkout, args.source.resolve(strict=True), args.task)
    parent = args.scratch_parent.resolve(strict=True)
    if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
        raise RuntimeError("scratch parent must be root-owned and private")
    if processes_in_uid_range(args.uid_base, args.uid_base + MAX_VM_ID):
        raise RuntimeError("dedicated VM UID range already occupied")
    if args.rootfs.stat().st_size != 10 * GIB:
        raise RuntimeError("selected tasks require a full 10 GiB rootfs")
    if shutil.disk_usage(parent).free < args.rootfs.stat().st_size * (len(selection) + 2) + GIB:
        raise RuntimeError("not enough disk for private base and fresh task VMs")
    source_base_sha = sha256_file(args.rootfs)
    root = Path(tempfile.mkdtemp(prefix="sp13-", dir=parent))
    namespace = secrets.token_hex(8)
    firecracker = args.firecracker.resolve(strict=True)
    if len(os.fsencode(root / "jails" / firecracker.name / f"vm-{namespace}-{MAX_VM_ID}" / "root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("Firecracker socket path too long")
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    binary, kernel, rootfs = root / "sandproof-server", root / "vmlinux", root / "rootfs.ext4"
    for source_file, destination in ((args.server, binary), (args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source_file.resolve(strict=True), destination)
    binary.chmod(0o700)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    assert sha256_file(rootfs) == source_base_sha
    key = secrets.token_hex(32)
    port = free_loopback_port()
    config = root / "config.json"
    config.write_text(json.dumps({
        "namespace": namespace, "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"), "jailer": str(args.jailer.resolve(strict=True)),
        "firecracker": str(firecracker), "chroot_base": str(root / "jails"),
        "kernel": str(kernel), "rootfs": str(rootfs), "uid_base": args.uid_base,
        "max_vm_id": MAX_VM_ID, "vcpu_count": 1, "memory_mib": 2048,
        "file_policy_digest": 1, "bind": f"127.0.0.1:{port}",
        "domain": "example.invalid", "credentials": [{"tenant": 1, "secret": key}],
        "max_active_global": 1, "max_active_per_tenant": 1,
        "max_disk_bytes_global": 30 * GIB, "max_disk_bytes_per_tenant": 30 * GIB,
        "cleanup_reserve_bytes": 10 * GIB, "max_timeout_secs": 86400,
        "max_lifetime_secs": 86400, "default_timeout_secs": 86400,
        "expiry_scan_seconds": 1, "public_ipv4_enabled": True,
    }))
    config.chmod(0o600)
    process = None
    completed = False
    results = []
    try:
        process = start_server(binary, config, root / "server.log", None)
        base_url = f"http://127.0.0.1:{port}"
        wait_ready(urllib.request.build_opener(urllib.request.ProxyHandler({})), base_url, key, process)
        for entry, pin in selection:
            results.append(run_original_task(entry, pin, args.source, base_url, key, root))
            wait_destroyed(root / "db/controller.db", len(results))
            no_exact_residual(root, namespace, args.uid_base, len(results), firecracker.name)
            assert sha256_file(rootfs) == source_base_sha, "private base changed"
        assert sha256_file(args.rootfs) == source_base_sha, "source base changed"
        completed = True
        print(json.dumps({"commit": commit, "host_kernel": os.uname().release,
                          "server_sha256": sha256_file(binary), "base_sha256": source_base_sha,
                          "source_commit": json.loads((checkout / "benchmarks/diagnostic-tasks.json").read_text())["source_commit"],
                          "memory_mib": 2048, "disk_mib": 10240, "vcpu_count": 1,
                          "network": "public IPv4 fixed for whole trial", "tasks": results,
                          "residual": "each VM durable Destroyed; no exact process, jail, netns, cgroup, test UID; base unchanged",
                          "scope": "selected original oracle/verifier chain, not full E2B equivalence or 89-task score"}))
    finally:
        stop_server(process)
        if completed:
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("server", "jailer", "firecracker", "kernel", "rootfs", "source", "scratch-parent"):
        parser.add_argument("--" + field, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--task", action="append", choices=("regex-log", "polyglot-c-py"), required=True)
    exercise(parser.parse_args())
