"""Exact-commit self-service HTTP -> isolated KVM -> registered template -> SDK.

Root-only disposable deployment, two disjoint pools. No E2B cloud/paid model.
--public requires an explicitly approved temporary IPv4 forwarding setup;
this script never changes global forwarding. Failure retains exact evidence.
"""
import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

from e2b import Sandbox
from sdk_server_smoke import options
from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from server_process_crash import fetch, free_loopback_port, start_server, stop_server, wait_ready
from server_templates import descriptor
from server_vm_id_capacity import check_no_resources
from template_build import BUILDKIT_HASHES, FIXTURE

MIB = 1024 * 1024


def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("isolated root Linux/KVM required")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("clean exact source commit required")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    if processes_in_uid_range(args.uid_base, args.uid_base + 120):
        raise RuntimeError("exclusive UID pools occupied")
    if args.public and Path("/proc/sys/net/ipv4/ip_forward").read_text().strip() != "1":
        raise RuntimeError("public test requires explicitly approved forwarding")
    for parent in (args.cgroup_parent, args.build_cgroup_parent):
        if (parent / "cgroup.procs").read_text().strip() or any(p.is_dir() for p in parent.iterdir()):
            raise RuntimeError("empty delegated exclusive cgroup required")
    if args.scratch_parent.stat().st_uid != 0 or args.scratch_parent.stat().st_mode & 0o077:
        raise RuntimeError("private root scratch required")
    if shutil.disk_usage(args.scratch_parent).free < 50 * 1024 * MIB:
        raise RuntimeError("50 GiB including cleanup reserve required")
    for name, digest in BUILDKIT_HASHES.items():
        if sha256_file(args.tools / name) != digest:
            raise RuntimeError("fixed tool fingerprint mismatch: " + name)
    root = Path(tempfile.mkdtemp(prefix="ts-", dir=args.scratch_parent))
    binary = root / "server"
    shutil.copyfile(args.server, binary)
    binary.chmod(0o700)
    print(f"root={root} commit={commit}", flush=True)
    namespaces = [secrets.token_hex(8), secrets.token_hex(8)]
    firecracker = args.firecracker.resolve(strict=True)
    settings = []
    ports = [free_loopback_port() for _ in range(3)]
    urls = [f"http://127.0.0.1:{port}" for port in ports]
    keys = [secrets.token_hex(32) for _ in range(3)]
    original = [sha256_file(path) for path in (args.kernel, args.rootfs)]
    for i in range(2):
        pool = root / str(i)
        pool.mkdir(mode=0o700)
        for name in ("db", "state", "jails"):
            (pool / name).mkdir(mode=0o700)
        cpu, memory = (2, 2048) if i else (1, 256)
        base = descriptor("builder-base" if i else "runtime-base", args.kernel, args.rootfs, cpu, memory)
        config = {"namespace": namespaces[i], "database": str(pool / "db/controller.db"),
            "node_state_dir": str(pool / "state"), "jailer": str(args.jailer), "firecracker": str(firecracker),
            "chroot_base": str(pool / "jails"), "kernel": str(args.kernel), "rootfs": str(args.rootfs),
            "uid_base": args.uid_base + i * 100, "max_vm_id": 20, "vcpu_count": cpu, "memory_mib": memory,
            "host_resources": {"cgroup_parent": str(args.build_cgroup_parent if i else args.cgroup_parent), "memory_max_bytes": (memory + 256) * MIB},
            "file_policy_digest": 1, "bind": f"127.0.0.1:{ports[i]}", "domain": "example.invalid",
            "credentials": [{"tenant": 1, "secret": keys[i]}], "max_active_global": 2,
            "max_active_per_tenant": 2, "max_disk_bytes_global": 24 * 1024 * MIB, "max_disk_bytes_per_tenant": 24 * 1024 * MIB,
            "cleanup_reserve_bytes": 256 * MIB, "max_timeout_secs": 1300,
            "startup_timeout_secs": 40, "expiry_scan_seconds": 2, "public_ipv4_enabled": bool(i and args.public),
            "templates": {"templates": [base], "aliases": {"base": base["template_id"]},
                "compute_limits": {"max_vcpus_global": 4, "max_vcpus_per_tenant": 4,
                    "max_memory_bytes_global": 4608 * MIB, "max_memory_bytes_per_tenant": 4608 * MIB}},
            "data_transfer": {"file_upload_bytes": 256 * MIB, "file_download_bytes": 2048 * MIB,
                "command_input_bytes": MIB, "command_output_bytes": 4 * MIB, "rpc_request_bytes": MIB,
                "rpc_response_bytes": MIB, "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
                "request_idle_seconds": 60, "file_idle_seconds": 60, "write_idle_seconds": 60}}
        if i:
            config["internal_build_vms"] = True
        else:
            for path in (root / "registry", root / "registry/inbox", root / "registry/artifacts"):
                path.mkdir(mode=0o700)
            config["credentials"].append({"tenant": 2, "secret": keys[2]})
            config["template_registry"] = {"root": str(root / "registry"), "max_templates_global": 8,
                "max_templates_per_tenant": 8, "max_artifact_bytes_global": 12 * 1024 * MIB,
                "max_artifact_bytes_per_tenant": 12 * 1024 * MIB, "validation_timeout_secs": 30}
        settings.append(config)
        (pool / "config.json").write_text(json.dumps(config))
        (pool / "config.json").chmod(0o600)
    (root / "builds").mkdir(mode=0o700)
    build = {"root": str(root / "builds"), "bind": f"127.0.0.1:{ports[2]}",
        "runtime_config": str(root / "0/config.json"), "builder_config": str(root / "1/config.json"),
        "tools": {name: {"path": str(args.tools / name), "sha256": digest} for name, digest in BUILDKIT_HASHES.items()},
        "dns_servers": ["1.1.1.1", "8.8.8.8"],
        "limits": {"max_cpu": 2, "max_memory_mib": 1024, "max_disk_mib": 2048, "timeout_seconds": 600,
            "max_jobs": 20, "max_jobs_per_tenant": 10, "max_pending": 3, "max_pending_per_tenant": 2,
            "workspace_bytes": 80 * 1024 * MIB, "workspace_bytes_per_tenant": 60 * 1024 * MIB,
            "max_input_bytes": 64 * MIB, "max_expanded_bytes": 512 * MIB, "max_entries": 10000, "max_log_bytes": 2 * MIB}}
    # Offline mode still has an explicit configured IPv4 build policy, but all
    # submitted Dockerfile jobs select deny-all; no public packets are sent.
    if not args.public:
        settings[1]["public_ipv4_enabled"] = True
        (root / "1/config.json").write_text(json.dumps(settings[1]))
    build_config = root / "build.json"
    build_config.write_text(json.dumps(build))
    build_config.chmod(0o600)
    fixture_root = root / "fixture"
    fixture_root.mkdir(mode=0o700)
    (fixture_root / "tools").mkdir()
    shutil.copyfile(args.tools / "busybox", fixture_root / "tools/busybox")
    # Trusted synthetic fixture generation only, never a submitted Dockerfile
    # or archive. User archives are only opened inside the build VM.
    fixture = FIXTURE.replace("/input", str(fixture_root))
    subprocess.run([sys.executable, "-c", fixture], check=True)
    context = fixture_root / "context.tar"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    controllers, process, log = [], None, None
    complete, evidence = False, []
    def start_builds():
        nonlocal log
        log = (root / "build-service.log").open("ab")
        return subprocess.Popen([sys.executable, str(checkout / "tools/template_build_service.py"), "--config", str(build_config)], stdout=log, stderr=subprocess.STDOUT)
    def request(method, path, body=None, key=keys[0]):
        code, data = fetch(opener, urls[2], method, path, key, body)
        return code, json.loads(data)
    def submit(body, source=None):
        code, result = request("POST", "/sandproof/template-builds", body)
        assert code == 202, (code, result)
        job = result["buildID"]
        if source:
            with source.open("rb") as incoming:
                upload = urllib.request.Request(urls[2] + "/sandproof/template-builds/" + job + "/context",
                    data=incoming, method="PUT", headers={"X-API-Key": keys[0], "Content-Length": str(source.stat().st_size)})
                with opener.open(upload, timeout=60) as response:
                    assert response.status == 200
        return job
    def wait_job(job, terminal=True):
        deadline = time.monotonic() + 660
        while time.monotonic() < deadline:
            assert process.poll() is None, "build service exited"
            code, result = request("GET", "/sandproof/template-builds/" + job)
            assert code == 200
            if result["state"] in (("Ready", "Failed") if terminal else ("Building",)):
                return result
            time.sleep(0.5)
        raise AssertionError("build deadline exceeded: " + job)
    try:
        for i in range(2):
            server = start_server(binary, root / str(i) / "config.json", root / str(i) / "server.log", None)
            controllers.append(server)
            wait_ready(opener, urls[i], keys[i], server)
        process = start_builds()
        for _ in range(60):
            try:
                if request("GET", "/missing")[0] == 404:
                    break
            except OSError:
                assert process.poll() is None
                time.sleep(0.5)
        basic = {"kind": "dockerfile", "allowInternetAccess": False, "timeoutSeconds": 600,
                 "resources": {"cpuCount": 1, "memoryMiB": 256, "diskMiB": 1024, "minFreeMiB": 64}}
        job = submit(basic, context)
        result = wait_job(job)
        assert result["state"] == "Ready", result
        assert request("GET", "/sandproof/template-builds/" + job, key=keys[2])[0] == 404
        template = result["template"]["templateId"]
        sandbox = Sandbox.create(template, timeout=120, allow_internet_access=False, **options(keys[0], urls[0], urls[0]))
        try:
            assert sandbox.commands.run("cat /workspace/payload", user="app").stdout == "copied-standard-context"
            assert "literal space" in sandbox.commands.run("cat /workspace/startup-result", user="app").stdout
            sandbox.files.write("/workspace/user-file", "self-service", user="app")
            assert sandbox.files.read("/workspace/user-file", user="app") == "self-service"
        finally:
            sandbox.kill()
        evidence.append({"scenario": "multistage_context_publish_sdk", "build": job, "template": template, "result": "passed"})
        print(json.dumps(evidence[-1]), flush=True)
        # Explicit worker failure produces bounded logs and exact VM reclaim.
        dockerfile = fixture_root / "context/Dockerfile"
        dockerfile.write_text("FROM scratch\nCOPY rootfs/ /\nRUN exit 17\n")
        failed_context = fixture_root / "failure.tar"
        subprocess.run(["tar", "-C", str(fixture_root / "context"), "-cf", str(failed_context), "."], check=True)
        job = submit(basic, failed_context)
        result = wait_job(job)
        assert result["state"] == "Failed", result
        code, logs = request("GET", "/sandproof/template-builds/" + job + "/logs?offset=0&limit=65536")
        assert code == 200 and len(logs["text"].encode()) <= 65536 * 3
        evidence.append({"scenario": "worker_failure_reclaimed", "build": job, "failure": result["failure"], "result": "passed"})
        print(json.dumps(evidence[-1]), flush=True)
        dockerfile.write_text("FROM scratch\nCOPY rootfs/ /\nRUN /bin/busybox sleep 120\n")
        interrupted = fixture_root / "interrupted.tar"
        subprocess.run(["tar", "-C", str(fixture_root / "context"), "-cf", str(interrupted), "."], check=True)
        job = submit(basic, interrupted)
        wait_job(job, terminal=False)
        for _ in range(120):
            code, data = fetch(opener, urls[1], "GET", "/sandproof/internal-build-vms/" + job, keys[1])
            if code == 200 and json.loads(data).get("session"):
                vm = json.loads(data)
                break
            time.sleep(0.5)
        else:
            raise AssertionError("original build VM did not launch")
        process.kill()
        process.wait(timeout=10)
        log.close()
        process = start_builds()
        result = wait_job(job)
        assert result["state"] == "Failed" and result["failure"] == "InterruptedBuild", result
        code, body = fetch(opener, urls[1], "POST", "/sandproof/internal-build-vms/" + job, keys[1], {"timeout": 660, "allowInternetAccess": False})
        after = json.loads(body)
        assert code == 200 and after["sandboxID"] == vm["sandboxID"] and after["cleanupComplete"]
        evidence.append({"scenario": "service_sigkill_after_vm_started", "build": job, "sandbox": vm["sandboxID"], "result": "passed"})
        print(json.dumps(evidence[-1]), flush=True)
        if args.public:
            job = submit({"kind": "oci", "image": args.public_image, "timeoutSeconds": 600})
            result = wait_job(job)
            assert result["state"] == "Ready", result
            sandbox = Sandbox.create(result["template"]["templateId"], timeout=120, allow_internet_access=False, **options(keys[0], urls[0], urls[0]))
            try:
                assert "ubuntu" in sandbox.commands.run("cat /etc/os-release", user="root").stdout.lower()
                sandbox.files.write("/tmp/public-image", "public-oci", user="root")
                assert sandbox.files.read("/tmp/public-image", user="root") == "public-oci"
            finally:
                sandbox.kill()
            evidence.append({"scenario": "public_oci_publish_sdk", "build": job, "image": args.public_image,
                             "digest": result["template"]["imageDigest"], "result": "passed"})
            print(json.dumps(evidence[-1]), flush=True)
        for i in range(2):
            check_no_resources(root / str(i), namespaces[i], firecracker, args.uid_base + i * 100)
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 120)
        assert original == [sha256_file(path) for path in (args.kernel, args.rootfs)]
        import sqlite3
        with sqlite3.connect(root / "builds/builds.db") as database:
            assert database.execute("SELECT SUM(reserved) FROM jobs").fetchone()[0] == 0
        complete = True
        (root / "report.json").write_text(json.dumps({"commit": commit, "scenarios": evidence,
            "kernel_sha256": original[0], "base_sha256": original[1], "tools": BUILDKIT_HASHES,
            "no_process_network_cgroup_jail_residue": True, "public": args.public}, indent=2))
        print("self-service acceptance passed; report=" + str(root / "report.json"), flush=True)
    finally:
        if process:
            stop_server(process)
        if log:
            log.close()
        for server in reversed(controllers):
            stop_server(server)
        if not complete:
            print("failure evidence retained at " + str(root), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "firecracker", "jailer", "kernel", "rootfs", "tools", "scratch-parent", "cgroup-parent", "build-cgroup-parent"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--public", action="store_true")
    parser.add_argument("--public-image", default="public.ecr.aws/docker/library/ubuntu:24.04")
    exercise(parser.parse_args())
