"""Shared protected configuration and cross-pool feasibility checks.

Reject a build service which advertises requests its configured transports or
empty runtime pool cannot accept. This is not a second controller admission
policy. Live usage, exact reclaim and physical free
space remain the existing controllers' and worker's responsibility.
"""
import json
import os
from pathlib import Path

MIB = 1024 * 1024
UPLOAD_ENVELOPE_BYTES = 64 * 1024
HOST_MEMORY_OVERHEAD_MIB = 256



def protected_json(path, maximum=65536):
    path = Path(path)
    if path.resolve(strict=True) != path or not path.is_file():
        raise ValueError("protected regular config required")
    meta = path.stat()
    if meta.st_uid != os.geteuid() or meta.st_mode & 0o077 or meta.st_size > maximum:
        raise ValueError("private owned bounded config required")
    return json.loads(path.read_text())


def bounded_data_route_limits(settings):
    """Resolve the persisted route-cell limits from explicit gateway limits."""
    transfer = settings.get("data_transfer")
    budgets = {"file_upload_bytes", "file_download_bytes", "command_input_bytes",
               "command_output_bytes", "rpc_request_bytes", "rpc_response_bytes"}
    concurrency = {"max_transfers", "max_transfers_per_vm", "control_reserve"}
    timeouts = {"request_idle_seconds", "file_idle_seconds", "write_idle_seconds"}
    if type(transfer) is not dict or set(transfer) != budgets | concurrency | timeouts:
        raise ValueError("bounded host requires explicit complete data_transfer limits")
    if any(type(value) is not int or not 0 < value <= 2**64 - 1 for value in transfer.values()):
        raise ValueError("invalid bounded data_transfer limits")
    if (transfer["rpc_request_bytes"] > 8 * 1024 * 1024
            or not transfer["control_reserve"] < transfer["max_transfers_per_vm"] <= transfer["max_transfers"]
            or any(transfer[name] > 2**63 - 1 for name in timeouts)):
        raise ValueError("invalid bounded data_transfer budgets or concurrency")
    # The reserve is included in the cap; ordinary traffic still obeys the
    # existing global/per-VM/control-reserve gateway admission checks.
    return {'max_data_routes_per_slot': transfer['max_transfers_per_vm'],
            'max_data_routes_global': transfer['max_transfers'],
            'data_route_control_reserve': transfer['control_reserve']}


def host_uid_range(settings):
    """Check the configured UID span, without asserting network disjointness.

    The sole host mapping reserves one UID per reusable physical position.
    The retired max_vm_id configuration is not a valid deployment input.
    """
    def integer(value, maximum, name, minimum=1):
        if type(value) is not int or not minimum <= value <= maximum:
            raise ValueError("invalid " + name)
        return value

    uid_base = integer(settings.get("uid_base"), 2**32 - 2, "UID base")
    if "max_vm_id" in settings:
        raise ValueError("retired max_vm_id configuration is unsupported")
    bounded = settings.get("bounded_host")
    if type(bounded) is not dict or set(bounded) - {"console_policy", "physical_storage"} != {"maintenance", "uid_base", "uplink_base"}:
        raise ValueError("invalid bounded host config")
    if "console_policy" in bounded and bounded["console_policy"] != "discard":
        raise ValueError("invalid bounded console policy")
    if settings.get("maintenance", {}).get("capacity") and bounded.get("console_policy") != "discard":
        raise ValueError("full bounded capacity requires explicit discarded console output")
    bounded_data_route_limits(settings)
    if integer(bounded["uid_base"], 2**32 - 2, "bounded UID base") != uid_base:
        raise ValueError("bounded host UID base differs from node UID base")
    maintenance = bounded["maintenance"]
    if (type(maintenance) is not dict
            or set(maintenance) != {"max_retained_results", "max_host_slots", "windows"}):
        raise ValueError("invalid bounded maintenance limits")
    integer(maintenance["max_retained_results"], 2**64 - 1, "retained result limit")
    count = integer(maintenance["max_host_slots"], 1_048_576, "host slot limit")
    uplink_base = bounded["uplink_base"]
    if type(uplink_base) is not int or not 0 <= uplink_base < 1_048_576:
        raise ValueError("invalid bounded uplink base")
    if uplink_base + count > 1_048_576:
        raise ValueError("bounded uplink range exceeds address pool")
    windows = maintenance["windows"]
    if type(windows) is not dict or set(windows) != {"max_open_windows", "max_duration_ms"}:
        raise ValueError("invalid bounded window limits")
    for name, value in windows.items():
        integer(value, 2**64 - 1, "window " + name)
    # Linux's all-ones UID is reserved, and a range must never wrap.
    if uid_base + count > 2**32 - 1:
        raise ValueError("host UID range exceeds usable UIDs")
    return range(uid_base, uid_base + count)


