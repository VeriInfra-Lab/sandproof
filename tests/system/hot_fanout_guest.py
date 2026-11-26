#!/usr/bin/env python3
"""Run in one dedicated Linux/KVM guest booted with the prepared image.

This exercises the guest adapter and pinned SDK data endpoints directly.
Launch with a private /run, cgroup v2, mounted /proc, and active loopback.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
import importlib.metadata
import json
import mmap
import os
from pathlib import Path
import secrets
import subprocess
from threading import Barrier
import time

from tools.hot_fanout import GuestFanout
from tools.hot_fanout import linux


def command(*args):
    subprocess.run(args, check=True, timeout=15, stdout=subprocess.DEVNULL)


def require(expectation, reason):
    if not expectation:
        raise RuntimeError(reason)


def make_package(work):
    tree = work / "package"
    control = tree / "DEBIAN/control"
    control.parent.mkdir(parents=True)
    control.write_text(
        "Package: sandproof-hot-fanout-check\nVersion: 1.0\nArchitecture: all\n"
        "Maintainer: Sandproof <sandproof@example.invalid>\n"
        "Description: Disposable root overlay fixture\n"
    )
    marker = tree / "usr/share/sandproof-hot-fanout-check/marker"
    marker.parent.mkdir(parents=True)
    marker.write_text("package-from-one-branch")
    # The launcher keeps the enclosing workspace private. The package tree
    # needs regular installation modes regardless of that launcher's umask.
    tree.chmod(0o755)
    for directory, subdirectories, files in os.walk(tree):
        for name in subdirectories:
            (Path(directory) / name).chmod(0o755)
        for name in files:
            (Path(directory) / name).chmod(0o644)
    artifact = work / "fixture.deb"
    command("/usr/bin/dpkg-deb", "--build", str(tree), str(artifact))
    return artifact.read_bytes()


def branch_plan(batch, first_port, count):
    return {"batch_id": batch, "checkpoint_id": "warm0", "branches": [
        {"branch_id": f"{batch}_{i}", "port": first_port + i, "access_token": secrets.token_urlsafe(32)}
        for i in range(count)
    ]}


def sdk_for(branch, guest_host="127.0.0.1"):
    from e2b import Sandbox
    from e2b.connection_config import ConnectionConfig
    from packaging.version import Version

    token = branch["access_token"]
    config = ConnectionConfig(
        api_key="guest-fixture", api_url="http://127.0.0.1:1", domain="example.invalid",
        sandbox_url=f"http://{guest_host}:{branch['port']}", retries=0, request_timeout=10,
        extra_sandbox_headers={"X-Access-Token": token},
    )
    return Sandbox(sandbox_id=branch["branch_id"], sandbox_domain="example.invalid",
                   envd_version=Version("0.9.0"), envd_access_token=token, connection_config=config)


def probe_sdk(branches, expected, guest_host="127.0.0.1"):
    barrier = Barrier(len(branches), timeout=10)

    def verify(branch):
        sandbox = sdk_for(branch, guest_host)
        deadline = time.monotonic() + 5
        inherited = False
        while not inherited and time.monotonic() < deadline:
            # Only the branch workload can create the marker. A normal SDK
            # read reports success once that workload has inherited the heap.
            probe = sandbox.commands.run(
                "cat /run/inherited 2>/dev/null || true", user="root", timeout=2,
            ).stdout
            inherited = probe == expected
            if not inherited:
                time.sleep(0.01)
        require(inherited, "branch did not inherit the prepared bytes")
        sandbox.commands.run(
            'test "$(cat /proc/$$/oom_score_adj)" = 100 '
            "&& test ! -e /proc/sys && test ! -e /proc/sysrq-trigger",
            user="root", timeout=5,
        )
        barrier.wait()
        value = branch["branch_id"]
        sandbox.files.write("/etc/fanout-sdk-marker", value, user="root")
        out = sandbox.commands.run(
            "sleep .05; cat /etc/fanout-sdk-marker; cp /etc/fanout-sdk-marker /root/fanout-sdk-marker",
            user="root", timeout=5,
        )
        require(out.stdout == value, "command saw the wrong branch file")
        require(sandbox.files.read("/root/fanout-sdk-marker", user="root") == value,
                "file request saw the wrong command write")
        return sandbox

    with ThreadPoolExecutor(max_workers=len(branches)) as pool:
        return list(pool.map(verify, branches))


def run(envd, count):
    require(2 <= count <= 32, "this package-and-peer fixture needs between 2 and 32 branches")
    require(importlib.metadata.version("e2b") == "2.51.0", "expected pinned E2B SDK")
    work = Path("/run") / ("sandproof-hot-fanout-" + secrets.token_hex(8))
    cgroups = Path("/sys/fs/cgroup") / work.name
    work.mkdir(mode=0o700)
    with ExitStack() as cleanup:
        import shutil
        cleanup.callback(shutil.rmtree, work)
        cgroups.mkdir()
        cleanup.callback(cgroups.rmdir)
        base = work / "base"
        base.mkdir()
        command("/usr/bin/mount", "--bind", "/", str(base))
        cleanup.callback(command, "/usr/bin/umount", str(base))
        command("/usr/bin/mount", "-o", "remount,bind,ro", str(base))

        package = make_package(work)
        heap = mmap.mmap(-1, 64 * 1024 * 1024, flags=mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS)
        page = os.sysconf("SC_PAGE_SIZE")
        marks = secrets.token_bytes(len(heap) // page)
        heap[::page] = marks
        expected = marks[:16].hex()

        def verify_inherited(_):
            require(heap[::page] == marks, "forked process did not inherit all touched pages")
            Path("/run/inherited").write_text(heap[::page][:16].hex())

        # The donor inherits this fixture-only injection. No runtime fault
        # option is available to a production caller.
        spawn = linux._spawn_member

        def fail_third(cgroup, child):
            if cgroup.name == "fault.2":
                raise OSError("fixture stopped the third fork")
            spawn(cgroup, child)

        linux._spawn_member = fail_third
        try:
            backend = linux.LinuxGuest(work / "slots", cgroups, [base], envd,
                                       default_user="root", default_workdir="/root")
            state = work / "state"
            bounds = {"max_branches": count + 3, "max_batches": 8,
                      "ports": range(31000, 31100)}
            with GuestFanout(state, "warm0", backend, **bounds) as owner:
                backend.attach_prepared_donor(verify_inherited, manager_fds=(owner._lock,))
                batch = branch_plan("steady", 31000, count)
                fault = branch_plan("fault", 31050, 3)
                refill = branch_plan("replacement", 31000, 1)
                try:
                    started = time.monotonic()
                    require(owner.start(batch)["status"] == "ready", "initial batch did not become ready")
                    ready = time.monotonic()
                    branches = probe_sdk(batch["branches"], expected)
                    sdk_ready = time.monotonic()
                    branches[0].files.write("/tmp/check.deb", package, user="root")
                    branches[0].commands.run("dpkg -i /tmp/check.deb", user="root", timeout=10)
                    marker = "/usr/share/sandproof-hot-fanout-check/marker"
                    require(branches[0].files.read(marker, user="root") == "package-from-one-branch",
                            "test package missing in its own branch")
                    for peer in branches[1:]:
                        peer.commands.run(f"test ! -e {marker}", user="root", timeout=5)
                    require(not Path(marker).exists(), "package escaped to the donor's root")
                    require(owner.start(fault)["status"] == "failed_clean", "partial fork did not clean")
                    require(not list(cgroups.glob("fault.*")) and not list(backend.data.glob("fault.*")),
                            "partially started branch resources remain")
                    released = owner.release("steady", "steady_0")
                    require(released["status"] == "ready" and released["released"] == ["steady_0"],
                            "individual release did not leave its peers available")
                    require(owner.start(refill)["status"] == "ready", "retired port could not be reused")
                    require(linux._status(31000, "GET", "/files?path=%2Fetc%2Fpasswd", 1,
                                          token=batch["branches"][0]["access_token"]) in (401, 403),
                            "reused port accepted the old branch token")
                    reply = branches[1].commands.run("cat /etc/fanout-sdk-marker", user="root", timeout=5)
                    require(reply.stdout == "steady_1", "released sibling affected surviving branch")
                    return {"branches": count, "envd_ready_ms": (ready - started) * 1000,
                            "sdk_ready_ms": (sdk_ready - started) * 1000,
                            "full_root_package_isolated": True, "partial_fork_cleanup": True,
                            "envd_oom_adjustment_in_task_only_procfs": True,
                            "reused_port_rejects_old_token": True}
                finally:
                    try:
                        for batch_id in ("steady", "fault", "replacement"):
                            owner.release(batch_id)
                        require(not any(path.is_dir() for path in cgroups.iterdir())
                                and not list(backend.data.iterdir()),
                                "a branch cgroup or overlay survived cleanup")
                    finally:
                        backend.close()
        finally:
            linux._spawn_member = spawn


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--envd", type=Path, required=True)
    parser.add_argument("--branches", type=int, default=4)
    args = parser.parse_args()
    print("SP_FANOUT_RESULT " + json.dumps(run(args.envd, args.branches)), flush=True)


if __name__ == "__main__":
    main()
