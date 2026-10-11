"""Static identity checks for the existing bounded KVM supervision layouts.

The operator chooses the root and both namespaces before preparing one
immutable physical profile. Fresh-root provisioning supplies its empty file;
Rust remains the only activator and the existing validators own all budgets.
"""
from contextlib import closing
from pathlib import Path
import copy
import json
import math
import os
import re
import sqlite3
import stat
import time

from .template_build_config import validate_acceptance_build_policy
from .template_build_store import TERMINAL
from .template_physical_storage import MAXIMUM, envelope


def completion_build_observation(row, *, read_build):
    """Keep an observed immutable terminal result after its query window expires.

    This supplies build evidence only. The caller must separately establish
    physical resource absence and verify the published template. Missing or
    uncertain results still require a successful read of the original build;
    a missing query anchor can never establish a terminal outcome.
    """
    job = row.get("build_id")
    if not job:
        raise ValueError("original build identity required")
    previous = row.get("build")
    saved = (isinstance(previous, dict)
             and previous.get("state") in TERMINAL)
    result = previous if saved else read_build(job)
    if (not isinstance(result, dict) or result.get("buildID") != job
            or result.get("state") not in TERMINAL):
        raise ValueError("original build terminal observation is missing or mismatched")
    return {"build": copy.deepcopy(result),
            "build_observation": "saved_terminal" if saved else "live_query"}


def wait_private_build_window(database, job, *, timeout=60, clock=time.monotonic, sleep=time.sleep):
    """Observe the original job's committed window; never create or retry work.

    Building is committed before the private window, so NULL is an observable
    intermediate state. Each observation closes its read-only connection before
    waiting, allowing the supervisor to checkpoint and persist the same window.
    """
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("positive finite private window observation timeout required")
    deadline = clock() + timeout
    uri = Path(database).as_uri() + "?mode=ro"
    while True:
        with closing(sqlite3.connect(uri, uri=True, timeout=0)) as connection:
            row = connection.execute("SELECT state,private_window FROM jobs WHERE id=?", (job,)).fetchone()
        if row is None:
            raise ValueError("original build record is missing")
        state, encoded = row
        if encoded is not None:
            try:
                window = json.loads(encoded) if type(encoded) is str else None
            except json.JSONDecodeError as error:
                raise ValueError("invalid original private request window") from error
            if type(window) is not str or re.fullmatch(r"[A-Za-z0-9_-]{1,512}", window) is None:
                raise ValueError("invalid original private request window")
            return window
        if state in TERMINAL:
            raise ValueError("original build ended before saving its private request window")
        remaining = deadline - clock()
        if remaining <= 0:
            raise TimeoutError("original private request window was not saved before the observation deadline")
        sleep(min(0.1, remaining))


def require_bounded_host_tools():
    """Read-only prerequisite; do not reach host initialization without it."""
    from .check_kvm_host import trusted_path
    try:
        path = trusted_path(Path('/usr/sbin/conntrack'))
        if not stat.S_ISREG(path.stat().st_mode) or not os.access(path, os.X_OK):
            raise ValueError('bounded host requires executable /usr/sbin/conntrack')
    except OSError as error:
        raise ValueError('bounded host requires trusted /usr/sbin/conntrack') from error
    return str(path)


def _acceptance_profile(policy):
    validate_acceptance_build_policy(policy)
    profiles = []
    for role in ("runtime", "builder"):
        section = policy[role]
        host, maintenance = section["bounded_host"], section["maintenance"]
        if (type(host) is not dict or type(maintenance) is not dict
                or type(maintenance.get("capacity")) is not dict or not maintenance["capacity"]
                or type(host.get("physical_storage")) is not dict):
            raise ValueError("complete bounded capacity and physical profile required: " + role)
        profiles.append(host["physical_storage"])
    if profiles[0] != profiles[1]:
        raise ValueError("identical immutable acceptance physical profiles required")
    profile = profiles[0]
    envelope(profile)  # Reuse the authoritative profile schema and envelope.
    if len(profile["pools"]) != 2 or profile["public_build"] is None:
        raise ValueError("bounded acceptance requires both pools and public workspace")
    return profile


