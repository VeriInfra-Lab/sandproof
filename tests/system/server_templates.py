"""Trusted multi-template SDK/KVM acceptance, without a builder or paid model.

Use an isolated host, two approved prebuilt images, an empty delegated cgroup
parent and an exclusive UID range. Failure retains exact private artifacts.
"""

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

from e2b import Sandbox, SandboxQuery

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import (
    fetch, free_loopback_port, process_start_ticks, start_server, stop_server, wait_ready,
)
from server_vm_id_capacity import check_no_resources, durable_snapshot


MIB = 1024 * 1024


def descriptor(ident, kernel, rootfs, cpu, memory):
    return {
        "template_id": ident, "revision": "r1", "owner": 1,
        "kernel": {"path": str(kernel), "sha256": sha256_file(kernel), "size_bytes": kernel.stat().st_size},
        "rootfs": {"path": str(rootfs), "sha256": sha256_file(rootfs), "size_bytes": rootfs.stat().st_size},
        "resources": {"cpu_count": cpu, "memory_mib": memory,
                      "disk_bytes": rootfs.stat().st_size, "host_memory_bytes": (memory + 256) * MIB},
        "envd_version": "0.9.0",
    }


def record_for(root, sandbox):
    suffix = sandbox.sandbox_id.rsplit("-", 1)[1]
    secret = json.loads((root / "state/secrets" / sandbox.sandbox_id).read_text())
    slots = [json.loads(path.read_text()) for path in (root / "state/slots").glob("*.json")]
    slot = next(value for value in slots if value["sandbox"] == int(suffix))
    name = f"vm-{sandbox.sandbox_id.split('-')[1]}-{slot['vm']}"
    claim = json.loads((root / "jails/.claims" / name / "record.json").read_text())
    return secret, name, claim


def assert_shape(root, firecracker, cgroup_parent, sandbox, expected, opener, url, key):
    info = sandbox.get_info()
    shape = expected["resources"]
    assert (info.template_id, info.cpu_count, info.memory_mb) == (
        expected["template_id"], shape["cpu_count"], shape["memory_mib"],
    )
    status, body = fetch(opener, url, "GET", f"/sandboxes/{sandbox.sandbox_id}", key)
    assert status == 200
    # The pinned SDK SandboxInfo intentionally omits diskSizeMB; check the
    # actual management response instead of inventing a Python SDK attribute.
    assert json.loads(body)["diskSizeMB"] == shape["disk_bytes"] // MIB
    secret, name, claim = record_for(root, sandbox)
    assert secret["template"] == expected
    assert process_start_ticks(claim["pid"]) == claim["start_time_ticks"]
    assert (root / "jails" / firecracker.name / name / "root/rootfs.ext4").stat().st_size == shape["disk_bytes"]
    cgroup = cgroup_parent / name
    assert int((cgroup / "memory.max").read_text()) == shape["host_memory_bytes"]
    assert (cgroup / "cpu.max").read_text().strip() == f"{shape['cpu_count'] * 100000} 100000"
    assert str(claim["pid"]) in (cgroup / "cgroup.procs").read_text().split()
    assert int(sandbox.commands.run("getconf _NPROCESSORS_ONLN").stdout.strip()) == shape["cpu_count"]
    return claim


