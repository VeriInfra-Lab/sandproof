"""Public OCI acquisition in disposable build VMs, then publication and SDK use.

Root-only isolated KVM probe with a fixed complete two-pool physical profile.
The operator supplies exclusive namespaces, UID ranges, delegated cgroups,
fixed tools and public IPv4 forwarding. Private builders use original request
windows. This is not the public build API; no layers are unpacked on the host.
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
from tools.capture_acceptance import original_chroot
import re
import shutil
import subprocess
import sys
import urllib.request

from e2b import Sandbox, CommandExitException
from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import free_loopback_port
from server_vm_id_capacity import check_no_resources
from template_build import BUILDKIT_HASHES, MIB, checked, upload
from template_stage import stage
from template_fetch import REGCTL_SHA256
from _complete_registry_runtime import (
    add_registry_arguments, prepare_registry_runtime, sandbox_highwater, assert_terminal_rows,
)


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
    if processes_in_uid_range(args.uid_base, args.uid_base + 164):
        raise RuntimeError("exclusive UID range occupied")
    for parent in (args.cgroup_parent, args.build_cgroup_parent):
        if (parent / "cgroup.procs").read_text().strip() or any(p.is_dir() for p in parent.iterdir()):
            raise RuntimeError("empty delegated cgroup pools required")
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
    deployment = prepare_registry_runtime(args, tool_hashes=hashes,
        registry_limits={"max_templates_global": 4, "max_templates_per_tenant": 4,
            "max_artifact_bytes_global": 8 * 1024 * MIB, "max_artifact_bytes_per_tenant": 8 * 1024 * MIB,
            "validation_timeout_secs": 20},
        build_limits={"max_cpu": 2, "max_memory_mib": 1024, "max_disk_mib": 2048,
            "timeout_seconds": 600, "max_jobs": 20, "max_jobs_per_tenant": 10,
            "max_pending": 3, "max_pending_per_tenant": 2,
            "workspace_bytes": 80 * 1024 * MIB, "workspace_bytes_per_tenant": 60 * 1024 * MIB,
            "max_input_bytes": 256 * MIB, "max_expanded_bytes": 1024 * MIB,
            "max_entries": 50000, "max_log_bytes": 2 * MIB},
        ports=[free_loopback_port() for _ in range(3)])
    root = deployment.root
    print(f"test root={root} namespace={args.namespace} commit={commit}", flush=True)
    services = deployment.services
    original_paths = (args.kernel, args.rootfs, args.builder_rootfs or args.rootfs)
    original = [sha256_file(path) for path in original_paths]
    url, key = services.sdk_url, deployment.key
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner = dict(options(key, url, url), request_timeout=180)
    active, evidence = [], []

    def no_resources():
        for index, namespace in enumerate(deployment.namespaces):
            pool = root / str(index)
            if (original_chroot(pool) / ".claims").is_dir():
                check_no_resources(pool, namespace, args.firecracker, args.uid_base + index * 100)
            assert_terminal_rows(pool / "db/controller.db")
            for parent in (original_chroot(pool) / args.firecracker.name, Path("/run/netns"), args.build_cgroup_parent if index else args.cgroup_parent):
                assert not list(parent.glob(f"vm-{namespace}-*")), "test resource remains"
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 164)

    request = {"kind": "oci-reference", "disk_mib": 1024, "min_free_mib": 64,
               "max_input_bytes": 256 * MIB, "max_expanded_bytes": 1024 * MIB, "max_entries": 50000,
               "timeout_seconds": 240, "max_log_bytes": 2 * MIB}
    try:
        services.start()
        services.wait_ready()
        jobs = list(IMAGES) + [dict(IMAGES[0], name="missing-digest", reference="docker.io/library/debian@sha256:" + "0" * 64),
                              dict(IMAGES[0], name="download-limit")]
        for item in jobs:
            name = item["name"]
            print("building " + name, flush=True)
            builder, builder_identity = deployment.open_builder(timeout=600, internet=True)
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
            deployment.close_builder(builder_identity)
            no_resources()
            if name not in ("missing-digest", "download-limit"):
                candidate = stage(services.layout['registry'], args.kernel, disk, status, 1, 1, 512, 768,
                                  registry_owner_uid=args.center_uid)
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
        allocated = sum(sandbox_highwater(root / str(index) / "db/controller.db") for index in range(2))
        assert allocated == 8, "unexpected replacement or validation VM"
        assert [sha256_file(path) for path in original_paths] == original
        no_resources()
        report = {"commit": commit, "namespace": args.namespace, "service_artifacts": services.manifest,
                  "base_sha256": original, "tools": hashes, "sdk": "2.51.0", "evidence": evidence,
                  "destroyed_vms": allocated, "cleanup": "passed", "scope": "complete two-pool guest public acquisition, publication and SDK; not public build API or recovery"}
        (root / "report.json").write_text(json.dumps(report, indent=2))
        print("passed: " + str(root / "report.json"), flush=True)
    finally:
        try:
            if list(root.glob("builder-intent-*.json")):
                deployment.close_builders()
            for sandbox_id in active:
                try:
                    Sandbox.kill(sandbox_id, **owner)
                except Exception as error:
                    print("cleanup unresolved: " + sandbox_id + " " + type(error).__name__, file=sys.stderr)
        finally:
            services.stop_all()
        print("retained private evidence: " + str(root), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("jailer", "firecracker", "kernel", "rootfs", "scratch-parent", "cgroup-parent", "tools"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    add_registry_arguments(parser)
    exercise(parser.parse_args())
