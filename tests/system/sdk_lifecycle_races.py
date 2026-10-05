"""One real VM: creation inputs, two synchronized kills, exact cleanup.

Run only as root on an isolated Linux/KVM host. The debug server's one-shot
Unix stop gate makes the second DELETE overlap the first accepted kill without
sleep-based race detection. On failure, retain only this run's exact resources.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import http.client
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import sqlite3
import sys
import tempfile
import threading
import time
import urllib.request

from e2b import Sandbox

from server_process_crash import (
    fetch, free_loopback_port, process_start_ticks, start_server, stop_server,
    wait_ready,
)


def second_delete(port: int, key: str, sandbox_id: str, sent: threading.Event) -> int:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=40)
    try:
        connection.connect()
        connection.putrequest("DELETE", f"/sandboxes/{sandbox_id}")
        connection.putheader("X-API-Key", key)
        connection.endheaders()
        sent.set()  # request is on a distinct socket while StopVm is gated
        response = connection.getresponse()
        response.read()
        return response.status
    finally:
        connection.close()


def wait_for_socket(path: Path, process: object) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        assert process.poll() is None, "server exited before stop synchronization"
        if path.is_socket():
            return
        time.sleep(0.02)  # only polls the explicit synchronization socket
    raise AssertionError("first kill never reached the durable stop gate")


def sha256_file(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    assert importlib.metadata.version("e2b") == "2.51.0"
    parent = args.scratch_parent.resolve(strict=True)
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError("insufficient room for private disk and cleanup")
    root = Path(tempfile.mkdtemp(prefix="s7-", dir=parent))
    namespace = secrets.token_hex(8)
    vm_name = f"vm-{namespace}-0"
    longest_socket = root / "jails" / args.firecracker.name / f"vm-{namespace}-1000" / "root/api.socket"
    if len(os.fsencode(longest_socket)) >= 108:
        root.rmdir()  # nothing has been written under this exact new root
        raise RuntimeError("dedicated test root is too long for Firecracker API socket")
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
        "max_disk_bytes_global": disk_bytes * 4,
        "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "default_timeout_secs": 300,
        "expiry_scan_seconds": 1,
    }))
    config.chmod(0o600)
    gate = root / "stop-gate.sock"
    log = root / "server.log"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{port}"
    process = None
    completed = False
    try:
        process = start_server(binary, config, log, None, gate)
        wait_ready(opener, base_url, key, process)
        sdk = {
            "api_url": base_url,
            "sandbox_url": base_url,
            "api_key": key,
            "domain": "example.invalid",
            "retries": 0,
            "request_timeout": 40,
        }
        sandbox = Sandbox.create(
            template="base", timeout=180, allow_internet_access=False,
            envs={"SP7_ENV": "lifecycle-ok"},
            metadata={"job": "issue-7"}, **sdk,
        )
        assert sandbox.commands.run('printf %s "$SP7_ENV"').stdout == "lifecycle-ok"
        assert sandbox.get_info().metadata == {"job": "issue-7"}
        listed = Sandbox.list(**sdk).next_items()
        assert len(listed) == 1 and listed[0].metadata == {"job": "issue-7"}

        record = json.loads((root / "jails/.claims" / vm_name / "record.json").read_text())
        pid, ticks = record["pid"], record["start_time_ticks"]
        assert process_start_ticks(pid) == ticks
        with ThreadPoolExecutor(max_workers=2) as pool:
            first = pool.submit(
                fetch, opener, base_url, "DELETE",
                f"/sandboxes/{sandbox.sandbox_id}", key,
            )
            wait_for_socket(gate, process)
            sent = threading.Event()
            second = pool.submit(second_delete, port, key, sandbox.sandbox_id, sent)
            assert sent.wait(timeout=10), "second kill never sent its HTTP request"
            with socket.socket(socket.AF_UNIX) as release:
                release.connect(str(gate))
                release.sendall(b"g")
            assert first.result(timeout=40)[0] == 204
            assert second.result(timeout=40) == 404

        assert fetch(opener, base_url, "GET", f"/sandboxes/{sandbox.sandbox_id}", key)[0] == 404
        with sqlite3.connect(root / "db/controller.db") as database:
            inputs = [row[0] for row in database.execute("SELECT input_json FROM controller_inputs")]
        assert sum(b'"Kill"' in row for row in inputs) == 1, "second kill was accepted"
        assert process_start_ticks(pid) != ticks, "original VM still runs or was reused"
        assert not Path(f"/run/netns/{vm_name}").exists(), "private network remains"
        assert not (root / "jails" / args.firecracker.name / vm_name).exists(), "private jail remains"
        assert not gate.exists(), "test gate remains"

        # A rejected request must never reach admission or physical work.
        with sqlite3.connect(root / "db/controller.db") as database:
            before_reject = database.execute("SELECT count(*) FROM sandboxes").fetchone()[0]
        rejected, _ = fetch(opener, base_url, "POST", "/v2/sandboxes", key, {
            "templateID": "base", "envVars": {"BAD=NAME": "rejected"},
            "allowInternetAccess": False,
        })
        assert rejected == 400, f"malformed create was not rejected: {rejected}"
        with sqlite3.connect(root / "db/controller.db") as database:
            assert database.execute("SELECT count(*) FROM sandboxes").fetchone()[0] == before_reject

        # Remove only this run's private kernel copy before the next create.
        # No VMM can start; the uncertain PrepareVm is reconciled against an
        # absent exact resource after the kernel is restored and service restarts.
        held_kernel = root / "kernel-held"
        kernel.rename(held_kernel)
        try:
            Sandbox.create(template="base", timeout=180, allow_internet_access=False, **sdk)
        except Exception as failure:
            assert "timeout" not in type(failure).__name__.lower(), failure
        else:
            raise AssertionError("missing private kernel was published as Running")
        assert process.poll() is None, "controller crashed during failed create"
        held_kernel.rename(kernel)
        stop_server(process)
        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, key, process)
        assert Sandbox.list(**sdk).next_items() == []
        failed_vm = f"vm-{namespace}-1"
        assert not Path(f"/run/netns/{failed_vm}").exists()
        assert not (root / "jails" / args.firecracker.name / failed_vm).exists()
        with sqlite3.connect(root / "db/controller.db") as database:
            states = [row[0] for row in database.execute("SELECT state_blob FROM sandboxes")]
        assert len(states) == before_reject + 1
        failed = [json.loads(state) for state in states if json.loads(state)["current_operation"]["kind"] == "Create"]
        assert len(failed) == 1
        assert failed[0]["phase"] == "Destroyed"
        assert "Failed" in failed[0]["current_operation"]["status"]
        assert sha256_file(rootfs) == base_digest
        completed = True
        print(json.dumps({
            "commit_scope": "real SDK/server/VM, fixed base and explicit no-internet",
            "binary_sha256": sha256_file(binary),
            "create_env_and_metadata": "observed in guest and management responses",
            "concurrent_kill": "204/404; one durable Kill; original VM stopped",
            "rejected_create": "400; no admission",
            "failed_create": "missing private kernel; recovery publishes failed operation and Destroyed; no live VM",
            "residual": "no exact process, netns, jail or stop gate; base unchanged",
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