def validate_pool_resource_ranges(runtime, builder):
    ranges = [host_uid_range(pool) for pool in (runtime, builder)]
    if max(value.start for value in ranges) < min(value.stop for value in ranges):
        raise ValueError("runtime/build UID pools overlap")
    uplinks = []
    for pool in (runtime, builder):
        enabled = pool.get("public_ipv4_enabled", False)
        if type(enabled) is not bool:
            raise ValueError("invalid public IPv4 enable flag")
        # Bounded deployments reserve their address ranges even while new
        # IPv4 admission is disabled: that flag does not prove older accepted
        # public sandboxes have drained.
        bounded = pool.get("bounded_host")
        start = bounded["uplink_base"]
        count = bounded["maintenance"]["max_host_slots"]
        uplinks.append(range(start, start + count))
    if len(uplinks) == 2 and max(value.start for value in uplinks) < min(value.stop for value in uplinks):
        raise ValueError("runtime/build public IPv4 uplink pools overlap")



def positive(settings, field):
    value = settings.get(field)
    if type(value) is not int or value <= 0:
        raise ValueError("positive configured budget required: " + field)
    return value


def lifetime(settings, required, name):
    maximum = positive(settings, "max_timeout_secs")
    total = settings.get("max_lifetime_secs")
    if total is None:  # serde Option, including an explicitly null field
        total = maximum
    if type(total) is not int or total <= 0 or min(maximum, total) < required:
        raise ValueError(name + " single and total lifetime must cover the request")


def pool_shape(settings, cpu, memory, disk, name):
    compute = settings["templates"]["compute_limits"]
    for field in ("max_vcpus_global", "max_vcpus_per_tenant"):
        if positive(compute, field) < cpu:
            raise ValueError(name + " CPU budget cannot admit the advertised shape")
    for field in ("max_memory_bytes_global", "max_memory_bytes_per_tenant"):
        if positive(compute, field) < memory:
            raise ValueError(name + " VMM memory budget cannot admit the advertised shape")
    if (positive(settings, "max_disk_bytes_global") < disk + positive(settings, "cleanup_reserve_bytes")
            or positive(settings, "max_disk_bytes_per_tenant") < disk):
        raise ValueError(name + " disk budget cannot admit the advertised shape and cleanup reserve")
    positive(settings, "max_active_global")
    positive(settings, "max_active_per_tenant")


