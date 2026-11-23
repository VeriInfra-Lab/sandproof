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
from contextlib import ExitStack, contextmanager
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import signal
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

from _complete_runtime import complete_runtime, terminal_cleanup_confirmed
from _hot_fanout_config import descriptor_digest
from _hot_fanout_evidence import automatic_expiry_acceptance, capture_live_native_ids
from _runtime_services import RuntimeServices
from hot_fanout_guest import make_package
from hot_fanout_host import SDK_COMMIT
from lifecycle_unload_runtime import original_json
from lifecycle_unload_scenario import PREFIX, PublicStatusMismatch, PublicUnloadScenario, failure_details
from sdk_c02_acceptance import processes_in_uid_range
from server_process_crash import fetch, free_loopback_port, process_start_ticks
from server_vm_id_capacity import check_no_resources
from tools import service_cgroup
from tools.build_hot_fanout_guest import PARTIAL_START, PARTIAL_START_EVIDENCE
from tools.capture_acceptance import bind_settings
from tools.ci_e2e_artifacts import publish
from tools.ci_e2e_prepare import contract_versions
from tools.ci_redis import check_material
from tools.runtime_acceptance_budget import profile_declaration, service_declarations
from tools.single_host_storage import original_project, plan, provision, reopen
from tools.system_acceptance import digest, json_file, private_path, unique_keys, write_once
from tools.template_physical_storage import activation_budget

ROOT = Path(__file__).resolve().parents[2]
MIB = 1024**2
GIB = 1024**3
ENV = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
MARKER = "/etc/fanout-sdk-marker"
COMMAND_MARKER = "/etc/fanout-command-marker"
PARENT_MARKER = "/tmp/sandproof-hot-parent-restart"
PACKAGE_MARKER = "/usr/share/sandproof-hot-fanout-check/marker"
EXPIRY_TTL_MS = 45_000


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


def build_deployment(args, receipt, storage, services_paths, report, *, slots=1, hot_fanout=True,
                     extra_credentials=None):
    data = storage / "data"
    root, staging = data / "r" / "a", data / "s"
    artifact = receipt["binding"]
    shape = artifact["resources"]
    disk = shape["disk_bytes"]
    retained_sessions = max(slots, 1 + len(extra_credentials or []))
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
        expiry_scan_seconds=1, public_ipv4_enabled=False, templates=catalog),
        slots=slots, inbox_sessions=retained_sessions)
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
    base_inodes = native["bounded_host"]["physical_storage"]["filesystem_overhead_inodes"]
    # Reserve room for the two declarations added by profile_declaration
    # (the static template cache and registry) as well as the service roots.
    physical_inodes = max(8192, base_inodes + 32 + sum(service["inode_budget"] for service in service_peaks))
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
        "127.0.0.1:" + str(free_loopback_port()), record, retained_sessions, registry_config=registry,
        extra_credentials=extra_credentials, hot_fanout=receipt if hot_fanout else None)
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


def batch_request(client, running, checkpoint, count):
    return dict(window=client.window, sequence=client.sequence + 1,
                expected_version=running["current_version"], checkpoint=checkpoint, count=count)


def check_batch_rejection(client, running, checkpoint, count, expected):
    path = client.path() + "/branch-batches"
    request = batch_request(client, running, checkpoint, count)
    status, _ = client.call("POST", path, key=client.owner_key, body=request)
    if status != expected:
        raise PublicStatusMismatch(status, expected)
    query = f'{path}/{request["window"]}/{request["sequence"]}'
    status, _ = client.call("GET", query, key=client.owner_key)
    if status != 404:
        raise PublicStatusMismatch(status, 404)
    return expected


def check_hot_unload_rejection(client, running):
    request = dict(window=client.window, sequence=client.sequence + 1,
                   expected_version=running["current_version"])
    status, _ = client.call("POST", client.path() + "/unload", key=client.owner_key, body=request)
    if status != 409:
        raise PublicStatusMismatch(status, 409)
    assert client.query() == running
    return status


def start_batch(client, running, checkpoint, count, report, *, await_reclaimed_port=False):
    path = client.path() + "/branch-batches"
    request = batch_request(client, running, checkpoint, count)
    sequence = request["sequence"]
    start = time.monotonic()
    reclaim_deadline = start + 30
    # This key remains reserved even when acceptance could not be read.
    client.sequence = sequence
    while True:
        try:
            accepted = client.read_json("POST", path, key=client.owner_key, body=request, status=202)
            break
        except PublicStatusMismatch as error:
            # Revocation is public before physical retirement. Retry only the
            # exact already-chosen request after an explicit capacity response.
            if not await_reclaimed_port or error.actual != 429 or time.monotonic() >= reclaim_deadline:
                raise
            time.sleep(min(.1, client.remaining()))
    report["batch_post_seconds"] = time.monotonic() - start
    return wait_batch_ready(client, request, accepted, start, report)


