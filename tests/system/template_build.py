"""Offline real-KVM experiment for the guest-only candidate builder (#123).

No public registry, host Docker, paid model or production server is used.
Synthetic Debian-compatible fixture only: not public OCI acceptance. Each
build job gets a new VM; candidate health probes are separate new VMs. This
operator-only harness is NOT a public build API or production publish gate.
"""

import argparse
import gzip
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import urllib.request

from e2b import Sandbox, CommandExitException

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import fetch, free_loopback_port, start_server, stop_server, wait_ready
from server_templates import descriptor
from server_vm_id_capacity import check_no_resources


MIB = 1024 * 1024
BUILDKIT_HASHES = {
    "buildctl": "ac7c993de512a1bc7da410e7a9e3424879a5e383d4c17d804ddc54ef6d036703",
    "buildkitd": "bf5d521895dd4fafdd9bb8ba3316f90fe0eae07b795ba979bd74fee816b29991",
    "buildkit-runc": "ed94b57511421489ba02594bf311020bbdc4fbd229447f97e2734e50a3e7eeb0",
    "umoci": "b51c267ec394499e42c6fde47f240b7b7dba57ea49df0b5acd304378b82a3b71",
}
FIXTURE = r"""import json, os, pathlib, shutil, subprocess, tarfile
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
(p/'rootfs/etc/passwd').write_text('root:x:0:0:root:/root:/bin/bash\n')
(p/'rootfs/etc/group').write_text('root:x:0:\n')
(p/'payload').write_text('copied-standard-context')
(p/'Dockerfile').write_text('''FROM scratch AS build
COPY rootfs/ /
RUN /bin/busybox mkdir -p /root /tmp /opt /workspace && echo build-ok > /workspace/built && echo old > /workspace/removed
RUN /bin/busybox rm /workspace/removed
FROM build AS final
COPY payload /workspace/payload
ENV FIXTURE_VALUE=preserved-not-yet-activated
WORKDIR /workspace
USER root
CMD ["/bin/bash", "-c", "printf startup"]
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
    if processes_in_uid_range(args.uid_base, args.uid_base + 12):
        raise RuntimeError("exclusive UID range occupied")
    if (args.cgroup_parent / "cgroup.procs").read_text().strip() or any(p.is_dir() for p in args.cgroup_parent.iterdir()):
        raise RuntimeError("empty delegated cgroup required")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("clean exact source commit required")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    for name, digest in BUILDKIT_HASHES.items():
        if sha256_file(args.tools / name) != digest:
            raise RuntimeError("fixed official tool fingerprint mismatch: " + name)
    if args.scratch_parent.stat().st_uid != 0 or args.scratch_parent.stat().st_mode & 0o077:
        raise RuntimeError("private root scratch required")
    if shutil.disk_usage(args.scratch_parent).free < 40 * 1024 * MIB:
        raise RuntimeError("40 GiB test disk including cleanup reserve required")
    root = Path(tempfile.mkdtemp(prefix="tb-", dir=args.scratch_parent))
    namespace, key = secrets.token_hex(8), secrets.token_hex(32)
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
        "cleanup_reserve_bytes": 256 * MIB, "max_timeout_secs": 1200, "expiry_scan_seconds": 3600,
        "templates": {"templates": [base], "aliases": {"base": "builder-base"},
                      "compute_limits": {"max_vcpus_global": 4, "max_vcpus_per_tenant": 4,
                                         "max_memory_bytes_global": 4608 * MIB, "max_memory_bytes_per_tenant": 4608 * MIB}},
        "data_transfer": {"file_upload_bytes": 512 * MIB, "file_download_bytes": 512 * MIB,
                          "command_input_bytes": MIB, "command_output_bytes": 4 * MIB,
                          "rpc_request_bytes": MIB, "rpc_response_bytes": MIB,
                          "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
                          "request_idle_seconds": 60, "file_idle_seconds": 60, "write_idle_seconds": 60},
    }
    config = root / "config.json"
    def save():
        config.write_text(json.dumps(settings))
        config.chmod(0o600)
    save()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner = options(key, url, url)
    process, active, complete, evidence = None, [], False, []
    request = {"kind": "dockerfile", "disk_mib": 1024, "min_free_mib": 64,
               "max_input_bytes": 256 * MIB, "max_expanded_bytes": 512 * MIB, "max_entries": 10000,
               "timeout_seconds": 120, "max_log_bytes": 2 * MIB}
    try:
        process = start_server(binary, config, root / "server.log", None)
        wait_ready(opener, url, key, process)
        for kind in ("dockerfile", "oci", "failure", "timeout"):
            builder = Sandbox.create(timeout=600, allow_internet_access=False, **owner)
            active.append(builder.sandbox_id)
            checked(builder, "mkdir -p /input/tools /output")
            profile = {}
            for name, digest in BUILDKIT_HASHES.items():
                upload(builder, args.tools / name, "/input/tools/" + name)
                profile[name] = {"path": "/input/tools/" + name, "sha256": digest}
            for name, source in (("busybox", args.tools / "busybox"), ("envd", args.tools / "envd")):
                upload(builder, source, "/input/tools/" + name)
                profile[name] = {"path": "/input/tools/" + name, "sha256": sha256_file(source)}
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
            if kind in ("failure", "timeout"):
                assert result.exit_code != 0 and status["state"] == "failed" and not status["ready"]
                assert not builder.files.exists("/output/job/candidate.ext4")
                evidence.append({"kind": kind, "status": status})
            else:
                assert result.exit_code == 0, (result.stderr, result.stdout)
                assert status["state"] == "candidate" and not status["ready"] and not status["startup_applied"]
                assert status["startup"]["WorkingDir"] == "/workspace"
                assert status["startup"]["Env"] == ["FIXTURE_VALUE=preserved-not-yet-activated"]
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
                evidence.append({"kind": kind, "status": status})
            assert Sandbox.kill(builder.sandbox_id, **owner)
            active.remove(builder.sandbox_id)
            check_no_resources(root, namespace, firecracker, args.uid_base)
            if kind in ("dockerfile", "oci"):
                stop_server(process)
                settings["templates"]["templates"].append(descriptor("candidate-" + kind, args.kernel, disk, 1, 256))
                save()
                process = start_server(binary, config, root / "server.log", None)
                wait_ready(opener, url, key, process)
                guest = Sandbox.create(template="candidate-" + kind, timeout=120, allow_internet_access=False, **owner)
                active.append(guest.sandbox_id)
                assert checked(guest, "printf sdk-boot-ok") == "sdk-boot-ok"
                assert guest.files.read("/workspace/payload") == "copied-standard-context"
                assert guest.files.read("/workspace/built").strip() == "build-ok"
                assert not guest.files.exists("/workspace/removed")
                guest.files.write("/workspace/private", "not-in-template")
                assert Sandbox.kill(guest.sandbox_id, **owner)
                active.remove(guest.sandbox_id)
                check_no_resources(root, namespace, firecracker, args.uid_base)
                assert sha256_file(disk) == status["rootfs_sha256"]
        assert original == [sha256_file(path) for path in (args.kernel, args.rootfs)]
        complete = True
        print(json.dumps({"source_commit": commit, "server_sha256": sha256_file(binary), "namespace": namespace,
                          "probe_sha256": sha256_file(Path(__file__)), "vms_destroyed": 6, "evidence": evidence,
                          "scope": "offline synthetic fixture; no public OCI, user API, startup defaults or production Ready gate"}))
    finally:
        for identity in active:
            try:
                if not Sandbox.kill(identity, **owner):
                    raise RuntimeError("cleanup unconfirmed")
            except Exception:
                print("cleanup unknown for exact sandbox " + identity, file=sys.stderr)
        stop_server(process)
        if complete:
            shutil.rmtree(root)
        else:
            print("Exact failed-run artifacts retained: " + str(root), file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent", "cgroup-parent", "tools"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    exercise(parser.parse_args())