def acceptance_pool_disk_budget(policy, role, base_bytes, *, max_active=2):
    """Generate fixture fields only from the explicit complete physical profile.

    This does not admit work or re-derive bounded capacity. Static feasibility
    and actual admission remain with validate_capacity and the controllers.
    """
    if role not in ("runtime", "builder"):
        raise ValueError("explicit acceptance pool role required")
    for value in (base_bytes, max_active):
        if type(value) is not int or not 0 < value <= MAXIMUM:
            raise ValueError("positive representable acceptance size and count required")
    profile = _acceptance_profile(policy)
    pool = next(pool for pool in profile["pools"] if pool["role"] == role)
    disk, cleanup = pool["rootfs_budget_bytes"], pool["cleanup_reserve_bytes"]
    maximum = disk + cleanup
    if not 0 < disk <= MAXIMUM or maximum > MAXIMUM:
        raise ValueError("acceptance disk budget overflow")
    return {"max_disk_bytes_global": maximum, "max_disk_bytes_per_tenant": disk,
            "cleanup_reserve_bytes": cleanup}


def acceptance_disk_budgets(deployment, policy):
    """Apply explicit physical quotas without reducing the task-derived floor.

    The shared profile may need a larger cleanup reserve for node metadata and
    the controller journal. Charge each increase once in host free-space
    preflight; task shapes and the physical activation checks remain unchanged.
    """
    _acceptance_profile(policy)
    result = copy.deepcopy(deployment)
    extra = 0
    for role in ("runtime", "builder"):
        original = deployment[role]
        requested = acceptance_pool_disk_budget(policy, role, original["max_disk_bytes_per_tenant"])
        if any(requested[field] < original[field] for field in requested):
            raise ValueError("physical pool cannot undercut the acceptance disk budget: " + role)
        extra += requested["max_disk_bytes_global"] - original["max_disk_bytes_global"]
        result[role].update(requested)
    required = result["host"]["minimum_free_disk_bytes"] + extra
    if required > MAXIMUM:
        raise ValueError("acceptance disk headroom overflow")
    result["host"]["minimum_free_disk_bytes"] = required
    result["host"]["runtime_disk_bytes"] = result["runtime"]["max_disk_bytes_per_tenant"]
    return result


def prepared_ci_physical_profile(policy, *, kernel_bytes, runtime_bytes, disk_budget_bytes):
    """Narrow the approved deployment to its static-template runtime pool.

    Prepared CI never imports/publishes templates or dispatches build VMs.
    Keep runtime/cleanup/metadata quotas and the original filesystem overhead;
    bound kernel/runtime copies by the pinned artifacts actually registered.
    Rust still activates the same profile with twice its complete envelope.
    """
    profile = copy.deepcopy(_acceptance_profile(policy))
    if type(disk_budget_bytes) is not int or not 0 < disk_budget_bytes <= 200 * 1024**3:
        raise ValueError("explicit prepared CI disk ceiling at most 200 GiB required")
    pool = next(pool for pool in profile["pools"] if pool["role"] == "runtime")
    for field, actual in (("kernel_copy_bytes", kernel_bytes), ("runtime_copy_bytes", runtime_bytes)):
        if type(actual) is not int or not 0 < actual <= pool[field]:
            raise ValueError("prepared CI artifact exceeds approved physical copy ceiling")
        pool[field] = actual
    profile.update(pools=[pool], public_build=None)
    peak, _ = envelope(profile)
    if 2 * peak > disk_budget_bytes:
        raise ValueError("prepared CI complete physical activation exceeds disk ceiling")
    return profile


def prepared_ci_runtime_configuration(settings, policy, profile):
    """Bind only prevalidated static CI descriptors; expose no build registry."""
    result = copy.deepcopy(settings)
    catalog = result["templates"]["templates"]
    if not catalog or any(not item["template_id"].startswith("ci-") for item in catalog):
        raise ValueError("prepared CI requires only approved static CI templates")
    base = catalog[0]
    pool = profile["pools"][0]
    if (len(profile["pools"]) != 1 or pool["role"] != "runtime" or profile["public_build"] is not None
            or any(pool[field] != result[field] for field in ("namespace", "database", "node_state_dir", "chroot_base"))):
        raise ValueError("prepared CI runtime profile differs from explicit configuration")
    for item in catalog:
        if (item["rootfs"]["size_bytes"] > pool["rootfs_budget_bytes"]
                or item["kernel"]["size_bytes"] > pool["kernel_copy_bytes"]):
            raise ValueError("prepared CI template exceeds runtime physical quota")
    result.pop("template_registry", None)
    result.update(rootfs=base["rootfs"]["path"], kernel=base["kernel"]["path"],
                  vcpu_count=base["resources"]["cpu_count"], memory_mib=base["resources"]["memory_mib"],
                  bounded_host=copy.deepcopy(policy["runtime"]["bounded_host"]),
                  maintenance=copy.deepcopy(policy["runtime"]["maintenance"]),
                  max_disk_bytes_global=pool["rootfs_budget_bytes"] + pool["cleanup_reserve_bytes"],
                  max_disk_bytes_per_tenant=pool["rootfs_budget_bytes"],
                  cleanup_reserve_bytes=pool["cleanup_reserve_bytes"])
    result["bounded_host"]["physical_storage"] = copy.deepcopy(profile)
    result["templates"]["aliases"] = {"base": base["template_id"]}
    return result


