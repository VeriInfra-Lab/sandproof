"""Manual real Redis/mTLS/original-host KVM smoke; no public service or paid model.

Requires an independently reviewed exact commit and root-protected artifacts.
Uses the installed CI host lease; unknown cleanup retains the exclusive marker.
No global process killing, network changes, image downloads or fixture rewrites.
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
import runpy
import secrets
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from tools.check_kvm_host import trusted_path, source_revision, device
from tools.system_acceptance import digest, json_file, write_once
from _complete_runtime import complete_runtime, provision_runtime, _envelope
from _resource_evidence import record_resource_identity
from sdk_c02_acceptance import assert_no_exact_resources, processes_in_uid_range


def approved(path, expected):
    path = trusted_path(Path(path))
    if digest(path) != expected:
        raise RuntimeError("approved artifact fingerprint changed")
    return path


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_file(path, child, seconds):
    deadline = time.monotonic() + seconds
    while not path.exists():
        if child.poll() is not None:
            raise RuntimeError("acceptance child exited before expected durable observation")
        if time.monotonic() >= deadline:
            raise RuntimeError("acceptance observation deadline; no successful cleanup inferred")
        time.sleep(0.1)


def launch(binary, role, config, output, *, uid=0):
    # Child environment contains no caller credentials/proxies or GitHub token.
    return subprocess.Popen([str(binary), role, str(config)], stdin=subprocess.DEVNULL,
                            stdout=output, stderr=subprocess.STDOUT,
                            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"},
                            user=uid, group=uid, extra_groups=[])


def prepare(args, installed, root, shared):
    manifest = json_file(approved(installed["ci_fixtures"], installed["host_assets"][installed["ci_fixtures"]]))
    fixture = manifest["fixtures"][0]
    kernel = approved(fixture["kernel"]["path"], fixture["kernel"]["sha256"])
    rootfs = approved(fixture["rootfs"]["path"], fixture["rootfs"]["sha256"])
    fc = approved(installed["firecracker"], installed["host_assets"][installed["firecracker"]])
    jailer = approved(installed["jailer"], installed["host_assets"][installed["jailer"]])
    relay = approved(args.relay, args.relay_sha256)
    binary = approved(args.binary, args.binary_sha256)
    redis = approved(args.redis_server, args.redis_server_sha256)
    namespace = secrets.token_hex(8)
    uid_base = installed["uid_base"]
    if processes_in_uid_range(uid_base, uid_base + 1):
        raise RuntimeError("test physical UID range already occupied")
    for name in ("db", "state", "jails", "tls", "resource-evidence"):
        (root / name).mkdir(mode=0o700)
    disk = rootfs.stat().st_size
    shape = fixture["resources"]
    cpu, memory = shape["cpus"], shape["memory_mib"]
    host_memory = (memory + 256) * 1024**2
    settings = complete_runtime({
        "namespace": namespace, "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"), "chroot_base": str(root / "jails"),
        "kernel": str(kernel), "rootfs": str(rootfs), "firecracker": str(fc), "jailer": str(jailer),
        "uid_base": uid_base, "max_active_global": 1, "max_active_per_tenant": 1,
        "max_disk_bytes_global": disk * 2, "max_disk_bytes_per_tenant": disk,
        "cleanup_reserve_bytes": disk, "file_policy_digest": 1,
    }, slots=1)
    peak, _ = _envelope(settings["bounded_host"]["physical_storage"])
    if peak * 2 > 200 * 1024**3:
        raise RuntimeError("minimum smoke exceeds approved total 200 GiB free-space ceiling")
    provision_runtime(settings)
    bounded = {**settings["bounded_host"], "max_data_routes_per_slot": 4,
               "max_data_routes_global": 8, "data_route_control_reserve": 1}
    node_identity = {"deployment_id": "smoke-" + namespace, "node_id": "host-a", "node_generation": 1}
    template = {
        "template_id": "kvm-smoke", "revision": "fixed", "owner": None,
        "kernel": {"path": str(kernel), "sha256": fixture["kernel"]["sha256"], "size_bytes": kernel.stat().st_size},
        "rootfs": {"path": str(rootfs), "sha256": fixture["rootfs"]["sha256"], "size_bytes": disk},
        "resources": {"cpu_count": cpu, "memory_mib": memory, "disk_bytes": disk, "host_memory_bytes": host_memory},
        "envd_version": "fixed-ci", "startup": fixture["startup"],
    }
    import hashlib
    template_digest = list(hashlib.sha256(json.dumps(template, sort_keys=True, separators=(",", ":")).encode()).digest())
    center_root = shared / "center"
    redis_root = shared / "redis"
    bin_root = shared / "bin"
    for directory, uid, mode in ((center_root, 65534, 0o700), (redis_root, 65533, 0o700), (bin_root, 0, 0o755)):
        directory.mkdir(mode=mode)
        os.chown(directory, uid, uid)
    for source, name, expected in ((binary, "kvm_lifecycle", args.binary_sha256), (redis, "redis-server", args.redis_server_sha256)):
        destination = bin_root / name
        shutil.copyfile(source, destination)
        destination.chmod(0o755)
        approved(destination, expected)
    binary = bin_root / "kvm_lifecycle"
    subprocess.run([str(binary), "certificates", str(root / "tls")], check=True, timeout=10,
                   env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"})
    tls_root = center_root / "tls"
    tls_root.mkdir(mode=0o700)
    os.chown(tls_root, 65534, 65534)
    for name in ("ca.der", "host.der", "center.der", "center.key"):
        shutil.copyfile(root / "tls" / name, tls_root / name)
        (tls_root / name).chmod(0o600)
        os.chown(tls_root / name, 65534, 65534)
    # The host never needs the center's private key once material is copied.
    (root / "tls/center.key").unlink()
    host_port, redis_port = port(), port()
    if host_port == redis_port:
        raise RuntimeError("independent test ports required")
    config = {
        "node": {"namespace": namespace, "state_dir": str(root / "state"), "jailer": str(jailer),
                 "firecracker": str(fc), "chroot_base": str(root / "jails"), "kernel": str(kernel),
                 "rootfs": str(rootfs), "bounded_host": bounded, "vcpu_count": cpu, "memory_mib": memory,
                 "host_resources": {"cgroup_parent": installed["cgroup_parent"], "memory_max_bytes": host_memory},
                 "file_policy_digest": 1},
        "maintenance": settings["maintenance"], "database": settings["database"],
        "identity": node_identity, "template": template, "template_digest": template_digest,
        "template_revision": "fixed-template", "tls": str(root / "tls"), "bind": f"127.0.0.1:{host_port}",
        "relay": str(relay),
    }
    write_once(root / "host.json", config)
    # Reuse the original physical-identity recorder and cleanup observer.
    write_once(root / "config.json", {**settings, "bounded_host": bounded})
    password = secrets.token_hex(32)
    binding = {"template_revision": "fixed-template", "template_digest": template_digest,
               "resources": {"instances": 1, "cpu": cpu, "memory_bytes": host_memory, "disk_bytes": disk},
               "options": list(json.dumps({"env_vars": {}, "metadata": {}, "networking": {"allow_internet_access": False, "network": None}}, separators=(",", ":")).encode()),
               "startup_deadline_ms": 0, "timeout_ms": 300000, "max_lifetime_ms": 600000}
    write_once(center_root / "config.json", {"identity": node_identity, "binding": binding, "redis": f"127.0.0.1:{redis_port}", "password": password, "tls": str(tls_root), "host": f"127.0.0.1:{host_port}", "root": str(center_root)})
    os.chown(center_root / "config.json", 65534, 65534)
    redis_config = (f"bind 127.0.0.1\nport {redis_port}\nprotected-mode yes\nrequirepass {password}\n"
                    "appendonly yes\nappendfsync always\nmaxmemory 67108864\nmaxmemory-policy noeviction\n"
                    "no-appendfsync-on-rewrite no\naof-load-truncated no\nauto-aof-rewrite-percentage 0\nsave \"\"\n"
                    f"dir {redis_root}\nlogfile {redis_root / 'redis.log'}\n")
    descriptor = os.open(redis_root / "redis.conf", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        stream.write(redis_config)
    os.chown(redis_root / "redis.conf", 65533, 65533)
    return binary, settings, center_root, redis_root, redis_port, template


def remove_test_credential(path, uid):
    """After all owned children exit, remove only the exact private test file."""
    try:
        before = path.lstat()
    except FileNotFoundError:
        return
    if (not stat.S_ISREG(before.st_mode) or before.st_uid != uid
            or before.st_mode & 0o077 or before.st_nlink != 1):
        raise RuntimeError("test credential identity changed; retained")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        opened = os.fstat(descriptor)
        current = path.lstat()
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino) or (current.st_dev, current.st_ino) != (before.st_dev, before.st_ino):
            raise RuntimeError("test credential identity changed; retained")
        path.unlink()
    finally:
        os.close(descriptor)


def run(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("real lifecycle acceptance requires isolated root Linux/KVM")
    if source_revision()["revision"] != args.expected_commit:
        raise RuntimeError("reviewed candidate commit mismatch")
    device(Path("/dev/kvm"), kvm=True)
    installed = json_file(trusted_path(args.host_config))
    # Import the installed, already reviewed exclusivity protocol, not a new
    # marker implementation. Never change supervisor policy or cancel CI jobs.
    control = trusted_path(Path(installed["control_checkout"]))
    sys.path.insert(0, str(control))
    lease_module = runpy.run_path(str(control / "tools/ci_e2e_supervisor.py"))
    lease_identity = secrets.token_hex(16)
    lease = lease_module["HostLease"](installed["ci_state_root"], lease_identity)
    root = Path(tempfile.mkdtemp(prefix="m.", dir=installed["scratch_parent"]))
    # A dedicated traversable parent; do not relax the existing private CI
    # installation directory to make the unprivileged processes work.
    shared_parent = Path("/opt/sandproof-min-runs")
    shared_parent.mkdir(mode=0o755, exist_ok=True)
    trusted_path(shared_parent)
    shared = Path(tempfile.mkdtemp(prefix="run-", dir=shared_parent))
    shared.chmod(0o711)
    report = {"commit": args.expected_commit, "scope": "real Redis + mTLS + original host create/destroy",
              "cleanup": "unknown", "result": "failed", "resources": "unknown",
              "host_lease_identity": lease_identity}
    children = []
    paused = False
    safe = False
    host_started = False
    try:
        lease.pause()
        paused = True
        if not lease.exclusive():
            lease.release()
            paused = False
            raise RuntimeError("host occupied; no KVM test executed")
        binary, settings, center_root, redis_root, redis_port, template = prepare(args, installed, root, shared)
        with (root / "redis-child.log").open("w") as redis_output, (root / "host.log").open("w") as host_output, (root / "center.log").open("w") as center_output:
            redis = subprocess.Popen([str(shared / "bin/redis-server"), str(redis_root / "redis.conf")], stdin=subprocess.DEVNULL, stdout=redis_output, stderr=subprocess.STDOUT, user=65533, group=65533, extra_groups=[], env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"})
            children.append(redis)
            deadline = time.monotonic() + 10
            while True:
                if redis.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("fixed Redis engine did not start")
                try:
                    with socket.create_connection(("127.0.0.1", redis_port), timeout=0.2):
                        break
                except OSError:
                    time.sleep(0.1)
            host = launch(binary, "host", root / "host.json", host_output)
            children.append(host)
            host_started = True
            wait_file(root / "tls/listening", host, 60)
            center = launch(binary, "center", center_root / "config.json", center_output, uid=65534)
            children.append(center)
            wait_file(center_root / "ready.json", center, 180)
            os.environ["SANDPROOF_TEST_EVIDENCE_ROOT"] = str(root)
            record_resource_identity("sb-" + settings["namespace"] + "-1")
            os.environ.pop("SANDPROOF_TEST_EVIDENCE_ROOT", None)
            report["live_identity"] = "exact PID/start ticks/UID/network identity observed"
            write_once(center_root / "release", {})
            os.chown(center_root / "release", 65534, 65534)
            if center.wait(timeout=180) != 0:
                raise RuntimeError("center create/destroy assertions failed")
            wait_file(root / "tls/cleaned", host, 15)
            if host.wait(timeout=15) != 0:
                raise RuntimeError("original host failed after cleanup")
            report["observed_instances"] = assert_no_exact_resources(root, settings["namespace"], settings["uid_base"], Path(settings["firecracker"]).name, expected=1)
            for item in (template["kernel"], template["rootfs"]):
                approved(item["path"], item["sha256"])
            report.update(result="passed", resources="confirmed absent", cleanup="confirmed", template="unchanged", accounts="center 65534 / Redis 65533 / host root")
            safe = True
    finally:
        # Child PIDs are owned Popen handles. Do not stop host/center while VM
        # responsibility is unknown; original local deadline remains active.
        if safe or not host_started:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    child.wait(timeout=15)
            for path, uid in ((root / "tls/host.key", 0), (root / "tls/center.key", 0),
                              (shared / "center/tls/center.key", 65534),
                              (shared / "center/config.json", 65534),
                              (shared / "redis/redis.conf", 65533)):
                remove_test_credential(path, uid)
            report["test_credentials"] = "removed"
            if not host_started:
                report["cleanup"] = "no VM execution attempted"
            if paused:
                lease.release()
        report["private_evidence"] = str(root)
        report["private_shared"] = str(shared)
        write_once(root / "report.json", report)
        print(json.dumps(report))
    if not safe:
        raise RuntimeError("acceptance incomplete; private evidence and host lease retained")


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host-config", type=Path, required=True)
    parser.add_argument("--expected-commit", required=True)
    for name in ("binary", "redis-server", "relay"):
        parser.add_argument("--" + name, type=Path, required=True)
        parser.add_argument("--" + name + "-sha256", required=True)
    return parser.parse_args(argv)


def main():
    run(arguments())


if __name__ == "__main__":
    main()
