"""Cross-pool feasibility checks, not a second controller admission policy.

Reject a build service which advertises requests its configured transports or
empty runtime pool cannot accept. Live usage, exact reclaim and physical free
space remain the existing controllers' and worker's responsibility.
"""
MIB = 1024 * 1024
UPLOAD_ENVELOPE_BYTES = 64 * 1024
HOST_MEMORY_OVERHEAD_MIB = 256


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


def acceptance_build_policy(path, *, legacy_unbounded=False):
    """Explicit acceptance mode; these values are never production defaults."""
    if (path is None) == (not legacy_unbounded):
        raise ValueError("choose an explicit bounded policy or --legacy-unbounded")
    if path is None:
        return None
    from .template_build_service import protected_json
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
    from .template_build_store import bounded_configuration
    bounded_configuration(public["bounded_builds"])
    for key in ("workspace_bytes", "workspace_bytes_per_tenant"):
        positive(public, key)
    if public["workspace_bytes_per_tenant"] > public["workspace_bytes"]:
        raise ValueError("public tenant workspace exceeds global policy")
    return value


def apply_acceptance_build_policy(runtime, builder, build, policy):
    """Pass through only authorized policy sections; keep pool identities fixed."""
    if policy is None:
        return "legacy-unbounded"
    validate_acceptance_build_policy(policy)
    import copy
    from .template_build_service import validate_pool_resource_ranges
    from .template_build_store import bounded_configuration
    runtime.update(copy.deepcopy(policy["runtime"]))
    builder.update(copy.deepcopy(policy["builder"]))
    bounded_configuration(policy["public"]["bounded_builds"])
    build["bounded_builds"] = copy.deepcopy(policy["public"]["bounded_builds"])
    for key in ("workspace_bytes", "workspace_bytes_per_tenant"):
        if policy["public"][key] < build["limits"][key]:
            raise ValueError("bounded acceptance workspace cannot undercut original task budget")
        build["limits"][key] = policy["public"][key]
    validate_pool_resource_ranges(runtime, builder)
    from .template_physical_storage import validate_configuration as validate_physical_configuration
    validate_physical_configuration(runtime, builder, build)
    return "windowed"
