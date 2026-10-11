"""Exit the real server at exact pre-host or post-host lifecycle windows.

Run only as root on an isolated Linux/KVM host with an approved debug build.
All test resources use a new namespace and private directory. A failed run
keeps its exact artifacts for manual inspection instead of guessing cleanup.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

_complete = _runpy.run_path(str(Path(__file__).with_name("_complete_runtime.py")))
complete_runtime = _complete["complete_runtime"]
provision_runtime = _complete["provision_runtime"]
terminal_cleanup_confirmed = _complete["terminal_cleanup_confirmed"]
idle_snapshot = _complete["idle_snapshot"]
assert_idle_reopen = _complete["assert_idle_reopen"]
retired_sandboxes = _complete["retired_sandboxes"]


EXIT_AFTER_HOST_ACTION = 86
EXIT_BEFORE_HOST_ACTION = 87


def free_loopback_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def process_start_ticks(pid: int) -> int | None:
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    except FileNotFoundError:
        return None
    if fields[0] == "Z":
        return None
    return int(fields[19])


def fetch(
    opener: urllib.request.OpenerDirector,
    base_url: str,
    method: str,
    path: str,
    key: str | None = None,
    body: dict | None = None,
) -> tuple[int, bytes]:
    headers = {"X-API-Key": key} if key is not None else {}
    if body is not None:
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(
        base_url + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers=headers,
        method=method,
    )
    try:
        with opener.open(request, timeout=40) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def interrupted_host_request(
    request: object, process: subprocess.Popen[bytes],
    expected_exit: int = EXIT_AFTER_HOST_ACTION,
) -> None:
    """A private host exit must not become a successful public completion.

    The center is an independent process, so its bounded unknown/error response
    is expected; a broken HTTP connection is no longer the only valid outcome.
    Recovery must use the original admitted identity, never another create.
    """
    try:
        response = request()
    except (urllib.error.URLError, http.client.HTTPException, OSError):
        pass
    else:
        assert response[0] >= 500, (
            "host action without a durable receipt was reported complete", response[0]
        )
    assert process.wait(timeout=30) == expected_exit, process.returncode


def prepare_fault_services(args, root, settings, *, slots, key, port, job_count=4,
                           extra_credentials=None, shared_template=False):
    """Construct fresh explicit native/catalog inputs for the one real fixture.

    No retired public-host field is translated or accepted. The production
    original descriptor/hash/cache and complete physical profile are validated
    independently by the center and host before the first sandbox request.
    """
    from _runtime_services import RuntimeServices

    retired = {"bind", "domain", "credentials", "request_admission",
               "template_registry", "internal_build_vms", "build_retry_windows"}
    if retired & settings.keys() or "templates" in settings or type(shared_template) is not bool:
        raise ValueError("fresh new native inputs required, not a legacy configuration")
    kernel, rootfs = Path(settings["kernel"]), Path(settings["rootfs"])
    memory = settings["memory_mib"]
    host_memory = settings.get("host_resources", {}).get(
        "memory_max_bytes", (memory + 256) * 1024**2
    )
    def artifact(path):
        with path.open("rb") as source:
            checksum = hashlib.file_digest(source, "sha256").hexdigest()
        return {"path": str(path), "sha256": checksum, "size_bytes": path.stat().st_size}
    template = {"template_id": "base", "revision": "r1", "owner": None if shared_template else 1,
        "kernel": artifact(kernel), "rootfs": artifact(rootfs),
        "resources": {"cpu_count": settings["vcpu_count"], "memory_mib": memory,
            "disk_bytes": rootfs.stat().st_size, "host_memory_bytes": host_memory},
        "envd_version": "0.9.0"}
    cpu = settings["vcpu_count"]
    global_slots, personal = settings["max_active_global"], settings["max_active_per_tenant"]
    catalog = {"templates": [template], "aliases": {"base": "base"}, "compute_limits": {
        "max_vcpus_global": cpu * global_slots, "max_vcpus_per_tenant": cpu * personal,
        "max_memory_bytes_global": memory * 1024**2 * global_slots,
        "max_memory_bytes_per_tenant": memory * 1024**2 * personal}}
    settings = {**settings, "templates": catalog,
        "max_lifetime_secs": settings.get("max_lifetime_secs", settings["max_timeout_secs"]),
        "startup_timeout_secs": settings.get("startup_timeout_secs", 300),
        "public_ipv4_enabled": settings.get("public_ipv4_enabled", False)}
    native = complete_runtime(settings, slots=slots)
    artifact_bytes = kernel.stat().st_size + rootfs.stat().st_size
    registry = {"root": str(root / "registry"), "max_templates_global": 1,
        "max_templates_per_tenant": 1, "max_artifact_bytes_global": artifact_bytes,
        "max_artifact_bytes_per_tenant": artifact_bytes, "validation_timeout_secs": 60}
    return RuntimeServices.prepare(args, root=root, native_pools=[native],
        registered_catalogs=[catalog], key=key, public_address=f"127.0.0.1:{port}",
        record=args.service_cgroup_record, job_count=job_count,
        private_builds=None, registry_config=registry, extra_credentials=extra_credentials)


def original_create_identity(services, metadata):
    """Read the original center intent through its offline checked authority."""
    from _center_host_services import find_runtime_create

    return find_runtime_create(original_center_audit(services), metadata,
                               deployment=services.center_config["deployment_id"])


def original_center_audit(services):
    """A planned original writer-lease audit; never implicitly stop a host."""

    processes = services.processes
    previous = processes.children.get("center")
    restart = previous is not None and previous.poll() is None
    processes.stop("center")
    try:
        return processes.audit("center")
    finally:
        if restart:
            processes.start("center")


def registered_cache_template(services, template_id, *, index=0):
    """Observe one controlled registered cache inode for a fault injection.

    This is test evidence only, not a cache validator or availability setter.
    The original Rust verifier is what subsequently accepts/refuses the file.
    """
    from tools.system_acceptance import digest, json_file, private_path

    config = services.host_configs[index]
    cache = private_path(config["template_cache"]["root"], directory=True)
    entries = list((cache / "artifacts").iterdir())
    assert 0 < len(entries) <= config["template_cache"]["max_entries"]
    selected = []
    for directory in entries:
        private_path(directory, directory=True)
        before = directory.stat()
        manifest = json_file(private_path(directory / "manifest.json"), limit=2*(65536+4096))
        request = manifest["request"]
        registered = [binding for binding in services.center_config["templates"]["templates"]
                      if binding == request["descriptor"]]
        assert manifest["version"] == 1 and request["node"] == config["inbox"]["node"]
        assert len(registered) == 1
        assert (manifest["device"], manifest["inode"]) == (before.st_dev, before.st_ino)
        if registered[0]["template_id"] != template_id:
            continue
        local = manifest["local"]
        projected = json.loads(json.dumps(registered[0]))
        for kind in ("kernel","rootfs"):
            path = private_path(local[kind]["path"])
            assert path == directory / ("rootfs.ext4" if kind=="rootfs" else "kernel")
            original = path.stat()
            assert original.st_size == registered[0][kind]["size_bytes"]
            assert digest(path) == registered[0][kind]["sha256"]
            projected[kind]["path"] = str(path)
        assert projected == local, "cache changed immutable registered descriptor"
        selected.append(local)
    assert len(selected) == 1, "one exact protected registered cache template required"
    return selected[0]


def registered_cache_kernel(services, *, index=0):
    local = registered_cache_template(services,"base",index=index)
    path = Path(local["kernel"]["path"])
    original = path.stat()
    return path,(original.st_dev,original.st_ino)


def exercise(args: argparse.Namespace) -> None:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    parent = Path(args.scratch_parent).resolve(strict=True)
    disk_bytes = Path(args.rootfs).stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError("test host lacks space for trusted template and private VM disk")
    root = Path(tempfile.mkdtemp(prefix="sp-crash-", dir=parent))
    namespace = secrets.token_hex(8)
    vm_name = f"vm-{namespace}-0"
    api_key = secrets.token_hex(32)
    port = free_loopback_port()
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    kernel = root / "vmlinux"
    rootfs = root / "rootfs.ext4"
    shutil.copyfile(args.kernel, kernel)
    shutil.copyfile(args.rootfs, rootfs)
    # Jailer drops to uid_base before Firecracker opens these chroot files.
    # The private parent remains root-only; the copies need guest-process read access.
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    settings = {
        "namespace": namespace,
        "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"),
        "jailer": str(Path(args.jailer).resolve(strict=True)),
        "firecracker": str(Path(args.firecracker).resolve(strict=True)),
        "chroot_base": str(root / "jails"),
        "kernel": str(kernel),
        "rootfs": str(rootfs),
        "uid_base": args.uid_base,
        "vcpu_count": 1,
        "memory_mib": 256,
        "file_policy_digest": 1,
        "max_active_global": 2,
        "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 4,
        "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes,
        "max_timeout_secs": 600,
        "default_timeout_secs": 300,
        "expiry_scan_seconds": 1,
    }
    services = prepare_fault_services(args, root, settings, slots=2, key=api_key, port=port)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{port}"
    process = None
    metadata = {"crash_intent": secrets.token_hex(16)}
    completed = False
    try:
        before_host = args.window == "before-host"
        activation = args.window in ("before-activation", "after-activation", "after-guest-init")
        create_after = (
            "activate_vm" if args.window == "after-activation"
            else "prepare_vm" if args.window == "after-host"
            else None
        )
        create_before = (
            "open_fence" if before_host
            else "activate_vm" if args.window == "before-activation"
            else None
        )
        faults = {}
        if create_after:
            faults["SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION"] = create_after
        if create_before:
            faults["SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION"] = create_before
        if args.window == "after-guest-init":
            faults["SANDPROOF_TEST_EXIT_AFTER_GUEST_INIT"] = "activate_vm"
        services.start(host_envs={"host-0": faults})
        services.wait_ready()
        process = services.processes.children["host-0"]
        interrupted_host_request(
            lambda: fetch(
                opener, base_url, "POST", "/v2/sandboxes", api_key,
                {"templateID": "base", "timeout": 180, "allow_internet_access": False,
                 "envVars": {"SP_RECOVERY": "original"}, "metadata": metadata},
            ),
            process,
            (EXIT_BEFORE_HOST_ACTION if create_before else
             88 if args.window == "after-guest-init" else EXIT_AFTER_HOST_ACTION),
        )
        if before_host:
            assert not list((root / "state/fences").glob("*.json")), (
                "opening fence was written before injected exit"
            )
            assert not (root / "jails/.claims" / vm_name).exists(), (
                "VM was launched before opening fence"
            )
            assert not Path(f"/run/netns/{vm_name}").exists(), (
                "network was created before opening fence"
            )
        else:
            record = json.loads((root / "jails/.claims" / vm_name / "record.json").read_text())
            assert record["resource_id"] == vm_name
            pid, ticks = record["pid"], record["start_time_ticks"]
            assert process_start_ticks(pid) == ticks, "first VM is not running after server exit"

        identity = original_create_identity(services, metadata)
        native_id = services.native_id_for(identity["sandbox_id"])
        process = services.processes.start("host-0")
        services.wait_ready()
        status, body = fetch(opener, base_url, "GET", "/v2/sandboxes", api_key)
        assert status == 200, status
        items = json.loads(body)
        assert len(items) == 1 and items[0]["state"] == "running", items
        sandbox_id = items[0]["sandboxID"]
        assert sandbox_id == identity["sandbox_id"], "recovery published a different admitted create"
        if before_host:
            record = json.loads((root / "jails/.claims" / vm_name / "record.json").read_text())
            assert record["resource_id"] == vm_name
            pid, ticks = record["pid"], record["start_time_ticks"]
        assert process_start_ticks(pid) == ticks, "server restart replaced the VM"
        if activation:
            import importlib.metadata
            from e2b import Sandbox
            from sdk_server_smoke import options

            assert importlib.metadata.version("e2b") == "2.51.0"
            sandbox = Sandbox.connect(sandbox_id, **options(api_key, base_url, base_url))
            assert sandbox.commands.run("printf %s \"$SP_RECOVERY\"").stdout == "original"
        services.processes.stop("host-0")
        stop_fault = "SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION" if before_host else "SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION"
        process = services.processes.start("host-0", env={stop_fault: "stop_vm"})
        services.wait_ready()
        interrupted_host_request(
            lambda: fetch(opener, base_url, "DELETE", f"/sandboxes/{sandbox_id}", api_key),
            process,
            EXIT_BEFORE_HOST_ACTION if before_host else EXIT_AFTER_HOST_ACTION,
        )
        if before_host:
            assert process_start_ticks(pid) == ticks, "VM stopped before injected exit"
            fence = root / "state/fences" / f"{native_id}.json"
            assert json.loads(fence.read_text())["terminal"] is True, "terminal fence not durable"
        else:
            assert process_start_ticks(pid) != ticks, "recorded VM survived StopVm"

        process = services.processes.start("host-0")
        services.wait_ready()
        status, body = fetch(opener, base_url, "GET", "/v2/sandboxes", api_key)
        assert status == 200 and json.loads(body) == [], status
        assert fetch(opener, base_url, "GET", f"/sandboxes/{sandbox_id}", api_key)[0] == 404
        assert not Path(f"/run/netns/{vm_name}").exists(), "network namespace survived cleanup"
        jail = root / "jails" / Path(args.firecracker).name / vm_name
        assert not jail.exists(), "private VM jail survived cleanup"
        assert process_start_ticks(pid) != ticks, "VM was relaunched during recovery"
        completed = True
        print(json.dumps({
            "namespace": namespace,
            "host_sha256": hashlib.sha256(Path(args.host).read_bytes()).hexdigest(),
            "center_sha256": hashlib.sha256(Path(args.center).read_bytes()).hexdigest(),
            "window": args.window,
            "create": "recovered the original VM after the selected crash",
            "stop": "published destruction only after exact stop and cleanup",
            "scope": "real server process exit; one VM; no public egress or arbitrary-position SIGKILL",
        }))
    finally:
        services.stop_all()
        if completed:
            services.cleanup_staging()
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    from _runtime_services import add_arguments
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    parser.add_argument("--jailer", required=True, type=Path)
    parser.add_argument("--firecracker", required=True, type=Path)
    parser.add_argument("--kernel", required=True, type=Path)
    parser.add_argument("--rootfs", required=True, type=Path)
    parser.add_argument("--uid-base", required=True, type=int)
    parser.add_argument("--scratch-parent", required=True, type=Path)
    parser.add_argument(
        "--window",
        choices=("before-host", "after-host", "before-activation", "after-activation", "after-guest-init"),
        default="after-host",
    )
    exercise(parser.parse_args())
