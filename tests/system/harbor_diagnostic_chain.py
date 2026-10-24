"""Run a small preselected Harbor oracle chain on fresh real KVM sandboxes.

Requires root on an isolated Linux/KVM host, a 10 GiB Ubuntu 24.04 base,
public IPv4 forwarding already enabled for the entire run, and the locked
Harbor/E2B Python environment. The single physical slot needs an empty delegated
cgroup parent and one exclusive UID. No cloud API key or paid model is used.
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
from tools.capture_acceptance import original_chroot
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

from e2b import Sandbox

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from _complete_runtime import complete_runtime, terminal_cleanup_confirmed
from _runtime_services import RuntimeServices, add_arguments
from server_process_crash import free_loopback_port, process_start_ticks
from server_template_isolation import audit_gone
from server_templates import descriptor, record_for


MIB = 1024 ** 2
GIB = 1024 ** 3
VERIFIER_PYTHON_SHA256 = "0c5450b5ba835b0f653809712c2e012e795dd602105383581476da118a981572"


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
            or selected["dataset"] != lock["dataset"]
            or selected["verifier_python"]["runtime_archive_sha256"] != VERIFIER_PYTHON_SHA256):
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


def save_identity(root: Path, sandbox: Sandbox, identities: list[dict], services) -> dict:
    # Observe the original live claim before any SDK assertion or DELETE. These
    # private test observations survive retirement; they are not worker receipts.
    services.processes.stop('host-0')
    native_id = services.native_id_for(sandbox.sandbox_id)
    services.processes.start('host-0')
    services.wait_ready(for_admission=False)
    _, name, record = record_for(root, native_id=native_id)
    network = json.loads((original_chroot(root) / ".claims" / name / "network.json").read_text())
    identity = {**record, "network_inode": network["namespace_inode"],
                "sandbox_id": sandbox.sandbox_id, "native_id": native_id}
    path = root / f"{name}.identity.json"
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(identity, output)
        output.flush()
        os.fsync(output.fileno())
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)
    identities.append(identity)
    assert identity["resource_id"] == name
    assert network["resource_id"] == name
    return identity


def no_exact_residual(root: Path, namespace: str, first_uid: int,
                      identities: list[dict], firecracker: Path, cgroup_parent: Path) -> None:
    assert identities, "original live VM identities were not saved"
    prefix = f"vm-{namespace}-"
    for identity in identities:
        name = identity["resource_id"]
        assert name.startswith(prefix), "saved claim belongs to another namespace"
        audit_gone(root, firecracker, cgroup_parent, name, identity=identity)
    for parent in (original_chroot(root) / firecracker.name, Path("/run/netns")):
        assert not list(parent.glob(f"{prefix}*")), f"VM residue under {parent}"
    assert not list(Path("/sys/fs/cgroup").rglob(f"{prefix}*")), "VM cgroup residue"
    assert not processes_in_uid_range(first_uid, first_uid), "physical-slot UID process residue"


def wait_destroyed(database: Path, sandbox_id: str) -> None:
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        if terminal_cleanup_confirmed(database, sandbox_id):
            return
        time.sleep(0.2)
    raise AssertionError(f"exact sandbox lacks durable terminal cleanup: {sandbox_id}")


def stage_verifier_python(sandbox: Sandbox, archive: Path) -> None:
    # The original test.sh installs uv and then asks uvx for Python 3.13.
    # Stage only that runtime after the oracle phase, preserving agent visibility.
    with archive.open("rb") as stream:
        index = 0
        while chunk := stream.read(2 * 1024 * 1024):
            sandbox.files.write(f"/tmp/sp13-verifier-python.part-{index:03d}", chunk)
            index += 1
    assert index > 0
    command = (
        "cat /tmp/sp13-verifier-python.part-* > /tmp/sp13-verifier-python.tar.gz && "
        f"echo '{VERIFIER_PYTHON_SHA256}  /tmp/sp13-verifier-python.tar.gz' | sha256sum -c - && "
        "tar -xzf /tmp/sp13-verifier-python.tar.gz -C / && "
        "ln -s /opt/sp13-python-3.13.9/bin/python3.13 /usr/local/bin/python3.13 && "
        "python3.13 --version && rm /tmp/sp13-verifier-python.part-* /tmp/sp13-verifier-python.tar.gz"
    )
    sandbox.commands.run(command, cwd="/app", timeout=180)


def run_original_task(entry: dict, pin: dict, source: Path, base_url: str, api_key: str,
                      root: Path, verifier_python: Path, identities: list[dict], services) -> dict:
    name = entry["name"]
    directory = source / entry["source_path"]
    sdk_options = options(api_key, base_url, base_url)
    sandbox = None
    identity = None
    try:
        sandbox = Sandbox.create(template="base", timeout=86400, allow_internet_access=True, **sdk_options)
        identity = save_identity(root, sandbox, identities, services)
        assert process_start_ticks(identity["pid"]) == identity["start_time_ticks"]
        assert Sandbox.connect(sandbox.sandbox_id, **sdk_options).sandbox_id == sandbox.sandbox_id
        assert sandbox.get_info().state == "running"
        assert any(item.sandbox_id == sandbox.sandbox_id for item in Sandbox.list(**sdk_options).next_items())
        Sandbox.set_timeout(sandbox.sandbox_id, 86400, **sdk_options)
        sandbox.commands.run("mkdir -p /app /tests /logs/verifier", cwd="/", timeout=30)
        prepared = sandbox.commands.run("test \"$(id -u)\" = 0 && grep -qx 'VERSION_ID=\"24.04\"' /etc/os-release && test ! -e " + pin["initial_absent"], cwd="/app", timeout=30)
        assert prepared.exit_code == 0
        profile = sandbox.commands.run("nproc; awk '/MemTotal/ {print $2}' /proc/meminfo; df -B1 --output=size / | tail -1", cwd="/app", timeout=30).stdout.splitlines()
        assert int(profile[0]) == 1 and int(profile[1]) >= 2_000_000 and int(profile[2]) >= 9 * GIB, profile
        # The prebuilt base has the host's 127.0.0.53 stub, which is not a
        # resolver inside the VM. Keep one reviewed DNS endpoint for the run.
        sandbox.commands.run("printf 'nameserver 1.1.1.1\\n' > /etc/resolv.conf && getent ahostsv4 archive.ubuntu.com >/dev/null", cwd="/app", timeout=30)
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
        stage_verifier_python(sandbox, verifier_python)
        sandbox.files.write("/tests/test.sh", (directory / "tests/test.sh").read_bytes())
        sandbox.files.write("/tests/test_outputs.py", (directory / "tests/test_outputs.py").read_bytes())
        scoring = sandbox.commands.run("bash /tests/test.sh", cwd="/app", timeout=int(entry["requirements"]["verifier_timeout_sec"]))
        (root / f"{name}.score.stdout").write_text(scoring.stdout)
        (root / f"{name}.score.stderr").write_text(scoring.stderr)
        reward = sandbox.files.read("/logs/verifier/reward.txt").strip()
        assert reward == "1", f"{name}: original scoring gave {reward!r}"
        assert sandbox.files.read("/logs/verifier/ctrf.json"), "original CTRF report missing"
        return {"name": name, "source_files_sha256": entry["files_sha256"], "reward": reward,
                "guest_versions": version, "guest_profile": profile, "sandbox_id": sandbox.sandbox_id,
                "native_id": identity['native_id'],
                "original_verifier_sha256": next(item["sha256"] for item in entry["files"] if item["path"] == "tests/test.sh")}
    finally:
        if sandbox is not None and identity is not None:
            _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **sdk_options))
            assert _kill_confirmed, f"{name}: kill failed"
        elif sandbox is not None:
            print(f"{name}: original identity not saved; exact live sandbox retained: {sandbox.sandbox_id}", file=sys.stderr)


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on isolated Linux/KVM host")
    if not 1000 <= args.uid_base < 2**32 - 1:
        raise RuntimeError("invalid dedicated physical-slot UID")
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
    if processes_in_uid_range(args.uid_base, args.uid_base):
        raise RuntimeError("dedicated physical-slot UID already occupied")
    cgroup_parent = args.cgroup_parent.resolve(strict=True)
    if (cgroup_parent / "cgroup.procs").read_text().strip() or any(path.is_dir() for path in cgroup_parent.iterdir()):
        raise RuntimeError("delegated cgroup parent must be empty")
    if args.rootfs.stat().st_size != 10 * GIB:
        raise RuntimeError("selected tasks require a full 10 GiB rootfs")
    if shutil.disk_usage(parent).free < args.rootfs.stat().st_size * (len(selection) + 2) + GIB:
        raise RuntimeError("not enough disk for private base and fresh task VMs")
    verifier_python = args.verifier_python.resolve(strict=True)
    if sha256_file(verifier_python) != VERIFIER_PYTHON_SHA256:
        raise RuntimeError("verifier Python archive differs from pinned SHA-256")
    source_base_sha = sha256_file(args.rootfs)
    root = Path(tempfile.mkdtemp(prefix="sp13-", dir=parent))
    namespace = secrets.token_hex(8)
    firecracker = args.firecracker.resolve(strict=True)
    if len(os.fsencode(root / "p/v" / firecracker.name / f"vm-{namespace}-{2**64 - 1}" / "root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("Firecracker socket path too long")
    for name in ("db", "state", "p"):
        (root / name).mkdir(mode=0o700)
    kernel, rootfs = root / "vmlinux", root / "rootfs.ext4"
    for source_file, destination in ((args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source_file.resolve(strict=True), destination)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    assert sha256_file(rootfs) == source_base_sha
    key = secrets.token_hex(32)
    port = free_loopback_port()
    template = descriptor('runtime-base', kernel, rootfs, 1, 2048)
    template['resources']['host_memory_bytes'] = 2304 * MIB
    catalog = dict(templates=[template], aliases={'base': template['template_id']}, compute_limits=dict(
        max_vcpus_global=1, max_vcpus_per_tenant=1,
        max_memory_bytes_global=2304*MIB, max_memory_bytes_per_tenant=2304*MIB))
    settings = complete_runtime({
        "namespace": namespace, "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"), "jailer": str(args.jailer.resolve(strict=True)),
        "firecracker": str(firecracker), "chroot_base": str(root / "p/v"),
        "kernel": str(kernel), "rootfs": str(rootfs), "uid_base": args.uid_base,
        "vcpu_count": 1, "memory_mib": 2048,
        "host_resources": {"cgroup_parent": str(cgroup_parent), "memory_max_bytes": 2304 * MIB},
        "file_policy_digest": 1, "templates": catalog,
        "max_active_global": 1, "max_active_per_tenant": 1,
        "max_disk_bytes_global": 2 * rootfs.stat().st_size,
        "max_disk_bytes_per_tenant": rootfs.stat().st_size,
        "cleanup_reserve_bytes": rootfs.stat().st_size, "max_timeout_secs": 86400,
        "max_lifetime_secs": 86400, "default_timeout_secs": 300,
        "expiry_scan_seconds": 1, "public_ipv4_enabled": True, "startup_timeout_secs": 300,
    }, slots=1)
    registry = dict(root=str(root / 'registry'), max_templates_global=1, max_templates_per_tenant=1,
        max_artifact_bytes_global=rootfs.stat().st_size+kernel.stat().st_size,
        max_artifact_bytes_per_tenant=rootfs.stat().st_size+kernel.stat().st_size, validation_timeout_secs=60)
    services = RuntimeServices.prepare(args, root, [settings], [catalog], key,
        f'127.0.0.1:{port}', args.service_cgroup_record, job_count=len(selection)+2, registry_config=registry)
    completed = False
    results = []
    identities = []
    try:
        services.start()
        base_url = f"http://127.0.0.1:{port}"
        services.wait_ready()
        for entry, pin in selection:
            result = run_original_task(entry, pin, args.source, base_url, key, root, verifier_python, identities, services)
            results.append(result)
            wait_destroyed(root / "db/controller.db", result["native_id"])
            no_exact_residual(root, namespace, args.uid_base, identities, firecracker, cgroup_parent)
            assert sha256_file(rootfs) == source_base_sha, "private base changed"
        assert sha256_file(args.rootfs) == source_base_sha, "source base changed"
        completed = True
        print(json.dumps({"commit": commit, "host_kernel": os.uname().release,
                          "center_sha256": sha256_file(args.center), "host_sha256": sha256_file(args.host),
                          "base_sha256": source_base_sha,
                          "source_commit": json.loads((checkout / "benchmarks/diagnostic-tasks.json").read_text())["source_commit"],
                          "memory_mib": 2048, "disk_mib": 10240, "vcpu_count": 1,
                          "verifier_python_sha256": VERIFIER_PYTHON_SHA256,
                          "network": "public IPv4 fixed for whole trial; guest DNS 1.1.1.1", "tasks": results,
                          "residual": "each exact VM terminal cleanup confirmed; saved PID/start/netns inode gone; no jail, netns, cgroup, physical-slot UID; base unchanged",
                          "scope": "selected original oracle/verifier chain, not full E2B equivalence or 89-task score"}))
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
    for field in ("jailer", "firecracker", "kernel", "rootfs", "source", "scratch-parent", "verifier-python", "cgroup-parent"):
        parser.add_argument("--" + field, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--task", action="append", choices=("regex-log", "polyglot-c-py"), required=True)
    exercise(parser.parse_args())