def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    assert importlib.metadata.version("e2b") == "2.51.0"
    if processes_in_uid_range(args.uid_base, args.uid_base + 2):
        raise RuntimeError("exclusive test UID range occupied")
    if args.uid_base < 1000 or args.uid_base + 2 >= 2**32 - 1:
        raise RuntimeError("invalid exclusive test UID range")
    parent = args.scratch_parent.resolve(strict=True)
    if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
        raise RuntimeError("scratch parent must be private/root-owned")
    if (args.cgroup_parent / "cgroup.procs").read_text().strip() or any(path.is_dir() for path in args.cgroup_parent.iterdir()):
        raise RuntimeError("cgroup parent must be empty")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("test checkout must be clean")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    original_digests = [sha256_file(path) for path in (args.kernel, args.rootfs, args.second_rootfs)]
    disk = args.rootfs.stat().st_size + args.second_rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk * 3 + 256 * MIB:
        raise RuntimeError("insufficient disk including cleanup reserve")
    root = Path(tempfile.mkdtemp(prefix="tm-", dir=parent))
    namespace = secrets.token_hex(8)
    firecracker = args.firecracker.resolve(strict=True)
    if len(os.fsencode(root / "jails" / firecracker.name / f"vm-{namespace}-2/root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("Firecracker socket path too long")
    for name in ("db", "state", "jails", "images"):
        (root / name).mkdir(mode=0o700)
    binary = root / "sandproof-server"
    shutil.copyfile(args.server.resolve(strict=True), binary)
    binary.chmod(0o700)
    kernel, first_image, second_image = (root / "images" / name for name in ("vmlinux", "small.ext4", "large.ext4"))
    for source, destination in ((args.kernel, kernel), (args.rootfs, first_image), (args.second_rootfs, second_image)):
        subprocess.run(["/usr/bin/cp", "--reflink=auto", "--sparse=always", str(source), str(destination)], check=True)
        destination.chmod(0o444)
    templates = [descriptor("small-r1", kernel, first_image, 1, 256), descriptor("large-r1", kernel, second_image, 2, 512)]
    key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    url = f"http://127.0.0.1:{free_loopback_port()}"
    database = root / "db/controller.db"
    settings = {
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)), "firecracker": str(firecracker),
        "chroot_base": str(root / "jails"), "kernel": str(args.kernel.resolve(strict=True)),
        "rootfs": str(args.rootfs.resolve(strict=True)), "uid_base": args.uid_base, "max_vm_id": 2,
        "vcpu_count": 1, "memory_mib": 256,
        "host_resources": {"cgroup_parent": str(args.cgroup_parent), "memory_max_bytes": 512 * MIB},
        "file_policy_digest": 1, "bind": url.removeprefix("http://"), "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": key}, {"tenant": 2, "secret": foreign_key}],
        "max_active_global": 5, "max_active_per_tenant": 5,
        "max_disk_bytes_global": disk * 4, "max_disk_bytes_per_tenant": disk * 4,
        "cleanup_reserve_bytes": 256 * MIB, "max_timeout_secs": 600, "expiry_scan_seconds": 3600,
        "templates": {"templates": templates, "aliases": {"chosen": "small-r1"},
                      "compute_limits": {"max_vcpus_global": 3, "max_vcpus_per_tenant": 3,
                                         "max_memory_bytes_global": 1280 * MIB, "max_memory_bytes_per_tenant": 1280 * MIB}},
    }
    config = root / "config.json"
    def save_config():
        config.write_text(json.dumps(settings))
        config.chmod(0o600)
    save_config()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner = options(key, url, url)
    process = None
    completed = False
    try:
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        for token, name, expected in ((foreign_key, "chosen", 404), (key, "missing", 404)):
            before = durable_snapshot(database)
            assert fetch(opener, url, "POST", "/v2/sandboxes", token, {"templateID": name, "allow_internet_access": False})[0] == expected
            assert durable_snapshot(database) == before
        small = Sandbox.create(template="chosen", timeout=300, allow_internet_access=False, **owner)
        large = Sandbox.create(template="large-r1", timeout=300, allow_internet_access=False, **owner)
        original_claims = [assert_shape(root, firecracker, args.cgroup_parent, sandbox, template, opener, url, key) for sandbox, template in zip((small, large), templates)]
        for sandbox, marker in ((small, "small-private"), (large, "large-private")):
            sandbox.files.write("/tmp/template-marker", marker)
        assert small.files.read("/tmp/template-marker") == "small-private"
        assert large.files.read("/tmp/template-marker") == "large-private"
        listing = Sandbox.list(query=SandboxQuery(template="large-r1"), **owner).next_items()
        assert [item.sandbox_id for item in listing] == [large.sandbox_id]
        before = durable_snapshot(database)
        assert fetch(opener, url, "POST", "/v2/sandboxes", key, {"templateID": "chosen", "allow_internet_access": False})[0] == 429
        assert durable_snapshot(database) == before
        stop_server(process)
        settings["templates"]["aliases"]["chosen"] = "large-r1"
        save_config()
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        assert [assert_shape(root, firecracker, args.cgroup_parent, sandbox, template, opener, url, key) for sandbox, template in zip((small, large), templates)] == original_claims
        assert small.files.read("/tmp/template-marker") == "small-private"
        assert Sandbox.connect(small.sandbox_id, **owner).get_info().template_id == "small-r1"
        # Validate the original artifact in this new service process, then
        # refuse on its persisted compute charge. This also warms the cache
        # whose invalidation is checked by the following mutation.
        before = durable_snapshot(database)
        assert fetch(opener, url, "POST", "/v2/sandboxes", key, {"templateID": "small-r1", "allow_internet_access": False})[0] == 429
        assert durable_snapshot(database) == before
        assert Sandbox.kill(large.sandbox_id, **owner)
        # Changing an artifact after its first cached validation must invalidate
        # that cache and fail before admission, without affecting the live VM.
        with first_image.open("r+b") as image:
            original_byte = image.read(1)
            image.seek(0)
            image.write(bytes([original_byte[0] ^ 1]))
            image.flush()
            os.fsync(image.fileno())
        before = durable_snapshot(database)
        assert fetch(opener, url, "POST", "/v2/sandboxes", key, {"templateID": "small-r1", "allow_internet_access": False})[0] == 503
        assert durable_snapshot(database) == before
        with first_image.open("r+b") as image:
            image.write(original_byte)
            image.flush()
            os.fsync(image.fileno())
        # The original small image is no longer available. It must not prevent
        # restart/observation/stop of its already-running private VM.
        first_image.rename(first_image.with_suffix(".unavailable"))
        stop_server(process)
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        assert small.commands.run("printf old-binding-ok").stdout == "old-binding-ok"
        assert Sandbox.kill(small.sandbox_id, **owner)
        # Same immutable ID with a different shape must fail without admitting
        # a third VM. Alias retargeting to a new ID is allowed.
        stop_server(process)
        settings["templates"]["templates"][1]["resources"]["cpu_count"] = 1
        save_config()
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        before = durable_snapshot(database)
        assert fetch(opener, url, "POST", "/v2/sandboxes", key, {"templateID": "chosen", "allow_internet_access": False})[0] == 400
        assert durable_snapshot(database) == before
        stop_server(process)
        settings["templates"]["templates"][1]["resources"]["cpu_count"] = 2
        save_config()
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        replacement = Sandbox.create(template="chosen", timeout=300, allow_internet_access=False, **owner)
        assert_shape(root, firecracker, args.cgroup_parent, replacement, templates[1], opener, url, key)
        assert not replacement.files.exists("/tmp/template-marker")
        assert Sandbox.kill(replacement.sandbox_id, **owner)
        check_no_resources(root, namespace, firecracker, args.uid_base)
        with sqlite3.connect(database) as connection:
            assert connection.execute("SELECT version FROM controller_meta").fetchone()[0] == 8
            assert len(connection.execute("SELECT * FROM sandbox_create_options").fetchall()) == 3
        assert original_digests == [sha256_file(path) for path in (args.kernel, args.rootfs, args.second_rootfs)]
        assert sha256_file(first_image.with_suffix(".unavailable")) == templates[0]["rootfs"]["sha256"]
        assert sha256_file(second_image) == templates[1]["rootfs"]["sha256"]
        completed = True
        print(json.dumps({"source_commit": commit, "server_sha256": sha256_file(binary),
                          "probe_sha256": sha256_file(Path(__file__)), "host_kernel": os.uname().release,
                          "sdk": "2.51.0", "templates": templates, "vms_destroyed": 3,
                          "result": "shapes/private files/owner/quota/immutable IDs/alias restart/source loss/precise cleanup passed",
                          "scope": "trusted prebuilt inputs only; no Dockerfile/OCI builder, self-service registry, public networking or proofs"}))
    finally:
        stop_server(process)
        if completed:
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("server", "jailer", "firecracker", "kernel", "rootfs", "second-rootfs", "scratch-parent", "cgroup-parent"):
        parser.add_argument(f"--{argument}", type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    exercise(parser.parse_args())