def wait_batch_ready(client, request, accepted, start, report):
    sequence, count = request["sequence"], request["count"]
    key = client.key(sequence)
    assert accepted["operation"] == key and accepted["parent"] == client.session
    assert accepted["result"] in ({"state": "pending"}, {"state": "cleaning_up"}) or (
        accepted["result"]["state"] == "ready" and len(accepted["result"]["branches"]) == count)
    path = f"{client.path()}/branch-batches/{client.window}/{sequence}"
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


def race_batch_and_parent_unload(client, running, request, report):
    unload = dict(window=request["window"], sequence=request["sequence"] + 1,
                  expected_version=request["expected_version"])
    client.sequence = unload["sequence"]
    began = time.monotonic()

    def send(path, body):
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        rendezvous.wait()
        sent = time.monotonic()
        status, encoded = fetch(opener, client.base_url, "POST", path, client.owner_key, body,
                                timeout=min(15, client.remaining()))
        return status, encoded, sent, time.monotonic()

    with limit_public_deadline(client, 30), ThreadPoolExecutor(max_workers=2) as executor:
        rendezvous = threading.Barrier(2, timeout=min(5, client.remaining()))
        batch = executor.submit(send, client.path() + "/branch-batches", request)
        pause = executor.submit(send, client.path() + "/unload", unload)
        started, paused = (future.result(timeout=client.remaining()) for future in (batch, pause))
        overlap = min(started[3], paused[3]) - max(started[2], paused[2])
        assert overlap > 0, "public StartBatch and Unload request intervals did not overlap"
        for actual, expected in ((started[0], 202), (paused[0], 409)):
            if actual != expected:
                raise PublicStatusMismatch(actual, expected)
        assert client.query() == running
    report["batch_post_seconds"] = started[3] - began
    report["unload_race"] = dict(batch_sequence=request["sequence"], unload_sequence=unload["sequence"],
                                 batch_status=202, unload_status=409, request_overlap_seconds=overlap)
    accepted = json.loads(started[1], object_pairs_hook=unique_keys)
    return wait_batch_ready(client, request, accepted, began, report)


def race_last_branch_slot(client, running, checkpoint, report):
    path = client.path() + "/branch-batches"
    first = batch_request(client, running, checkpoint, 1)
    second = dict(first, sequence=first["sequence"] + 1)
    client.sequence = second["sequence"]
    requests = (first, second)
    start = time.monotonic()

    def dispatch(request, barrier):
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        barrier.wait()
        return fetch(opener, client.base_url, "POST", path, client.owner_key, request,
                     timeout=min(15, client.remaining()))

    with limit_public_deadline(client, 30), ThreadPoolExecutor(max_workers=2) as executor:
        attempts = 0
        while True:
            attempts += 1
            barrier = threading.Barrier(2, timeout=min(5, client.remaining()))
            pending = [executor.submit(dispatch, request, barrier) for request in requests]
            replies = [call.result(timeout=client.remaining()) for call in pending]
            statuses = [status for status, _ in replies]
            if any(status not in (202, 429) for status in statuses):
                raise AssertionError(f"unexpected concurrent admission statuses: {statuses}")
            for request, status in zip(requests, statuses):
                if status == 429:
                    query = f'{path}/{request["window"]}/{request["sequence"]}'
                    actual, _ = client.call("GET", query, key=client.owner_key)
                    if actual != 404:
                        raise PublicStatusMismatch(actual, 404)
            if sorted(statuses) == [202, 429]:
                [winner] = [i for i, status in enumerate(statuses) if status == 202]
                accepted = json.loads(replies[winner][1], object_pairs_hook=unique_keys)
                break
            assert statuses == [429, 429], "two requests acquired the sole final branch slot"
            # The individual close may still be draining. Both requests had
            # definite refusals and are absent. Run the same two keys again.
            time.sleep(min(.1, client.remaining()))
    report["last_slot_race"] = dict(attempts=attempts, public_statuses=statuses,
                                    winner_sequence=requests[winner]["sequence"],
                                    rejected_sequence=requests[1 - winner]["sequence"])
    report["batch_post_seconds"] = time.monotonic() - start
    ready = wait_batch_ready(client, requests[winner], accepted, start, report)
    report["last_slot_race"]["full_after_winner_ready"] = check_batch_rejection(
        client, running, checkpoint, 1, 429)
    return ready


def concurrent_sdk(branch_ids, options, inherited, report, *, connect=None):
    from e2b import Sandbox

    count = len(branch_ids)
    order = threading.Barrier(count, timeout=45)
    start = time.monotonic()

    def exercise(index, branch_id):
        marks = {}
        order.wait()
        before = time.monotonic()
        sandbox = Sandbox.connect(branch_id, **options) if connect is None else connect(index)
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


