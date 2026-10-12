"""Exact-commit self-service HTTP -> isolated KVM -> registered template -> SDK.

Root-only disposable deployment, two disjoint pools. No E2B cloud/paid model.
--temporarily-forward enables IPv4 forwarding for the approved public probe
and restores its original value on exit. Failure retains exact evidence.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import sys
sys.path.insert(0, str(_GuardPath(__file__).resolve().parents[2]))

from tools.template_build_config import acceptance_build_policy, apply_acceptance_build_policy
from tools.bounded_acceptance import (
    bounded_acceptance_layout, acceptance_pool_disk_budget, provision_acceptance_coordination,
    require_bounded_host_tools, validate_acceptance_socket_paths, wait_private_build_window,
)

import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import time
import urllib.parse
import urllib.request

from e2b import Sandbox
from sdk_server_smoke import options
from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from server_process_crash import fetch, free_loopback_port
from _process_utils import stop_child
from server_templates import descriptor
from server_vm_id_capacity import check_no_resources
from template_build import BUILDKIT_HASHES, FIXTURE, IGZIP_HASHES
from template_fetch import REGCTL_SHA256

MIB = 1024 * 1024
BUILD_TOOLS = dict(BUILDKIT_HASHES, regctl=REGCTL_SHA256)


def exercise(args):
    bounded_policy = acceptance_build_policy(getattr(args, "bounded_policy", None))
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("isolated root Linux/KVM required")
    require_bounded_host_tools()
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
    build_tools = BUILD_TOOLS | (IGZIP_HASHES if args.igzip else {})
    for name, digest in build_tools.items():
        if sha256_file(args.tools / name) != digest:
            raise RuntimeError("fixed tool fingerprint mismatch: " + name)
    from _complete_registry_runtime import prepare_registry_runtime
    from tools.template_build_service import Backend, configuration
    deployment = prepare_registry_runtime(args, tool_hashes=build_tools,
        registry_limits={"max_templates_global": 8, "max_templates_per_tenant": 8,
            "max_artifact_bytes_global": 12 * 1024 * MIB,
            "max_artifact_bytes_per_tenant": 12 * 1024 * MIB, "validation_timeout_secs": 30},
        build_limits={"max_cpu": 2, "max_memory_mib": 1024, "max_disk_mib": 2048, "timeout_seconds": 600,
            "max_jobs": 20, "max_jobs_per_tenant": 10, "max_pending": 3, "max_pending_per_tenant": 2,
            "workspace_bytes": 80 * 1024 * MIB, "workspace_bytes_per_tenant": 60 * 1024 * MIB,
            "max_input_bytes": 64 * MIB, "max_expanded_bytes": 512 * MIB,
            "max_entries": 10000, "max_log_bytes": 2 * MIB},
        ports=[free_loopback_port() for _ in range(3)])
    root, namespaces, services = deployment.root, deployment.namespaces, deployment.services
    key, foreign_key, url = deployment.key, deployment.foreign_key, services.sdk_url
    build_config = deployment.build_config
    build = json.loads(build_config.read_text())
    build_mode = "windowed"
    backend = Backend(configuration(build_config, require_physical_lease=False))
    print(f"root={root} commit={commit}", flush=True)
    firecracker = args.firecracker.resolve(strict=True)
    original_paths = (args.kernel, args.rootfs, args.builder_rootfs or args.rootfs)
    original = [sha256_file(path) for path in original_paths]
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
    process, log = None, None
    complete, evidence = False, []
    def start_builds():
        nonlocal log
        log = (root / "build-service.log").open("ab")
        return subprocess.Popen([sys.executable, str(checkout / "tools/template_build_service.py"), "--config", str(build_config)], stdout=log, stderr=subprocess.STDOUT)
    def request(method, path, body=None, key=key):
        code, data = fetch(opener, url, method, path, key, body)
        return code, json.loads(data)
    def wait_builds():
        # A child PID is not a listening endpoint. Observe readiness with GET;
        # never repair a startup race by retrying a mutating build submission.
        for _ in range(60):
            try:
                if request("GET", "/sandproof/template-builds/" + "0" * 31 + "1")[0] == 404:
                    return
            except OSError:
                assert process.poll() is None, "build service exited before readiness"
            time.sleep(0.5)
        raise AssertionError("build endpoint did not become ready")
    def submit(body, source=None):
        from tools.template_build_client import BuildClient, saved_intent
        client = BuildClient(url, key)
        intent_path = root / ("build-intent-" + secrets.token_hex(16) + ".json")
        intent = saved_intent(intent_path, client, body | {"timeoutSeconds": body.get("timeoutSeconds", build["limits"]["timeout_seconds"])},
                              context=source)
        result = client.request("POST", "/sandproof/template-builds", body | {"requestWindow": intent["requestWindow"]}, token=intent["idempotency_key"])
        job = result["buildID"]
        if source:
            with source.open("rb") as incoming:
                upload = urllib.request.Request(url + "/sandproof/template-builds/" + job + "/context",
                    data=incoming, method="PUT", headers={"X-API-Key": key, "Content-Length": str(source.stat().st_size)})
                with opener.open(upload, timeout=60) as response:
                    assert response.status == 200
        return job
    def wait_job(job, terminal=True):
        deadline = time.monotonic() + 660
        while time.monotonic() < deadline:
            assert process.poll() is None, "build service exited"
            try:
                code, result = request("GET", "/sandproof/template-builds/" + job)
            except OSError:
                # Read-only query during explicit restart. Never repeat a
                # lost create/submission; the original job ID stays fixed.
                time.sleep(0.5)
                continue
            assert code == 200
            if result["state"] in (("Ready", "Failed") if terminal else ("Building",)):
                return result
            time.sleep(0.5)
        raise AssertionError("build deadline exceeded: " + job)
    try:
        services.start()
        services.wait_ready()
        # Only Rust activates the unchanged physical profile. Joining before
        # either server exists would turn a valid fresh deployment into a
        # missing-file error; skipping this check at service start is unsafe.
        configuration(build_config)
        process = start_builds()
        wait_builds()
        basic = {"kind": "dockerfile", "allowInternetAccess": False, "timeoutSeconds": 600,
                 "resources": {"cpuCount": 1, "memoryMiB": 256, "diskMiB": 1024, "minFreeMiB": 64}}
        job = submit(basic, context)
        result = wait_job(job)
        assert result["state"] == "Ready", result
        assert request("GET", "/sandproof/template-builds/" + job, key=foreign_key)[0] == 404
        template = result["template"]["templateId"]
        sandbox = Sandbox.create(template, timeout=120, allow_internet_access=False, **options(key, url, url))
        try:
            assert sandbox.commands.run("/bin/busybox cat /workspace/payload", user="app").stdout == "copied-standard-context"
            assert "literal space" in sandbox.commands.run("/bin/busybox cat /workspace/startup-result", user="app").stdout
            sandbox.files.write("/workspace/user-file", "self-service", user="app")
            assert sandbox.files.read("/workspace/user-file", user="app") == "self-service"
        finally:
            sandbox.kill()
        evidence.append({"scenario": "multistage_context_publish_sdk", "build": job, "template": template, "result": "passed"})
        print(json.dumps(evidence[-1]), flush=True)
        if getattr(args, "publication_unknown", False):
            # One exclusive store lease: stop the background supervisor before
            # driving the actual production functions through the HTTP barrier.
            stop_child(process)
            log.close()
            from template_publication_unknown import exercise as publication_unknown
            evidence.append(publication_unknown(build_config, context, basic))
            print(json.dumps(evidence[-1]), flush=True)
            process = start_builds()
            wait_builds()
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
        # Building is published before the backend persists its private
        # window. Observe the original row only, then query its real VM.
        private_window = wait_private_build_window(root / "builds/builds.db", job)
        query = "?" + urllib.parse.urlencode({"requestWindow": private_window})
        for _ in range(120):
            observed = backend.request('builder', 'GET', '/sandproof/internal-build-vms/' + job + query, owner=1)
            if observed.get("session"):
                vm = observed
                break
            time.sleep(0.5)
        else:
            raise AssertionError("original build VM did not launch")
        process.kill()
        process.wait(timeout=10)
        log.close()
        process = start_builds()
        wait_builds()
        result = wait_job(job)
        assert result["state"] == "Failed" and result["failure"] == "InterruptedBuild", result
        retry = {"timeout": 660, "allowInternetAccess": False, "requestWindow": private_window}
        from tools.template_build_store import BuildError
        try:
            backend.request('builder', 'POST', '/sandproof/internal-build-vms/' + job, retry, owner=1)
        except BuildError as error:
            assert error.code == 409, error.code
        else:
            raise AssertionError('retired private tuple admitted a new builder')
        evidence.append({"scenario": "service_sigkill_after_vm_started", "build": job, "sandbox": vm["sandboxID"], "result": "passed"})
        print(json.dumps(evidence[-1]), flush=True)
        if args.public:
            job = submit({"kind": "oci", "image": args.public_image, "timeoutSeconds": 600})
            result = wait_job(job)
            assert result["state"] == "Ready", result
            import sqlite3
            with closing(sqlite3.connect((root / "builds/builds.db").as_uri() + "?mode=ro", uri=True)) as database:
                candidate = json.loads(database.execute("SELECT candidate FROM jobs WHERE id=?", (job,)).fetchone()[0])
            provenance = candidate["public_source"]
            assert provenance["selected_digest"] == result["template"]["imageDigest"]
            assert provenance["regctl_sha256"] == REGCTL_SHA256
            assert provenance["download_bytes"] <= build["limits"]["max_input_bytes"]
            sandbox = Sandbox.create(result["template"]["templateId"], timeout=120, allow_internet_access=False, **options(key, url, url))
            try:
                assert "ubuntu" in sandbox.commands.run("cat /etc/os-release", user="root").stdout.lower()
                sandbox.files.write("/tmp/public-image", "public-oci", user="root")
                assert sandbox.files.read("/tmp/public-image", user="root") == "public-oci"
            finally:
                sandbox.kill()
            evidence.append({"scenario": "public_oci_publish_sdk", "build": job, "image": args.public_image,
                             "digest": result["template"]["imageDigest"], "public_source": provenance, "result": "passed"})
            print(json.dumps(evidence[-1]), flush=True)
        for i in range(2):
            check_no_resources(root / str(i), namespaces[i], firecracker, args.uid_base + i * 100)
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 120)
        assert original == [sha256_file(path) for path in original_paths]
        import sqlite3
        with closing(sqlite3.connect((root / "builds/builds.db").as_uri() + "?mode=ro", uri=True)) as database:
            assert database.execute("SELECT COALESCE(SUM(reserved), 0) FROM jobs").fetchone()[0] == 0
        complete = True
        (root / "report.json").write_text(json.dumps({"commit": commit, "scenarios": evidence,
            "service_artifacts": services.manifest,
            "kernel_sha256": original[0], "base_sha256": original[1], "builder_base_sha256": original[2],
            "build_mode": build_mode, "physical_profile": True, "tools": build_tools,
            "no_process_network_cgroup_jail_residue": True, "public": args.public}, indent=2))
        print("self-service acceptance passed; report=" + str(root / "report.json"), flush=True)
    finally:
        if process:
            stop_child(process)
        if log:
            log.close()
        services.stop_all()
        if not complete:
            print("failure evidence retained at " + str(root), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    from _runtime_services import add_arguments
    add_arguments(parser)
    for name in ("firecracker", "jailer", "kernel", "rootfs", "tools", "scratch-parent", "cgroup-parent", "build-cgroup-parent"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--builder-rootfs", type=Path, help="fixed builder base; defaults to --rootfs")
    parser.add_argument("--run-root", type=Path, help="fresh exact root named by the immutable bounded profile")
    parser.add_argument("--namespace", help="explicit runtime namespace required for bounded acceptance")
    parser.add_argument("--build-namespace", help="explicit builder namespace required for bounded acceptance")
    parser.add_argument("--bounded-policy", type=Path, required=True,
                        help="explicit complete policies for both pools and public build storage")
    parser.add_argument("--public", action="store_true")
    parser.add_argument("--publication-unknown", action="store_true", help="delay the original publication POST across expiry and SQLite restart (#189)")
    parser.add_argument("--igzip", action="store_true", help="check and use the fixed optional ISA-L decoder pair")
    parser.add_argument("--temporarily-forward", action="store_true", help="temporarily enable IPv4 forwarding for approved public test; restore on exit")
    parser.add_argument("--public-image", default="public.ecr.aws/docker/library/ubuntu:24.04")
    args = parser.parse_args()
    original_forward = Path("/proc/sys/net/ipv4/ip_forward").read_text().strip()
    if args.temporarily_forward and not args.public:
        raise RuntimeError("temporary forwarding only applies to the public test")
    changed_forward = args.public and args.temporarily_forward and original_forward == "0"
    try:
        if changed_forward:
            subprocess.run(["/usr/sbin/sysctl", "-w", "net.ipv4.ip_forward=1"], check=True)
        exercise(args)
    finally:
        if changed_forward:
            subprocess.run(["/usr/sbin/sysctl", "-w", "net.ipv4.ip_forward=0"], check=True)
