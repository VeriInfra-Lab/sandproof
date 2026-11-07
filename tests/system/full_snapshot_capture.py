"""Approved isolated Pause/Full/seal probe, not a lifecycle end-to-end test.

Never starts follow-promotions, changes shared host configuration or cancels jobs.
An unexpected result retains its exact VM/files and exclusive-host responsibility.
Successful teardown is explicit discard of this test's original VM and copies.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from pathlib import Path

import argparse
import fcntl
import hashlib
import http.client
import json
import os
import secrets
import socket
import stat
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.runtime_create_recovery import LIFECYCLE_PROTOCOL_VERSION
from tools.check_kvm_host import trusted_path
from tools.ci_e2e_supervisor import HostLease

EXPECTED = {
    "firecracker": "2fd0171309af7e24cf8dafc8a6f921c1434c49b5f9349bb996b7ed0a4deb8aa7",
    "jailer": "1f3a0c1fe86212d0001819bfe0819071c01208b3ccc9398c3b3bc1b84cf21edd",
    "kernel": "b20af7585283b051f16f6ece46e7064165054efe112c4ab0e26a06ff8ebe9da4",
    "rootfs": "30a3861b873e077d2f9a1c215e1e68702261ef3a9af1db3e2c6e6f07b73d5391",
}

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def write_new(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2); stream.flush(); os.fsync(stream.fileno())

def fingerprint(value):
    return list(hashlib.sha256(json.dumps(value, sort_keys=True).encode()).digest())

def process_matches(source):
    if Path("/proc/sys/kernel/random/boot_id").read_text().strip() != source["boot_id"]:
        return False
    try:
        fields = Path(f"/proc/{source['pid']}/stat").read_text().rsplit(")", 1)[1].split()
    except FileNotFoundError:
        return False
    return fields[0] not in ("Z", "X") and int(fields[19]) == source["start_time_ticks"]

def uid_processes(uid):
    return sum(int(value) == uid for value in subprocess.check_output(["ps", "-eo", "uid="], text=True).split())

def paused_api(path):
    connection = http.client.HTTPConnection("localhost", timeout=5)
    connection.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.sock.settimeout(5); connection.sock.connect(str(path))
    try:
        connection.request("GET", "/")
        response = connection.getresponse(); body = response.read(1_048_577)
        assert response.status == 200 and len(body) <= 1_048_576
        assert json.loads(body)["state"] == "Paused"
    finally:
        connection.close()

def fixture(case, assets, uid, tag):
    compatibility = {
        "snapshot_format": 1, "tool": fingerprint({k: EXPECTED[k] for k in ("firecracker", "jailer")}),
        "cpu": fingerprint(Path("/proc/cpuinfo").read_text().split("\n\n", 1)[0]),
        "devices": fingerprint({"vcpu": 1, "memory_mib": 512, "drives": ["rootfs.ext4"]}),
        "image_configuration": fingerprint({"kernel": EXPECTED["kernel"], "rootfs": EXPECTED["rootfs"], "boot_args": "console=ttyS0 reboot=k panic=1 pci=off"}),
        "writable_drives": [{"drive_id": "rootfs", "jail_relative_path": "rootfs.ext4"}],
    }
    deployment = "snapshot-component-" + tag
    return {
        "runtime": {"console_policy": None, "host_slot": None, **assets,
            "chroot_base": str(case / "p" / "v"), "vm_uid": uid, "vm_gid": uid,
            "vcpu_count": 1, "memory_mib": 512, "host_resources": None,
            "private_netns": None, "access_token_hash": None},
        "resource_id": "snapshot-" + tag, "store": str(case / "p" / "s"),
        "layout_parent": str(case / "p"),
        "layout_binding": fingerprint({"profile":"stage1-siblings-v1","deployment":deployment,"assets":EXPECTED,"uid":uid,"parent":str(case / "p")}),
        "binding": {"operation": {"protocol_version": LIFECYCLE_PROTOCOL_VERSION,
            "node": {"deployment_id": deployment, "node_id": "component-host", "node_generation": 1},
            "session": "session-" + tag, "key": {"deployment": deployment, "owner": "component-owner", "window": 1, "sequence": 2},
            "expected": {"state": "version", "version": 1}}, "source_incarnation": 1, "capture": 2},
        "compatibility": compatibility, "expected_source": None,
    }

def invoke(binary, mode, inputs, report, log, timeout):
    with log.open("xb") as stream:
        subprocess.run([str(binary), mode, str(inputs), str(report)], stdin=subprocess.DEVNULL,
            stdout=stream, stderr=subprocess.STDOUT, timeout=timeout, check=True)

def discard_copies(case, records):
    saved = records[-1]
    paths = [entry["relative_path"] for entry in saved["manifest"]["files"]]
    assert paths == ["state.bin", "memory.bin", "disk-0.img"]
    key = bytes(records[2]["directory_key"]).hex()
    failed = case / "p" / "s" / ("staging-" + key)
    assert list(failed.iterdir()) == []
    failed.rmdir()
    # Original VMM and both managers have exited; this probe never loads/maps
    # its saved bundle. This is explicit test discard, not production GC.
    assert len(saved["directory_key"]) == 32
    bundle = case / "p" / "s" / ("capture-" + bytes(saved["directory_key"]).hex())
    assert list((case / "p" / "s").iterdir()) == [bundle]
    with (bundle / "manifest.json").open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").digest() == bytes(saved["manifest_sha256"])
    assert set(p.name for p in bundle.iterdir()) == set(paths + ["manifest.json"])
    for name in paths + ["manifest.json"]:
        path = bundle / name; metadata = path.lstat()
        assert stat.S_ISREG(metadata.st_mode) and metadata.st_uid == 0 and metadata.st_nlink == 1
        path.unlink()
    bundle.rmdir()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--scratch-parent", type=Path, required=True)
    parser.add_argument("--ci-state-root", type=Path, required=True)
    parser.add_argument("--native-lock", type=Path, required=True)
    parser.add_argument("--vm-uid", type=int, required=True)
    args = parser.parse_args()
    if os.geteuid() != 0: raise ValueError("approved isolated root host required")
    os.umask(0o077)
    for path in (Path(__file__), args.binary, args.assets, args.scratch_parent, args.ci_state_root): trusted_path(path)
    assert digest(args.binary) == args.binary_sha256
    assert args.assets.stat().st_size <= 16384
    assets = json.loads(args.assets.read_text()); assert set(assets) == set(EXPECTED)
    for name, expected in EXPECTED.items():
        path = trusted_path(Path(assets[name])); assert path.is_file() and digest(path) == expected
    for name in ("firecracker", "jailer"):
        assert "1.16.1" in subprocess.check_output([assets[name], "--version"], text=True, timeout=10)
    assert args.vm_uid > 65535 and uid_processes(args.vm_uid) == 0
    tag = secrets.token_hex(6); lease = HostLease(args.ci_state_root, "snapshot-component-" + tag)
    leased = False; possibly_launched = False; cleaned = False; case = None
    result = {"complete": False, "kind": "physical-capture-component-not-full-lifecycle", "assets": EXPECTED}
    start = time.monotonic()
    try:
        lease.pause(); leased = True
        deadline = time.monotonic() + 180
        while not lease.exclusive():
            if time.monotonic() >= deadline: raise TimeoutError("existing CI work did not finish; none cancelled")
            time.sleep(0.2)
        lock = os.open(args.native_lock, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
        with os.fdopen(lock, "a") as stream:
            while True:
                try: fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB); break
                except BlockingIOError:
                    if time.monotonic() >= deadline: raise TimeoutError("existing native work did not finish")
                    time.sleep(0.2)
            assert uid_processes(args.vm_uid) == 0
            assert os.statvfs(args.scratch_parent).f_bavail * os.statvfs(args.scratch_parent).f_frsize >= 8 * 1024**3
            memory = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
            assert int(memory["MemAvailable"].split()[0]) * 1024 >= 4 * 1024**3
            case = Path(tempfile.mkdtemp(prefix="sp638-", dir=args.scratch_parent))
            for name in ("p", "e"): (case / name).mkdir(mode=0o700)
            inputs = fixture(case, assets, args.vm_uid, tag); write_new(case / "e/input.json", inputs)
            invoke(args.binary, "initialize-layout", case / "e/input.json", case / "e/initialize.jsonl", case / "e/initialize.log", 10)
            assert json.loads((case / "e/initialize.jsonl").read_text())["phase"] == "layout_initialized"
            possibly_launched = True
            invoke(args.binary, "capture", case / "e/input.json", case / "e/capture.jsonl", case / "e/capture.log", 180)
            records = [json.loads(line) for line in (case / "e/capture.jsonl").read_text().splitlines()]
            assert [r["phase"] for r in records] == ["launched", "original_paused", "failed_capture_preserved_original", "physical_bundle_sealed_original_still_paused"]
            source = records[0]["source"]; assert process_matches(source)
            socket_path = case / "p" / "v" / Path(assets["firecracker"]).name / inputs["resource_id"] / "root/api.socket"
            paused_api(socket_path)
            inputs["expected_source"] = source; write_new(case / "e/cleanup-input.json", inputs)
            invoke(args.binary, "cleanup-test-original", case / "e/cleanup-input.json", case / "e/cleanup.jsonl", case / "e/cleanup.log", 30)
            assert not process_matches(source)
            assert not socket_path.parent.parent.exists()
            deadline = time.monotonic() + 10
            while uid_processes(args.vm_uid):
                if time.monotonic() >= deadline: raise TimeoutError("original UID still owns tasks")
                time.sleep(0.1)
            discard_copies(case, records)
            invoke(args.binary, "check-layout", case / "e/input.json", case / "e/reopened.jsonl", case / "e/reopened.log", 10)
            assert json.loads((case / "e/reopened.jsonl").read_text())["phase"] == "original_layout_reopened"
            cleaned = True; result.update(complete=True, source=source, resource_id=inputs["resource_id"], test_payload_paths_removed=True, source_exited=True)
    finally:
        result.update(seconds=time.monotonic()-start, possibly_launched=possibly_launched, cleaned=cleaned)
        if case is not None: write_new(case / "e/result.json", result)
        if leased and (not possibly_launched or cleaned): lease.release()
        print(json.dumps(result))

if __name__ == "__main__": main()
