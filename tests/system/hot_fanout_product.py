#!/usr/bin/env python3
"""Exercise one protected hot parent through the production center, host, and stock SDK."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import time

from _complete_runtime import complete_runtime, terminal_cleanup_confirmed
from _hot_fanout_config import descriptor_digest
from _runtime_services import RuntimeServices
from hot_fanout_guest import make_package
from hot_fanout_host import SDK_COMMIT
from lifecycle_unload_runtime import original_json
from lifecycle_unload_scenario import PublicUnloadScenario, failure_details
from sdk_c02_acceptance import processes_in_uid_range
from server_process_crash import free_loopback_port, process_start_ticks
from server_vm_id_capacity import check_no_resources
from tools import service_cgroup
from tools.capture_acceptance import bind_settings
from tools.ci_e2e_artifacts import publish
from tools.ci_e2e_prepare import contract_versions
from tools.ci_redis import check_material
from tools.runtime_acceptance_budget import profile_declaration, service_declarations
from tools.single_host_storage import original_project, plan, provision, reopen
from tools.system_acceptance import digest, json_file, private_path, write_once
from tools.template_physical_storage import activation_budget

ROOT = Path(__file__).resolve().parents[2]
MIB = 1024**2
GIB = 1024**3
ENV = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
MARKER = "/etc/fanout-sdk-marker"
PACKAGE_MARKER = "/usr/share/sandproof-hot-fanout-check/marker"


def command(*args):
    return subprocess.check_output(args, text=True, env=ENV, timeout=90).strip()


@contextmanager
def phase(report, name, client=None):
    start = time.monotonic()
    try:
        yield
    except Exception as failure:
        report.setdefault("failure", dict(error=type(failure).__name__, phase=name,
                                          **failure_details(failure, client, name)))
        raise
    finally:
        report["phase_seconds"][name] = time.monotonic() - start


def make_cgroup(path):
    path.mkdir(mode=0o700)
    (path / "cgroup.subtree_control").write_text("+cpu +memory +pids")


def build_deployment(args, receipt, storage, services_paths, report, *, slots=1, hot_fanout=True):
    data = storage / "data"
    root, staging = data / "r" / "a", data / "s"
    artifact = receipt["binding"]
    shape = artifact["resources"]
    disk = shape["disk_bytes"]
    kernel, rootfs = (Path(artifact[key]["path"]) for key in ("kernel", "rootfs"))
    for path, metadata in ((kernel, artifact["kernel"]), (rootfs, artifact["rootfs"])):
        assert path.resolve(strict=True) == path and path.parent == args.receipt.parent
        assert path.stat().st_size == metadata["size_bytes"]
    catalog = dict(templates=[artifact], aliases={"base": artifact["template_id"]}, compute_limits={
        f"max_{key}_{level}": slots * value
        for key, value in (("vcpus", shape["cpu_count"]), ("memory_bytes", shape["host_memory_bytes"]))
        for level in ("global", "per_tenant")})
    namespace = secrets.token_hex(8)
    vm_parent = Path("/sys/fs/cgroup") / ("sp1026-" + namespace + "-v")
    service_parent = Path("/sys/fs/cgroup") / ("sp1026-" + namespace + "-s")
    report["namespace"] = namespace
    report["cgroup_parents"] = list(map(str, (vm_parent, service_parent)))
    native = complete_runtime(dict(
        namespace=namespace, database=str(root/"0/db/controller.db"),
        node_state_dir=str(root/"0/state"), chroot_base=str(root/"0/p/v"), uid_base=args.vm_uid,
        max_active_global=slots, max_active_per_tenant=slots,
        max_disk_bytes_global=slots * disk + GIB,
        max_disk_bytes_per_tenant=slots * disk, cleanup_reserve_bytes=GIB, kernel=str(kernel),
        rootfs=str(rootfs), firecracker=str(args.firecracker), jailer=str(args.jailer),
        file_policy_digest=1, vcpu_count=shape["cpu_count"], memory_mib=shape["memory_mib"],
        host_resources=dict(cgroup_parent=str(vm_parent), memory_max_bytes=shape["host_memory_bytes"]),
        max_timeout_secs=600, max_lifetime_secs=600, startup_timeout_secs=90,
        expiry_scan_seconds=1, public_ipv4_enabled=False, templates=catalog), slots=slots, inbox_sessions=slots)
    native["bounded_host"]["physical_storage"]["filesystem_overhead_bytes"] = 64*MIB
    transfers = native["data_transfer"]
    transfers["max_transfers"] = max(transfers["max_transfers"], args.n + 6)
    transfers["max_transfers_per_vm"] = max(transfers["max_transfers_per_vm"], args.n + 2)
    capture_state_bytes = 10_000_000
    capture_memory_bytes = shape["memory_mib"] * MIB
    capture_overlap = slots * (4 * (capture_state_bytes + capture_memory_bytes) + 2 * disk)
    project_bytes = max(56 * GIB, ((capture_overlap // GIB) + 2) * GIB)
    project = dict(project_id=args.project_id, hard_bytes=project_bytes,
                   hard_inodes=max(4096, 512 * slots),
                   **{tool.name: dict(path=str(tool), sha256=list(bytes.fromhex(digest(tool))))
                      for tool in map(Path, ("/usr/bin/chattr", "/usr/bin/lsattr"))})
    captures = bind_settings(dict(schema_version=1, hosts=[dict(state_copy_bytes=capture_state_bytes,
        memory_copy_bytes=capture_memory_bytes, disk_copy_bytes=disk, api_timeout_ms=30000,
        worker_timeout_ms=120000, material_project=project)]), [root/"0"])
    capture_file = args.output / "capture.json"
    write_once(capture_file, captures)
    registry = dict(max_artifact_bytes_global=1, max_templates_global=1,
                    max_artifact_bytes_per_tenant=1, max_templates_per_tenant=1,
                    validation_timeout_secs=30)
    layout = dict(hosts=[root/"0"], center_identity=(args.center_uid, args.center_uid),
        host_controls=[root/"0/control"], registry=staging/"planned/center/registry",
        center_control=staging/"planned/center/control", assets=staging/"planned/center/assets",
        writer=staging/"planned/center/writer", redis=staging/"planned/redis", logs=root/"control-logs")
    ids = dict(center=layout["center_identity"], redis=(args.redis_uid, args.redis_uid), host=(0, 0))
    service_peaks = service_declarations(layout, ids, services_paths, kernel.stat().st_size + disk,
                                        check_material(args.redis_material, protected=True), None, captures)
    physical_inodes = max(8192, native["bounded_host"]["physical_storage"]["filesystem_overhead_inodes"])
    profile = profile_declaration([native], [catalog], layout, registry=registry,
                                  service_peaks=service_peaks, physical_inodes=physical_inodes)
    required = activation_budget(profile)["required_bytes"]
    image_bytes = max(74 * GIB, ((required // GIB) + 2) * GIB)
    storage_plan = plan(profile, storage, image_bytes=image_bytes, host_reserve_bytes=GIB)
    assert storage_plan["fits"]
    write_once(args.output/"storage-plan.json", storage_plan)
    installation = provision(storage, profile, original_project(profile, captures, data),
                             image_bytes=image_bytes, host_reserve_bytes=GIB)
    write_once(args.output/"installation.json", installation)
    staging.mkdir(mode=0o711)
    staging.chmod(0o711)
    root.mkdir(mode=0o700, parents=True)
    (root/"0").mkdir(mode=0o700)
    for name in ("db", "state", "p"):
        (root/"0"/name).mkdir(mode=0o700)
    for group in (vm_parent, service_parent):
        make_cgroup(group)
    record = root/"service-cgroup.json"
    service_cgroup.prepare(service_parent, "svc-" + namespace, record,
                           cpu_quota_us=400000, memory_bytes=8*GIB)
    params = argparse.Namespace(center=services_paths["center"], host=services_paths["host"],
        candidate_manifest=args.output/"candidate.json", redis_material=args.redis_material,
        tls_material=args.tls_material, service_staging_parent=staging, service_cgroup_record=record,
        capture_settings=capture_file, center_uid=args.center_uid, center_gid=args.center_uid,
        redis_uid=args.redis_uid, redis_gid=args.redis_uid, physical_inodes=physical_inodes,
        redis_operation_timeout_ms=20000)
    key = secrets.token_hex(32)
    services = RuntimeServices.prepare(params, root, [native], [catalog], key,
        "127.0.0.1:" + str(free_loopback_port()), record, slots, registry_config=registry,
        hot_fanout=receipt if hot_fanout else None)
    return services, root, record, (vm_parent, service_parent), installation


def create_parameters(services, ready, artifact):
    node = services.host_configs[0]["inbox"]["node"]["node_id"]
    revision = artifact["template_id"] + "-" + artifact["revision"]
    [registered] = ready["catalogs"][node]
    assert registered["revision"] == revision
    [staged] = services.center_config["templates"]["templates"]
    assert registered["digest"] == list(descriptor_digest(staged))
    shape = artifact["resources"]
    options = dict(env_vars={}, metadata={"scenario": "1026-hot-fanout"},
                   networking=dict(allow_internet_access=False, network=None))
    return dict(template_revision=revision, template_digest=registered["digest"],
                resources=dict(instances=1, cpu=shape["cpu_count"],
                               memory_bytes=shape["host_memory_bytes"], disk_bytes=shape["disk_bytes"]),
                options=list(json.dumps(options, separators=(",", ":")).encode()),
                startup_timeout_ms=90000, timeout_ms=540000, max_lifetime_ms=600000)


def get_batch(client, path, key):
    result = client.read_json("GET", path, key=client.owner_key)
    assert set(result) == {"operation", "parent", "result"}
    assert result["operation"] == key and result["parent"] == client.session
    return result["result"]


def start_batch(client, running, checkpoint, count, report):
    sequence = client.sequence + 1
    path = client.path() + "/branch-batches"
    request = dict(window=client.window, sequence=sequence, expected_version=running["current_version"],
                   checkpoint=checkpoint, count=count)
    start = time.monotonic()
    # This key remains consumed even when acceptance could not be read.
    client.sequence = sequence
    accepted = client.read_json("POST", path, key=client.owner_key, body=request, status=202)
    report["batch_post_seconds"] = time.monotonic() - start
    key = client.key(sequence)
    assert accepted["operation"] == key and accepted["parent"] == client.session
    assert accepted["result"] in ({"state": "pending"}, {"state": "cleaning_up"}) or (
        accepted["result"]["state"] == "ready" and len(accepted["result"]["branches"]) == count)
    path += f"/{client.window}/{sequence}"
    while True:
        result = get_batch(client, path, key)
        if result["state"] == "ready":
            branches = result["branches"]
            assert set(result) == {"state", "branches"}
            assert len(branches) == len(set(branches)) == count and client.session not in branches
            assert all(isinstance(branch, str) and re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", branch)
                       for branch in branches)
            report["batch_public_ready_seconds"] = time.monotonic() - start
            return path, key, branches
        assert result in ({"state": "pending"}, {"state": "cleaning_up"}), "batch did not become ready"
        time.sleep(min(.1, client.remaining()))


def concurrent_sdk(branch_ids, options, inherited, report):
    from e2b import Sandbox

    count = len(branch_ids)
    order = threading.Barrier(count, timeout=45)
    start = time.monotonic()

    def exercise(index, branch_id):
        marks = {}
        order.wait()
        before = time.monotonic()
        sandbox = Sandbox.connect(branch_id, **options)
        assert sandbox.sandbox_id == branch_id
        marks["connect"] = time.monotonic() - before
        order.wait()
        before = time.monotonic()
        assert sandbox.files.read("/run/inherited", user="root") == inherited
        marks["first_file"] = time.monotonic() - before
        content = "branch-" + str(index)
        sandbox.files.write(MARKER, content, user="root", request_timeout=30)
        order.wait()
        before = time.monotonic()
        result = sandbox.commands.run("cat " + MARKER, user="root", timeout=15, request_timeout=30)
        marks["first_command"] = time.monotonic() - before
        assert result.exit_code == 0 and result.stdout == content
        assert sandbox.files.read(MARKER, user="root") == content
        return sandbox, marks

    with ThreadPoolExecutor(max_workers=count, thread_name_prefix="hot-fanout-sdk") as pool:
        tasks = [pool.submit(exercise, index, branch) for index, branch in enumerate(branch_ids)]
        connected = [task.result(timeout=120) for task in tasks]
    report["all_sdk_ready_seconds"] = time.monotonic() - start
    report["branches"] = [dict(branch=branch, seconds=marks) for branch, (_, marks) in
                          zip(branch_ids, connected)]
    return [sandbox for sandbox, _ in connected]


def check_sdk_not_found(branch_id, options):
    from e2b import Sandbox
    from e2b.exceptions import SandboxNotFoundException

    try:
        Sandbox.connect(branch_id, **options)
    except SandboxNotFoundException:
        return
    raise AssertionError("stock SDK still connects to a released branch")


def slot_and_claim(native):
    path = Path(native["node_state_dir"])/"host-slots/0.json"
    slot = original_json(path, 65536)
    assert slot["position"] == 0 and slot["value"]["phase"] == "Occupied"
    [record] = (Path(native["chroot_base"])/".claims").glob("*/record.json")
    claim = original_json(record, 65536)
    assert claim["resource_id"].startswith("vm-" + native["namespace"] + "-")
    assert process_start_ticks(claim["pid"]) == claim["start_time_ticks"]
    return path, slot["value"]["context"], claim


def run(args, receipt, report):
    paths = dict(center=args.center, host=args.host)
    versions = {role: contract_versions(path, ENV) for role, path in paths.items()}
    assert versions["center"] == versions["host"]
    publish(args.output/"candidate.json", candidate=args.candidate,
            paths={role: str(path) for role, path in paths.items()}, versions=versions["center"])
    storage = Path(tempfile.mkdtemp(prefix="h.", dir=args.storage_parent))
    storage.chmod(0o711)
    report["storage"] = str(storage)
    write_once(args.output/"location.json", {"storage": str(storage)})
    with phase(report, "provision"):
        services, root, record, groups, installation = build_deployment(args, receipt, storage, paths, report)
    client = None
    cleaned = False
    native = services.native_pools[0]
    claim = None
    try:
        with phase(report, "service_readiness"):
            services.start()
            ready = services.wait_ready(timeout=180)
            original_pids = {name: child.pid for name, child in services.processes.children.items()}
        client = PublicUnloadScenario(services.sdk_url, services.key, secrets.token_hex(32),
                                      services.center_config["deployment_id"], time.monotonic() + 480)
        with phase(report, "parent_create", client):
            from e2b import Sandbox
            from sdk_server_smoke import options as sdk_options

            client.create(create_parameters(services, ready, receipt["binding"]))
            report["parent"] = client.session
            running = client.wait_running()
            owner = sdk_options(services.key, services.sdk_url, services.sdk_url)
            parent = Sandbox.connect(client.session, **owner)
            slot_path, context, claim = slot_and_claim(native)
        with phase(report, "branch_batch", client):
            checkpoint = receipt["native_hot_fanout"]["template"]["checkpoint_id"]
            path, batch_key, branches = start_batch(client, running, checkpoint, args.n, report)
        with phase(report, "stock_sdk_concurrent", client):
            sandboxes = concurrent_sdk(branches, owner, receipt["expected_inherited_mark"], report)
        with phase(report, "package_isolation", client):
            with tempfile.TemporaryDirectory(dir=args.output, prefix="package-") as work:
                package = make_package(Path(work))
                sandboxes[0].files.write("/tmp/fanout-check.deb", package, user="root")
            sandboxes[0].commands.run("dpkg -i /tmp/fanout-check.deb", user="root", timeout=20)
            assert sandboxes[0].files.read(PACKAGE_MARKER, user="root") == "package-from-one-branch"
            for sibling in [*sandboxes[1:], parent]:
                sibling.commands.run("test ! -e " + shlex.quote(PACKAGE_MARKER), user="root", timeout=10)
            parent.commands.run("test ! -e " + shlex.quote(MARKER), user="root", timeout=10)
            assert slot_and_claim(native)[1:] == (context, claim)
            report["single_original_vm"] = True
        with phase(report, "individual_sdk_kill", client):
            killed = sandboxes[0].kill()
            assert killed is True
            check_sdk_not_found(branches[0], owner)
            for index, sibling in enumerate(sandboxes[1:], start=1):
                assert sibling.files.read(MARKER, user="root") == "branch-" + str(index)
            assert parent.commands.run("printf parent-alive", timeout=10).stdout == "parent-alive"
            report["remaining_siblings_after_kill"] = len(sandboxes) - 1
        with phase(report, "parent_destroy", client):
            client.destroy(running)
            client.wait_cleaned()
            cleaned = True
            while get_batch(client, path, batch_key) != {"state": "failed", "reason": "parent_ended"}:
                time.sleep(min(.1, client.remaining()))
            for branch in branches[1:]:
                check_sdk_not_found(branch, owner)
            report["parent_ended_batch"] = True
        with phase(report, "native_release", client):
            slot = original_json(slot_path, 65536)
            assert slot["position"] == 0 and slot["value"]["context"] == context
            assert slot["value"]["phase"] == "Free"
            check_no_resources(root/"0", native["namespace"], args.firecracker, args.vm_uid,
                               [claim], native=native)
            assert {name: child.pid for name, child in services.processes.children.items()} == original_pids
            report["exact_parent_slot_free"] = True
    finally:
        if not cleaned and client is not None and client.session is not None:
            try:
                client.deadline = time.monotonic() + 120
                current = client.query()
                if current["phase"] not in ("terminating", "cleaned"):
                    client.destroy(current)
                if current["phase"] != "cleaned":
                    client.wait_cleaned()
                cleaned = True
            except Exception as failure:
                report["cleanup_request_error"] = type(failure).__name__
        with phase(report, "reclaim_test_resources", client):
            services.stop_all()
            for log in (root/"initialize.log", *(root/"control-logs").glob("*.log")):
                if log.is_file():
                    shutil.copyfile(log, args.output/log.name)
            if cleaned:
                check_no_resources(root/"0", native["namespace"], args.firecracker, args.vm_uid,
                                   [] if claim is None else [claim], native=native)
                native_id = services.native_id_for(client.session)
                assert terminal_cleanup_confirmed(Path(native["database"]), native_id)
                report["native_parent_terminal"] = True
            report["service_cgroup"] = service_cgroup.remove(record)
            if cleaned:
                services.cleanup_staging()
                for group in groups:
                    group.rmdir()
                assert reopen(storage, installation["receipt"]["project"]) == installation["receipt"]
                command("/usr/bin/umount", storage/"data")
                assert not os.path.ismount(storage/"data")
                (storage/"control/volume.xfs").unlink()
                report["physical_test_volume_removed"] = True


def checked_args(parser=None):
    if parser is None:
        parser = argparse.ArgumentParser(description=__doc__)
    for name in ("receipt", "center", "host", "firecracker", "jailer", "redis-material", "tls-material",
                 "output", "data-mount", "storage-parent"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("center-uid", "redis-uid", "vm-uid", "project-id", "n"):
        parser.add_argument("--" + name, type=int, required=True)
    parser.add_argument("--candidate", required=True)
    args = parser.parse_args()
    if not sys.platform.startswith("linux") or os.geteuid() != 0:
        parser.error("requires root on the isolated Linux/KVM host")
    if not os.access("/dev/kvm", os.R_OK | os.W_OK):
        parser.error("requires usable /dev/kvm")
    if re.fullmatch(r"[a-f0-9]{40}", args.candidate) is None:
        parser.error("exact candidate commit required")
    git = ("git", "-c", f"safe.directory={ROOT}", "-C", ROOT)
    if command(*git, "rev-parse", "HEAD") != args.candidate or command(
            *git, "status", "--porcelain", "--untracked-files=no"):
        parser.error("exact clean build source required")
    mount = args.data_mount.resolve(strict=True)
    args.storage_parent = args.storage_parent.resolve(strict=True)
    args.output = args.output.parent.resolve(strict=True) / args.output.name
    if (not os.path.ismount(mount) or mount.stat().st_dev == Path("/").stat().st_dev
            or not args.output.parent.is_relative_to(mount)
            or args.output.parent.stat().st_dev != mount.stat().st_dev
            or args.storage_parent.stat().st_dev != mount.stat().st_dev):
        parser.error("evidence and runtime volume must stay on the designated separate data mount")
    for name in ("receipt", "center", "host", "firecracker", "jailer", "redis_material", "tls_material"):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    uids = (args.center_uid, args.redis_uid, args.vm_uid)
    if not (1000 <= min(uids) and max(uids) < 2**32 - 1
            and args.center_uid < args.vm_uid and args.redis_uid < args.vm_uid
            and len(set(uids)) == 3 and 0 < args.project_id < 2**32
            and 2 <= args.n <= 64):
        parser.error("explicit distinct test identities and 2..64 branches required")
    if any(processes_in_uid_range(uid, uid) for uid in uids):
        parser.error("reserved test UID is already in use")
    return args


def main():
    args = checked_args()
    package = importlib.metadata.distribution("e2b")
    provenance = json.loads(package.read_text("direct_url.json"))
    assert package.version == "2.51.0" and provenance["vcs_info"]["commit_id"] == SDK_COMMIT
    os.umask(0o077)
    receipt = json_file(private_path(args.receipt), limit=65536)
    assert args.n <= receipt["native_hot_fanout"]["template"]["max_branches"]
    args.output.mkdir(mode=0o700)
    report = dict(candidate=args.candidate, fixture_sha256=digest(args.receipt),
                  binaries={role: digest(getattr(args, role)) for role in ("center", "host")},
                  sdk_commit=SDK_COMMIT, requested_branches=args.n, phase_seconds={}, passed=False)
    try:
        run(args, receipt, report)
        report["passed"] = True
    except Exception as failure:
        report.setdefault("failure", dict(error=type(failure).__name__,
                                          **failure_details(failure, None, "setup")))
    finally:
        write_once(args.output/"result.json", report)
        print(json.dumps(report, sort_keys=True))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
