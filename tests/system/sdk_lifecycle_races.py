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
    fetch, free_loopback_port, original_center_audit, original_create_identity,
    prepare_fault_services, process_start_ticks, registered_cache_kernel,
    terminal_cleanup_confirmed,
)
from _runtime_services import add_arguments


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
    kernel = root / "vmlinux"
    rootfs = root / "rootfs.ext4"
    shutil.copyfile(args.kernel, kernel)
    shutil.copyfile(args.rootfs, rootfs)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    base_digest = sha256_file(rootfs)
    settings = {
        "namespace": namespace,
        "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)),
        "firecracker": str(args.firecracker.resolve(strict=True)),
        "chroot_base": str(root / "jails"),
        "kernel": str(kernel),
        "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "vcpu_count": 1,
        "memory_mib": 256,
        "file_policy_digest": 1,
        "max_active_global": 2,
        "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 4,
        "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "default_timeout_secs": 300,
        # Explicit restart owns failure recovery in this lifecycle race probe;
        # the dedicated expiry tests exercise the periodic scanner separately.
        "expiry_scan_seconds": 3600,
    }
    services = prepare_fault_services(args, root, settings, slots=2, key=key, port=port)
    gate = root / "stop-gate.sock"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{port}"
    process = None
    completed = False
    try:
        gate_env = {"SANDPROOF_TEST_STOP_GATE": str(gate)}
        services.start(host_envs={"host-0": gate_env})
        services.wait_ready()
        process = services.processes.children["host-0"]
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

        services.processes.stop("host-0")
        native_id = services.native_id_for(sandbox.sandbox_id)
        process = services.processes.start("host-0", env=gate_env)
        services.wait_ready(for_admission=False)

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
            assert not first.done() and not second.done(), "cleanup was reported before the physical stop"
            with socket.socket(socket.AF_UNIX) as release:
                release.connect(str(gate))
                release.sendall(b"g")
            assert first.result(timeout=40)[0] == 204
            # 两个真正重叠请求可等待同一回执；中心不持远程执行锁。
            # 完全释放后的后置请求仍必须404，绝不能生成第二个Destroy。
            assert second.result(timeout=40) in (204, 404)

        assert fetch(opener, base_url, "GET", f"/sandboxes/{sandbox.sandbox_id}", key)[0] == 404
        assert fetch(opener, base_url, "DELETE", f"/sandboxes/{sandbox.sandbox_id}", key)[0] == 404
        center = original_center_audit(services)
        saved = [row for row in center["sandboxes"]
                 if row["create"]["identity"]["sandbox_id"] == sandbox.sandbox_id]
        assert len(saved) == 1 and saved[0]["released"] is True
        destroys = [operation for operation in saved[0]["operations"] if operation["intent"] == "Destroy"]
        assert len(destroys) == 1 and destroys[0]["response"]["fact"] == "Cleaned"
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

        # 新入口在本地准入前验证真实缓存。精确移走登记的缓存inode，
        # 不删除无关bootstrap文件，也不伪造一个原本没有准入的native2。
        cache_kernel, kernel_identity = registered_cache_kernel(services)
        held_kernel = cache_kernel.with_name("kernel-held")
        with sqlite3.connect(root / "db/controller.db") as database:
            before_missing = database.execute(
                "SELECT high_water FROM controller_id_highwaters WHERE kind='sandbox' AND scope=''"
            ).fetchone()[0]
        marker = {"missing_cache_intent": secrets.token_hex(16)}
        cache_kernel.rename(held_kernel)
        try:
            status, _ = fetch(opener, base_url, "POST", "/v2/sandboxes", key,
                {"templateID": "base", "timeout": 180, "allow_internet_access": False, "metadata": marker})
            assert status >= 500, "missing cache kernel was published as Running"
            assert process.poll() is None, "host crashed during an unavailable template"
            original = original_create_identity(services, marker)
            # Cleaned来自同一真实全局请求和原宿主持久终止围栏，不能从SQL缺席推断。
            assert fetch(opener, base_url, "DELETE", "/sandboxes/" + original["sandbox_id"], key)[0] == 204
            with sqlite3.connect(root / "db/controller.db") as database:
                assert database.execute(
                    "SELECT high_water FROM controller_id_highwaters WHERE kind='sandbox' AND scope=''"
                ).fetchone()[0] == before_missing
                assert not database.execute("SELECT state_blob FROM sandboxes WHERE sandbox_id=?",
                                            (f"sb-{namespace}-2",)).fetchall()
        finally:
            held_kernel.rename(cache_kernel)
            restored = cache_kernel.stat()
            assert (restored.st_dev, restored.st_ino) == kernel_identity
        services.processes.stop("host-0")
        process = services.processes.start("host-0")
        services.wait_ready(for_admission=False)
        assert Sandbox.list(**sdk).next_items() == []
        failed_vm = f"vm-{namespace}-1"
        assert not Path(f"/run/netns/{failed_vm}").exists()
        assert not (root / "jails" / args.firecracker.name / failed_vm).exists()
        assert fetch(opener, base_url, "GET", "/sandboxes/" + original["sandbox_id"], key)[0] == 404
        assert terminal_cleanup_confirmed(root / "db/controller.db", native_id)
        assert sha256_file(rootfs) == base_digest
        completed = True
        print(json.dumps({
            "commit_scope": "real SDK/server/VM, fixed base and explicit no-internet",
            "host_sha256": sha256_file(args.host), "center_sha256": sha256_file(args.center),
            "create_env_and_metadata": "observed in guest and management responses",
            "concurrent_kill": "overlapping204/204-or404; later404; one globalDestroy/nativeKill; original VM stopped",
            "rejected_create": "400; no admission",
            "failed_create": "exact missing cache inode rejected before native admission; original globalDestroy Cleaned; no VM",
            "residual": "no exact process, netns, jail or stop gate; base unchanged",
        }))
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
    parser.add_argument("--jailer", required=True, type=Path)
    parser.add_argument("--firecracker", required=True, type=Path)
    parser.add_argument("--kernel", required=True, type=Path)
    parser.add_argument("--rootfs", required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    parser.add_argument("--scratch-parent", required=True, type=Path)
    exercise(parser.parse_args())
