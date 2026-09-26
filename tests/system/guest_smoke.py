#!/usr/bin/env python3
"""Real Linux/KVM + pinned E2B SDK guest test; never part of synthetic CI.

Uses the SDK constructor only for the already-created guest. This deliberately
does not claim to test Sandbox.create/kill, controller authorization, or Harbor.
"""

import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time


URL = "http://172.16.0.2:49983"


def probe_edge_cases(sandbox, config):
    """Exercise pinned SDK semantics without inventing a second data plane."""
    import httpx
    from e2b.sandbox.commands.command_handle import CommandExitException

    # A timed-out stream is not evidence that its guest process never ran, or
    # that the sandbox died. Do not automatically replay a timed-out shell.
    try:
        sandbox.commands.run("sleep 1; printf t >> /tmp/timeout-once", timeout=0.2)
    except Exception as error:
        if not any(marker in type(error).__name__.lower()
                   for marker in ("timeout", "deadline")):
            raise AssertionError(f"unexpected command timeout error: {type(error).__name__}") from error
    else:
        raise AssertionError("command stream did not respect the explicit timeout")
    assert sandbox.commands.run("printf alive").stdout == "alive"

    pending = sandbox.commands.run("sleep 30", background=True)
    assert pending.kill() is True
    try:
        pending.wait()
    except CommandExitException:
        pass
    else:
        raise AssertionError("explicitly killed command returned successful exit")

    # Closing a streaming connection is not a request to kill or re-execute.
    pending = sandbox.commands.run("sleep 1; printf reconnect-ok", background=True)
    pid = pending.pid
    pending.disconnect()
    reconnected = sandbox.commands.connect(pid)
    assert reconnected.pid == pid
    assert reconnected.wait().stdout == "reconnect-ok"

    output = sandbox.commands.run(
        "python3 -c 'import sys; sys.stdout.write(\"X\" * 1048576); "
        "sys.stderr.write(\"E\" * 262144)'"
    )
    assert len(output.stdout) == 1048576 and set(output.stdout) == {"X"}
    assert len(output.stderr) == 262144 and set(output.stderr) == {"E"}

    # The same absolute path on the host and in the guest names different
    # storage; the test also checks the host sentinel after every VM exits.
    host_marker = config["host_marker"]
    assert sandbox.commands.run(f"test ! -e {host_marker}").exit_code == 0
    sandbox.files.write(host_marker, "guest-only\n")
    assert sandbox.files.read(host_marker) == "guest-only\n"
    assert sandbox.files.read("/tmp/input.txt") == "real guest file\n"
    path_statuses = {}
    with httpx.Client(base_url=URL, trust_env=False, timeout=5) as client:
        headers = {"X-Access-Token": config["token"]}
        for label, suspicious in (("nul", "\x00"),
                                  ("proc_root", "/proc/1/root" + host_marker),
                                  ("traversal", "/tmp/../../" + host_marker.lstrip("/"))):
            response = client.get("/files", params={"path": suspicious}, headers=headers)
            path_statuses[label] = response.status_code
            if label == "nul":
                assert response.status_code >= 400
            else:
                assert response.status_code < 500, (suspicious, response.status_code)
            assert b"host-only sentinel" not in response.content
    assert sandbox.files.read("/tmp/input.txt") == "real guest file\n"
    return path_statuses


