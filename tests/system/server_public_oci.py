"""Public OCI acquisition in disposable build VMs, then publication and SDK use.

Root-only isolated KVM probe. The operator supplies an exclusive namespace,
UID range, delegated cgroup, fixed tools and public IPv4 forwarding. This is
not the public build API. No image layers are unpacked on the host.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import gzip
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request

from e2b import Sandbox, CommandExitException
from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import free_loopback_port, start_server, stop_server, wait_ready
from server_templates import descriptor
from server_vm_id_capacity import check_no_resources
from template_build import BUILDKIT_HASHES, MIB, checked, upload
from template_stage import stage
from template_fetch import REGCTL_SHA256


IMAGES = (
    {"name": "debian13", "reference": "docker.io/library/debian@sha256:f2306da7a8fb6440535937baed0ce0018736742d6dc5beec9d6a31355b259726",
     "selected": "sha256:f2306da7a8fb6440535937baed0ce0018736742d6dc5beec9d6a31355b259726", "os": "debian", "version": "13"},
    {"name": "ubuntu2404", "reference": "docker.io/library/ubuntu@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3",
     "selected": "sha256:496754492fb28b4d3049432f2ca787449331e23fb14f0dd3fffea86bf5a93eb4", "os": "ubuntu", "version": "24.04"},
)


def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0 or importlib.metadata.version("e2b") != "2.51.0":
        raise RuntimeError("isolated root Linux/KVM with pinned SDK required")
    if not re.fullmatch("[0-9a-f]{16}", args.namespace):
        raise ValueError("exclusive 16-hex namespace required")
    if processes_in_uid_range(args.uid_base, args.uid_base + 10):
        raise RuntimeError("exclusive UID range occupied")
    if (args.cgroup_parent / "cgroup.procs").read_text().strip() or any(p.is_dir() for p in args.cgroup_parent.iterdir()):
        raise RuntimeError("empty delegated cgroup required")
    checkout = Path(__file__).resolve().parents[2]
    git = ["git", "-C", str(checkout)]
    if subprocess.check_output(git + ["status", "--porcelain"]).strip():
        raise RuntimeError("clean exact source commit required")
    commit = subprocess.check_output(git + ["rev-parse", "HEAD"], text=True).strip()
    hashes = dict(BUILDKIT_HASHES, regctl=REGCTL_SHA256)
    for name, digest in hashes.items():
        if sha256_file(args.tools / name) != digest:
            raise RuntimeError("fixed tool fingerprint mismatch: " + name)
    if args.scratch_parent.stat().st_uid != 0 or args.scratch_parent.stat().st_mode & 0o077:
        raise RuntimeError("private root scratch required")
    if shutil.disk_usage(args.scratch_parent).free < 40 * 1024 * MIB:
        raise RuntimeError("40 GiB test disk including cleanup reserve required")
    root = Path(tempfile.mkdtemp(prefix="po-", dir=args.scratch_parent))
    print(f"test root={root} namespace={args.namespace} commit={commit}", flush=True)
    for name in ("db", "state", "jails", "images", "inputs", "registry", "registry/inbox", "registry/artifacts"):
        (root / name).mkdir(mode=0o700)
    binary = root / "server"
    shutil.copyfile(args.server, binary); binary.chmod(0o700)
    original = [sha256_file(path) for path in (args.kernel, args.rootfs)]
    base = descriptor("builder-base", args.kernel, args.rootfs, 2, 2048)
    url = f"http://127.0.0.1:{free_loopback_port()}"
    key = secrets.token_hex(32)
    settings = {
        "namespace": args.namespace, "database": str(root / "db/controller.db"), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer), "firecracker": str(args.firecracker), "chroot_base": str(root / "jails"),
        "kernel": str(args.kernel), "rootfs": str(args.rootfs), "uid_base": args.uid_base, "max_vm_id": 10,
        "vcpu_count": 2, "memory_mib": 2048,
        "host_resources": {"cgroup_parent": str(args.cgroup_parent), "memory_max_bytes": 2304 * MIB},
        "file_policy_digest": 1, "bind": url.removeprefix("http://"), "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": key}], "max_active_global": 1, "max_active_per_tenant": 1,
        "max_disk_bytes_global": 24 * 1024 * MIB, "max_disk_bytes_per_tenant": 24 * 1024 * MIB,
        "cleanup_reserve_bytes": 256 * MIB, "max_timeout_secs": 1200,
        "startup_timeout_secs": 40, "expiry_scan_seconds": 2, "public_ipv4_enabled": True,
        "templates": {"templates": [base], "aliases": {"base": "builder-base"},
                      "compute_limits": {"max_vcpus_global": 2, "max_vcpus_per_tenant": 2,
                                         "max_memory_bytes_global": 2304 * MIB, "max_memory_bytes_per_tenant": 2304 * MIB}},
        "template_registry": {"root": str(root / "registry"), "max_templates_global": 4,
            "max_templates_per_tenant": 4, "max_artifact_bytes_global": 8 * 1024 * MIB,
            "max_artifact_bytes_per_tenant": 8 * 1024 * MIB, "validation_timeout_secs": 20},
        "data_transfer": {"file_upload_bytes": 512 * MIB, "file_download_bytes": 512 * MIB,
                          "command_input_bytes": MIB, "command_output_bytes": 4 * MIB,
                          "rpc_request_bytes": MIB, "rpc_response_bytes": MIB,
                          "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
                          "request_idle_seconds": 60, "file_idle_seconds": 60, "write_idle_seconds": 60},
    }
    config = root / "config.json"
    config.write_text(json.dumps(settings)); config.chmod(0o600)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner = dict(options(key, url, url), request_timeout=180)
    process, active, evidence = None, [], []

    def no_resources():
        if (root / "jails/.claims").is_dir():
            check_no_resources(root, args.namespace, args.firecracker, args.uid_base)
        for parent in (root / "jails" / args.firecracker.name, Path("/run/netns"), args.cgroup_parent):
            assert not list(parent.glob(f"vm-{args.namespace}-*")), "test resource remains"
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 10)

    request = {"kind": "oci-reference", "disk_mib": 1024, "min_free_mib": 64,
               "max_input_bytes": 256 * MIB, "max_expanded_bytes": 1024 * MIB, "max_entries": 50000,
               "timeout_seconds": 240, "max_log_bytes": 2 * MIB}
    try:
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        jobs = list(IMAGES) + [dict(IMAGES[0], name="missing-digest", reference="docker.io/library/debian@sha256:" + "0" * 64),
                              dict(IMAGES[0], name="download-limit")]
        for item in jobs:
            name = item["name"]
            print("building " + name, flush=True)
            builder = Sandbox.create(timeout=600, allow_internet_access=True, **owner)
            active.append(builder.sandbox_id)
            checked(builder, "mkdir -p /input/tools /output")
            # Trusted public DNS for this test builder; never inherit host DNS.
            builder.files.write("/etc/resolv.conf", "nameserver 1.1.1.1\nnameserver 8.8.8.8\n", user="root")
            profile = {}
            for tool, digest in hashes.items():
                upload(builder, args.tools / tool, "/input/tools/" + tool)
                profile[tool] = {"path": "/input/tools/" + tool, "sha256": digest}
            checked(builder, "chmod 755 /input/tools/*")
            for tool in ("template_worker.py", "template_inputs.py", "template_fetch.py"):
                upload(builder, checkout / "tools" / tool, "/input/" + tool)
            builder.files.write("/input/profile.json", json.dumps(profile))
            builder.files.write("/input/source.json", json.dumps({"image": item["reference"]}))
            job = dict(request, max_input_bytes=4096) if name == "download-limit" else request
            builder.files.write("/input/request.json", json.dumps(job))
            boot_id = checked(builder, "cat /proc/sys/kernel/random/boot_id").strip()
            try:
                result = builder.commands.run("python3 /input/template_worker.py --work /output/job --source /input/source.json"
                    + " --request /input/request.json --profile /input/profile.json --guest-boot-id " + boot_id,
                    user="root", timeout=270)
            except CommandExitException as error:
                result = error
            status = json.loads(builder.files.read("/output/job/status.json"))
            (root / "inputs" / (name + "-status.json")).write_text(json.dumps(status))
            (root / "inputs" / (name + "-stderr.txt")).write_text(result.stderr)
            log = builder.files.read("/output/job/build.log")
            (root / "inputs" / (name + "-build.log")).write_text(log)
            assert len(log.encode()) <= job["max_log_bytes"]
            record = {"kind": name, "status": status}
            if name in ("missing-digest", "download-limit"):
                assert result.exit_code != 0 and status["state"] == "failed" and not status["ready"]
                assert status["error_type"] == "InvalidTemplate", (status, result.stderr)
                expected = "public image fetch failed" if name == "missing-digest" else "public image download byte limit"
                assert expected in result.stderr, result.stderr
                assert not builder.files.exists("/output/job/candidate.ext4")
                if name == "download-limit":
                    assert int(checked(builder, "stat -c %s /output/job/download.tar")) <= 4096
                record["rejected"] = expected
            else:
                assert result.exit_code == 0, (status, result.stderr)
                assert status["state"] == "candidate" and not status["ready"] and not status["startup_applied"]
                source = status["public_source"]
                assert source["reference"] == item["reference"] and source["source_digest"] == item["reference"].split("@")[1]
                assert source["selected_digest"] == status["image_digest"] == item["selected"]
                assert source["platform"] == "linux/amd64" and source["download_bytes"] <= request["max_input_bytes"]
                assert status["free_bytes"] >= status["min_free_bytes"]
                checked(builder, "gzip -1 -c /output/job/candidate.ext4 > /output/candidate.gz")
                compressed = root / "images" / (name + ".gz")
                with compressed.open("xb") as output:
                    for block in builder.files.read("/output/candidate.gz", format="stream", user="root"):
                        output.write(block)
                disk = root / "images" / (name + ".ext4")
                with gzip.open(compressed, "rb") as incoming, disk.open("xb") as outgoing:
                    remaining = status["disk_bytes"]
                    while remaining:
                        block = incoming.read(min(remaining, MIB))
                        assert block, "truncated candidate"
                        outgoing.write(block); remaining -= len(block)
                    assert not incoming.read(1)
                assert sha256_file(disk) == status["rootfs_sha256"]
                disk.chmod(0o444)
            _kill_confirmed = (Sandbox.kill(builder.sandbox_id, **owner))
            assert _kill_confirmed
            active.remove(builder.sandbox_id)
            no_resources()
            if name not in ("missing-digest", "download-limit"):
                candidate = stage(root / "registry", args.kernel, disk, status, 1, 1, 512, 768)
                publication = urllib.request.Request(url + "/sandproof/template-publications",
                    data=json.dumps({"candidateID": candidate}).encode(), method="POST",
                    headers={"X-API-Key": key, "Content-Type": "application/json"})
                with opener.open(publication, timeout=180) as response:
                    published = json.load(response)
                assert published["state"] == "ready", published
                no_resources()
                sandbox = Sandbox.create(template="tpl-" + candidate, timeout=120, allow_internet_access=False, **owner)
                active.append(sandbox.sandbox_id)
                release = checked(sandbox, '. /etc/os-release; printf "%s\\n%s\\n" "$ID" "$VERSION_ID"').splitlines()
                assert release == [item["os"], item["version"]], release
                assert checked(sandbox, "id -un; pwd").splitlines() == ["root", "/"]
                sandbox.files.write("/tmp/public-import", name)
                assert sandbox.files.read("/tmp/public-import") == name
                assert checked(sandbox, "cat /tmp/public-import").strip() == name
                _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **owner))
                assert _kill_confirmed
                active.remove(sandbox.sandbox_id)
                no_resources()
                record.update(publication="ready", sdk_file_command="passed", release=release)
            evidence.append(record)
            print(name + " passed", flush=True)
        with sqlite3.connect(root / "db/controller.db") as database:
            phases = [json.loads(blob)["phase"] for (blob,) in database.execute("SELECT state_blob FROM sandboxes")]
        assert phases == ["Destroyed"] * 8, phases
        assert [sha256_file(path) for path in (args.kernel, args.rootfs)] == original
        no_resources()
        report = {"commit": commit, "namespace": args.namespace, "server_sha256": sha256_file(binary),
                  "base_sha256": original, "tools": hashes, "sdk": "2.51.0", "evidence": evidence,
                  "destroyed_vms": len(phases), "cleanup": "passed", "scope": "guest public acquisition, publication and SDK; not public build API or recovery"}
        (root / "report.json").write_text(json.dumps(report, indent=2))
        print("passed: " + str(root / "report.json"), flush=True)
    finally:
        for sandbox_id in active:
            try:
                Sandbox.kill(sandbox_id, **owner)
            except Exception as error:
                print("cleanup unresolved: " + sandbox_id + " " + type(error).__name__, file=sys.stderr)
        if process is not None:
            stop_server(process)
        print("retained private evidence: " + str(root), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent", "cgroup-parent", "tools"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--namespace", required=True)
    exercise(parser.parse_args())
