"""Exit the real server at exact pre-host or post-host lifecycle windows.

Run only as root on an isolated Linux/KVM host with an approved debug build.
All test resources use a new namespace and private directory. A failed run
keeps its exact artifacts for manual inspection instead of guessing cleanup.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

_complete = _runpy.run_path(str(Path(__file__).with_name("_complete_runtime.py")))
complete_runtime = _complete["complete_runtime"]
provision_runtime = _complete["provision_runtime"]
terminal_cleanup_confirmed = _complete["terminal_cleanup_confirmed"]
idle_snapshot = _complete["idle_snapshot"]
assert_idle_reopen = _complete["assert_idle_reopen"]
retired_sandboxes = _complete["retired_sandboxes"]


EXIT_AFTER_HOST_ACTION = 86
EXIT_BEFORE_HOST_ACTION = 87


def free_loopback_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def process_start_ticks(pid: int) -> int | None:
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    except FileNotFoundError:
        return None
    if fields[0] == "Z":
        return None
    return int(fields[19])


def fetch(
    opener: urllib.request.OpenerDirector,
    base_url: str,
    method: str,
    path: str,
    key: str | None = None,
    body: dict | None = None,
) -> tuple[int, bytes]:
    headers = {"X-API-Key": key} if key is not None else {}
    if body is not None:
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(
        base_url + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers=headers,
        method=method,
    )
    try:
        with opener.open(request, timeout=40) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def wait_ready(
    opener: urllib.request.OpenerDirector,
    base_url: str,
    api_key: str,
    process: subprocess.Popen[bytes],
) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"server exited during startup: {process.returncode}")
        try:
            if fetch(opener, base_url, "GET", "/v2/sandboxes", api_key)[0] == 200:
                return
        except (urllib.error.URLError, http.client.HTTPException, OSError):
            pass
        time.sleep(0.1)
    raise AssertionError("server did not become ready")


def start_server(
    binary: Path, config: Path, log: Path, target: str | None,
    stop_gate: Path | None = None,
    before_target: str | None = None,
    guest_init_target: str | None = None,
    fail_reclaim_stop_sandbox: int | None = None,
    fail_reclaim_prepare_sandbox: int | None = None,
    exit_after_vm_claim: str | None = None,
    template_start_target: str | None = None,
    command_prefix: list[str] | None = None,
) -> subprocess.Popen[bytes]:
    environment = {"PATH": "/usr/bin:/bin", "LANG": "C"}
    if target is not None:
        environment["SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION"] = target
    if before_target is not None:
        environment["SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION"] = before_target
    if guest_init_target is not None:
        environment["SANDPROOF_TEST_EXIT_AFTER_GUEST_INIT"] = guest_init_target
    if fail_reclaim_stop_sandbox is not None:
        environment["SANDPROOF_TEST_FAIL_RECLAIM_STOP_SANDBOX"] = str(fail_reclaim_stop_sandbox)
    if fail_reclaim_prepare_sandbox is not None:
        environment["SANDPROOF_TEST_FAIL_RECLAIM_PREPARE_SANDBOX"] = str(fail_reclaim_prepare_sandbox)
    if exit_after_vm_claim is not None:
        environment["SANDPROOF_TEST_EXIT_AFTER_VM_CLAIM"] = exit_after_vm_claim
    if template_start_target is not None:
        environment["SANDPROOF_TEST_EXIT_AFTER_TEMPLATE_START"] = template_start_target
    if stop_gate is not None:
        environment["SANDPROOF_TEST_STOP_GATE"] = str(stop_gate)
    with log.open("ab") as output:
        return subprocess.Popen(
            [*(command_prefix or []), str(binary), str(config)],
            stdin=subprocess.DEVNULL,
            stdout=output,
            stderr=subprocess.STDOUT,
            env=environment,
            start_new_session=True,
        )


def stop_server(process: subprocess.Popen[bytes] | None) -> None:
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


def interrupted_request(
    request: object, process: subprocess.Popen[bytes],
    expected_exit: int = EXIT_AFTER_HOST_ACTION,
) -> None:
    try:
        response = request()
    except (urllib.error.URLError, http.client.HTTPException, OSError):
        pass
    else:
        raise AssertionError(f"request returned before injected exit: {response[0]}")
    assert process.wait(timeout=30) == expected_exit, process.returncode


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    parent = Path(args.scratch_parent).resolve(strict=True)
    disk_bytes = Path(args.rootfs).stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError("test host lacks space for trusted template and private VM disk")
    root = Path(tempfile.mkdtemp(prefix="sp-crash-", dir=parent))
    namespace = secrets.token_hex(8)
    vm_name = f"vm-{namespace}-0"
    api_key = secrets.token_hex(32)
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
    # Jailer drops to uid_base before Firecracker opens these chroot files.
    # The private parent remains root-only; the copies need guest-process read access.
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    config = root / "config.json"
    settings = {
        "namespace": namespace,
        "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"),
        "jailer": str(Path(args.jailer).resolve(strict=True)),
        "firecracker": str(Path(args.firecracker).resolve(strict=True)),
        "chroot_base": str(root / "jails"),
        "kernel": str(kernel),
        "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "vcpu_count": 1,
        "memory_mib": 256,
        "file_policy_digest": 1,
        "bind": f"127.0.0.1:{port}",
        "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": api_key}],
        "max_active_global": 2,
        "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 4,
        "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "default_timeout_secs": 300,
        "expiry_scan_seconds": 1,
    }
    settings = complete_runtime(settings, slots=2)
    provision_runtime(settings)
    config.write_text(json.dumps(settings))
    config.chmod(0o600)
    log = root / "server.log"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{port}"
    process = None
    completed = False
    try:
        before_host = args.window == "before-host"
        activation = args.window in ("before-activation", "after-activation", "after-guest-init")
        create_after = (
            "activate_vm" if args.window == "after-activation"
            else "prepare_vm" if args.window == "after-host"
            else None
        )
        create_before = (
            "open_fence" if before_host
            else "activate_vm" if args.window == "before-activation"
            else None
        )
        process = start_server(
            binary, config, log,
            create_after,
            before_target=create_before,
            guest_init_target="activate_vm" if args.window == "after-guest-init" else None,
        )
        wait_ready(opener, base_url, api_key, process)
        interrupted_request(
            lambda: fetch(
                opener, base_url, "POST", "/v2/sandboxes", api_key,
                {"templateID": "base", "timeout": 180, "allow_internet_access": False,
                 "envVars": {"SP_RECOVERY": "original"}},
            ),
            process,
            (EXIT_BEFORE_HOST_ACTION if create_before else
             88 if args.window == "after-guest-init" else EXIT_AFTER_HOST_ACTION),
        )
        if before_host:
            assert not list((root / "state/fences").glob("*.json")), (
                "opening fence was written before injected exit"
            )
            assert not (root / "jails/.claims" / vm_name).exists(), (
                "VM was launched before opening fence"
            )
            assert not Path(f"/run/netns/{vm_name}").exists(), (
                "network was created before opening fence"
            )
        else:
            record = json.loads((root / "jails/.claims" / vm_name / "record.json").read_text())
            assert record["resource_id"] == vm_name
            pid, ticks = record["pid"], record["start_time_ticks"]
            assert process_start_ticks(pid) == ticks, "first VM is not running after server exit"

        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, api_key, process)
        status, body = fetch(opener, base_url, "GET", "/v2/sandboxes", api_key)
        assert status == 200, status
        items = json.loads(body)
        assert len(items) == 1 and items[0]["state"] == "running", items
        sandbox_id = items[0]["sandboxID"]
        if before_host:
            record = json.loads((root / "jails/.claims" / vm_name / "record.json").read_text())
            assert record["resource_id"] == vm_name
            pid, ticks = record["pid"], record["start_time_ticks"]
        assert process_start_ticks(pid) == ticks, "server restart replaced the VM"
        if activation:
            import importlib.metadata
            from e2b import Sandbox
            from sdk_server_smoke import options

            assert importlib.metadata.version("e2b") == "2.51.0"
            sandbox = Sandbox.connect(sandbox_id, **options(api_key, base_url, base_url))
            assert sandbox.commands.run("printf %s \"$SP_RECOVERY\"").stdout == "original"
        stop_server(process)

        process = start_server(
            binary, config, log,
            None if before_host else "stop_vm",
            before_target="stop_vm" if before_host else None,
        )
        wait_ready(opener, base_url, api_key, process)
        interrupted_request(
            lambda: fetch(opener, base_url, "DELETE", f"/sandboxes/{sandbox_id}", api_key),
            process,
            EXIT_BEFORE_HOST_ACTION if before_host else EXIT_AFTER_HOST_ACTION,
        )
        if before_host:
            assert process_start_ticks(pid) == ticks, "VM stopped before injected exit"
            fence = root / "state/fences" / f"{sandbox_id}.json"
            assert json.loads(fence.read_text())["terminal"] is True, "terminal fence not durable"
        else:
            assert process_start_ticks(pid) != ticks, "recorded VM survived StopVm"

        process = start_server(binary, config, log, None)
        wait_ready(opener, base_url, api_key, process)
        status, body = fetch(opener, base_url, "GET", "/v2/sandboxes", api_key)
        assert status == 200 and json.loads(body) == [], status
        assert fetch(opener, base_url, "GET", f"/sandboxes/{sandbox_id}", api_key)[0] == 404
        assert not Path(f"/run/netns/{vm_name}").exists(), "network namespace survived cleanup"
        jail = root / "jails" / Path(args.firecracker).name / vm_name
        assert not jail.exists(), "private VM jail survived cleanup"
        assert process_start_ticks(pid) != ticks, "VM was relaunched during recovery"
        completed = True
        print(json.dumps({
            "namespace": namespace,
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "window": args.window,
            "create": "recovered the original VM after the selected crash",
            "stop": "published destruction only after exact stop and cleanup",
            "scope": "real server process exit; one VM; no public egress or arbitrary-position SIGKILL",
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
    parser.add_argument(
        "--window",
        choices=("before-host", "after-host", "before-activation", "after-activation", "after-guest-init"),
        default="after-host",
    )
    exercise(parser.parse_args())