def probe(config):
    import httpx
    from e2b import AsyncSandbox, Sandbox
    from e2b.connection_config import ConnectionConfig
    from e2b.sandbox.commands.command_handle import CommandExitException
    from packaging.version import Version

    assert importlib.metadata.version("e2b") == "2.51.0"
    token = config["token"]
    with httpx.Client(base_url=URL, trust_env=False, timeout=15) as client:
        if not config.get("verify_only"):
            deadline = time.monotonic() + 30
            while True:
                try:
                    if client.get("/health").status_code == 204:
                        break
                except httpx.TransportError:
                    pass
                if time.monotonic() >= deadline:
                    raise TimeoutError("envd did not become healthy")
                time.sleep(0.1)
            # Must be rejected even before the first successful /init. This
            # catches a missing MMDS route/hash, not just post-init token auth.
            wrong = client.post("/init", json={"accessToken": "wrong-test-token"})
            assert wrong.status_code == 401, f"pre-init wrong token status: {wrong.status_code}"
            initialized = client.post("/init", json={
                "accessToken": token, "defaultUser": "root", "defaultWorkdir": "/root",
            })
            assert initialized.status_code == 204, f"init status: {initialized.status_code}"
        rejected = client.get("/files", params={"path": "/etc/passwd"},
                              headers={"X-Access-Token": "wrong-test-token"})
        assert rejected.status_code == 401
        rejected = client.post("/process.Process/Start", content=b"",
                               headers={"X-Access-Token": "wrong-test-token"})
        assert rejected.status_code == 401

    connection = ConnectionConfig(
        api_key="local-guest-test-only", api_url="http://127.0.0.1:1",
        sandbox_url=URL, domain="example.invalid", retries=0, request_timeout=10,
        extra_sandbox_headers={"X-Access-Token": token},
    )
    sandbox = Sandbox(
        sandbox_id=config["id"], sandbox_domain="example.invalid",
        envd_version=Version("0.9.0"), envd_access_token=token,
        connection_config=connection,
    )
    if config.get("verify_only"):
        assert sandbox.files.read("/etc/sandproof-identity") == config["id"]
        print(json.dumps({"resource_id": config["id"], "independent_writes_preserved": True}))
        return

    assert sandbox.commands.run("test ! -e /etc/sandproof-identity").exit_code == 0
    sandbox.files.write("/etc/sandproof-identity", config["id"])
    sandbox.files.write("/tmp/input.txt", "real guest file\n")
    assert sandbox.files.read("/tmp/input.txt") == "real guest file\n"
    assert sandbox.files.get_info("/tmp/input.txt").size == 16
    assert any(entry.name == "input.txt" for entry in sandbox.files.list("/tmp"))
    result = sandbox.commands.run("pwd; printf '%s\\n' \"$SP_TEST\"; cat input.txt; printf 'err\\n' >&2",
                                  cwd="/tmp", envs={"SP_TEST": "guest-env"})
    assert (result.stdout, result.stderr, result.exit_code) == (
        "/tmp\nguest-env\nreal guest file\n", "err\n", 0,
    )
    try:
        sandbox.commands.run("exit 17")
    except CommandExitException as failure:
        assert failure.exit_code == 17
    else:
        raise AssertionError("nonzero exit was reported as success")
    handle = sandbox.commands.run("sleep 0.2; printf background-done", background=True)
    assert handle.pid > 0 and handle.wait().stdout == "background-done"
    assert sandbox.commands.run("id -un", user="ubuntu", cwd="/tmp").stdout.strip() == "ubuntu"
    assert sandbox.commands.run(f"test ! -e {config['host_marker']}").exit_code == 0
    cgroup = sandbox.commands.run("cat /proc/self/cgroup").stdout.strip()

    async def async_client_checks():
        asynchronous = AsyncSandbox(
            sandbox_id=config["id"], sandbox_domain="example.invalid",
            envd_version=Version("0.9.0"), envd_access_token=token,
            connection_config=connection,
        )
        await asynchronous.files.write("/tmp/async-input.txt", "async file\n")
        pending = await asynchronous.commands.run(
            "cat async-input.txt; printf '%s' \"$SP_ASYNC\"", background=True,
            cwd="/tmp", envs={"SP_ASYNC": "done"}, user="root", timeout=5,
        )
        result = await pending.wait()
        assert (result.stdout, result.exit_code) == ("async file\ndone", 0)
        assert await asynchronous.files.read("/tmp/async-input.txt") == "async file\n"

    asyncio.run(async_client_checks())
    path_statuses = probe_edge_cases(sandbox, config) if config.get("edge_cases") else {}
    print(json.dumps({
        "resource_id": config["id"], "sdk_version": "2.51.0", "envd_version": "0.9.0",
        "pre_init_wrong_token_rejected": True, "wrong_file_and_command_token_rejected": True,
        "command_cwd_env_user_exit_background": True, "files_read_write_list_info": True,
        "async_background_wait_and_files": True,
        "host_marker_not_visible": True, "guest_command_cgroup": cgroup,
        "edge_cases": bool(config.get("edge_cases")),
        "malformed_file_statuses": path_statuses,
    }))


