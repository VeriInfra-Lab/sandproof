"""Two real VMs: one damaged host credential must not block its healthy peer.

Run as root on an approved Linux/KVM host, against a clean exact checkout.
No public network, cloud service or model is used. Faults replace only this
test's first credential; its original inode is restored before precise cleanup.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
from tools.capture_acceptance import original_chroot
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

from e2b import Sandbox

from sdk_c02_acceptance import processes_in_uid_range
from sdk_server_smoke import options
from server_process_crash import (
    fetch, free_loopback_port, prepare_fault_services, process_start_ticks,
    terminal_cleanup_confirmed,
)
from _runtime_services import add_arguments


def artifact_hash(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def data_status(opener, base, token):
    request = urllib.request.Request(base + "/health", headers={"X-Access-Token": token})
    try:
        with opener.open(request, timeout=15) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


def original_bindings(database, native_ids):
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        connection.execute("BEGIN")
        credentials = [connection.execute(
            "SELECT sandbox_id,generation,digest FROM controller_data_credentials WHERE sandbox_id=?",
            (sandbox,),
        ).fetchone() for sandbox in native_ids]
        contexts = connection.execute(
            "SELECT position,context_json FROM controller_host_slots ORDER BY position"
        ).fetchall()
    assert all(row is not None for row in credentials)
    return credentials, contexts


def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an approved Linux/KVM host")
    assert importlib.metadata.version("e2b") == "2.51.0"
    checkout = Path(__file__).resolve().parents[2]
    assert not subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip()
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    parent = args.scratch_parent.resolve(strict=True)
    assert parent.stat().st_uid == 0 and not parent.stat().st_mode & 0o077
    assert 1000 <= args.uid_base < 2**32 - 2
    assert not processes_in_uid_range(args.uid_base, args.uid_base + 1)
    root = Path(tempfile.mkdtemp(prefix="sp-cred-", dir=parent))
    for name in ("db", "state", "p"):
        (root / name).mkdir(mode=0o700)
    namespace = secrets.token_hex(8)
    kernel, image = (root / name for name in ("vmlinux", "rootfs.ext4"))
    for source, destination in ((args.kernel, kernel), (args.rootfs, image)):
        shutil.copyfile(source, destination)
    kernel.chmod(0o644)
    image.chmod(0o644)
    keys = [secrets.token_hex(32), secrets.token_hex(32)]
    port = free_loopback_port()
    base = f"http://127.0.0.1:{port}"
    disk_bytes = image.stat().st_size
    settings = {
        "namespace": namespace, "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"), "chroot_base": str(root / "p/v"),
        "jailer": str(args.jailer.resolve(strict=True)),
        "firecracker": str(args.firecracker.resolve(strict=True)),
        "kernel": str(kernel), "rootfs": str(image), "uid_base": args.uid_base,
        "vcpu_count": 1, "memory_mib": 512, "file_policy_digest": 1,
        "max_active_global": 2, "max_active_per_tenant": 1,
        "max_disk_bytes_global": 3 * disk_bytes,
        "max_disk_bytes_per_tenant": disk_bytes, "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600, "default_timeout_secs": 300, "expiry_scan_seconds": 3600,
    }
    services = prepare_fault_services(args, root, settings, slots=2, key=keys[0], port=port,
        extra_credentials=[{"tenant": 2, "secret": keys[1]}], shared_template=True)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    process = None
    live, claims = [], []
    credential, backup = None, root / "original-credential"
    completed = False
    try:
        services.start()
        services.wait_ready()
        process = services.processes.children["host-0"]
        for key in keys:
            live.append(Sandbox.create(template="base", timeout=300, allow_internet_access=False,
                                       **options(key, base, base)))
        first, second = live
        for number in range(2):
            name = f"vm-{namespace}-{number}"
            claim = json.loads((original_chroot(root) / ".claims" / name / "record.json").read_text())
            assert process_start_ticks(claim["pid"]) == claim["start_time_ticks"]
            claims.append((name, claim))
        services.processes.stop("host-0")
        native_ids = [services.native_id_for(sandbox.sandbox_id) for sandbox in live]
        assert len(set(native_ids)) == 2
        process = services.processes.start("host-0")
        services.wait_ready(for_admission=False)
        assert first.commands.run("printf before-fault").stdout == "before-fault"
        bindings = original_bindings(root / "db/controller.db", native_ids)
        network_inodes = [Path("/run/netns", name).stat().st_ino for name, _ in claims]
        credential = root / "state/secrets" / native_ids[0]
        original = credential.read_bytes()
        original_identity = (credential.stat().st_dev, credential.stat().st_ino)
        for fault in ("missing", "malformed", "changed"):
            os.replace(credential, backup)
            try:
                if fault != "missing":
                    replacement = root / "replacement-credential"
                    if fault == "malformed":
                        payload = b"{"
                    else:
                        value = json.loads(original)
                        token = "f" * 64
                        assert token not in (first._envd_access_token, second._envd_access_token)
                        value.update(token=token, sha512=hashlib.sha512(token.encode()).hexdigest())
                        payload = json.dumps(value).encode()
                    descriptor = os.open(replacement, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                    with os.fdopen(descriptor, "wb") as output:
                        output.write(payload)
                        output.flush()
                        os.fsync(output.fileno())
                    os.replace(replacement, credential)
                # The same faulty leaf must remain locally unavailable across
                # the real service's physical inventory and recovery sequence.
                services.processes.stop("host-0")
                process = services.processes.start("host-0")
                services.wait_ready(for_admission=False)
                assert original_bindings(root / "db/controller.db", native_ids) == bindings
                for (name, claim), inode in zip(claims, network_inodes, strict=True):
                    assert process_start_ticks(claim["pid"]) == claim["start_time_ticks"]
                    assert Path("/run/netns", name).stat().st_ino == inode
                if fault == "missing":
                    assert not credential.exists(), "recovery invented a replacement credential"
                else:
                    assert credential.read_bytes() == payload, "recovery rewrote the faulted credential"
                assert data_status(opener, base, first._envd_access_token) == 503
                # 管理查询只读中心已确认的信息，不用损坏的宿主凭据授权数据。
                status, body = fetch(opener, base, "GET", "/sandboxes/" + first.sandbox_id, keys[0])
                assert status == 200 and json.loads(body)["sandboxID"] == first.sandbox_id
                assert fetch(opener, base, "GET", "/sandboxes/" + first.sandbox_id, keys[1])[0] == 404
                if fault == "changed":
                    assert data_status(opener, base, "f" * 64) == 401
                assert second.commands.run("printf healthy-peer").stdout == "healthy-peer"
                second.files.write("/tmp/credential-isolation", fault)
                assert second.files.read("/tmp/credential-isolation") == fault
                assert fetch(opener, base, "GET", "/sandboxes/" + second.sandbox_id, keys[1])[0] == 200
                assert fetch(opener, base, "GET", "/sandboxes/" + second.sandbox_id, keys[0])[0] == 404
            finally:
                os.replace(backup, credential)
                assert (credential.stat().st_dev, credential.stat().st_ino) == original_identity
                assert credential.read_bytes() == original
            assert first.commands.run("printf restored").stdout == "restored"
        for key, sandbox, native_id in zip(keys, live, native_ids, strict=True):
            killed = Sandbox.kill(sandbox.sandbox_id, **options(key, base, base))
            assert killed
            assert terminal_cleanup_confirmed(root / "db/controller.db", native_id)
            assert data_status(opener, base, sandbox._envd_access_token) == 401
        live.clear()
        # Reuse a retired physical position with a new permanent VM identity.
        # Neither old digest may authorize the new VM occupying that position.
        reused = Sandbox.create(template="base", timeout=300, allow_internet_access=False,
                                **options(keys[0], base, base))
        live.append(reused)
        services.processes.stop("host-0")
        reused_native_id = services.native_id_for(reused.sandbox_id)
        number = services.local_for(reused.sandbox_id)
        process = services.processes.start("host-0")
        services.wait_ready(for_admission=False)
        _, contexts = original_bindings(root / "db/controller.db", [reused_native_id])
        matched = [(position, json.loads(encoded)) for position, encoded in contexts
                   if json.loads(encoded)["binding"]["sandbox"] == number]
        assert len(matched) == 1 and matched[0][0] == 0
        vm = matched[0][1]["binding"]["vm"]
        assert vm not in (0, 1)
        name = f"vm-{namespace}-{vm}"
        claim = json.loads((original_chroot(root) / ".claims" / name / "record.json").read_text())
        assert process_start_ticks(claim["pid"]) == claim["start_time_ticks"]
        claims.append((name, claim))
        status = Path(f"/proc/{claim['pid']}/status").read_text().splitlines()
        assert int(next(line for line in status if line.startswith("Uid:")).split()[1]) == args.uid_base
        for old in (first, second):
            assert data_status(opener, base, old._envd_access_token) == 401
        assert reused.commands.run("printf new-incarnation").stdout == "new-incarnation"
        killed = Sandbox.kill(reused.sandbox_id, **options(keys[0], base, base))
        assert killed
        assert terminal_cleanup_confirmed(root / "db/controller.db", reused_native_id)
        assert data_status(opener, base, reused._envd_access_token) == 401
        live.clear()
        for name, claim in claims:
            assert process_start_ticks(claim["pid"]) != claim["start_time_ticks"]
            assert not Path("/run/netns", name).exists()
            assert not (original_chroot(root) / Path(settings["firecracker"]).name / name).exists()
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 1)
        completed = True
        print(json.dumps({"source_commit": commit, "host_sha256": artifact_hash(args.host),
                          "center_sha256": artifact_hash(args.center),
                          "base_sha256": artifact_hash(image),
                          "faults": ["missing", "malformed", "valid-but-changed"],
                          "restart": "each fault preserved across the full service startup; original bindings/processes/networks unchanged",
                          "reuse": "retired physical UID reused by a new VM; old tokens remain unauthorized",
                          "result": "healthy peer commands/files/owner checks pass; damaged target denied",
                          "residual": "both exact VMs stopped; no test namespace/jail/UID process"}))
    finally:
        if credential is not None and backup.exists():
            os.replace(backup, credential)
        if live and (process is None or process.poll() is not None):
            try:
                process = services.processes.start("host-0")
                services.wait_ready(for_admission=False)
            except Exception:
                pass  # Original resources remain private and unknown, not released.
        # Even an assertion failure attempts only these two exact SDK kills.
        if process is not None and process.poll() is None:
            for key, sandbox in zip(keys, live, strict=False):
                try:
                    Sandbox.kill(sandbox.sandbox_id, **options(key, base, base))
                except Exception:
                    pass  # Failed/unknown cleanup is retained, never reported successful.
        services.stop_all()
        if completed:
            services.cleanup_staging()
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    for name in ("jailer", "firecracker", "kernel", "rootfs", "scratch-parent"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    exercise(parser.parse_args())