def bounded_acceptance_layout(policy, *, run_root, namespace, build_namespace):
    """Return the explicit layout only when the unchanged profile matches it.

    This performs no filesystem writes, lease acquisition or activation. The
    caller separately creates a fresh private root, writes configs and starts
    Rust before the Python service's default physical-lease validation.
    """
    if run_root is None or namespace is None or build_namespace is None:
        raise ValueError("bounded acceptance requires explicit run root and both namespaces")
    spelling = str(run_root)
    root = Path(spelling)
    if (not root.is_absolute() or root == Path("/") or "\0" in spelling
            or str(root) != spelling or any(part in ("", ".", "..") for part in spelling.split("/")[1:])
            or root.resolve(strict=False) != root):
        raise ValueError("absolute canonical non-symlink acceptance root required")
    namespaces = [namespace, build_namespace]
    if (any(type(value) is not str or re.fullmatch(r"[0-9a-f]{16}", value) is None for value in namespaces)
            or namespace[:6] == build_namespace[:6]):
        raise ValueError("distinct explicit physical pool namespaces required")
    profile = _acceptance_profile(policy)
    for index, role in enumerate(("runtime", "builder")):
        matches = [pool for pool in profile["pools"] if pool["role"] == role]
        expected = {"namespace": namespaces[index], "database": str(root / str(index) / "db/controller.db"),
                    "node_state_dir": str(root / str(index) / "state"), "chroot_base": str(root / str(index) / "jails")}
        if len(matches) != 1 or any(matches[0][field] != value for field, value in expected.items()):
            raise ValueError("fixed physical profile differs from explicit acceptance layout: " + role)
    public = profile["public_build"]
    if public["root"] != str(root / "builds") or public["registry_root"] != str(root / "registry"):
        raise ValueError("fixed public physical profile differs from explicit acceptance layout")
    return root, namespaces


def validate_acceptance_socket_paths(root, namespaces, firecracker):
    """Reject an impossible layout before starting either native controller.

    Bounded host IDs are monotonic u64 values, independent of the reusable UID
    slots. Match LinuxWorker's longest-path check, including encoded bytes.
    """
    identifier = 2**64 - 1
    for index, namespace in enumerate(namespaces):
        path = (Path(root) / str(index) / "jails" / Path(firecracker).name
                / f"vm-{namespace}-{identifier}" / "root/api.socket")
        if len(os.fsencode(path)) >= 108:
            raise ValueError("acceptance path too long for Firecracker socket")


def provision_acceptance_coordination(policy, *, run_root, namespace, build_namespace):
    """Supply the operator-selected empty file only in a fresh private root.

    Call immediately after exclusive root creation, before configs or state.
    Never adopt an existing file, recreate a lost identity in an old deployment,
    or write an activation marker. Only Rust activates the physical profile.
    """
    root, _ = bounded_acceptance_layout(policy, run_root=run_root, namespace=namespace,
                                        build_namespace=build_namespace)
    path = Path(_acceptance_profile(policy)["coordination_file"])
    if path.parent != root:
        raise ValueError("acceptance coordination file must be directly inside the fresh root")
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        info = os.fstat(directory)
        if info.st_uid != os.geteuid() or info.st_mode & 0o077 or os.listdir(directory):
            raise ValueError("empty private acceptance root required for coordination provisioning")
        fd = os.open(path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                     0o600, dir_fd=directory)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
        os.fsync(directory)
    finally:
        os.close(directory)