def check_reverse_branch_writes(client, sandboxes):
    def exercise(index, sandbox):
        content = "command-" + str(index)
        command_line = "printf %s " + shlex.quote(content) + " > " + shlex.quote(COMMAND_MARKER)
        result = sandbox.commands.run(command_line, user="root", timeout=min(15, client.remaining()),
                                      request_timeout=min(30, client.remaining()))
        assert result.exit_code == 0
        assert sandbox.files.read(COMMAND_MARKER, user="root",
                                  request_timeout=min(30, client.remaining())) == content

    with ThreadPoolExecutor(max_workers=len(sandboxes)) as executor:
        checks = [executor.submit(exercise, index, sandbox) for index, sandbox in enumerate(sandboxes)]
        for check in checks:
            check.result(timeout=client.remaining())


def check_foreign_branch_access(client, running, checkpoint, batch_path, branch_id, options):
    # Obtain this registered tenant's own request window so the foreign POST
    # reaches the parent ownership check instead of failing a stale window.
    foreign = client.read_json("GET", PREFIX + "/request-window", key=client.foreign_key)
    assert set(foreign) == {"window"} and type(foreign["window"]) is int and foreign["window"] > 0
    request = dict(window=foreign["window"], sequence=1, expected_version=running["current_version"],
                   checkpoint=checkpoint, count=1)
    status_checks = (
        ("parent_query", "GET", client.path(), None),
        ("batch_query", "GET", batch_path, None),
        ("batch_start", "POST", client.path() + "/branch-batches", request),
    )
    result = {}
    for label, method, path, body in status_checks:
        status, _ = client.call(method, path, key=client.foreign_key, body=body)
        if status != 404:
            raise PublicStatusMismatch(status, 404)
        result[label] = status
    check_sdk_not_found(branch_id, options | {"api_key": client.foreign_key})
    result["sdk_connect"] = "not_found"
    return result


def check_batch_replay(client, path, key, request, branches):
    post = client.path() + "/branch-batches"
    expected = {"state": "ready", "branches": branches}
    replay = client.read_json("POST", post, key=client.owner_key, body=request, status=202)
    assert replay == {"operation": key, "parent": client.session, "result": expected}
    changed = dict(request, count=request["count"] - 1)
    status, _ = client.call("POST", post, key=client.owner_key, body=changed)
    if status != 409:
        raise PublicStatusMismatch(status, 409)
    assert get_batch(client, path, key) == expected
    return status


def check_stale_data_route(client, sandbox):
    import httpx

    # Use the token issued to the original stock SDK instance. Connect's 404
    # alone wouldn't detect a data endpoint accepting an old token.
    with httpx.Client(base_url=client.base_url, trust_env=False,
                      timeout=min(10, client.remaining())) as http:
        response = http.get("/files", params={"path": MARKER},
                            headers={"X-Access-Token": sandbox._envd_access_token})
    if response.status_code != 401:
        raise PublicStatusMismatch(response.status_code, 401)
    return response.status_code


def check_sdk_not_found(branch_id, options):
    from e2b import Sandbox
    from e2b.exceptions import SandboxNotFoundException

    try:
        Sandbox.connect(branch_id, **options)
    except SandboxNotFoundException:
        return
    raise AssertionError("stock SDK connected to an unavailable branch")


def slot_and_claim(native):
    path = Path(native["node_state_dir"])/"host-slots/0.json"
    slot = original_json(path, 65536)
    assert slot["position"] == 0 and slot["value"]["phase"] == "Occupied"
    [record] = (Path(native["chroot_base"])/".claims").glob("*/record.json")
    claim = original_json(record, 65536)
    assert claim["resource_id"].startswith("vm-" + native["namespace"] + "-")
    assert process_start_ticks(claim["pid"]) == claim["start_time_ticks"]
    return path, slot["value"]["context"], claim


@contextmanager
def limit_public_deadline(client, seconds):
    original = client.deadline
    client.deadline = min(original, time.monotonic() + seconds)
    try:
        yield
    finally:
        client.deadline = original


def restart_management_role(services, client, role, expected_processes, original_slot):
    from _center_host_services import process_facts, role_facts

    processes = services.processes
    native = services.native_pools[0]
    assert role in ("center", "host-0")
    assert processes.observe() == expected_processes
    assert slot_and_claim(native) == original_slot

    def staged_roles():
        return {name: role_facts(Path(binding["config"]).parent, binding["config"],
                                 uid=binding["uid"], gid=binding["gid"])
                for name, binding in processes.bindings.items()}

    approved = staged_roles()
    old = processes.children[role]
    try:
        processes.stop(role)
        assert old.poll() is not None
        assert slot_and_claim(native) == original_slot
        for name, binding in processes.bindings.items():
            if name == role:
                continue
            argv = [str(binding["binary"]), str(binding["config"])] if name == "redis" else None
            actual = process_facts(processes.children[name], **binding, record=processes.record, argv=argv)
            assert actual == expected_processes[name]
    finally:
        if old.poll() is not None:
            # Leave the existing cleanup path an opportunity to finish even
            # after failed evidence. Never initialize or substitute a role.
            assert staged_roles() == approved
            processes.start(role)
    services.wait_ready(timeout=client.remaining(), for_admission=False)
    observed = processes.observe()
    assert staged_roles() == approved
    assert slot_and_claim(native) == original_slot
    for name, previous in expected_processes.items():
        current = observed[name]
        if name == role:
            assert (current["pid"], current["start_time_ticks"]) != (previous["pid"], previous["start_time_ticks"])
            assert {key: current[key] for key in ("uid", "gid", "executable")} == {
                key: previous[key] for key in ("uid", "gid", "executable")}
        else:
            assert current == previous
    return observed


