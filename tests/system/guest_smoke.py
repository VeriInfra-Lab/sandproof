#!/usr/bin/env python3
"""Real Linux/KVM + pinned E2B SDK guest test; never part of synthetic CI.

Uses the SDK constructor only for the already-created guest. This deliberately
does not claim to test Sandbox.create/kill, controller authorization, or Harbor.
"""

import argparse
import asyncio
from contextlib import contextmanager
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
OUTSIDE_ADDRESS = "198.18.255.1"
OUTSIDE_PORT = 18080


def ip(*arguments):
    subprocess.run(["/usr/bin/ip", *arguments], check=True, timeout=10)


def network_socket_probe(namespace, source):
    code = ("import socket; s=socket.socket(); s.settimeout(1); "
            f"s.bind(({source!r}, 0)); s.connect(({OUTSIDE_ADDRESS!r}, {OUTSIDE_PORT})); "
            "s.close()")
    return subprocess.run(["/usr/bin/ip", "netns", "exec", namespace,
                           sys.executable, "-c", code], capture_output=True,
                          text=True, timeout=3)


@contextmanager
def live_outside_route(resource_id):
    """A reachable test-only network, so a denied guest connection is not vacuous."""
    suffix = secrets.token_hex(3)
    outside = f"sp-out-{suffix}"
    outside_link = f"spo{suffix}"
    inside_link = f"spi{suffix}"
    listener = None
    forwarding_before = None
    outside_created = False
    pair_created = False
    pair_moved = False
    try:
        ip("netns", "add", outside)
        outside_created = True
        ip("link", "add", outside_link, "type", "veth", "peer", "name", inside_link,
           "netns", resource_id)
        pair_created = True
        ip("link", "set", outside_link, "netns", outside)
        pair_moved = True
        ip("netns", "exec", outside, "/usr/bin/ip", "addr", "add",
           f"{OUTSIDE_ADDRESS}/30", "dev", outside_link)
        ip("netns", "exec", outside, "/usr/bin/ip", "link", "set", outside_link, "up")
        ip("netns", "exec", outside, "/usr/bin/ip", "route", "add",
           "172.16.0.0/30", "via", "198.18.255.2")
        ip("netns", "exec", resource_id, "/usr/bin/ip", "addr", "add",
           "198.18.255.2/30", "dev", inside_link)
        ip("netns", "exec", resource_id, "/usr/bin/ip", "link", "set", inside_link, "up")
        forwarding_before = subprocess.check_output(
            ["/usr/bin/ip", "netns", "exec", resource_id, "/usr/sbin/sysctl", "-n",
             "net.ipv4.ip_forward"], text=True, timeout=10).strip()
        subprocess.run(["/usr/bin/ip", "netns", "exec", resource_id, "/usr/sbin/sysctl",
                        "-w", "net.ipv4.ip_forward=1"], check=True, capture_output=True,
                       timeout=10)
        guest_route = subprocess.check_output(
            ["/usr/bin/ip", "netns", "exec", resource_id, "/usr/bin/ip",
             "-4", "route", "get", OUTSIDE_ADDRESS, "from", "172.16.0.2",
             "iif", "tap0"], text=True, timeout=10)
        assert inside_link in guest_route, guest_route
        listener_code = ("import socket; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, "
                         "socket.SO_REUSEADDR, 1); s.bind(('198.18.255.1', 18080)); "
                         "s.listen();\nwhile True:\n c,_=s.accept(); c.close()")
        listener = subprocess.Popen(
            ["/usr/bin/ip", "netns", "exec", outside, sys.executable, "-c", listener_code],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if listener.poll() is not None:
                raise RuntimeError(f"test listener exited: {listener.stderr.read()!r}")
            # This process is inside the VM's host namespace, not the guest.
            # Bind the host side of the guest's TAP subnet to test the return route.
            if network_socket_probe(resource_id, "172.16.0.1").returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("test outside route is not reachable from the VM namespace")
        yield {"outside_address": OUTSIDE_ADDRESS, "outside_port": OUTSIDE_PORT}
    finally:
        if listener is not None:
            listener.terminate()
            try:
                listener.wait(timeout=5)
            except subprocess.TimeoutExpired:
                listener.kill()
                listener.wait(timeout=5)
        try:
            if forwarding_before is not None:
                subprocess.run(["/usr/bin/ip", "netns", "exec", resource_id,
                                "/usr/sbin/sysctl", "-w",
                                f"net.ipv4.ip_forward={forwarding_before}"], check=True,
                               capture_output=True, timeout=10)
        finally:
            if outside_created:
                ip("netns", "delete", outside)
            if pair_created and not pair_moved:
                ip("link", "delete", outside_link)


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
    # Guest root may replace its resolver configuration. The host namespace
    # guard must still reject the resulting DNS and non-DNS egress, while the
    # authenticated host-initiated envd connection remains usable.
    sandbox.commands.run(
        "printf 'nameserver 1.1.1.1\\noptions timeout:1 attempts:1\\n' > /etc/resolv.conf"
    )
    network_probe = sandbox.commands.run("""python3 - <<'PY'
import socket
targets = [
    (socket.AF_INET, ('1.1.1.1', 53)),
    (socket.AF_INET, ('8.8.8.8', 853)),
    (socket.AF_INET, ('1.1.1.1', 443)),
    (socket.AF_INET6, ('2606:4700:4700::1111', 53)),
]
for family, target in targets:
    with socket.socket(family, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.5)
        try:
            sock.connect(target)
        except (OSError, TimeoutError):
            continue
        raise AssertionError(f'guest egress unexpectedly connected to {target}')
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
    sock.settimeout(0.5)
    try:
        sock.sendto(b'\\x00' * 12, ('1.1.1.1', 53))
        sock.recvfrom(512)
    except (OSError, TimeoutError):
        pass
    else:
        raise AssertionError('guest received external DNS response')
print('guest-egress-denied')
PY""")
    assert network_probe.stdout.strip() == "guest-egress-denied"
    if config.get("outside_address"):
        address = config["outside_address"]
        port = config["outside_port"]
        route = sandbox.commands.run(
            f"ip -4 route replace {address}/32 via 172.16.0.1 dev eth0"
        )
        assert route.exit_code == 0, route.stderr
        # The host-side namespace has already reached this listener with the
        # guest's source address. This packet must still be stopped at tap0.
        guarded_probe = sandbox.commands.run(f"""python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.settimeout(1)
    try:
        sock.connect(({address!r}, {port}))
    except OSError:
        print('live-route-guest-denied')
    else:
        raise AssertionError('guest reached the test-only outside network')
PY""")
        assert (guarded_probe.stdout.strip(), guarded_probe.exit_code) == (
            "live-route-guest-denied", 0
        )
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
        "live_outside_route_denied": bool(config.get("outside_address")),
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
            if index == 0:
                with live_outside_route(resource_id) as route:
                    config.update(route)
                    subprocess.run(["/usr/bin/ip", "netns", "exec", resource_id,
                                    sys.executable, script, "--probe"], env=environment,
                                   input=json.dumps(config), text=True, check=True, timeout=90)
            else:
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
                      "live_outside_route_denied": True,
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
