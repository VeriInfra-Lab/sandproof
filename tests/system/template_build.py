"""Offline real-KVM experiment for the guest-only candidate builder (#123).

No public registry, host Docker, paid model or production server is used.
Synthetic Debian-compatible fixture only: not public OCI acceptance. Each
build job gets a new VM; candidate health probes are separate new VMs. This
operator-only harness is NOT a public build API. --registry also exercises
the production publication gate, immutable registry and owner-scoped aliases.
The server must include debug fault-injection hooks. For a CPU-limited service,
use an optimized build with CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true.
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
import time
import urllib.request

# Operator staging consumes bytes only; all untrusted build/unpack stays in VMs.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
from template_stage import stage

from e2b import Sandbox, CommandExitException

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import fetch, free_loopback_port, interrupted_request, process_start_ticks, start_server, stop_server, wait_ready
from server_templates import descriptor
from server_vm_id_capacity import check_no_resources


MIB = 1024 * 1024
BUILDKIT_HASHES = {
    "buildctl": "ac7c993de512a1bc7da410e7a9e3424879a5e383d4c17d804ddc54ef6d036703",
    "buildkitd": "bf5d521895dd4fafdd9bb8ba3316f90fe0eae07b795ba979bd74fee816b29991",
    "buildkit-runc": "ed94b57511421489ba02594bf311020bbdc4fbd229447f97e2734e50a3e7eeb0",
    "umoci": "b51c267ec394499e42c6fde47f240b7b7dba57ea49df0b5acd304378b82a3b71",
    "busybox": "dbac288c29ba568459550a2da9e7ae0ded6b1fc728ee9fad3044c44e62d6ac14",
    "envd": "cef25869662d730fcfe0df28dc1a43d1cf019abd9285d1aa33505940b6adc749",
}
# Optional Ubuntu 24.04 ISA-L 2.31.0 artifacts; validated inside the build VM.
IGZIP_HASHES = {
    "igzip": "1e0f85d554ae96ec35fe709c0be256bce1764e18b4e0a6ffa8db9f9cb3c0a51e",
    "libisal.so.2": "40ccd5e5abaff0451feff750d2b011370a546ac4706d1b06b9285ac26eda2c3c",
}

FIXTURE = r"""import json, os, pathlib, shutil, subprocess, tarfile
# The caller may protect host evidence with umask 077. These synthetic image
# inputs need ordinary guest-readable modes; the private parent stays private.
os.umask(0o022)
p=pathlib.Path('/input/context')
(p/'rootfs/bin').mkdir(parents=True)
(p/'rootfs/etc').mkdir()
shutil.copyfile('/input/tools/busybox', p/'rootfs/bin/busybox')
(p/'rootfs/bin/busybox').chmod(0o755)
(p/'rootfs/bin/sh').symlink_to('busybox')
shutil.copyfile('/bin/bash', p/'rootfs/bin/bash')
(p/'rootfs/bin/bash').chmod(0o755)
for line in subprocess.check_output(['ldd','/bin/bash'], text=True).splitlines():
    for part in line.split():
        if part.startswith('/'):
            dst=p/'rootfs'/part.lstrip('/')
            dst.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(part,dst)
            dst.chmod(0o755)
(p/'rootfs/etc/os-release').write_text('ID=debian\nNAME="synthetic build fixture"\n')
(p/'rootfs/etc/passwd').write_text('root:x:0:0:root:/root:/bin/bash\napp:x:1000:1000:app:/home/app:/bin/bash\n')
(p/'rootfs/etc/group').write_text('root:x:0:\napp:x:1000:\n')
(p/'rootfs/workspace').mkdir()
(p/'rootfs/workspace/start.sh').write_text('''#!/bin/bash
printf 'started\\n' >> /workspace/start-count
printf '%s\\n' "$USER" "$HOME" "$PWD" "$FIXTURE_VALUE" "$@" > /workspace/startup-result
exec /bin/busybox sleep 600
''')
(p/'payload').write_text('copied-standard-context')
(p/'Dockerfile').write_text('''FROM scratch AS build
COPY rootfs/ /
RUN /bin/busybox mkdir -p /root /tmp /opt /home/app && /bin/busybox chown 1000:1000 /workspace /home/app && echo build-ok > /workspace/built && echo old > /workspace/removed
RUN /bin/busybox rm /workspace/removed
FROM build AS final
COPY payload /workspace/payload
ENV FIXTURE_VALUE=image-value
WORKDIR /workspace
USER 1000:1000
ENTRYPOINT ["/bin/bash", "/workspace/start.sh", "entry arg"]
CMD ["literal space", "$FIXTURE_VALUE; touch /tmp/injected"]
''')
with tarfile.open('/input/context.tar','w') as out:
    for child in sorted(p.iterdir()): out.add(child,arcname=child.name)