def check_live_routes_after_restart(client, running, options, batches, witnesses, released):
    from e2b import Sandbox

    assert client.query() == running
    for path, key, identities in batches:
        assert get_batch(client, path, key) == {"state": "ready", "branches": identities}

    def check_witness(witness):
        original, path, contents = witness
        chosen = options | {"request_timeout": min(15, client.remaining())}
        # Each command and file request runs once through the stock SDK. Any
        # unknown result fails the check under the original operation deadline.
        current = (client.sdk() if original.sandbox_id == client.session
                   else Sandbox.connect(original.sandbox_id, **chosen))
        assert current.sandbox_id == original.sandbox_id
        assert current._envd_access_token == original._envd_access_token
        response = current.commands.run("cat " + shlex.quote(path), user="root",
                                        timeout=min(10, client.remaining()),
                                        request_timeout=min(15, client.remaining()))
        assert response.exit_code == 0 and response.stdout == contents
        assert current.files.read(path, user="root", request_timeout=min(15, client.remaining())) == contents

    with ThreadPoolExecutor(max_workers=len(witnesses), thread_name_prefix="hot-fanout-reopen") as pool:
        observations = [pool.submit(check_witness, witness) for witness in witnesses]
        for observed in observations:
            observed.result(timeout=client.remaining())
    check_sdk_not_found(released.sandbox_id, options | {"request_timeout": min(15, client.remaining())})
    return check_stale_data_route(client, released)