def validate_capacity(runtime, builder, limits, base, *, kernel_bytes, largest_tool_bytes):
    """Checks static necessary conditions; returns the actual bound builder disk."""
    lifetime(builder, limits["timeout_seconds"] + 60, "builder")
    registry = runtime["template_registry"]
    lifetime(runtime, positive(registry, "validation_timeout_secs"), "publication")
    shape = base["resources"]
    disk = positive(shape, "disk_bytes")
    if base["rootfs"]["size_bytes"] != disk:
        raise ValueError("builder base disk descriptor mismatch")
    pool_shape(builder, positive(shape, "cpu_count"), positive(shape, "host_memory_bytes"), disk, "builder")
    target = limits["max_disk_mib"] * MIB
    # Same necessary-space formula as the OCI-reference worker preflight.
    # A filesystem's actual free blocks are still checked inside the guest.
    if disk < target + limits["max_expanded_bytes"] + 2 * limits["max_input_bytes"] + 64 * MIB:
        raise ValueError("builder disk cannot contain target, expanded input and download staging")
    transfer = builder.get("data_transfer")
    if not isinstance(transfer, dict):
        raise ValueError("explicit builder streaming transfer budgets required")
    if positive(transfer, "file_upload_bytes") < max(limits["max_input_bytes"], largest_tool_bytes) + UPLOAD_ENVELOPE_BYTES:
        raise ValueError("builder upload budget must cover input/tools and HTTP envelope")
    if positive(transfer, "file_download_bytes") < max(target, limits["max_log_bytes"], 128 * 1024):
        raise ValueError("builder download budget must cover a complete candidate and logs")
    pool_shape(runtime, limits["max_cpu"], (limits["max_memory_mib"] + HOST_MEMORY_OVERHEAD_MIB) * MIB, target, "runtime")
    for field in ("max_artifact_bytes_global", "max_artifact_bytes_per_tenant"):
        if positive(registry, field) < target + kernel_bytes:
            raise ValueError("publication artifact budget cannot contain one maximum candidate")
    return disk


def acceptance_build_policy(path):
    """The sole formal acceptance policy; fixture budgets are not defaults."""
    if path is None:
        raise ValueError("explicit complete bounded policy required")
    value = protected_json(path)
    return validate_acceptance_build_policy(value)


def validate_acceptance_build_policy(value):
    if type(value) is not dict or set(value) != {"runtime", "builder", "public"}:
        raise ValueError("invalid bounded acceptance policy")
    for pool in ("runtime", "builder"):
        fields = {"bounded_host", "maintenance"} | ({"build_retry_windows"} if pool == "builder" else set())
        if type(value[pool]) is not dict or set(value[pool]) != fields:
            raise ValueError("explicit bounded pool policy required: " + pool)
    public = value["public"]
    if type(public) is not dict or set(public) != {"bounded_builds", "workspace_bytes", "workspace_bytes_per_tenant"}:
        raise ValueError("explicit public bounded policy required")
    if __package__:
        from .template_build_store import bounded_configuration
    else:
        from template_build_store import bounded_configuration
    bounded_configuration(public["bounded_builds"])
    for key in ("workspace_bytes", "workspace_bytes_per_tenant"):
        positive(public, key)
    if public["workspace_bytes_per_tenant"] > public["workspace_bytes"]:
        raise ValueError("public tenant workspace exceeds global policy")
    return value


def apply_acceptance_build_policy(runtime, builder, build, policy):
    """Pass through only authorized policy sections; keep pool identities fixed."""
    for name, settings in (("runtime", runtime), ("builder", builder)):
        if "max_vm_id" in settings:
            raise ValueError("retired max_vm_id configuration is not supported: " + name)
    validate_acceptance_build_policy(policy)
    import copy
    if __package__:
        from .template_build_store import bounded_configuration
    else:
        from template_build_store import bounded_configuration
    runtime.update(copy.deepcopy(policy["runtime"]))
    builder.update(copy.deepcopy(policy["builder"]))
    bounded_configuration(policy["public"]["bounded_builds"])
    build["bounded_builds"] = copy.deepcopy(policy["public"]["bounded_builds"])
    for key in ("workspace_bytes", "workspace_bytes_per_tenant"):
        if policy["public"][key] < build["limits"][key]:
            raise ValueError("bounded acceptance workspace cannot undercut original task budget")
        build["limits"][key] = policy["public"][key]
    validate_pool_resource_ranges(runtime, builder)
    if __package__:
        from .template_physical_storage import validate_configuration as validate_physical_configuration
    else:
        from template_physical_storage import validate_configuration as validate_physical_configuration
    validate_physical_configuration(runtime, builder, build)
    return "windowed"