"""


def upload(sandbox, source, destination):
    with source.open("rb") as stream:
        sandbox.files.write(destination, stream, user="root")


def checked(sandbox, command, timeout=60):
    result = sandbox.commands.run(command, user="root", timeout=timeout)
    if result.exit_code:
        raise AssertionError("guest command failed: " + command[:80])
    return result.stdout


def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("isolated root Linux/KVM required")
    if importlib.metadata.version("e2b") != "2.51.0":
        raise RuntimeError("pinned SDK required")
    identity_count = 20 if args.registry else 12
    if processes_in_uid_range(args.uid_base, args.uid_base + identity_count):
        raise RuntimeError("exclusive UID range occupied")
    if (args.cgroup_parent / "cgroup.procs").read_text().strip() or any(p.is_dir() for p in args.cgroup_parent.iterdir()):
        raise RuntimeError("empty delegated cgroup required")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("clean exact source commit required")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    build_tools = BUILDKIT_HASHES | (IGZIP_HASHES if args.igzip else {})
    for name, digest in build_tools.items():
        if sha256_file(args.tools / name) != digest:
            raise RuntimeError("fixed official tool fingerprint mismatch: " + name)
    if args.scratch_parent.stat().st_uid != 0 or args.scratch_parent.stat().st_mode & 0o077:
        raise RuntimeError("private root scratch required")
    if shutil.disk_usage(args.scratch_parent).free < 40 * 1024 * MIB:
        raise RuntimeError("40 GiB test disk including cleanup reserve required")
    root = Path(tempfile.mkdtemp(prefix="tb-", dir=args.scratch_parent))
    namespace, key = secrets.token_hex(8), secrets.token_hex(32)
    print(f"test root={root} namespace={namespace} commit={commit}", flush=True)
    for name in ("db", "state", "jails", "images", "inputs"):
        (root / name).mkdir(mode=0o700)
    binary = root / "server"
    shutil.copyfile(args.server, binary)
    binary.chmod(0o700)
    firecracker = args.firecracker.resolve(strict=True)
    if len(os.fsencode(root / "jails" / firecracker.name / f"vm-{namespace}-12/root/api.socket")) >= 108:
        raise RuntimeError("socket path too long")
    original = [sha256_file(path) for path in (args.kernel, args.rootfs)]
    base = descriptor("builder-base", args.kernel, args.rootfs, 2, 2048)
    url = f"http://127.0.0.1:{free_loopback_port()}"
    settings = {
        "namespace": namespace, "database": str(root / "db/controller.db"), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer), "firecracker": str(firecracker), "chroot_base": str(root / "jails"),
        "kernel": str(args.kernel), "rootfs": str(args.rootfs), "uid_base": args.uid_base, "max_vm_id": 12,
        "vcpu_count": 2, "memory_mib": 2048,
        "host_resources": {"cgroup_parent": str(args.cgroup_parent), "memory_max_bytes": 2304 * MIB},
        "file_policy_digest": 1, "bind": url.removeprefix("http://"), "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": key}], "max_active_global": 2, "max_active_per_tenant": 2,
        "max_disk_bytes_global": 24 * 1024 * MIB, "max_disk_bytes_per_tenant": 24 * 1024 * MIB,
        "cleanup_reserve_bytes": 256 * MIB, "max_timeout_secs": 1200,
        "startup_timeout_secs": 40, "expiry_scan_seconds": 2,
        "templates": {"templates": [base], "aliases": {"base": "builder-base"},
                      "compute_limits": {"max_vcpus_global": 4, "max_vcpus_per_tenant": 4,
                                         "max_memory_bytes_global": 4608 * MIB, "max_memory_bytes_per_tenant": 4608 * MIB}},
        "data_transfer": {"file_upload_bytes": 512 * MIB, "file_download_bytes": 512 * MIB,
                          "command_input_bytes": MIB, "command_output_bytes": 4 * MIB,
                          "rpc_request_bytes": MIB, "rpc_response_bytes": MIB,
                          "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
                          "request_idle_seconds": 60, "file_idle_seconds": 60, "write_idle_seconds": 60},
    }
    if args.registry:
        for path in (root / "registry", root / "registry/inbox", root / "registry/artifacts"):
            path.mkdir(mode=0o700)
        settings["template_registry"] = {"root": str(root / "registry"), "max_templates_global": 8,
            "max_templates_per_tenant": 8, "max_artifact_bytes_global": 12 * 1024 * MIB,
            "max_artifact_bytes_per_tenant": 12 * 1024 * MIB, "validation_timeout_secs": 20}
        settings["max_vm_id"] = 20
        foreign_key = secrets.token_hex(32)
        settings["credentials"].append({"tenant": 2, "secret": foreign_key})
    config = root / "config.json"
    def save():
        config.write_text(json.dumps(settings))
        config.chmod(0o600)
    save()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner = options(key, url, url)
    process, active, complete, evidence = None, [], False, []
    def no_resources():
        if (root / "jails/.claims").is_dir():
            check_no_resources(root, namespace, firecracker, args.uid_base)
        # .claims intentionally retains stopped identity tombstones; inspect
        # their PID/start time above, not their directory names as live VMs.
        for parent in (root / "jails" / firecracker.name, Path("/run/netns"), Path("/sys/fs/cgroup")):
            assert not list(parent.rglob(f"vm-{namespace}-*")), "test resource remains"
        assert not processes_in_uid_range(args.uid_base, args.uid_base + identity_count)
    request = {"kind": "dockerfile", "disk_mib": 1024, "min_free_mib": 64,
               "max_input_bytes": 256 * MIB, "max_expanded_bytes": 512 * MIB, "max_entries": 10000,
               "timeout_seconds": 120, "max_log_bytes": 2 * MIB}
    published = {}
    def publication(candidate):
        code, body = fetch(opener, url, "POST", "/sandproof/template-publications", key, {"candidateID": candidate})
        assert code == 200, (code, body)
        return json.loads(body)
    def staging(kernel=args.kernel):
        return stage(root / "registry", kernel, disk, status, 1, 1, 256, 512)
    try:
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        for kind in ("dockerfile", "oci", "failure", "timeout"):
            builder = Sandbox.create(timeout=600, allow_internet_access=False, **owner)
            active.append(builder.sandbox_id)
            checked(builder, "mkdir -p /input/tools /output")
            profile = {}
            for name, digest in build_tools.items():
                upload(builder, args.tools / name, "/input/tools/" + name)
                profile[name] = {"path": "/input/tools/" + name, "sha256": digest}
            checked(builder, "chmod 755 /input/tools/*")
            for name in ("template_worker.py", "template_inputs.py"):
                upload(builder, checkout / "tools" / name, "/input/" + name)
            builder.files.write("/input/profile.json", json.dumps(profile))
            boot_id = checked(builder, "cat /proc/sys/kernel/random/boot_id").strip()
            if kind == "oci":
                upload(builder, root / "inputs/built-oci.tar.gz", "/input/source.tar.gz")
                source = "/input/source.tar.gz"
            else:
                builder.files.write("/input/fixture.py", FIXTURE)
                checked(builder, "python3 /input/fixture.py")
                if kind in ("failure", "timeout"):
                    suffix = "RUN exit 17\n" if kind == "failure" else "RUN /bin/busybox sleep 120\n"
                    builder.files.write("/input/context/Dockerfile", "FROM scratch\nCOPY rootfs/ /\n" + suffix)
                    checked(builder, "tar -C /input/context -cf /input/context.tar .")
                source = "/input/context.tar"
            job_request = dict(request, kind="oci" if kind == "oci" else "dockerfile",
                               timeout_seconds=8 if kind == "timeout" else 120)
            builder.files.write("/input/request.json", json.dumps(job_request))
            try:
                result = builder.commands.run(
                    "python3 /input/template_worker.py --work /output/job --source " + source
                    + " --request /input/request.json --profile /input/profile.json --guest-boot-id " + boot_id,
                    user="root", timeout=150)
            except CommandExitException as error:
                result = error
            status = json.loads(builder.files.read("/output/job/status.json"))
            # Save bounded evidence before destroying the build VM, including
            # failures. No public/user inputs or credentials in this fixture.
            (root / "inputs" / (kind + "-status.json")).write_text(json.dumps(status))
            for name in ("build.log", "daemon.log"):
                if builder.files.exists("/output/job/" + name):
                    (root / "inputs" / (kind + "-" + name)).write_text(builder.files.read("/output/job/" + name))
            daemon_log = root / "inputs" / (kind + "-daemon.log")
            selected = re.findall(r'msg="auto snapshotter: using (overlayfs|fuse-overlayfs|native)"',
                                  daemon_log.read_text()) if daemon_log.exists() else []
            assert len(selected) <= 1, "one private BuildKit worker per build"
            snapshotter = selected[0] if selected else None
            if kind in ("failure", "timeout"):
                assert result.exit_code != 0 and status["state"] == "failed" and not status["ready"]
                if kind == "timeout":
                    assert status["error_type"] == "TimeoutError"
                else:
                    assert "exit code: 17" in (root / "inputs/failure-build.log").read_text()
                assert not builder.files.exists("/output/job/candidate.ext4")
                evidence.append({"kind": kind, "status": status, "snapshotter": snapshotter})
            else:
                assert result.exit_code == 0, (result.stderr, result.stdout)
                upload(builder, checkout / "tests/system/umoci_unpack_guest.py", "/input/unpack-check.py")
                comparison = json.loads(checked(builder, "python3 /input/unpack-check.py", timeout=150))
                evidence.append({"kind": kind + "-unpack-equivalence", **comparison})
                if kind == "dockerfile":
                    assert snapshotter is not None, "retain actual automatic snapshotter selection"
                    assert not builder.files.exists("/output/job/cache"), "release private build snapshots before candidate conversion"
                    upload(builder, checkout / "tests/system/buildkit_export_guest.py", "/input/export-check.py")
                    comparison = json.loads(checked(builder, "python3 /input/export-check.py", timeout=150))
                    evidence.append({"kind": "buildkit-export-equivalence", **comparison})
                assert status["state"] == "candidate" and not status["ready"] and not status["startup_applied"]
                assert status["startup"]["WorkingDir"] == "/workspace"
                assert "FIXTURE_VALUE=image-value" in status["startup"]["Env"]
                assert status["runtime_defaults"]["user"] == "app"
                assert any(value.startswith("PATH=") for value in status["startup"]["Env"])
                assert status["free_bytes"] >= status["min_free_bytes"]
                checked(builder, "gzip -1 -c /output/job/candidate.ext4 > /output/candidate.gz")
                compressed = root / "images" / (kind + ".gz")
                with compressed.open("xb") as output:
                    for block in builder.files.read("/output/candidate.gz", format="stream", user="root"):
                        output.write(block)
                disk = root / "images" / (kind + ".ext4")
                with gzip.open(compressed, "rb") as incoming, disk.open("xb") as outgoing:
                    remaining = status["disk_bytes"]
                    while remaining:
                        block = incoming.read(min(remaining, MIB))
                        if not block:
                            raise AssertionError("truncated candidate")
                        outgoing.write(block)
                        remaining -= len(block)
                    assert not incoming.read(1)
                assert sha256_file(disk) == status["rootfs_sha256"]
                disk.chmod(0o444)
                if kind == "dockerfile":
                    checked(builder, "tar -C /output/job/oci -czf /output/oci.tar.gz .")
                    with (root / "inputs/built-oci.tar.gz").open("xb") as output:
                        for block in builder.files.read("/output/oci.tar.gz", format="stream", user="root"):
                            output.write(block)
                evidence.append({"kind": kind, "status": status, "snapshotter": snapshotter})
            _kill_confirmed = (Sandbox.kill(builder.sandbox_id, **owner))
            assert _kill_confirmed
            active.remove(builder.sandbox_id)
            no_resources()
            if kind in ("dockerfile", "oci"):
                if args.registry:
                    candidate_id = staging()
                    result = publication(candidate_id)
                    assert result["state"] == "ready", result
                    assert result["template"]["diskBytes"] == status["disk_bytes"]
                    assert result["template"]["minFreeBytes"] == status["min_free_bytes"]
                    published[kind] = result["template"]["templateId"]
                    with sqlite3.connect(root / "db/controller.db") as database:
                        count = database.execute("SELECT count(*) FROM sandboxes").fetchone()[0]
                    assert publication(candidate_id) == result
                    with sqlite3.connect(root / "db/controller.db") as database:
                        assert database.execute("SELECT count(*) FROM sandboxes").fetchone()[0] == count
                    assert fetch(opener,url,"GET","/sandproof/template-publications/" + candidate_id,foreign_key)[0] == 404
                    assert fetch(opener,url,"GET","/sandproof/templates/" + published[kind],foreign_key)[0] == 404
                    code, _ = fetch(opener,url,"PUT","/sandproof/template-aliases/candidate-" + kind,key,
                                    {"templateID": published[kind]})
                    assert code == 200
                    no_resources()
                    evidence.append({"kind": "published-" + kind, "template_id": published[kind],
                                     "cleanup_before_ready": True, "duplicate_no_vm": True, "foreign_hidden": True})
                else:
                    stop_server(process)
                    candidate = descriptor("candidate-" + kind, args.kernel, disk, 1, 256)
                    candidate["startup"] = status["runtime_defaults"]
                    settings["templates"]["templates"].append(candidate)
                    save()
                    process = start_server(binary, config, root / "server.log", None)
                    wait_ready(opener, url, key, process)
                guest = Sandbox.create(template="candidate-" + kind, envs={"FIXTURE_VALUE": "create-value"},
                                       timeout=120, allow_internet_access=False, **owner)
                active.append(guest.sandbox_id)
                assert checked(guest, "printf sdk-boot-ok") == "sdk-boot-ok"
                defaults = guest.commands.run('printf "%s|%s|%s" "$USER" "$PWD" "$FIXTURE_VALUE"')
                assert defaults.stdout == "app|/workspace|create-value", defaults
                explicit = guest.commands.run('printf "%s|%s|%s" "$USER" "$PWD" "$FIXTURE_VALUE"',
                                              user="root", cwd="/tmp", envs={"FIXTURE_VALUE": "request-value"})
                assert explicit.stdout == "root|/tmp|request-value"
                deadline = time.monotonic() + 5
                while not guest.files.exists("/workspace/startup-result"):
                    assert time.monotonic() < deadline
                    time.sleep(0.05)
                expected = ["app", "/home/app", "/workspace", "create-value", "entry arg",
                            "literal space", "$FIXTURE_VALUE; touch /tmp/injected"]
                assert guest.files.read("/workspace/startup-result").splitlines() == expected
                assert guest.files.read("/workspace/start-count") == "started\n"
                assert not guest.files.exists("/tmp/injected")
                assert guest.files.read("/workspace/payload") == "copied-standard-context"
                assert guest.files.read("/workspace/built").strip() == "build-ok"
                assert not guest.files.exists("/workspace/removed")
                guest.files.write("/workspace/private", "not-in-template")
                if args.registry and kind == "oci":
                    code, _ = fetch(opener,url,"PUT","/sandproof/template-aliases/candidate-oci",key,
                                    {"templateID": published["dockerfile"], "expectedTemplateID": published["oci"]})
                    assert code == 200
                    assert Sandbox.get_info(guest.sandbox_id, **owner).template_id == published["oci"]
                stop_server(process)
                process = start_server(binary, config, root / "server.log", None)
                wait_ready(opener, url, key, process)
                assert guest.files.read("/workspace/start-count") == "started\n"
                if args.registry:
                    assert Sandbox.get_info(guest.sandbox_id, **owner).template_id == published[kind]
                    code, body = fetch(opener,url,"GET","/sandproof/templates/candidate-" + kind,key)
                    assert code == 200
                    assert json.loads(body)["templateId"] == published["dockerfile"]
                    code, _ = fetch(opener,url,"PUT","/sandproof/template-aliases/candidate-" + kind,key,
                                    {"templateID": published[kind]})
                    assert code == 200
                _kill_confirmed = (Sandbox.kill(guest.sandbox_id, **owner))
                assert _kill_confirmed
                active.remove(guest.sandbox_id)
                no_resources()
                assert sha256_file(disk) == status["rootfs_sha256"]
        if args.registry:
            # A kernel can match its own hash yet still fail a real VM launch.
            bad_kernel = root / "images/bad-kernel"
            bad_kernel.write_bytes(b"not-a-linux-kernel")
            bad_kernel.chmod(0o444)
            # Last loop statuses are failures; reuse the successful OCI evidence.
            status = next(v["status"] for v in evidence if v["kind"] == "oci")
            disk = root / "images/oci.ext4"
            bad = staging(bad_kernel)
            rejected = publication(bad)
            deadline = time.monotonic() + 60
            while rejected["state"] == "checking":
                assert time.monotonic() < deadline, "failed validation was not reclaimed"
                time.sleep(0.2)
                rejected = publication(bad)
            assert rejected["state"] == "failed" and rejected["template"] is None, rejected
            no_resources()
            assert fetch(opener,url,"POST","/v2/sandboxes",key,
                         {"templateID": "tpl-" + bad, "allow_internet_access": False})[0] == 404
            # Exit after successful validation, during cleanup, before its receipt commits.
            pending = staging()
            stop_server(process)
            process = start_server(binary, config, root / "server.log", "cleanup_vm")
            wait_ready(opener,url,key,process)
            interrupted_request(lambda: publication(pending),process)
            with sqlite3.connect(root / "db/controller.db") as database:
                saved = json.loads(database.execute("SELECT record_json FROM template_publications WHERE candidate_id=?",(pending,)).fetchone()[0])
                identities = database.execute("SELECT count(*) FROM sandboxes").fetchone()[0]
            assert saved["state"] == "checking" and saved["checked"] is True
            process = start_server(binary,config,root / "server.log",None)
            wait_ready(opener,url,key,process)
            deadline = time.monotonic() + 20
            while publication(pending)["state"] == "checking":
                assert time.monotonic() < deadline
                time.sleep(0.2)
            assert publication(pending)["state"] == "ready"
            with sqlite3.connect(root / "db/controller.db") as database:
                assert database.execute("SELECT count(*) FROM sandboxes").fetchone()[0] == identities
            no_resources()
            evidence.append({"kind": "publication-restart", "cleanup_receipt_lost": True,
                             "no_reprobe": True, "failed_kernel_not_ready": True, "alias_binding_persisted": True})
        # Two finite activation crash windows: confirmed guest Start but lost
        # controller receipt, and Start accepted but host confirmation lost.
        # This is not arbitrary application readiness or crash-site coverage.
        def startup_count(name, sandbox_id):
            secret = json.loads((root / "state/secrets" / sandbox_id).read_text())
            curl_config = (f'url = "http://172.16.0.2:49983/files?path=%2Fworkspace%2Fstart-count"\n'
                           f'header = "X-Access-Token: {secret["token"]}"\n')
            result = subprocess.run(["/usr/bin/ip", "netns", "exec", name, "/usr/bin/curl", "-q", "--noproxy", "*",
                                     "-fs", "--max-time", "5", "-K", "-"], input=curl_config, text=True, capture_output=True)
            assert result.returncode == 0, "private guest observation failed"
            return result.stdout
        def wait_started(name, sandbox_id):
            # Start acknowledges process creation; the child may not yet have
            # written its first file. Observe that witness before crashing or
            # restarting again, without replaying Start or extending a VM lease.
            deadline = time.monotonic() + 5
            while True:
                try:
                    count = startup_count(name, sandbox_id)
                except AssertionError:
                    if time.monotonic() >= deadline:
                        raise
                else:
                    assert count in ("", "started\n"), "startup ran more than once"
                    if count:
                        return
                    assert time.monotonic() < deadline, "startup witness missing"
                time.sleep(0.05)
        if args.registry:
            # Activation succeeded physically, but the publisher never durably
            # confirmed validation. Recovery may NOT turn that into Ready.
            pending = staging()
            stop_server(process)
            process = start_server(binary,config,root / "server.log","activate_vm")
            wait_ready(opener,url,key,process)
            interrupted_request(lambda: publication(pending),process)
            with sqlite3.connect(root / "db/controller.db") as database:
                saved = json.loads(database.execute("SELECT record_json FROM template_publications WHERE candidate_id=?",(pending,)).fetchone()[0])
                identities = database.execute("SELECT count(*) FROM sandboxes").fetchone()[0]
            assert saved["state"] == "checking" and saved["checked"] is False
            sandbox_id = f'sb-{namespace}-{saved["validation_sandbox"]}'
            record = next(
                json.loads(p.read_text()) for p in (root / "state/startup").glob("*.json")
                if json.loads(p.read_text())["sandbox"] == saved["validation_sandbox"])
            name = f'vm-{namespace}-{record["vm"]}'
            wait_started(name, sandbox_id)
            process = start_server(binary,config,root / "server.log",None)
            wait_ready(opener,url,key,process)
            assert fetch(opener,url,"GET","/sandboxes/" + sandbox_id,key)[0] == 404
            assert startup_count(name,sandbox_id) == "started\n", "validation startup replayed"
            deadline = time.monotonic() + 60
            state = publication(pending)["state"]
            while state == "checking":
                assert time.monotonic() < deadline
                time.sleep(0.2)
                state = publication(pending)["state"]
            assert state == "failed", "unconfirmed validation became Ready"
            with sqlite3.connect(root / "db/controller.db") as database:
                assert database.execute("SELECT count(*) FROM sandboxes").fetchone()[0] == identities
            no_resources()
            evidence.append({"kind": "publication-unconfirmed", "never_ready": True,
                             "internal_session_hidden": True, "no_reprobe": True, "exact_cleanup": True})
        for window in ("confirmed", "unknown"):
            stop_server(process)
            process = start_server(binary, config, root / "server.log", "activate_vm" if window == "confirmed" else None,
                                   template_start_target="activate_vm" if window == "unknown" else None)
            wait_ready(opener, url, key, process)
            interrupted_request(lambda: fetch(opener, url, "POST", "/v2/sandboxes", key,
                {"templateID": "candidate-oci", "timeout": 120, "allow_internet_access": False}),
                process, 86 if window == "confirmed" else 88)
            records = [json.loads(path.read_text()) for path in (root / "state/startup").glob("*.json")]
            record = max(records, key=lambda value: value["vm"])
            sandbox_id, name = f'sb-{namespace}-{record["sandbox"]}', f'vm-{namespace}-{record["vm"]}'
            assert (record["guest_pid"] is not None) == (window == "confirmed")
            assert process_start_ticks(record["vmm_pid"]) == record["start_ticks"]
            wait_started(name, sandbox_id)
            process = start_server(binary, config, root / "server.log", None)
            wait_ready(opener, url, key, process)
            assert startup_count(name, sandbox_id) == "started\n", "startup was replayed"
            if window == "confirmed":
                guest = Sandbox.connect(sandbox_id, **owner)
                assert guest.files.read("/workspace/start-count") == "started\n"
                _kill_confirmed = (Sandbox.kill(sandbox_id, **owner))
                assert _kill_confirmed
            else:
                code, body = fetch(opener, url, "GET", "/sandboxes/" + sandbox_id, key)
                # Existing detail contract exposes only live Running instances,
                # not a Starting response containing a usable guest credential.
                assert code == 404, (code, body)
                with sqlite3.connect(root / "db/controller.db") as database:
                    saved = json.loads(database.execute("SELECT state_blob FROM sandboxes WHERE sandbox_id=?",
                                                        (sandbox_id,)).fetchone()[0])
                assert saved["phase"] == "Starting", saved["phase"]
                deadline = time.monotonic() + 50
                while process_start_ticks(record["vmm_pid"]) == record["start_ticks"]:
                    try:
                        assert startup_count(name, sandbox_id) == "started\n"
                    except AssertionError:
                        # Exact stop can win between the liveness check and
                        # this read; a live guest must still answer exactly once.
                        if process_start_ticks(record["vmm_pid"]) != record["start_ticks"]:
                            break
                        raise
                    assert time.monotonic() < deadline, "unknown startup was not retired"
                    time.sleep(0.2)
            deadline = time.monotonic() + 10
            while True:
                try:
                    no_resources()
                    break
                except AssertionError:
                    assert time.monotonic() < deadline
                    time.sleep(0.1)
            evidence.append({"kind": "activation-" + window, "no_startup_replay": True, "exact_cleanup": True})
        assert original == [sha256_file(path) for path in (args.kernel, args.rootfs)]
        complete = True
        print(json.dumps({"source_commit": commit, "server_sha256": sha256_file(binary), "namespace": namespace,
                          "probe_sha256": sha256_file(Path(__file__)), "vms_destroyed": 13 if args.registry else 8, "evidence": evidence,
                          "scope": "formal publication/registry with offline synthetic builder fixture" if args.registry else
                          "offline synthetic fixture with defaults and argv dispatch; no public OCI, user API or production Ready gate"}))
    finally:
        if not complete:
            print("Exact failed-run artifacts retained: " + str(root), file=sys.stderr)
        # SDK create can fail after durable acceptance without returning an ID.
        # Recover only identities in this fresh private journal, not global VM
        # names or a Running-only list. Never allocate a replacement create.
        try:
            identities = []
            if (root / "db/controller.db").is_file():
                with sqlite3.connect("file:" + str(root / "db/controller.db") + "?mode=ro",uri=True) as database:
                    if database.execute("SELECT count(*) FROM sqlite_master WHERE name='sandboxes'").fetchone()[0]:
                        identities = [row[0] for row in database.execute("SELECT sandbox_id FROM sandboxes")]
            if identities and (process is None or process.poll() is not None):
                # A failed observation can happen after the intentional service
                # crash. Resume its original journal without injection so exact
                # cleanup can reconcile; never replace the pending sandbox.
                process = start_server(binary, config, root / "server.log", None)
                wait_ready(opener, url, key, process)
            for identity in identities:
                assert identity.startswith("sb-" + namespace + "-")
                try:
                    status, _ = fetch(opener, url, "DELETE", "/sandboxes/" + identity, key)
                    if status not in (200, 204, 404):
                        raise RuntimeError("cleanup unconfirmed")
                except Exception:
                    print("cleanup unknown for exact sandbox " + identity, file=sys.stderr)
            # A Starting create may have no accepted kill yet. The ordinary
            # startup deadline scanner must fail-close and reclaim it; this
            # harness never changes journal rows or fabricates host receipts.
            deadline = time.monotonic() + 50
            while True:
                try:
                    no_resources()
                    break
                except AssertionError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.2)
        finally:
            stop_server(process)
        if complete:
            shutil.rmtree(root)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent", "cgroup-parent", "tools"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--registry", action="store_true")
    parser.add_argument("--igzip", action="store_true", help="check and use the fixed optional ISA-L decoder pair")
    exercise(parser.parse_args())
