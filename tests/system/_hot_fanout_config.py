"""Protected inputs for one prepared template on one fresh isolated host."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


from copy import deepcopy
import hashlib
import json
from pathlib import Path
import re


def descriptor_digest(binding):
    """Exact TemplateBinding serialization for this fixed Go fixture."""
    fields = ("template_id", "revision", "owner", "kernel", "rootfs", "resources", "envd_version")
    if set(binding) != set(fields):
        raise ValueError("expected fixed Go binding with no optional metadata")
    ordered = {key: binding[key] for key in fields}
    for key, names in (
        ("kernel", ("path", "sha256", "size_bytes")),
        ("rootfs", ("path", "sha256", "size_bytes")),
        ("resources", ("cpu_count", "memory_mib", "disk_bytes", "host_memory_bytes")),
    ):
        if set(binding[key]) != set(names):
            raise ValueError("unexpected fixture binding shape")
        ordered[key] = {name: binding[key][name] for name in names}
    return hashlib.sha256(json.dumps(ordered, separators=(",", ":"), ensure_ascii=False).encode()).digest()


def protected_configs(center, hosts, receipt):
    """Install exact catalog declarations before the original roles seal configuration."""
    [host] = hosts
    [staged] = center["templates"]["templates"]
    approved = receipt["binding"]
    if type(approved["owner"]) is not int or approved["owner"] != 1:
        raise ValueError("fixture's first and only allocating owner must be 1")
    for key, value in approved.items():
        if key in ("kernel", "rootfs"):
            if any(staged[key][field] != value[field] for field in ("sha256", "size_bytes")):
                raise ValueError("staged template artifact differs from the receipt")
        elif staged[key] != value:
            raise ValueError("staged template binding differs from the receipt")
    hot = deepcopy(receipt["native_hot_fanout"])
    pin = hot["template"]
    if pin["descriptor_digest"] != list(descriptor_digest(approved)):
        raise ValueError("original approved source descriptor digest differs")
    max_branches = pin["max_branches"]
    max_batches = hot["max_batches"]
    checkpoint = pin["checkpoint_id"]
    if (set(hot) != {"template", "max_batches", "max_branches_per_vm"}
            or set(pin) != {"descriptor_digest", "max_branches", "checkpoint_id"}
            or type(max_branches) is not int or not 2 <= max_branches <= 64
            or type(hot["max_branches_per_vm"]) is not int or hot["max_branches_per_vm"] != max_branches
            or type(max_batches) is not int or not 1 <= max_batches <= 1024
            or not isinstance(checkpoint, str) or re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", checkpoint) is None):
        raise ValueError("unexpected prepared checkpoint or bounds")

    revision = f'{staged["template_id"]}-{staged["revision"]}'
    if center["template_registrations"] != [{"template_id": staged["template_id"], "revision": revision}]:
        raise ValueError("expected one exact originally registered revision")
    cache_key = hashlib.sha256(revision.encode() + b"\0" + descriptor_digest(staged)).hexdigest()
    directory = Path(host["template_cache"]["root"]) / "artifacts" / cache_key
    local = deepcopy(staged)
    # A pristine host's first lifecycle owner is projected to local owner 1.
    local["kernel"]["path"] = str(directory / "kernel")
    local["rootfs"]["path"] = str(directory / "rootfs.ext4")
    pin["descriptor_digest"] = list(descriptor_digest(local))
    if "hot_fanout" in host["native"] or "branching" in center:
        raise ValueError("refusing to overwrite protected fan-out configuration")
    host["native"]["hot_fanout"] = hot
    center["branching"] = {
        "limits": {"max_batches": max_batches, "max_branches_per_vm": max_branches, "first_port": 31000},
        "template_revisions": [revision],
    }