def observe_automatic_destroy(client):
    assert client.destroy_request is None, "explicit Destroy cannot witness natural expiry"
    recovered = None
    client.step("automatic-expiry-destroy", 200)
    while True:
        status, raw = client.call("GET", client.path(), key=client.owner_key)
        if status == 200:
            view = json.loads(raw, object_pairs_hook=unique_keys)
            if view["phase"] in ("terminating", "cleaned"):
                accepted = automatic_expiry_acceptance(view, client.create_acceptance)
                if recovered is not None:
                    assert accepted == recovered["acceptance"]
                else:
                    recovered = dict(acceptance=accepted, first_phase=view["phase"],
                                     first_observed_at_ms=time.time_ns() // 1_000_000)
                client.check_view(view, accepted)
                if view["phase"] == "cleaned":
                    assert client.destroy_request is None
                    recovered["cleaned_observed_at_ms"] = time.time_ns() // 1_000_000
                    return recovered
            else:
                assert recovered is None and view["phase"] in ("creating", "running")
                client.check_view(view, client.create_acceptance)
        elif status != 503:
            raise PublicStatusMismatch(status, 200)
        time.sleep(min(.1, client.remaining()))


def verify_expiring_branches(client, running, parent, owner, checkpoint, count, inherited, report):
    initial = parent.get_info()
    assert initial.sandbox_id == client.session
    assert round((initial.end_at - initial.started_at).total_seconds() * 1000) == EXPIRY_TTL_MS
    path, key, branches = start_batch(client, running, checkpoint, count, report)
    sandboxes = concurrent_sdk(branches, owner, inherited, report)
    current = parent.get_info()
    assert (current.started_at, current.end_at) == (initial.started_at, initial.end_at)
    assert client.query() == running
    end_at_ms = round(initial.end_at.timestamp() * 1000)
    useful_at_ms = time.time_ns() // 1_000_000
    assert useful_at_ms < end_at_ms, "SDK work must be observed before the original parent expiry"
    report["natural_expiry"] = dict(requested_ttl_ms=EXPIRY_TTL_MS, end_at_ms=end_at_ms,
                                    sdk_all_usable_at_ms=useful_at_ms, usable_branches=len(sandboxes))
    return path, key, sandboxes


def verify_public_branch_rejection_after_parent_exit(
        client, running, parent, owner, checkpoint, count, inherited, native, original_slot, report):
    from lifecycle_unload_runtime import (
        exit_original_candidate, open_original_candidate_pidfd, pidfd_exited,
    )

    initial = parent.get_info()
    assert initial.sandbox_id == client.session
    assert initial.state == "running"
    end_at_ms = round(initial.end_at.timestamp() * 1000)

    def before_ttl():
        observed = time.time_ns() // 1_000_000
        assert observed < end_at_ms, "original parent lease expired before branch-loss evidence"
        assert client.destroy_request is None, "public Destroy preceded branch-loss evidence"
        return observed

    path, key, branches = start_batch(client, running, checkpoint, count, report)
    sandboxes = concurrent_sdk(branches, owner, inherited, report)
    assert client.query() == running
    assert parent.get_info().end_at == initial.end_at
    ready_at_ms = before_ttl()
    evidence = report["parent_vmm_loss"] = dict(
        end_at_ms=end_at_ms, sdk_usable_at_ms=ready_at_ms, usable_branches=len(sandboxes))
    minimum_remaining_ms = 60_000
    assert end_at_ms - ready_at_ms >= minimum_remaining_ms, "need an original live lease beyond loss observation"
    _, _, claim = original_slot
    with limit_public_deadline(client, 45):
        assert slot_and_claim(native) == original_slot
        descriptor = open_original_candidate_pidfd(claim)
        try:
            assert slot_and_claim(native) == original_slot
            before_ttl()
            exit_original_candidate(descriptor, claim, client.deadline)
            assert pidfd_exited(descriptor)
            evidence.update(pidfd_exit_observed_at_ms=before_ttl(), original_vm_pid=claim["pid"],
                            original_vm_start_time_ticks=claim["start_time_ticks"],
                            original_vm_resource_id=claim["resource_id"])

            expected_ready = {"state": "ready", "branches": branches}
            while True:
                before_ttl()
                result = get_batch(client, path, key)
                if result == {"state": "cleaning_up"}:
                    break
                assert result == expected_ready, "unexpected original branch disposition before confirmed loss"
                time.sleep(min(.1, client.remaining()))
            evidence["batch_cleaning_observed_at_ms"] = before_ttl()
            for sandbox in sandboxes:
                check_sdk_not_found(sandbox.sandbox_id,
                                    owner | {"request_timeout": min(10, client.remaining())})
                check_stale_data_route(client, sandbox)
            evidence.update(old_branch_connect_not_found=len(sandboxes), old_branch_token_status=401,
                            denied_before_destroy_at_ms=before_ttl(), explicit_destroy_sent_before_denial=False)
        finally:
            os.close(descriptor)
    return path, key


def check_batch_parent_ended(client, path, key):
    while get_batch(client, path, key) != {"state": "failed", "reason": "parent_ended"}:
        time.sleep(min(.1, client.remaining()))


def guest_branch_scopes(client, parent):
    scopes = []
    for directory in ("/run/sandproof-hot-fanout/branches", "/sys/fs/cgroup/sandproof-hot-fanout"):
        result = parent.commands.run(
            f"find {directory} -mindepth 1 -maxdepth 1 -type d -printf '%f\\n'",
            user="root", timeout=min(10, client.remaining()),
        )
        assert result.exit_code == 0
        scopes.append(sorted(result.stdout.splitlines()))
    assert scopes[0] == scopes[1]
    return scopes[0]


def verify_public_partial_start(client, running, parent, owner, checkpoint, inherited, report):
    steady_path, steady_key, steady_ids = start_batch(client, running, checkpoint, 1, report["steady"])
    [steady] = concurrent_sdk(steady_ids, owner, inherited, report["steady"])
    steady.files.write(MARKER, "partial-steady", user="root")
    original_scopes = guest_branch_scopes(client, parent)
    assert len(original_scopes) == 1

    post = client.path() + "/branch-batches"
    request = batch_request(client, running, checkpoint, 3)
    client.sequence = request["sequence"]
    key = client.key(client.sequence)
    path = f"{post}/{client.window}/{client.sequence}"
    pending = ({"state": "pending"}, {"state": "cleaning_up"})
    failure = {"state": "failed", "reason": "guest_setup_failed"}
    accepted = client.read_json("POST", post, key=client.owner_key, body=request, status=202)
    assert set(accepted) == {"operation", "parent", "result"}
    assert accepted["operation"] == key and accepted["parent"] == client.session
    assert accepted["result"] in (*pending, failure)
    observed_states = {accepted["result"]["state"]}
    while True:
        result = get_batch(client, path, key)
        assert result in (*pending, failure), "partially started batch exposed a public handle or wrong failure"
        observed_states.add(result["state"])
        if result == failure:
            break
        time.sleep(min(.1, client.remaining()))

    raw_evidence = parent.files.read(PARTIAL_START_EVIDENCE, user="root",
                                     request_timeout=min(10, client.remaining()))
    assert 0 < len(raw_evidence) <= 256
    evidence = json.loads(raw_evidence, object_pairs_hook=unique_keys)
    assert evidence == {"live_ordinals_before_failure": [0, 1], "failed_ordinal": 2}
    assert guest_branch_scopes(client, parent) == original_scopes
    replay = client.read_json("POST", post, key=client.owner_key, body=request, status=202)
    assert replay == {"operation": key, "parent": client.session, "result": failure}
    assert get_batch(client, path, key) == failure
    assert guest_branch_scopes(client, parent) == original_scopes
    assert steady.files.read(MARKER, user="root") == "partial-steady"

    refill_path, refill_key, refill_ids = start_batch(client, running, checkpoint, 3, report["refill"])
    assert not set(steady_ids).intersection(refill_ids)
    refilled = concurrent_sdk(refill_ids, owner, inherited, report["refill"])
    assert len(guest_branch_scopes(client, parent)) == 4
    assert steady.commands.run("cat " + shlex.quote(MARKER), user="root", timeout=10).stdout == "partial-steady"
    assert client.query() == running
    report.update(evidence, public_failure=failure, operation=key, exact_key_replayed=True,
                  observed_public_states=sorted(observed_states), public_handles_exposed=False,
                  fault_scopes_removed=True, steady_branch_preserved=True, refilled_branches=len(refilled))
    return (path, key, failure), ((steady_path, steady_key), (refill_path, refill_key)), [steady, *refilled]


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
    foreign_key = secrets.token_hex(32)
    with phase(report, "provision"):
        services, root, record, groups, installation = build_deployment(
            args, receipt, storage, paths, report,
            extra_credentials=[dict(tenant=2, secret=foreign_key)])
    client = None
    cleaned = False
    native = services.native_pools[0]
    claim = None
    native_id = None
    try:
        with phase(report, "service_readiness"):
            services.start()
            ready = services.wait_ready(timeout=180)
            expected_processes = services.processes.observe()
        client = PublicUnloadScenario(services.sdk_url, services.key, foreign_key,
                                      services.center_config["deployment_id"], time.monotonic() + 480)
        with phase(report, "parent_create", client):
            from e2b import Sandbox
            from sdk_server_smoke import options as sdk_options

            parameters = create_parameters(services, ready, receipt["binding"])
            if args.fault_case == "natural-expiry":
                parameters["timeout_ms"] = EXPIRY_TTL_MS
            client.create(parameters)
            report["parent"] = client.session
            running = client.wait_running()
            owner = sdk_options(services.key, services.sdk_url, services.sdk_url)
            slot_path, context, claim = slot_and_claim(native)
            [native_id] = capture_live_native_ids(native, [claim], original_json)
            # Preserve the original accepted parent version for the keyed
            # operations below. Branch connections use the stock Connect API.
            parent = client.sdk()

        def confirm_native_release():
            with phase(report, "native_release", client):
                slot = original_json(slot_path, 65536)
                assert slot["position"] == 0 and slot["value"]["context"] == context
                assert slot["value"]["phase"] == "Free"
                check_no_resources(root/"0", native["namespace"], args.firecracker, args.vm_uid,
                                   [claim], native=native)
                assert services.processes.observe() == expected_processes
                report["exact_parent_slot_free"] = True

        pin = receipt["native_hot_fanout"]["template"]
        checkpoint, branch_limit = pin["checkpoint_id"], pin["max_branches"]
        if args.fault_case == PARTIAL_START:
            with phase(report, "partial_start", client), limit_public_deadline(client, 90):
                partial = report["partial_start"] = {"steady": {}, "refill": {}}
                (path, key, failure), active, sandboxes = verify_public_partial_start(
                    client, running, parent, owner, checkpoint, receipt["expected_inherited_mark"], partial)
                assert slot_and_claim(native) == (slot_path, context, claim)
            with phase(report, "parent_destroy_after_partial_start", client):
                client.destroy(running)
                client.wait_cleaned()
                cleaned = True
                assert get_batch(client, path, key) == failure
                for active_path, active_key in active:
                    check_batch_parent_ended(client, active_path, active_key)
                for sandbox in sandboxes:
                    check_sdk_not_found(sandbox.sandbox_id, owner)
                    check_stale_data_route(client, sandbox)
                report["parent_ended_batch"] = True
            confirm_native_release()
            return

        if args.fault_case == "natural-expiry":
            with phase(report, "sdk_before_expiry", client):
                path, key, sandboxes = verify_expiring_branches(
                    client, running, parent, owner, checkpoint, args.n, receipt["expected_inherited_mark"], report)
                assert slot_and_claim(native) == (slot_path, context, claim)
            with phase(report, "natural_expiry", client), limit_public_deadline(client, 100):
                observed = observe_automatic_destroy(client)
                cleaned = True
                expiry = report["natural_expiry"]
                assert observed["first_observed_at_ms"] >= expiry["end_at_ms"]
                expiry.update(automatic_destroy=observed, explicit_destroy_sent=False)
                check_batch_parent_ended(client, path, key)
                for sandbox in sandboxes:
                    check_sdk_not_found(sandbox.sandbox_id, owner)
                    check_stale_data_route(client, sandbox)
                expiry["old_branch_tokens_refused"] = len(sandboxes)
                expiry["old_parent_token_status"] = check_stale_data_route(client, parent)
                report["parent_ended_batch"] = True
            confirm_native_release()
            return

        if args.fault_case == "parent-vmm-loss":
            with phase(report, "parent_vmm_loss", client):
                path, key = verify_public_branch_rejection_after_parent_exit(
                    client, running, parent, owner, checkpoint, args.n,
                    receipt["expected_inherited_mark"], native, (slot_path, context, claim), report)
            with phase(report, "parent_destroy_after_loss", client):
                # Use the original Create version and public lifecycle path.
                # Cleanup cannot satisfy the strictly earlier denial check.
                report["parent_vmm_loss"]["later_destroy_sent_at_ms"] = time.time_ns() // 1_000_000
                client.destroy(running)
                client.wait_cleaned()
                cleaned = True
                report["parent_vmm_loss"]["later_destroy_acceptance"] = client.current_acceptance
                check_batch_parent_ended(client, path, key)
                report["parent_ended_batch"] = True
            confirm_native_release()
            return

        with phase(report, "public_idle_boundaries", client):
            report["public_rejections"] = {
                "empty_batch": check_batch_rejection(client, running, checkpoint, 0, 400),
                "over_template_branch_limit": check_batch_rejection(
                    client, running, checkpoint, branch_limit + 1, 429),
                "hot_parent_idle_unload": check_hot_unload_rejection(client, running),
            }
        with phase(report, "branch_batch", client):
            request = batch_request(client, running, checkpoint, args.n)
            if args.fault_case == "unload-race":
                path, batch_key, branches = race_batch_and_parent_unload(client, running, request, report)
            else:
                path, batch_key, branches = start_batch(client, running, checkpoint, args.n, report)
        with phase(report, "public_active_boundaries", client):
            report["public_rejections"]["same_key_different_count"] = check_batch_replay(
                client, path, batch_key, request, branches)
            report["public_rejections"]["over_remaining_branch_limit"] = check_batch_rejection(
                client, running, checkpoint, branch_limit - args.n + 1, 429)
            report["public_rejections"]["hot_parent_active_unload"] = check_hot_unload_rejection(client, running)
        with phase(report, "authorized_tenant_isolation", client):
            report["foreign_authorized_tenant"] = check_foreign_branch_access(
                client, running, checkpoint, path, branches[0], owner)
        with phase(report, "stock_sdk_concurrent", client):
            sandboxes = concurrent_sdk(branches, owner, receipt["expected_inherited_mark"], report)
        with phase(report, "command_to_file", client):
            check_reverse_branch_writes(client, sandboxes)
            parent.commands.run("test ! -e " + shlex.quote(COMMAND_MARKER), user="root", timeout=10)
            report["command_to_file_branches"] = len(sandboxes)
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
        with ExitStack() as held:
            streams = None
            if args.fault_case == "inflight-close":
                from _hot_fanout_inflight import observe_inflight_branch

                with phase(report, "hold_original_sdk_streams", client):
                    held.enter_context(limit_public_deadline(client, 40))
                    first_port = services.center_config["branching"]["limits"]["first_port"]
                    report["inflight_close"] = {}
                    streams = held.enter_context(observe_inflight_branch(
                        client, sandboxes[0], native, context, first_port, report["inflight_close"]))
            with phase(report, "individual_sdk_kill", client):
                killed = sandboxes[0].kill()
                assert killed is True
                check_sdk_not_found(branches[0], owner)
                report["public_rejections"]["old_branch_data_token"] = check_stale_data_route(client, sandboxes[0])
                for index, sibling in enumerate(sandboxes[1:], start=1):
                    assert sibling.files.read(MARKER, user="root") == "branch-" + str(index)
                assert parent.commands.run("printf parent-alive", timeout=10).stdout == "parent-alive"
                report["remaining_siblings_after_kill"] = len(sandboxes) - 1
                if streams is not None:
                    streams.wait_drained()
            active_batches = [(path, batch_key, branches)]
            active_branches = list(zip(branches[1:], sandboxes[1:]))
            witnesses = [(branch, MARKER, "branch-" + str(index))
                         for index, branch in enumerate(sandboxes[1:], start=1)]
            if args.n == branch_limit and receipt["native_hot_fanout"]["max_batches"] >= 2:
                with phase(report, "reclaimed_port_reuse", client):
                    reused = report["port_reuse"] = {}
                    if receipt["native_hot_fanout"]["max_batches"] >= 3:
                        new_path, new_key, replacement = race_last_branch_slot(client, running, checkpoint, reused)
                    else:
                        new_path, new_key, replacement = start_batch(
                            client, running, checkpoint, 1, reused, await_reclaimed_port=True)
                    [new_branch] = replacement
                    # Before the close the original batch owned the full port range.
                    # With no other release, only branch zero's port can be reused.
                    assert new_branch not in branches
                    replacement_sdk = Sandbox.connect(new_branch, **owner)
                    assert replacement_sdk.files.read("/run/inherited", user="root") == receipt["expected_inherited_mark"]
                    fresh = replacement_sdk.commands.run("test ! -e " + shlex.quote(MARKER), user="root", timeout=10)
                    assert fresh.exit_code == 0
                    reused["old_branch_data_token"] = check_stale_data_route(client, sandboxes[0])
                    if streams is not None:
                        streams.finish_after_reuse(replacement_sdk)
                    active_batches.append((new_path, new_key, replacement))
                    active_branches.append((new_branch, replacement_sdk))
                    witnesses.append((replacement_sdk, "/run/inherited", receipt["expected_inherited_mark"]))
        with phase(report, "mark_original_parent", client):
            parent_marker = secrets.token_hex(16)
            parent.files.write(PARENT_MARKER, parent_marker, user="root",
                               request_timeout=min(15, client.remaining()))
            witnesses.append((parent, PARENT_MARKER, parent_marker))

        restarts = report["management_restart"] = {}
        for role in ("center", "host-0"):
            with phase(report, "restart_" + role, client), limit_public_deadline(client, 75):
                previous = expected_processes[role]
                expected_processes = restart_management_role(
                    services, client, role, expected_processes, (slot_path, context, claim))
                denied = check_live_routes_after_restart(
                    client, running, owner, active_batches, witnesses, sandboxes[0])
                restarts[role] = dict(previous_process=previous, new_process=expected_processes[role],
                                     redis_process=expected_processes["redis"],
                                     original_vm_pid=claim["pid"],
                                     original_vm_start_time_ticks=claim["start_time_ticks"],
                                     recovered_parent=True, recovered_branches=len(active_branches),
                                     released_branch_token_status=denied)
        with phase(report, "parent_destroy", client):
            client.destroy(running)
            client.wait_cleaned()
            cleaned = True
            for batch_path, key, _ in active_batches:
                check_batch_parent_ended(client, batch_path, key)
            for branch, sandbox in active_branches:
                check_sdk_not_found(branch, owner)
                check_stale_data_route(client, sandbox)
            report["parent_ended_batch"] = True
        confirm_native_release()
    finally:
        if not cleaned and client is not None and client.session is not None:
            try:
                client.deadline = time.monotonic() + 120
                if args.fault_case == "natural-expiry":
                    observe_automatic_destroy(client)
                else:
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
                assert native_id is not None, "no original occupied native binding was observed"
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
    git = ("git", "--no-optional-locks", "-c", f"safe.directory={ROOT}", "-C", ROOT)
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fault-case", choices=("inflight-close", "unload-race", "natural-expiry", "parent-vmm-loss",
                                                 PARTIAL_START),
                        help="run one explicit independent branch acceptance case")
    args = checked_args(parser)
    if args.fault_case == "parent-vmm-loss" and not (
            callable(getattr(os, "pidfd_open", None))
            and callable(getattr(signal, "pidfd_send_signal", None))):
        parser.error("parent-vmm-loss requires Linux os.pidfd_open and signal.pidfd_send_signal "
                     f"in the same interpreter as the pinned SDK (current: {sys.executable})")
    package = importlib.metadata.distribution("e2b")
    provenance = json.loads(package.read_text("direct_url.json"))
    assert package.version == "2.51.0" and provenance["vcs_info"]["commit_id"] == SDK_COMMIT
    os.umask(0o077)
    receipt = json_file(private_path(args.receipt), limit=65536)
    limits = receipt["native_hot_fanout"]
    if args.fault_case == PARTIAL_START:
        if (receipt.get("fault_case") != PARTIAL_START or args.n != 4
                or limits["template"]["max_branches"] != 4 or limits["max_batches"] < 3):
            parser.error("partial-start requires its separate fixture, --n 4 and three batch records")
    elif "fault_case" in receipt:
        parser.error("this acceptance requires an ordinary checkpoint fixture")
    assert args.n <= limits["template"]["max_branches"]
    if args.fault_case == "inflight-close" and not (
            args.n == limits["template"]["max_branches"] and limits["max_batches"] >= 2):
        parser.error("inflight-close requires the full branch capacity and one replacement batch")
    args.output.mkdir(mode=0o700)
    report = dict(candidate=args.candidate, fixture_sha256=digest(args.receipt),
                  binaries={role: digest(getattr(args, role)) for role in ("center", "host")},
                  sdk_commit=SDK_COMMIT, requested_branches=args.n, phase_seconds={}, passed=False)
    if args.fault_case is not None:
        report["fault_case"] = args.fault_case
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
