"""Static identity checks for the existing bounded KVM supervision layouts.

No deployment identity is generated or rewritten here. The operator chooses
the root and both namespaces before preparing one immutable physical profile;
Rust remains the only activator and the existing validators own all budgets.
"""
from pathlib import Path
import os
import re
import stat

from .template_build_config import MIB, validate_acceptance_build_policy
from .template_physical_storage import MAXIMUM, envelope


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


def acceptance_pool_disk_budget(policy, role, base_bytes, *, max_active=2,
                                legacy_bytes=24 * 1024 * MIB, legacy_cleanup_bytes=256 * MIB):
    """Generate fixture fields from explicit profile values or a legacy floor.

    This does not admit work or re-derive bounded capacity. Static feasibility
    and actual admission remain with validate_capacity and the controllers.
    """
    if role not in ("runtime", "builder"):
        raise ValueError("explicit acceptance pool role required")
    for value in (base_bytes, max_active, legacy_bytes, legacy_cleanup_bytes):
        if type(value) is not int or not 0 < value <= MAXIMUM:
            raise ValueError("positive representable acceptance size and count required")
    if policy is None:
        maximum = max(legacy_bytes, max_active * base_bytes + legacy_cleanup_bytes)
        cleanup = legacy_cleanup_bytes
        disk = maximum - cleanup
    else:
        profile = _acceptance_profile(policy)
        pool = next(pool for pool in profile["pools"] if pool["role"] == role)
        disk, cleanup = pool["rootfs_budget_bytes"], pool["cleanup_reserve_bytes"]
        maximum = disk + cleanup
    if not 0 < disk <= MAXIMUM or maximum > MAXIMUM:
        raise ValueError("acceptance disk budget overflow")
    return {"max_disk_bytes_global": maximum, "max_disk_bytes_per_tenant": disk,
            "cleanup_reserve_bytes": cleanup}


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