def orchestrate(args):
    if os.geteuid() != 0:
        raise PermissionError("the real VM test must run as root on the isolated KVM host")
    runtime = str(Path(args.runtime).resolve())
    script = str(Path(__file__).resolve())
    image = Path(args.rootfs)
    with image.open("rb") as stream:
        before = hashlib.file_digest(stream, "sha256").hexdigest()
    resources = []
    cleanup_errors = []
    marker = Path(f"/tmp/sandproof-host-marker-{secrets.token_hex(8)}")
    marker.write_text("host-only sentinel")
    try:
        for index in range(2):
            resource_id = f"sp-envd-{secrets.token_hex(6)}"
            token = secrets.token_urlsafe(32)
            config = {"id": resource_id, "token": token, "host_marker": str(marker),
                      "edge_cases": index == 0}
            environment = {
                key: value for key, value in os.environ.items()
                if not key.startswith("E2B_")
                and key.lower() not in {"http_proxy", "https_proxy", "all_proxy", "no_proxy"}
            }
            environment["SANDBOX_ENVD_TOKEN_SHA512"] = hashlib.sha512(token.encode()).hexdigest()
            values = [resource_id, args.jailer, args.firecracker, args.chroot_base,
                      args.kernel, args.rootfs, str(args.uid_base + index),
                      str(args.uid_base + index), f"/run/netns/{resource_id}"]
            resources.append((config, environment, values))
            subprocess.run([runtime, "launch-net", *values], env=environment, check=True, timeout=30)
            subprocess.run(["/usr/bin/ip", "netns", "exec", resource_id,
                            sys.executable, script, "--probe"], env=environment,
                           input=json.dumps(config), text=True, check=True, timeout=90)
        # Both VMs are still alive; writes to the same absolute system path
        # must retain their own value after the other VM has modified its disk.
        for config, environment, _values in resources:
            subprocess.run(["/usr/bin/ip", "netns", "exec", config["id"],
                            sys.executable, script, "--probe"], env=environment,
                           input=json.dumps({**config, "verify_only": True}),
                           text=True, check=True, timeout=30)
    finally:
        for config, environment, values in reversed(resources):
            try:
                subprocess.run([runtime, "stop-net", *values], env=environment, check=True, timeout=20)
                subprocess.run([runtime, "cleanup-net", *values], env=environment, check=True, timeout=20)
                assert not Path(values[-1]).exists()
                assert not (Path(args.chroot_base) / Path(args.firecracker).name / config["id"]).exists()
            except Exception as error:
                cleanup_errors.append(f"{config['id']}: {error}")
        assert marker.read_text() == "host-only sentinel", "guest file operation changed host data"
        marker.unlink()
        with image.open("rb") as stream:
            assert hashlib.file_digest(stream, "sha256").hexdigest() == before
        if cleanup_errors:
            raise RuntimeError("exact resource cleanup requires inspection: " + "; ".join(cleanup_errors))
    print(json.dumps({"two_vm_isolation": True, "base_image_unchanged": before,
                      "exact_vm_disk_network_cleanup": True,
                      "scope": "guest SDK data plane only; controller/API/Harbor not yet exercised"}))


if __name__ == "__main__":
    if sys.argv[1:] == ["--probe"]:
        probe(json.load(sys.stdin))
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        for name in ["runtime", "jailer", "firecracker", "chroot-base", "kernel", "rootfs"]:
            parser.add_argument(f"--{name}", required=True)
        parser.add_argument("--uid-base", type=int, default=22001)
        orchestrate(parser.parse_args())
