"""Entry point for a protected template's cooperative preparation process."""

import os
from pathlib import Path

from .control import identifier
from .linux import LinuxGuest
from .service import serve_from_checkpoint

WORK = Path("/run/sandproof-hot-fanout")
CGROUP = Path("/sys/fs/cgroup/sandproof-hot-fanout")
PORTS = range(31000, 31100)
MOUNTINFO_LIMIT = 1024 * 1024


def validate_frozen_mounts(lowers, mountinfo):
    """Require whole read-only filesystems with no visible writable alias."""
    mounts = []
    for row in mountinfo.splitlines():
        before, after = row.split(" - ", 1)
        _, _, device, root, target, options, *_ = before.split()
        _, _, super_options = after.split()
        mounts.append((target, device, root, "ro" in options.split(","),
                       "ro" in super_options.split(",")))
    for lower in lowers:
        matches = [mount for mount in mounts if mount[0] == os.fspath(lower)]
        if len(matches) != 1:
            raise ValueError("every frozen layer must be an exact mountpoint")
        _, device, root, readable_only, super_readonly = matches[0]
        if (root != "/" or not readable_only or not super_readonly
                or any(other_dev == device and not other_ro
                       for _, other_dev, _, other_ro, _ in mounts)):
            raise ValueError("frozen layer must have a read-only superblock and no writable alias")


def serve_prepared_checkpoint(
    checkpoint_id,
    frozen_lowers,
    envd,
    *,
    max_branches,
    max_batches,
    workload,
    default_user="ubuntu",
    default_workdir="/home/ubuntu",
):
    """Called from the administrator-installed template at its first safe point.

    The template prepares the workload's memory in this process and supplies
    its sealed filesystem mounts. It leaves no other threads, children or
    inherited open descriptors at the boundary. This starts one donor in the
    current VM and binds control to its host-only vsock.
    """
    identifier(checkpoint_id)
    if (not callable(workload)
            or type(max_branches) is not int or not 1 <= max_branches <= 64
            or type(max_batches) is not int or not 1 <= max_batches <= 4096):
        raise ValueError("invalid prepared checkpoint configuration")
    lowers = tuple(Path(path) for path in frozen_lowers)
    if not lowers or any(not path.is_absolute() or path.resolve() != path for path in lowers):
        raise ValueError("frozen layer paths must be absolute and canonical")
    with Path("/proc/self/mountinfo").open("rb") as mountinfo:
        raw = mountinfo.read(MOUNTINFO_LIMIT + 1)
    if len(raw) > MOUNTINFO_LIMIT:
        raise ValueError("prepared checkpoint mount inventory exceeds its bound")
    validate_frozen_mounts(lowers, raw.decode())
    # Exclusive to the initial process. Restart can observe the existing
    # journal, but cannot create another donor with this checkpoint identity.
    WORK.mkdir(mode=0o700)
    CGROUP.mkdir()
    backend = LinuxGuest(WORK / "branches", CGROUP, lowers, envd,
                         default_user=default_user, default_workdir=default_workdir)
    serve_from_checkpoint(backend, state=WORK / "journal", checkpoint_id=checkpoint_id,
                          max_branches=max_branches, max_batches=max_batches,
                          ports=PORTS, workload=workload)
